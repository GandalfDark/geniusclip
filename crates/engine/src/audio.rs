//! Audio: WASAPI capture (system loopback + microphone), resampling to
//! 48 kHz stereo, timestamp-driven mixing and AAC encoding.
//!
//! Each capture thread writes samples into a [`SourceRing`] at positions
//! derived from WASAPI's QPC timestamps. The mixer pulls fixed 1024-sample
//! blocks a little behind real time, so late/missing data becomes silence and
//! clock drift between devices never accumulates into A/V desync.

use crate::clock;
use crate::config::EngineConfig;
use crate::ffutil::*;
use anyhow::{bail, Context, Result};
use ffmpeg_sys_next as ff;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

pub const RATE: i64 = 48_000;
pub(crate) const BLOCK: usize = 1024;
/// How far behind real time the mixer runs (gives capture threads slack).
const MIX_LATENCY: i64 = RATE / 8;
/// Timestamp error tolerated before inserting silence / dropping samples.
const RESYNC_TOLERANCE: i64 = RATE / 40;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

struct ComInit;
impl ComInit {
    fn new() -> Self {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        ComInit
    }
}
impl Drop for ComInit {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

unsafe fn pwstr_take(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator") }
}

/// Lists active output (`capture == false`) or input devices.
pub fn list_devices(capture: bool) -> Result<Vec<AudioDevice>> {
    let _com = ComInit::new();
    let flow = if capture { eCapture } else { eRender };
    let mut out = Vec::new();
    unsafe {
        let en = enumerator()?;
        let default_id = en.GetDefaultAudioEndpoint(flow, eConsole).ok().and_then(|d| d.GetId().ok()).map(|p| pwstr_take(p));
        let coll = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..coll.GetCount()? {
            let dev = coll.Item(i)?;
            let id = pwstr_take(dev.GetId()?);
            let name = dev
                .OpenPropertyStore(STGM_READ)
                .and_then(|ps| ps.GetValue(&PKEY_Device_FriendlyName))
                .map(|v| v.to_string())
                .unwrap_or_else(|_| id.clone());
            out.push(AudioDevice { is_default: default_id.as_deref() == Some(id.as_str()), id, name });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Ring buffer on the shared 48 kHz timeline (sample index 0 = engine start).

#[derive(Default)]
struct RingInner {
    /// Interleaved stereo samples; data[0] is sample index `start`.
    data: VecDeque<f32>,
    start: i64,
    /// True once the writer has produced anything (before that reads are silent).
    primed: bool,
}

#[derive(Default)]
pub struct SourceRing {
    inner: Mutex<RingInner>,
}

impl SourceRing {
    /// Writes interleaved stereo samples whose first sample belongs at `idx`.
    fn write(&self, idx: i64, mut samples: &[f32]) {
        let mut r = self.inner.lock();
        if !r.primed {
            r.primed = true;
            r.start = r.start.max(idx);
            if idx < r.start {
                let skip = ((r.start - idx) as usize * 2).min(samples.len());
                samples = &samples[skip..];
            }
        }
        let end = r.start + (r.data.len() / 2) as i64;
        if idx > end + RESYNC_TOLERANCE {
            // Gap (device paused, dropped packets): pad with silence.
            let gap = ((idx - end) as usize).min(RATE as usize * 2);
            r.data.extend(std::iter::repeat(0.0).take(gap * 2));
        } else if idx < end - RESYNC_TOLERANCE {
            // Data arrived too late or device clock runs fast: drop the overlap.
            let skip = ((end - idx) as usize * 2).min(samples.len());
            samples = &samples[skip..];
        }
        r.data.extend(samples.iter().copied());
        // Never hold more than ~2 s (mixer should be consuming).
        let max = RATE as usize * 2 * 2;
        if r.data.len() > max {
            let drop = r.data.len() - max;
            r.data.drain(..drop);
            r.start += (drop / 2) as i64;
        }
    }

    /// Reads `out.len()/2` stereo samples starting at `idx` (silence where missing)
    /// and discards everything before the end of the read window.
    fn read(&self, idx: i64, out: &mut [f32]) {
        out.fill(0.0);
        let n = (out.len() / 2) as i64;
        let mut r = self.inner.lock();
        let (s, e) = (r.start, r.start + (r.data.len() / 2) as i64);
        let from = idx.max(s);
        let to = (idx + n).min(e);
        if from < to {
            let src = ((from - s) * 2) as usize;
            let dst = ((from - idx) * 2) as usize;
            let len = ((to - from) * 2) as usize;
            for k in 0..len {
                out[dst + k] = r.data[src + k];
            }
        }
        let consume_to = idx + n;
        if consume_to > s {
            let drop = (((consume_to - s) * 2) as usize).min(r.data.len());
            r.data.drain(..drop);
            r.start = consume_to;
        }
    }
}

// ---------------------------------------------------------------------------
// Resampler

struct Resampler {
    swr: *mut ff::SwrContext,
    in_fmt: ff::AVSampleFormat,
    in_rate: i64,
    out: Vec<f32>,
}

impl Resampler {
    fn new(in_fmt: ff::AVSampleFormat, channels: usize, rate: u32) -> Result<Self> {
        unsafe {
            let mut out_layout: ff::AVChannelLayout = std::mem::zeroed();
            ff::av_channel_layout_default(&mut out_layout, 2);
            let mut in_layout: ff::AVChannelLayout = std::mem::zeroed();
            ff::av_channel_layout_default(&mut in_layout, channels as c_int);
            let mut swr = ptr::null_mut();
            check(
                ff::swr_alloc_set_opts2(
                    &mut swr,
                    &out_layout,
                    ff::AVSampleFormat::AV_SAMPLE_FMT_FLT,
                    RATE as c_int,
                    &in_layout,
                    in_fmt,
                    rate as c_int,
                    0,
                    ptr::null_mut(),
                ),
                "swr_alloc_set_opts2",
            )?;
            ff::av_channel_layout_uninit(&mut in_layout);
            ff::av_channel_layout_uninit(&mut out_layout);
            if let Err(e) = check(ff::swr_init(swr), "swr_init") {
                ff::swr_free(&mut swr);
                return Err(e);
            }
            Ok(Resampler { swr, in_fmt, in_rate: rate as i64, out: Vec::new() })
        }
    }

    /// Returns (output samples interleaved stereo, resampler delay in output samples before this call).
    fn convert(&mut self, data: *const u8, frames: usize) -> Result<(&[f32], i64)> {
        unsafe {
            let delay = ff::swr_get_delay(self.swr, RATE);
            let cap = (frames as i64 * RATE / self.in_rate + 64 + delay) as usize;
            self.out.resize(cap * 2, 0.0);
            let out_ptr = self.out.as_mut_ptr() as *mut u8;
            let in_ptr = data;
            let n = ff::swr_convert(self.swr, &out_ptr, cap as c_int, &in_ptr, frames as c_int);
            check(n, "swr_convert")?;
            Ok((&self.out[..n as usize * 2], delay))
        }
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        unsafe { ff::swr_free(&mut self.swr) };
    }
}

unsafe impl Send for Resampler {}

// ---------------------------------------------------------------------------
// WASAPI capture thread

#[derive(Clone, Copy, PartialEq)]
enum SourceKind {
    Loopback,
    Mic,
}

fn sample_format(wf: &WAVEFORMATEX) -> Option<(ff::AVSampleFormat, bool)> {
    let tag = wf.wFormatTag as u32;
    let bits = wf.wBitsPerSample;
    let (is_float, is_pcm) = if tag == WAVE_FORMAT_EXTENSIBLE {
        let ext = unsafe { &*(wf as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
        let sub = ext.SubFormat;
        (sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, sub == KSDATAFORMAT_SUBTYPE_PCM)
    } else {
        (tag == WAVE_FORMAT_IEEE_FLOAT, tag == 1)
    };
    match (is_float, is_pcm, bits) {
        (true, _, 32) => Some((ff::AVSampleFormat::AV_SAMPLE_FMT_FLT, false)),
        (_, true, 16) => Some((ff::AVSampleFormat::AV_SAMPLE_FMT_S16, false)),
        (_, true, 32) => Some((ff::AVSampleFormat::AV_SAMPLE_FMT_S32, false)),
        // Packed 24-bit is widened to S32 by hand.
        (_, true, 24) => Some((ff::AVSampleFormat::AV_SAMPLE_FMT_S32, true)),
        _ => None,
    }
}

struct CaptureSession {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: HANDLE,
    resampler: Resampler,
    channels: usize,
    widen24: bool,
    device_id: String,
    /// Keeps a loopback device producing packets while nothing is playing.
    silence: Option<(IAudioClient, IAudioRenderClient, u32)>,
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            if let Some((c, _, _)) = &self.silence {
                let _ = c.Stop();
            }
            let _ = CloseHandle(self.event);
        }
    }
}

unsafe fn open_session(kind: SourceKind, device_id: Option<&str>) -> Result<CaptureSession> {
    let en = enumerator()?;
    let flow = if kind == SourceKind::Loopback { eRender } else { eCapture };
    let device = match device_id {
        Some(id) => en.GetDevice(&HSTRING::from(id)).context("GetDevice")?,
        None => en.GetDefaultAudioEndpoint(flow, eConsole).context("no default audio device")?,
    };
    let id = pwstr_take(device.GetId()?);
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let fmt = client.GetMixFormat()?;
    let wf = *fmt;
    let parsed = sample_format(&*fmt);
    let res = (|| -> Result<CaptureSession> {
        let (in_fmt, widen24) = parsed.context("unsupported mix format")?;
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | if kind == SourceKind::Loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, fmt, None).context("IAudioClient::Initialize")?;
        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;

        let silence = if kind == SourceKind::Loopback {
            (|| -> Result<_> {
                let rc: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                rc.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 2_000_000, 0, fmt, None)?;
                let frames = rc.GetBufferSize()?;
                let render: IAudioRenderClient = rc.GetService()?;
                let _buf = render.GetBuffer(frames)?;
                render.ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
                rc.Start()?;
                Ok((rc, render, frames))
            })()
            .map_err(|e| log::warn!("silence keeper unavailable: {e:#}"))
            .ok()
        } else {
            None
        };

        client.Start()?;
        Ok(CaptureSession {
            client: client.clone(),
            capture,
            event,
            resampler: Resampler::new(in_fmt, wf.nChannels as usize, wf.nSamplesPerSec)?,
            channels: wf.nChannels as usize,
            widen24,
            device_id: id.clone(),
            silence,
        })
    })();
    CoTaskMemFree(Some(fmt as *const _));
    res
}

fn capture_thread(kind: SourceKind, device_id: Option<String>, ring: Arc<SourceRing>, t0_us: i64, stop: Arc<AtomicBool>) {
    let _com = ComInit::new();
    let label = if kind == SourceKind::Loopback { "system audio" } else { "microphone" };
    let mut scratch: Vec<u8> = Vec::new();
    let mut zeros: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let mut sess = match unsafe { open_session(kind, device_id.as_deref()) } {
            Ok(s) => {
                log::info!("{label}: capturing from {}", s.device_id);
                s
            }
            Err(e) => {
                log::warn!("{label}: {e:#}");
                sleep_unless(&stop, Duration::from_secs(2));
                continue;
            }
        };
        let en = enumerator().ok();
        let mut next_default_check = Instant::now() + Duration::from_secs(2);

        'run: while !stop.load(Ordering::Relaxed) {
            unsafe {
                let _ = WaitForSingleObject(sess.event, 10) == WAIT_OBJECT_0;
                // Keep the silence stream topped up.
                if let Some((rc, render, frames)) = &sess.silence {
                    if let Ok(pad) = rc.GetCurrentPadding() {
                        let free = frames - pad;
                        if free > 0 && render.GetBuffer(free).is_ok() {
                            let _ = render.ReleaseBuffer(free, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                        }
                    }
                }
                loop {
                    match sess.capture.GetNextPacketSize() {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(e) => {
                            log::info!("{label}: device lost ({e}), reopening");
                            break 'run;
                        }
                    }
                    let mut data = ptr::null_mut();
                    let (mut frames, mut flags, mut qpc_pos) = (0u32, 0u32, 0u64);
                    if let Err(e) = sess.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc_pos)) {
                        log::info!("{label}: GetBuffer failed ({e}), reopening");
                        break 'run;
                    }
                    let n = frames as usize;
                    let bytes_per_frame = sess.channels * if sess.widen24 { 3 } else { sess.resampler_bytes() };
                    let mut src = data as *const u8;
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        zeros.clear();
                        zeros.resize(n * sess.channels * sess.resampler_bytes(), 0);
                        src = zeros.as_ptr();
                    } else if sess.widen24 {
                        let raw = std::slice::from_raw_parts(src, n * bytes_per_frame);
                        scratch.clear();
                        for s in raw.chunks_exact(3) {
                            scratch.extend_from_slice(&[0, s[0], s[1], s[2]]);
                        }
                        src = scratch.as_ptr();
                    }
                    // WASAPI QPC position is in 100 ns units.
                    let ts_us = if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0 && qpc_pos != 0 {
                        (qpc_pos / 10) as i64
                    } else {
                        clock::now_us()
                    };
                    let converted = sess.resampler.convert(src, n);
                    let _ = sess.capture.ReleaseBuffer(frames);
                    match converted {
                        Ok((samples, delay)) => {
                            let idx = (ts_us - t0_us) * RATE / 1_000_000 - delay;
                            ring.write(idx, samples);
                        }
                        Err(e) => log::warn!("{label}: resample: {e:#}"),
                    }
                }
            }
            // Follow the Windows default device when none is pinned.
            if device_id.is_none() && Instant::now() >= next_default_check {
                next_default_check = Instant::now() + Duration::from_secs(2);
                let flow = if kind == SourceKind::Loopback { eRender } else { eCapture };
                let cur = en
                    .as_ref()
                    .and_then(|en| unsafe { en.GetDefaultAudioEndpoint(flow, eConsole).ok() })
                    .and_then(|d| unsafe { d.GetId().ok() })
                    .map(|p| unsafe { pwstr_take(p) });
                if cur.is_some_and(|c| c != sess.device_id) {
                    log::info!("{label}: default device changed");
                    break 'run;
                }
            }
        }
        drop(sess);
        sleep_unless(&stop, Duration::from_millis(300));
    }
}

