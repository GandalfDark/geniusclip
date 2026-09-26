//! Microphone check for the settings page: plays the microphone back on the
//! default output device through the same noise suppression (and its live
//! settings) as recordings, and reports levels before and after it.
//!
//! Playback opens the device in its own mix format (like any media player)
//! and converts with swresample; it starts only once the microphone delivers
//! audio, so the two streams are never set up at the same moment.

use crate::audio::{capture_thread, enumerator, sample_format, ComInit, SourceKind, SourceRing, RATE};
use crate::clock;
use crate::denoise::{self, DenoiseControl, Levels};
use crate::ffutil::check;
use anyhow::{bail, Context, Result};
use ffmpeg_sys_next as ff;
use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

/// Playback runs this far behind the microphone: capture packets (~10 ms),
/// one model hop (10 ms) and the model's 30 ms latency, plus slack.
const LATENCY: i64 = RATE * 90 / 1000;
/// Safety net in case the UI never stops it (you would hear yourself forever).
const MAX_RUN: Duration = Duration::from_secs(10 * 60);
/// How long to wait for the first microphone samples.
const MIC_TIMEOUT: Duration = Duration::from_secs(5);

pub enum MonitorEvent {
    /// Peak levels (0..1) before and after noise suppression, ~20 per second.
    Level { before: f32, after: f32 },
    /// The monitor ended by itself (error or time limit).
    Stopped { error: Option<String> },
}

pub type MonitorSink = Box<dyn Fn(MonitorEvent) + Send>;

/// Threads are not joined on drop: a WASAPI call stuck in a hung Windows
/// audio service must not freeze the caller (the UI). They exit on `stop`.
pub struct MicMonitor {
    stop: Arc<AtomicBool>,
}

impl MicMonitor {
    pub fn start(device: Option<String>, volume: f32, control: Arc<DenoiseControl>, sink: MonitorSink) -> Result<MicMonitor> {
        let stop = Arc::new(AtomicBool::new(false));
        let t0 = clock::now_us();
        let ring = Arc::new(SourceRing::default());
        let levels = Arc::new(Levels::default());
        let r = ring.clone();
        let tx = denoise::spawn(control, Some(levels.clone()), move |idx, s| r.write(idx, s))?;
        {
            let (ring, stop) = (ring.clone(), stop.clone());
            std::thread::Builder::new()
                .name("gc-monitor-mic".into())
                .spawn(move || capture_thread(SourceKind::Mic, device, ring, Some(tx), t0, stop))?;
        }
        {
            let stop = stop.clone();
            std::thread::Builder::new().name("gc-monitor-play".into()).spawn(move || {
                let _com = ComInit::new();
                let res = unsafe { play(&ring, t0, volume.clamp(0.0, 4.0), &levels, &sink, &stop) };
                let user_stopped = stop.swap(true, Ordering::Relaxed);
                if let Err(e) = &res {
                    log::warn!("mic monitor: {e:#}");
                }
                if !user_stopped {
                    sink(MonitorEvent::Stopped { error: res.err().map(|e| format!("{e:#}")) });
                }
            })?;
        }
        Ok(MicMonitor { stop })
    }
}

impl Drop for MicMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 48 kHz stereo float → the device's rate (still stereo float).
struct OutResampler {
    swr: *mut ff::SwrContext,
    rate: i64,
}

impl OutResampler {
    unsafe fn new(rate: u32) -> Result<OutResampler> {
        let mut layout: ff::AVChannelLayout = std::mem::zeroed();
        ff::av_channel_layout_default(&mut layout, 2);
        let mut swr = ptr::null_mut();
        let fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_FLT;
        let r = ff::swr_alloc_set_opts2(&mut swr, &layout, fmt, rate as c_int, &layout, fmt, RATE as c_int, 0, ptr::null_mut());
        ff::av_channel_layout_uninit(&mut layout);
        check(r, "swr_alloc_set_opts2")?;
        if let Err(e) = check(ff::swr_init(swr), "swr_init") {
            ff::swr_free(&mut swr);
            return Err(e);
        }
        Ok(OutResampler { swr, rate: rate as i64 })
    }

