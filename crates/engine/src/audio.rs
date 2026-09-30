//! Audio: WASAPI capture (system loopback + microphone, and optionally
//! Discord's voices split off the system audio by process loopback),
//! resampling to 48 kHz stereo, timestamp-driven mixing and AAC encoding.
//!
//! Each capture thread writes samples into a [`SourceRing`] at positions
//! derived from WASAPI's QPC timestamps. The mixer pulls fixed 1024-sample
//! blocks a little behind real time, so late/missing data becomes silence and
//! clock drift between devices never accumulates into A/V desync.

use crate::clock;
use crate::config::EngineConfig;
use crate::denoise::{self, Chunk, DenoiseControl};
use crate::ffutil::*;
use crate::voice::{self, Discord};
use crossbeam_channel::Sender;
use anyhow::{bail, Context, Result};
use ffmpeg_sys_next as ff;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::{PKEY_Device_EnumeratorName, PKEY_Device_FriendlyName};
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
/// Longest wait for the audio threads when the pipeline stops.
const JOIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the game and voice captures ask whether Discord's process
/// tree changed.
const DISCORD_POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// Connected over Bluetooth. An open Bluetooth microphone switches the
    /// headphones to the low-quality hands-free (headset) profile.
    pub bluetooth: bool,
}

/// Balances only its own successful init: on a thread that is already STA
/// (the UI thread) CoInitializeEx fails with RPC_E_CHANGED_MODE, and an
/// extra CoUninitialize would tear down that thread's COM.
pub(crate) struct ComInit(bool);
impl ComInit {
    pub(crate) fn new() -> Self {
        // S_OK or S_FALSE (already initialized) both need a CoUninitialize.
        ComInit(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}
impl Drop for ComInit {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

unsafe fn pwstr_take(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

pub(crate) fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator") }
}

fn default_id(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Option<String> {
    unsafe { en.GetDefaultAudioEndpoint(flow, eConsole).ok().and_then(|d| d.GetId().ok()).map(|p| pwstr_take(p)) }
}

/// Lists active output (`capture == false`) or input devices.
pub fn list_devices(capture: bool) -> Result<Vec<AudioDevice>> {
    let _com = ComInit::new();
    let flow = if capture { eCapture } else { eRender };
    let mut out = Vec::new();
    unsafe {
        let en = enumerator()?;
        let default_id = default_id(&en, flow);
        let coll = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..coll.GetCount()? {
            let dev = coll.Item(i)?;
            let id = pwstr_take(dev.GetId()?);
            let props = dev.OpenPropertyStore(STGM_READ).ok();
            let prop = |key| props.as_ref().and_then(|ps| ps.GetValue(key).ok()).map(|v| v.to_string()).unwrap_or_default();
            let name = Some(prop(&PKEY_Device_FriendlyName)).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
            // "BTHENUM" (A2DP/LE) or "BTHHFENUM" (hands-free).
            let bluetooth = prop(&PKEY_Device_EnumeratorName).to_ascii_uppercase().starts_with("BTH");
            out.push(AudioDevice { is_default: default_id.as_deref() == Some(id.as_str()), id, name, bluetooth });
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
pub(crate) struct SourceRing {
    inner: Mutex<RingInner>,
}

impl SourceRing {
    /// Writes interleaved stereo samples whose first sample belongs at `idx`.
    pub(crate) fn write(&self, idx: i64, mut samples: &[f32]) {
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

    /// True once anything has been written.
    pub(crate) fn primed(&self) -> bool {
        self.inner.lock().primed
    }

    /// Reads `out.len()/2` stereo samples starting at `idx` (silence where missing)
    /// and discards everything before the end of the read window.
    pub(crate) fn read(&self, idx: i64, out: &mut [f32]) {
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

#[derive(Clone)]
pub(crate) enum SourceKind {
    Loopback,
    Mic,
    /// System audio without Discord's process tree; plain loopback while
    /// Discord isn't running or process loopback is unavailable.
    Game(Arc<Discord>),
    /// Discord's process tree only; nothing (a silent track) while Discord
    /// isn't running or process loopback is unavailable.
    Voice(Arc<Discord>),
}

pub(crate) fn sample_format(wf: &WAVEFORMATEX) -> Option<(ff::AVSampleFormat, bool)> {
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

unsafe fn open_session(loopback: bool, device_id: Option<&str>) -> Result<CaptureSession> {
    let en = enumerator()?;
    let flow = if loopback { eRender } else { eCapture };
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
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, fmt, None).context("IAudioClient::Initialize")?;
        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;

        let silence = if loopback {
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

/// Process loopback: the process tree of `pid` only (`include`) or all but
/// it. It has no endpoint of its own: it captures the mix of the default
/// output device, so a pinned `system_device` does not apply to it. Nor has
/// it a mix format: it converts to the one asked for.
unsafe fn open_process_session(pid: u32, include: bool, stop: &AtomicBool) -> Result<CaptureSession> {
    let client = voice::process_loopback(pid, include, stop)?;
    let wf = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 2,
        nSamplesPerSec: RATE as u32,
        nAvgBytesPerSec: RATE as u32 * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &wf, None).context("IAudioClient::Initialize")?;
    let resampler = Resampler::new(ff::AVSampleFormat::AV_SAMPLE_FMT_FLT, 2, RATE as u32)?;
    let event = CreateEventW(None, false, false, None)?;
    let started = (|| -> Result<IAudioCaptureClient> {
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok(capture)
    })();
    let capture = started.inspect_err(|_| {
        let _ = CloseHandle(event);
    })?;
    // The default device's id: when it changes, the session is reopened
    // like a plain loopback one.
    let device_id = enumerator().ok().and_then(|en| default_id(&en, eRender)).unwrap_or_default();
    Ok(CaptureSession { client, capture, event, resampler, channels: 2, widen24: false, device_id, silence: None })
}

/// `denoise`: when set, chunks go through the denoise thread, which writes
/// them to the ring itself (unless its queue is full).
/// `bypass`: while its suppression is off, chunks skip the denoise thread
/// (a copy and a thread hop for each, 100 times a second, for nothing).
/// The mic check leaves it out: its meters need every chunk.
pub(crate) fn capture_thread(
    kind: SourceKind,
    device_id: Option<String>,
    ring: Arc<SourceRing>,
    denoise: Option<Sender<Chunk>>,
    bypass: Option<Arc<DenoiseControl>>,
    t0_us: i64,
    stop: Arc<AtomicBool>,
) {
    let _com = ComInit::new();
    let loopback = !matches!(kind, SourceKind::Mic);
    let voice = matches!(kind, SourceKind::Voice(_));
    let label = match kind {
        SourceKind::Mic => "microphone",
        SourceKind::Voice(_) => "Discord voices",
        _ => "system audio",
    };
    let discord = match &kind {
        SourceKind::Game(d) | SourceKind::Voice(d) => Some(d.clone()),
        _ => None,
    };
    let mut scratch: Vec<u8> = Vec::new();
    let mut zeros: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        // Root of Discord's process tree this session splits off (0: none).
        let target = discord.as_ref().map_or(0, |d| d.target());
        let opened = match (target, voice) {
            (0, true) => {
                // No Discord: the voice track stays silent until it starts.
                sleep_unless(&stop, DISCORD_POLL);
                continue;
            }
            (0, false) => unsafe { open_session(loopback, device_id.as_deref()) },
            (pid, _) => unsafe { open_process_session(pid, voice, &stop) },
        };
        let mut sess = match opened {
            Ok(s) => {
                match (target, voice) {
                    (0, _) => log::info!("{label}: capturing from {}", s.device_id),
                    (pid, true) => log::info!("{label}: capturing Discord (pid {pid})"),
                    (pid, false) => log::info!("{label}: capturing all but Discord (pid {pid})"),
                }
                s
            }
            Err(e) if target != 0 => {
                // Falls back right away: the next round sees no target.
                if let Some(d) = discord.as_ref().filter(|_| !stop.load(Ordering::Relaxed)) {
                    d.failed(target, &e);
                }
                continue;
            }
            Err(e) => {
                log::warn!("{label}: {e:#}");
                sleep_unless(&stop, Duration::from_secs(2));
                continue;
            }
        };
        let en = enumerator().ok();
        let mut next_default_check = Instant::now() + Duration::from_secs(2);
        let mut next_discord_check = Instant::now() + DISCORD_POLL;

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
                            let off = bypass.as_ref().is_some_and(|c| !c.get().0);
                            match denoise.as_ref().filter(|_| !off) {
                                Some(tx) => {
                                    // Queue full (denoise thread starved) or gone:
                                    // unprocessed audio beats none.
                                    if let Err(e) = tx.try_send((idx, samples.to_vec(), Instant::now())) {
                                        let (idx, samples, _) = e.into_inner();
                                        ring.write(idx, &samples);
                                    }
                                }
                                None => ring.write(idx, samples),
                            }
                        }
                        Err(e) => log::warn!("{label}: resample: {e:#}"),
                    }
                }
            }
            // Follow the Windows default device when none is pinned (process
            // loopback always captures the default one).
            if (device_id.is_none() || target != 0) && Instant::now() >= next_default_check {
                next_default_check = Instant::now() + Duration::from_secs(2);
                let flow = if loopback { eRender } else { eCapture };
                let cur = en.as_ref().and_then(|en| default_id(en, flow));
                if cur.is_some_and(|c| c != sess.device_id) {
                    log::info!("{label}: default device changed");
                    break 'run;
                }
            }
            // Discord started, restarted or closed. Asked often (the scan
            // itself runs every few seconds) so that the game and voice
            // captures switch at nearly the same moment.
            if let Some(d) = discord.as_ref().filter(|_| Instant::now() >= next_discord_check) {
                next_discord_check = Instant::now() + DISCORD_POLL;
                if d.target() != target {
                    log::info!("{label}: Discord started or closed, reopening");
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
    /// `fast`: live capture, where up to four tracks are encoded all the
    /// time. The fast coder takes well under half the CPU of the default
    /// (two-loop) one, with no audible difference at these bitrates.
    pub fn new(bitrate: i64, fast: bool) -> Result<Self> {
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
            if fast {
                set_opt(c.priv_data, "aac_coder", "fast");
            }
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
    Voice,
}

/// Bits per second of all the audio tracks for the config.
pub fn audio_bitrate(cfg: &EngineConfig) -> i64 {
    layout(cfg).iter().map(|t| t.2).sum()
}

/// The audio tracks for the config: (content, title, bitrate). A mix comes
/// first, followed by the tracks it is made of when they are kept apart;
/// `remix::is_mix_layout` must recognize every such layout.
fn layout(cfg: &EngineConfig) -> Vec<(TrackMix, &'static str, i64)> {
    let voice = cfg.voice_track();
    let mut tracks = Vec::new();
    match (cfg.system_audio, cfg.mic) {
        (true, true) => {
            tracks.push((TrackMix::Mix, if voice { "Game + Voice + Mic" } else { "Game + Mic" }, 192_000));
            if cfg.separate_tracks {
                tracks.push((TrackMix::System, "Game", 160_000));
                tracks.push((TrackMix::Mic, "Mic", 128_000));
            }
        }
        (true, false) if voice => {
            tracks.push((TrackMix::Mix, "Game + Voice", 192_000));
            tracks.push((TrackMix::System, "Game", 160_000));
        }
        (true, false) => tracks.push((TrackMix::System, "Game", 192_000)),
        (false, true) => tracks.push((TrackMix::Mic, "Mic", 160_000)),
        (false, false) => {}
    }
    if voice {
        tracks.push((TrackMix::Voice, "Voice", 128_000));
    }
    tracks
}

pub struct AudioPipeline {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

pub type AudioSink = Arc<dyn Fn(usize, Packet) + Send + Sync>;

/// Mix settings that apply live (no pipeline restart, the replay buffer is kept).
pub struct LiveAudio {
    system_volume: AtomicU32,
    mic_volume: AtomicU32,
    mic_muted: AtomicBool,
}

impl Default for LiveAudio {
    fn default() -> Self {
        LiveAudio { system_volume: AtomicU32::new(1f32.to_bits()), mic_volume: AtomicU32::new(1f32.to_bits()), mic_muted: AtomicBool::new(false) }
    }
}

impl LiveAudio {
    pub fn set(&self, system_volume: f32, mic_volume: f32, mic_muted: bool) {
        self.system_volume.store(system_volume.clamp(0.0, 4.0).to_bits(), Ordering::Relaxed);
        self.mic_volume.store(mic_volume.clamp(0.0, 4.0).to_bits(), Ordering::Relaxed);
        self.mic_muted.store(mic_muted, Ordering::Relaxed);
    }

    /// (system volume, effective mic volume)
    fn gains(&self) -> (f32, f32) {
        let mic = if self.mic_muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(self.mic_volume.load(Ordering::Relaxed)) };
        (f32::from_bits(self.system_volume.load(Ordering::Relaxed)), mic)
    }

    /// The microphone is muted or at volume 0: nothing of it is heard.
    pub(crate) fn mic_silent(&self) -> bool {
        self.gains().1 <= 0.0
    }
}

impl AudioPipeline {
    /// Builds the track layout for the config (see `layout`). Returns
    /// (pipeline, stream descriptions). Packets are delivered to `sink` with
    /// the index into the returned streams.
    pub fn start(cfg: &EngineConfig, t0_us: i64, sink: AudioSink, denoise: Arc<DenoiseControl>, live: Arc<LiveAudio>) -> Result<Option<(AudioPipeline, Vec<StreamDesc>)>> {
        let tracks = layout(cfg);
        if tracks.is_empty() {
            return Ok(None);
        }

        let mut encoders = Vec::new();
        let mut descs = Vec::new();
        for (mix, title, br) in &tracks {
            let enc = AudioEncoder::new(*br, true)?;
            descs.push(StreamDesc {
                kind: StreamKind::Audio,
                params: enc.params.clone(),
                time_base: enc.time_base,
                title: title.to_string(),
            });
            encoders.push((*mix, enc));
        }

        let stop = Arc::new(AtomicBool::new(false));
        // Built first: if a later step fails, dropping it stops the threads
        // already started.
        let mut pipeline = AudioPipeline { stop: stop.clone(), threads: Vec::new() };
        let threads = &mut pipeline.threads;
        let sys_ring = Arc::new(SourceRing::default());
        let mic_ring = Arc::new(SourceRing::default());
        let voice_ring = Arc::new(SourceRing::default());
        let discord = cfg.voice_track().then(|| Arc::new(Discord::default()));
        if cfg.system_audio {
            let (ring, stop, dev) = (sys_ring.clone(), stop.clone(), cfg.system_device.clone());
            let kind = match &discord {
                Some(d) => SourceKind::Game(d.clone()),
                None => SourceKind::Loopback,
            };
            threads.push(
                std::thread::Builder::new()
                    .name("gc-audio-system".into())
                    .spawn(move || capture_thread(kind, dev, ring, None, None, t0_us, stop))?,
            );
        }
        if let Some(d) = discord {
            let (ring, stop) = (voice_ring.clone(), stop.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("gc-audio-voice".into())
                    .spawn(move || capture_thread(SourceKind::Voice(d), None, ring, None, None, t0_us, stop))?,
            );
        }
        if cfg.mic {
            let (ring, stop, dev) = (mic_ring.clone(), stop.clone(), cfg.mic_device.clone());
            let (r, live) = (mic_ring.clone(), live.clone());
            let bypass = Some(denoise.clone());
            let tx = denoise::spawn(denoise, None, move || live.mic_silent(), move |idx, s| r.write(idx, s))?;
            threads.push(
                std::thread::Builder::new()
                    .name("gc-audio-mic".into())
                    .spawn(move || capture_thread(SourceKind::Mic, dev, ring, Some(tx), bypass, t0_us, stop))?,
            );
        }

        let (use_sys, use_mic, use_voice) = (cfg.system_audio, cfg.mic, cfg.voice_track());
        let stop2 = stop.clone();
        threads.push(std::thread::Builder::new().name("gc-audio-mix".into()).spawn(move || {
            let now_idx = || (clock::now_us() - t0_us) * RATE / 1_000_000;
            let mut pos = (now_idx() - MIX_LATENCY).max(0);
            let mut sys = vec![0f32; BLOCK * 2];
            let mut mic = vec![0f32; BLOCK * 2];
            let mut voice = vec![0f32; BLOCK * 2];
            let mut mix = vec![0f32; BLOCK * 2];
            while !stop2.load(Ordering::Relaxed) {
                // Sleep until the next block is due (≈21 ms) rather than
                // waking every 10 ms to find nothing to do; capped so a
                // stop is noticed quickly.
                let due_us = t0_us + (pos + BLOCK as i64 + MIX_LATENCY) * 1_000_000 / RATE;
                let wait = (due_us - clock::now_us()).clamp(1_000, 50_000);
                std::thread::sleep(Duration::from_micros(wait as u64));
                let target = now_idx() - MIX_LATENCY;
                if target - pos > RATE {
                    // Fell far behind (system sleep, debugger): skip ahead.
                    pos = target - BLOCK as i64;
                }
                while pos + BLOCK as i64 <= target {
                    let (sys_vol, mic_vol) = live.gains();
                    if use_sys {
                        sys_ring.read(pos, &mut sys);
                        sys.iter_mut().for_each(|s| *s *= sys_vol);
                    }
                    if use_mic {
                        mic_ring.read(pos, &mut mic);
                        mic.iter_mut().for_each(|s| *s *= mic_vol);
                    }
                    if use_voice {
                        voice_ring.read(pos, &mut voice);
                        // Split off the system audio, so at its volume.
                        voice.iter_mut().for_each(|s| *s *= sys_vol);
                    }
                    for i in 0..mix.len() {
                        mix[i] = (sys[i] + mic[i] + voice[i]).clamp(-1.0, 1.0);
                    }
                    for (idx, (kind, enc)) in encoders.iter_mut().enumerate() {
                        let buf = match kind {
                            TrackMix::Mix => &mix,
                            TrackMix::System => &sys,
                            TrackMix::Mic => &mic,
                            TrackMix::Voice => &voice,
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

        Ok(Some((pipeline, descs)))
    }

    pub fn stop(mut self) {
        self.join();
    }

    /// Stops the threads, waiting at most `JOIN_TIMEOUT` in all. The caller
    /// holds the engine lock (status, settings and the UI wait on it), so a
    /// WASAPI call stuck in a hung Windows audio service must not block it:
    /// such a thread is left behind and exits once the call returns.
    fn join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + JOIN_TIMEOUT;
        for t in self.threads.drain(..) {
            while !t.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if t.is_finished() {
                let _ = t.join();
            } else {
                log::warn!("{} did not stop within {JOIN_TIMEOUT:?}, leaving it behind", t.thread().name().unwrap_or("audio thread"));
            }
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Process loopback on the running Discord: its tree only, everything
    /// but it, then both at once, as the pipeline opens them. Reads packets
    /// for a moment each; opens no render stream. Needs Discord running:
    ///   cargo test -p geniusclip-engine discord_process_loopback -- --ignored --nocapture
    #[test]
    #[ignore]
    fn discord_process_loopback() {
        let pid = crate::voice::Discord::default().target();
        assert!(pid != 0, "Discord isn't running");
        let stop = AtomicBool::new(false);
        let run = |include: bool| {
            let _com = ComInit::new();
            let mut sess = unsafe { open_process_session(pid, include, &stop) }.expect("open");
            eprintln!("include={include}: opened");
            let (mut packets, mut frames_total, mut qpc_zero, mut silent) = (0, 0u64, 0, 0);
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                unsafe {
                    let _ = WaitForSingleObject(sess.event, 20);
                    while sess.capture.GetNextPacketSize().expect("GetNextPacketSize") > 0 {
                        let mut data = ptr::null_mut();
                        let (mut frames, mut flags, mut qpc) = (0u32, 0u32, 0u64);
                        sess.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc)).expect("GetBuffer");
                        packets += 1;
                        frames_total += frames as u64;
                        qpc_zero += (qpc == 0) as u32;
                        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            silent += 1;
                        } else {
                            sess.resampler.convert(data, frames as usize).expect("convert");
                        }
                        sess.capture.ReleaseBuffer(frames).expect("ReleaseBuffer");
                    }
                }
            }
            eprintln!("include={include}: {packets} packets, {frames_total} frames, qpc 0: {qpc_zero}, silent: {silent}");
        };
        run(true);
        run(false);
        std::thread::scope(|s| {
            s.spawn(|| run(true));
            s.spawn(|| run(false));
        });
    }

    /// The live encoder uses the fast AAC coder (no devices involved).
    #[test]
    fn live_aac_encoder_is_fast() {
        let block: Vec<f32> = (0..BLOCK * 2).map(|i| ((i / 2) as f32 * 0.05).sin() * 0.3).collect();
        let mut timing = Vec::new();
        for fast in [false, true] {
            let mut enc = AudioEncoder::new(192_000, fast).expect("aac");
            let (coder, fast_value) = unsafe {
                let obj = (*enc.ctx.0).priv_data;
                let mut coder = -1i64;
                ff::av_opt_get_int(obj, cstr("aac_coder").as_ptr(), 0, &mut coder);
                let opt = ff::av_opt_find(obj, cstr("fast").as_ptr(), cstr("coder").as_ptr(), 0, 0);
                assert!(!opt.is_null(), "no fast AAC coder");
                (coder, (*opt).default_val.i64_)
            };
            assert_eq!(coder == fast_value, fast);
            let (t, mut bytes) = (Instant::now(), 0);
            for k in 0..500 {
                enc.encode(&block, k * BLOCK as i64, &mut |p| bytes += p.data.len()).unwrap();
            }
            assert!(bytes > 0);
            timing.push(t.elapsed());
        }
        println!("500 AAC blocks: two-loop {:?}, fast {:?}", timing[0], timing[1]);
    }

    /// Without the voice track the layouts are what they always were, and
    /// the trim editor rebuilds the mix of every layout that has one.
    #[test]
    fn track_layouts() {
        let titles = |system_audio, mic, separate_tracks, voice_separate| {
            let cfg = EngineConfig { system_audio, mic, separate_tracks, voice_separate, ..Default::default() };
            layout(&cfg).iter().map(|t| t.1.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(titles(true, true, true, false), ["Game + Mic", "Game", "Mic"]);
        assert_eq!(titles(true, true, false, false), ["Game + Mic"]);
        assert_eq!(titles(true, false, true, false), ["Game"]);
        assert_eq!(titles(false, true, true, false), ["Mic"]);
        assert!(titles(false, false, true, false).is_empty());
        assert_eq!(titles(true, true, true, true), ["Game + Voice + Mic", "Game", "Mic", "Voice"]);
        assert_eq!(titles(true, false, true, true), ["Game + Voice", "Game", "Voice"]);
        // Without separate tracks (or system audio) the setting changes nothing.
        assert_eq!(titles(true, true, false, true), ["Game + Mic"]);
        assert_eq!(titles(true, false, false, true), ["Game"]);
        assert_eq!(titles(false, true, true, true), ["Mic"]);
        for sys in [true, false] {
            for mic in [true, false] {
                for voice in [true, false] {
                    let t = titles(sys, mic, true, voice);
                    assert_eq!(crate::remix::is_mix_layout(&t), t.len() > 1, "{t:?}");
                }
            }
        }
    }
}