impl CaptureSession {
    fn resampler_bytes(&self) -> usize {
        unsafe { ff::av_get_bytes_per_sample(self.resampler.in_fmt) as usize }
    }
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let until = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------------------
// AAC encoder

pub(crate) struct AudioEncoder {
    ctx: CodecCtx,
    frame: AvFrame,
    pkt: AvPacket,
    pub params: Arc<CodecParams>,
    pub time_base: ff::AVRational,
}

unsafe impl Send for AudioEncoder {}

impl AudioEncoder {
    pub fn new(bitrate: i64) -> Result<Self> {
        unsafe {
            let codec = ff::avcodec_find_encoder(ff::AVCodecID::AV_CODEC_ID_AAC);
            if codec.is_null() {
                bail!("AAC encoder missing");
            }
            let ctx = CodecCtx(ff::avcodec_alloc_context3(codec));
            let c = &mut *ctx.0;
            c.sample_fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP;
            c.sample_rate = RATE as c_int;
            ff::av_channel_layout_default(&mut c.ch_layout, 2);
            c.bit_rate = bitrate;
            c.time_base = q(1, RATE as i32);
            c.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
            check(ff::avcodec_open2(ctx.0, codec, ptr::null_mut()), "avcodec_open2(aac)")?;

            let frame = AvFrame::new();
            let f = &mut *frame.0;
            f.nb_samples = (*ctx.0).frame_size.max(BLOCK as c_int);
            f.format = ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int;
            ff::av_channel_layout_default(&mut f.ch_layout, 2);
            f.sample_rate = RATE as c_int;
            check(ff::av_frame_get_buffer(frame.0, 0), "av_frame_get_buffer")?;

            Ok(AudioEncoder {
                params: CodecParams::from_context(ctx.0)?,
                time_base: (*ctx.0).time_base,
                ctx,
                frame,
                pkt: AvPacket::new(),
            })
        }
    }