    /// Converts interleaved stereo input, appending to `out`.
    unsafe fn convert(&mut self, input: &[f32], out: &mut VecDeque<f32>, scratch: &mut Vec<f32>) -> Result<()> {
        let frames = input.len() / 2;
        let cap = frames * self.rate as usize / RATE as usize + 64;
        scratch.resize(cap * 2, 0.0);
        let out_ptr = scratch.as_mut_ptr() as *mut u8;
        let in_ptr = input.as_ptr() as *const u8;
        let n = check(ff::swr_convert(self.swr, &out_ptr, cap as c_int, &in_ptr, frames as c_int), "swr_convert")?;
        out.extend(&scratch[..n as usize * 2]);
        Ok(())
    }
}

impl Drop for OutResampler {
    fn drop(&mut self) {
        unsafe { ff::swr_free(&mut self.swr) };
    }
}

unsafe fn play(ring: &SourceRing, t0: i64, volume: f32, levels: &Levels, sink: &MonitorSink, stop: &AtomicBool) -> Result<()> {
    // Open the output only once the microphone is running.
    let waited = Instant::now();
    while !ring.primed() {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if waited.elapsed() > MIC_TIMEOUT {
            bail!("the microphone did not start");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let device = enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).context("no output device")?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let fmt = client.GetMixFormat()?;
    let (channels, rate, format) = ((*fmt).nChannels as usize, (*fmt).nSamplesPerSec, sample_format(&*fmt));
    let init = if format.map(|f| f.0) == Some(ff::AVSampleFormat::AV_SAMPLE_FMT_FLT) {
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 500_000, 0, fmt, None)
            .context("IAudioClient::Initialize")
    } else {
        Err(anyhow::anyhow!("unsupported output format"))
    };
    CoTaskMemFree(Some(fmt as *const _));
    init?;

    let mut resampler = OutResampler::new(rate)?;
    let event = CreateEventW(None, false, false, None)?;
    let res = (|| -> Result<()> {
        client.SetEventHandle(event)?;
        let render: IAudioRenderClient = client.GetService()?;
        let frames = client.GetBufferSize()?;
        client.Start()?;
        let started = Instant::now();
        let now_idx = || (clock::now_us() - t0) * RATE / 1_000_000;
        // `pos`: timeline index of the next input sample to read; `fifo`:
        // converted stereo frames at the device rate not yet played.
        let mut pos: Option<i64> = None;
        let mut fifo: VecDeque<f32> = VecDeque::new();
        let (mut input, mut scratch) = (Vec::new(), Vec::new());
        let mut next_level = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if started.elapsed() > MAX_RUN {
                bail!("time limit");
            }
            let _ = WaitForSingleObject(event, 20);
            let pad = client.GetCurrentPadding()?;
            let free = (frames - pad) as usize;
            if free > 0 {
                // Keep what is heard LATENCY behind the microphone; re-sync if
                // the device clock drifted away from ours.
                let queued = (pad as i64 + (fifo.len() / 2) as i64) * RATE / rate as i64;
                let ideal = now_idx() + queued - LATENCY;
                if !pos.is_some_and(|p| (p - ideal).abs() < RATE / 20) {
                    pos = Some(now_idx() + pad as i64 * RATE / rate as i64 - LATENCY);
                    fifo.clear();
                }
                while fifo.len() / 2 < free {
                    let need = (free - fifo.len() / 2) * RATE as usize / rate as usize + 32;
                    input.resize(need * 2, 0.0);
                    let p = pos.unwrap();
                    ring.read(p, &mut input);
                    pos = Some(p + need as i64);
                    resampler.convert(&input, &mut fifo, &mut scratch)?;
                }
                let dst = render.GetBuffer(free as u32)? as *mut f32;
                for i in 0..free {
                    let (l, r) = (fifo.pop_front().unwrap_or(0.0) * volume, fifo.pop_front().unwrap_or(0.0) * volume);
                    let frame = dst.add(i * channels);
                    match channels {
                        1 => *frame = ((l + r) * 0.5).clamp(-1.0, 1.0),
                        _ => {
                            *frame = l.clamp(-1.0, 1.0);
                            *frame.add(1) = r.clamp(-1.0, 1.0);
                            for c in 2..channels {
                                *frame.add(c) = 0.0;
                            }
                        }
                    }
                }
                render.ReleaseBuffer(free as u32, 0)?;
            }
            if Instant::now() >= next_level {
                next_level = Instant::now() + Duration::from_millis(50);
                let (before, after) = levels.take();
                sink(MonitorEvent::Level { before: (before * volume).min(1.0), after: (after * volume).min(1.0) });
            }
        }
        Ok(())
    })();
    let _ = client.Stop();
    let _ = CloseHandle(event);
    res
}