    /// Encodes one block of interleaved stereo samples (BLOCK frames) at `pts`.
    /// Drains the encoder's delayed frames at the end of a stream.
    pub(crate) fn flush(&mut self, out: &mut dyn FnMut(Packet)) {
        unsafe {
            ff::avcodec_send_frame(self.ctx.0, ptr::null());
            while ff::avcodec_receive_packet(self.ctx.0, self.pkt.0) >= 0 {
                out(Packet::from_av(self.pkt.0, self.time_base));
                ff::av_packet_unref(self.pkt.0);
            }
        }
    }

    pub(crate) fn encode(&mut self, interleaved: &[f32], pts: i64, out: &mut dyn FnMut(Packet)) -> Result<()> {
        unsafe {
            check(ff::av_frame_make_writable(self.frame.0), "av_frame_make_writable")?;
            let f = &mut *self.frame.0;
            let n = (interleaved.len() / 2).min(f.nb_samples as usize);
            let l = std::slice::from_raw_parts_mut(f.data[0] as *mut f32, n);
            let r = std::slice::from_raw_parts_mut(f.data[1] as *mut f32, n);
            for i in 0..n {
                l[i] = interleaved[i * 2];
                r[i] = interleaved[i * 2 + 1];
            }
            f.pts = pts;
            let res = ff::avcodec_send_frame(self.ctx.0, self.frame.0);
            if res < 0 && !is_eagain(res) {
                check(res, "avcodec_send_frame(aac)")?;
            }
            loop {
                let r = ff::avcodec_receive_packet(self.ctx.0, self.pkt.0);
                if is_eagain(r) || r == ff::AVERROR_EOF {
                    break;
                }
                check(r, "avcodec_receive_packet(aac)")?;
                out(Packet::from_av(self.pkt.0, self.time_base));
                ff::av_packet_unref(self.pkt.0);
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline

#[derive(Clone, Copy)]
enum TrackMix {
    Mix,
    System,
    Mic,
}

pub struct AudioPipeline {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

pub type AudioSink = Arc<dyn Fn(usize, Packet) + Send + Sync>;

impl AudioPipeline {
    /// Builds the track layout for the config. Returns (pipeline, stream descriptions).
    /// Packets are delivered to `sink` with the index into the returned streams.
    pub fn start(cfg: &EngineConfig, t0_us: i64, sink: AudioSink) -> Result<Option<(AudioPipeline, Vec<StreamDesc>)>> {
        let mut tracks: Vec<(TrackMix, &str, i64)> = Vec::new();
        match (cfg.system_audio, cfg.mic) {
            (true, true) => {
                tracks.push((TrackMix::Mix, "Game + Mic", 192_000));
                if cfg.separate_tracks {
                    tracks.push((TrackMix::System, "Game", 160_000));
                    tracks.push((TrackMix::Mic, "Mic", 128_000));
                }
            }
            (true, false) => tracks.push((TrackMix::System, "Game", 192_000)),
            (false, true) => tracks.push((TrackMix::Mic, "Mic", 160_000)),
            (false, false) => return Ok(None),
        }

        let mut encoders = Vec::new();
        let mut descs = Vec::new();
        for (mix, title, br) in &tracks {
            let enc = AudioEncoder::new(*br)?;
            descs.push(StreamDesc {
                kind: StreamKind::Audio,
                params: enc.params.clone(),
                time_base: enc.time_base,
                title: title.to_string(),
            });
            encoders.push((*mix, enc));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let sys_ring = Arc::new(SourceRing::default());
        let mic_ring = Arc::new(SourceRing::default());
        if cfg.system_audio {
            let (ring, stop, dev) = (sys_ring.clone(), stop.clone(), cfg.system_device.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("gc-audio-system".into())
                    .spawn(move || capture_thread(SourceKind::Loopback, dev, ring, t0_us, stop))?,
            );
        }
        if cfg.mic {
            let (ring, stop, dev) = (mic_ring.clone(), stop.clone(), cfg.mic_device.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("gc-audio-mic".into())
                    .spawn(move || capture_thread(SourceKind::Mic, dev, ring, t0_us, stop))?,
            );
        }

        let (sys_vol, mic_vol) = (cfg.system_volume.clamp(0.0, 4.0), cfg.mic_volume.clamp(0.0, 4.0));
        let (use_sys, use_mic) = (cfg.system_audio, cfg.mic);
        let stop2 = stop.clone();
        threads.push(std::thread::Builder::new().name("gc-audio-mix".into()).spawn(move || {
            let now_idx = || (clock::now_us() - t0_us) * RATE / 1_000_000;
            let mut pos = (now_idx() - MIX_LATENCY).max(0);
            let mut sys = vec![0f32; BLOCK * 2];
            let mut mic = vec![0f32; BLOCK * 2];
            let mut mix = vec![0f32; BLOCK * 2];
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                let target = now_idx() - MIX_LATENCY;
                if target - pos > RATE {
                    // Fell far behind (system sleep, debugger): skip ahead.
                    pos = target - BLOCK as i64;
                }
                while pos + BLOCK as i64 <= target {
                    if use_sys {
                        sys_ring.read(pos, &mut sys);
                        sys.iter_mut().for_each(|s| *s *= sys_vol);
                    }
                    if use_mic {
                        mic_ring.read(pos, &mut mic);
                        mic.iter_mut().for_each(|s| *s *= mic_vol);
                    }
                    for i in 0..mix.len() {
                        mix[i] = (sys[i] + mic[i]).clamp(-1.0, 1.0);
                    }
                    for (idx, (kind, enc)) in encoders.iter_mut().enumerate() {
                        let buf = match kind {
                            TrackMix::Mix => &mix,
                            TrackMix::System => &sys,
                            TrackMix::Mic => &mic,
                        };
                        let sink = &sink;
                        if let Err(e) = enc.encode(buf, pos, &mut |p| sink(idx, p)) {
                            log::warn!("audio encode: {e:#}");
                        }
                    }
                    pos += BLOCK as i64;
                }
            }
        })?);

        Ok(Some((AudioPipeline { stop, threads }, descs)))
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
