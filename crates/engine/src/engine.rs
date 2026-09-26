//! Engine orchestration: owns the capture/encode pipeline, the replay buffer,
//! live recording and screenshots, and reports events to the host app.

use crate::audio::AudioPipeline;
use crate::buffer::ReplayBuffer;
use crate::denoise::DenoiseControl;
use crate::disk::DiskStore;
use crate::monitor::MicMonitor;
pub use crate::monitor::MonitorEvent;
use crate::clock;
use crate::config::EngineConfig;
use crate::convert::Converter;
use crate::cursor::{CursorRenderer, CursorState};
use crate::d3d::{create_device, create_texture, find_output};
use crate::dup::{Duplicator, Poll};
use crate::ffutil::{Packet, StreamDesc, StreamKind};
use crate::media;
use crate::mux::{self, Recorder};
use crate::venc::{output_size, VideoEncoder};
use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentThread, SetThreadPriority, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, THREAD_PRIORITY_HIGHEST, TIMER_ALL_ACCESS,
};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EngineEvent {
    Status(EngineStatus),
    #[serde(rename_all = "camelCase")]
    ClipSaved { path: PathBuf, seconds: f64 },
    #[serde(rename_all = "camelCase")]
    ClipFailed { error: String },
    #[serde(rename_all = "camelCase")]
    RecordingStarted { path: PathBuf },
    #[serde(rename_all = "camelCase")]
    RecordingSaved { path: PathBuf },
    #[serde(rename_all = "camelCase")]
    RecordingFailed { error: String },
    #[serde(rename_all = "camelCase")]
    ScreenshotSaved { path: PathBuf },
    #[serde(rename_all = "camelCase")]
    ScreenshotFailed { error: String },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// Capture pipeline is running.
    pub running: bool,
    pub replay_enabled: bool,
    pub replay_seconds: u32,
    pub recording: bool,
    pub recording_seconds: f64,
    pub buffer_seconds: f64,
    pub buffer_bytes: u64,
    pub encoder: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub dropped_frames: u64,
    pub last_error: Option<String>,
}

type EventSink = Arc<dyn Fn(EngineEvent) + Send + Sync>;

/// Result of a save request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Writing started; `ClipSaved`/`ClipFailed` follows.
    Started,
    /// Everything in the buffer is already in the previous clip.
    NothingNew,
}

/// A continuation clip shorter than this is not worth a file.
const MIN_NEW_US: i64 = 1_000_000;
/// Mixer latency (≈125 ms) plus AAC encoder delay, with margin.
const AUDIO_CATCH_UP: Duration = Duration::from_millis(260);

struct Shot {
    width: u32,
    height: u32,
    pitch: usize,
    data: Vec<u8>,
}

/// State shared between the pipeline threads and the API.
struct Shared {
    buffer: Mutex<Option<ReplayBuffer>>,
    denoise: Arc<DenoiseControl>,
    recorder: Mutex<Option<(Recorder, Instant)>>,
    replay_on: AtomicBool,
    force_key: AtomicBool,
    shots: Mutex<Vec<Sender<Result<Shot>>>>,
    fps_x100: AtomicU64,
    dropped: AtomicU64,
    failed: Mutex<Option<String>>,
}

impl Shared {
    fn on_packet(&self, stream: usize, p: Packet) {
        let p = Arc::new(p);
        if let Some((rec, _)) = self.recorder.lock().as_ref() {
            rec.push(stream, p.clone());
        }
        if self.replay_on.load(Ordering::Relaxed) {
            if let Some(b) = self.buffer.lock().as_mut() {
                b.push(stream, p);
            }
        }
    }
}

struct VideoInfo {
    desc: StreamDesc,
    encoder: String,
    width: u32,
    height: u32,
}

struct Pipeline {
    stop: Arc<AtomicBool>,
    video: Option<JoinHandle<()>>,
    audio: Option<AudioPipeline>,
    streams: Vec<StreamDesc>,
    info: VideoInfo,
}

impl Pipeline {
    fn is_alive(&self) -> bool {
        self.video.as_ref().is_some_and(|h| !h.is_finished())
    }
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(a) = self.audio.take() {
            a.stop();
        }
        if let Some(v) = self.video.take() {
            let _ = v.join();
        }
    }
}

struct State {
    cfg: EngineConfig,
    replay_enabled: bool,
    replay_seconds: u32,
    pipeline: Option<Pipeline>,
    last_error: Option<String>,
    shutdown: bool,
}

pub struct Engine {
    state: Arc<Mutex<State>>,
    shared: Arc<Shared>,
    events: EventSink,
    supervisor: Option<JoinHandle<()>>,
    alive: Arc<AtomicBool>,
    monitor: Mutex<Option<MicMonitor>>,
}

impl Engine {
    pub fn new(events: impl Fn(EngineEvent) + Send + Sync + 'static) -> Engine {
        crate::ffutil::quiet_logs();
        let events: EventSink = Arc::new(events);
        let state = Arc::new(Mutex::new(State {
            cfg: EngineConfig::default(),
            replay_enabled: false,
            replay_seconds: 300,
            pipeline: None,
            last_error: None,
            shutdown: false,
        }));
        let shared = Arc::new(Shared {
            buffer: Mutex::new(None),
            denoise: Arc::default(),
            recorder: Mutex::new(None),
            replay_on: AtomicBool::new(false),
            force_key: AtomicBool::new(false),
            shots: Mutex::new(Vec::new()),
            fps_x100: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            failed: Mutex::new(None),
        });
        let alive = Arc::new(AtomicBool::new(true));
        let mut me = Engine { state, shared, events, supervisor: None, alive, monitor: Mutex::new(None) };
        me.supervisor = Some(me.spawn_supervisor());
        me
    }

    /// Restarts a crashed pipeline (GPU reset, driver update…) after a pause.
    fn spawn_supervisor(&self) -> JoinHandle<()> {
        let (state, shared, events, alive) = (self.state.clone(), self.shared.clone(), self.events.clone(), self.alive.clone());
        std::thread::Builder::new()
            .name("gc-supervisor".into())
            .spawn(move || {
                let mut last_restart = Instant::now() - Duration::from_secs(60);
                while alive.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(500));
                    let mut st = state.lock();
                    if st.shutdown {
                        break;
                    }
                    let dead = st.pipeline.as_ref().is_some_and(|p| !p.is_alive());
                    if dead && last_restart.elapsed() > Duration::from_secs(3) {
                        let err = shared.failed.lock().take().unwrap_or_else(|| "capture stopped".into());
                        log::warn!("pipeline died: {err}; restarting");
                        events(EngineEvent::Error { message: err.clone() });
                        st.last_error = Some(err);
                        if let Some(p) = st.pipeline.take() {
                            p.shutdown();
                        }
                        last_restart = Instant::now();
                        let want = st.replay_enabled || shared.recorder.lock().is_some();
                        if want {
                            if let Err(e) = start_pipeline(&mut st, &shared) {
                                st.last_error = Some(format!("{e:#}"));
                            }
                        }
                        drop(st);
                        events(EngineEvent::Status(Engine::status_of(&state, &shared)));
                    }
                }
            })
            .expect("spawn supervisor")
    }

    /// Applies a new pipeline configuration; restarts capture if it changed.
    pub fn configure(&self, cfg: EngineConfig, replay_seconds: u32) -> Result<()> {
        let mut st = self.state.lock();
        let replay_seconds = replay_seconds.clamp(10, 3600);
        st.replay_seconds = replay_seconds;
        if let Some(b) = self.shared.buffer.lock().as_mut() {
            b.set_max_seconds(replay_seconds);
        }
        self.shared.denoise.set(cfg.noise_suppression, cfg.noise_strength);
        if st.cfg.same_pipeline(&cfg) {
            st.cfg = cfg;
            return Ok(());
        }
        st.cfg = cfg;
        let res = if st.pipeline.is_some() {
            if let Some(p) = st.pipeline.take() {
                p.shutdown();
            }
            start_pipeline(&mut st, &self.shared)
        } else {
            Ok(())
        };
        self.after_change(&mut st, res)
    }

    fn after_change(&self, st: &mut State, res: Result<()>) -> Result<()> {
        st.last_error = res.as_ref().err().map(|e| format!("{e:#}"));
        if let Err(e) = &res {
            (self.events)(EngineEvent::Error { message: format!("{e:#}") });
        }
        res
    }

    pub fn set_replay_enabled(&self, on: bool) -> Result<()> {
        let mut st = self.state.lock();
        st.replay_enabled = on;
        self.shared.replay_on.store(on, Ordering::Relaxed);
        let res = if on {
            if st.pipeline.is_none() {
                start_pipeline(&mut st, &self.shared)
            } else {
                Ok(())
            }
        } else {
            if let Some(b) = self.shared.buffer.lock().as_mut() {
                b.clear();
            }
            if self.shared.recorder.lock().is_none() {
                if let Some(p) = st.pipeline.take() {
                    p.shutdown();
                }
            }
            Ok(())
        };
        let r = self.after_change(&mut st, res);
        drop(st);
        self.emit_status();
        r
    }

    fn status_of(state: &Mutex<State>, shared: &Shared) -> EngineStatus {
        let st = state.lock();
        let (buffer_seconds, buffer_bytes) = shared
            .buffer
            .lock()
            .as_ref()
            .map(|b| (b.duration_us() as f64 / 1e6, b.bytes() as u64))
            .unwrap_or((0.0, 0));
        let rec = shared.recorder.lock();
        let p = st.pipeline.as_ref();
        EngineStatus {
            running: p.is_some_and(|p| p.is_alive()),
            replay_enabled: st.replay_enabled,
            replay_seconds: st.replay_seconds,
            recording: rec.is_some(),
            recording_seconds: rec.as_ref().map(|(_, t)| t.elapsed().as_secs_f64()).unwrap_or(0.0),
            buffer_seconds,
            buffer_bytes,
            encoder: p.map(|p| p.info.encoder.clone()).unwrap_or_default(),
            width: p.map(|p| p.info.width).unwrap_or(0),
            height: p.map(|p| p.info.height).unwrap_or(0),
            fps: shared.fps_x100.load(Ordering::Relaxed) as f32 / 100.0,
            dropped_frames: shared.dropped.load(Ordering::Relaxed),
            last_error: st.last_error.clone(),
        }
    }

    pub fn status(&self) -> EngineStatus {
        Self::status_of(&self.state, &self.shared)
    }

    fn emit_status(&self) {
        (self.events)(EngineEvent::Status(self.status()));
    }

    /// Saves the last `seconds` (default: whole buffer) to `path` in the
    /// background. With `continue_after_last`, footage already saved by the
    /// previous clip is skipped, so the new clip starts where that one ended.
    pub fn save_replay(&self, path: PathBuf, seconds: Option<u32>, comment: String, continue_after_last: bool) -> Result<SaveOutcome> {
        let secs = seconds.unwrap_or(self.state.lock().replay_seconds);
        // The clip ends at the moment of the key press; claim that range now so
        // a quick second press continues after it.
        let (since, end, prev_end) = {
            let mut guard = self.shared.buffer.lock();
            let b = guard.as_mut().ok_or_else(|| anyhow!("replay buffer is off"))?;
            let since = if continue_after_last { b.last_saved_end_us } else { None };
            let end = b.end_us().ok_or_else(|| anyhow!("replay buffer is empty"))?;
            if since.is_some_and(|s| end - s < MIN_NEW_US) {
                return Ok(SaveOutcome::NothingNew);
            }
            (since, end, b.last_saved_end_us.replace(end))
        };
        let (events, shared) = (self.events.clone(), self.shared.clone());
        std::thread::Builder::new().name("gc-save".into()).spawn(move || {
            // Audio is encoded slightly behind video; let it catch up to `end`.
            std::thread::sleep(AUDIO_CATCH_UP);
            let clip = match shared.buffer.lock().as_ref().and_then(|b| b.snapshot(secs, since, Some(end))) {
                Some(c) => c,
                None => {
                    events(EngineEvent::ClipFailed { error: "replay buffer is empty".into() });
                    return;
                }
            };
            let seconds = (clip.end_us - clip.origin_us) as f64 / 1e6;
            match mux::write_clip(&path, &clip, &comment) {
                Ok(path) => events(EngineEvent::ClipSaved { path, seconds }),
                Err(e) => {
                    log::error!("save clip: {e:#}");
                    // Not saved after all: let the next clip cover this footage.
                    if let Some(b) = shared.buffer.lock().as_mut() {
                        if b.last_saved_end_us == Some(clip.end_us) {
                            b.last_saved_end_us = prev_end;
                        }
                    }
                    events(EngineEvent::ClipFailed { error: format!("{e:#}") })
                }
            }
        })?;
        Ok(SaveOutcome::Started)
    }

    pub fn is_recording(&self) -> bool {
        self.shared.recorder.lock().is_some()
    }

    pub fn start_recording(&self, path: PathBuf, comment: String) -> Result<()> {
        let mut st = self.state.lock();
        if self.shared.recorder.lock().is_some() {
            bail!("already recording");
        }
        if st.pipeline.is_none() {
            let r = start_pipeline(&mut st, &self.shared);
            self.after_change(&mut st, r)?;
        }
        let streams = st.pipeline.as_ref().unwrap().streams.clone();
        let rec = Recorder::start(path.clone(), streams, comment)?;
        *self.shared.recorder.lock() = Some((rec, Instant::now()));
        self.shared.force_key.store(true, Ordering::Relaxed);
        drop(st);
        (self.events)(EngineEvent::RecordingStarted { path });
        self.emit_status();
        Ok(())
    }

    pub fn stop_recording(&self) -> Result<()> {
        let rec = self.shared.recorder.lock().take();
        let Some((rec, _)) = rec else { bail!("not recording") };
        {
            let mut st = self.state.lock();
            if !st.replay_enabled {
                if let Some(p) = st.pipeline.take() {
                    p.shutdown();
                }
            }
        }
        let events = self.events.clone();
        std::thread::Builder::new().name("gc-rec-finish".into()).spawn(move || match rec.stop() {
            Ok(path) => events(EngineEvent::RecordingSaved { path }),
            Err(e) => events(EngineEvent::RecordingFailed { error: format!("{e:#}") }),
        })?;
        self.emit_status();
        Ok(())
    }

    /// Captures the current desktop to a PNG in the background.
    pub fn screenshot(&self, path: PathBuf) -> Result<()> {
        let running = self.state.lock().pipeline.as_ref().is_some_and(|p| p.is_alive());
        let monitor = self.state.lock().cfg.monitor.clone();
        let rx: Option<Receiver<Result<Shot>>> = if running {
            let (tx, rx) = bounded(1);
            self.shared.shots.lock().push(tx);
            Some(rx)
        } else {
            None
        };
        let events = self.events.clone();
        std::thread::Builder::new().name("gc-shot".into()).spawn(move || {
            let shot = match rx {
                Some(rx) => rx.recv_timeout(Duration::from_secs(3)).map_err(|_| anyhow!("capture timed out")).and_then(|r| r),
                None => capture_once(monitor.as_deref()),
            };
            let res = shot.and_then(|s| media::save_png_bgra(&path, s.width, s.height, s.pitch, &s.data));
            match res {
                Ok(()) => events(EngineEvent::ScreenshotSaved { path }),
                Err(e) => events(EngineEvent::ScreenshotFailed { error: format!("{e:#}") }),
            }
        })?;
        Ok(())
    }

    /// Plays the microphone back (with noise suppression as configured) so
    /// it can be checked; `on_level(before, after)` gets peak levels.
    pub fn start_mic_monitor(&self, on_event: impl Fn(MonitorEvent) + Send + 'static) -> Result<()> {
        let cfg = self.state.lock().cfg.clone();
        let mut slot = self.monitor.lock();
        slot.take();
        *slot = Some(MicMonitor::start(cfg.mic_device, cfg.mic_volume, self.shared.denoise.clone(), Box::new(on_event))?);
        Ok(())
    }

    pub fn stop_mic_monitor(&self) {
        self.monitor.lock().take();
    }

    pub fn shutdown(&self) {
        self.stop_mic_monitor();
        let rec = self.shared.recorder.lock().take();
        if let Some((rec, _)) = rec {
            let _ = rec.stop();
        }
        let mut st = self.state.lock();
        st.shutdown = true;
        if let Some(p) = st.pipeline.take() {
            p.shutdown();
        }
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(s) = self.supervisor.take() {
            let _ = s.join();
        }
    }
}

fn start_pipeline(st: &mut State, shared: &Arc<Shared>) -> Result<()> {
    let cfg = st.cfg.clone();
    let t0 = clock::now_us();
    let stop = Arc::new(AtomicBool::new(false));
    *shared.failed.lock() = None;
    shared.dropped.store(0, Ordering::Relaxed);

    let (init_tx, init_rx) = bounded::<Result<VideoInfo>>(1);
    let (go_tx, go_rx) = bounded::<bool>(1);
    let video = {
        let (cfg, shared, stop) = (cfg.clone(), shared.clone(), stop.clone());
        std::thread::Builder::new().name("gc-video".into()).spawn(move || {
            if let Err(e) = video_thread(cfg, t0, shared.clone(), stop, init_tx, go_rx) {
                log::error!("video pipeline: {e:#}");
                *shared.failed.lock() = Some(format!("{e:#}"));
            }
        })?
    };
    let info = match init_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => {
            let _ = video.join();
            return Err(e);
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            let _ = go_tx.send(false);
            bail!("video pipeline did not start");
        }
    };

    let mut streams = vec![info.desc.clone()];
    let audio = {
        let shared2 = shared.clone();
        let sink: crate::audio::AudioSink = Arc::new(move |i, p| shared2.on_packet(1 + i, p));
        match AudioPipeline::start(&cfg, t0, sink, shared.denoise.clone()) {
            Ok(Some((a, descs))) => {
                streams.extend(descs);
                Some(a)
            }
            Ok(None) => None,
            Err(e) => {
                log::error!("audio disabled: {e:#}");
                None
            }
        }
    };
    let disk = cfg.disk_buffer.then(|| DiskStore::new(DiskStore::default_dir()));
    *shared.buffer.lock() = Some(ReplayBuffer::new(streams.clone(), st.replay_seconds, disk));
    let _ = go_tx.send(true);
    st.pipeline = Some(Pipeline { stop, video: Some(video), audio, streams, info });
    Ok(())
}

struct Timer(HANDLE);
impl Timer {
    fn new() -> Self {
        unsafe {
            let h = CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
                .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))
                .unwrap_or_default();
            Timer(h)
        }
    }
    fn wait_until(&self, target_us: i64) {
        let now = clock::now_us();
        if target_us <= now {
            return;
        }
        let due = -((target_us - now) * 10);
        unsafe {
            if self.0.is_invalid() || SetWaitableTimer(self.0, &due, 0, None, None, false).is_err() {
                std::thread::sleep(Duration::from_micros((target_us - now) as u64));
            } else {
                WaitForSingleObject(self.0, INFINITE);
            }
        }
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn readback(device: &ID3D11Device, ctx: &ID3D11DeviceContext, tex: &ID3D11Texture2D) -> Result<Shot> {
    unsafe {
        let mut d = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut d);
        let sd = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..d
        };
        let mut staging = None;
        device.CreateTexture2D(&sd, None, Some(&mut staging))?;
        let staging = staging.unwrap();
        ctx.CopyResource(&staging, tex);
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
        let pitch = m.RowPitch as usize;
        let data = std::slice::from_raw_parts(m.pData as *const u8, pitch * d.Height as usize).to_vec();
        ctx.Unmap(&staging, 0);
        Ok(Shot { width: d.Width, height: d.Height, pitch, data })
    }
}

fn capture_once(monitor: Option<&str>) -> Result<Shot> {
    let out = find_output(monitor)?;
    let (device, ctx) = create_device(&out.adapter)?;
    let mut dup = Duplicator::new(&device, &out.output)?;
    let mut desktop = None;
    let mut cursor = CursorState::default();
    let dev2 = device.clone();
    let mut make = move |w, h| create_texture(&dev2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE);
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        if let Poll::Changed { desktop: true } = dup.poll(&ctx, &mut desktop, &mut make, &mut cursor)? {
            return readback(&device, &ctx, desktop.as_ref().unwrap());
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    bail!("no desktop frame available")
}

fn video_thread(
    cfg: EngineConfig,
    t0: i64,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    init_tx: Sender<Result<VideoInfo>>,
    go_rx: Receiver<bool>,
) -> Result<()> {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
    let fps = cfg.fps.clamp(10, 240);
    let setup = (|| -> Result<_> {
        let out = find_output(cfg.monitor.as_deref())?;
        log::info!("capturing {} ({}x{}) on {}", out.info.name, out.info.width, out.info.height, out.info.adapter);
        let (device, ctx) = create_device(&out.adapter)?;
        let dup = Duplicator::new(&device, &out.output)?;
        let (sw, sh) = (dup.width, dup.height);
        let (ow, oh) = output_size(&cfg, sw, sh);
        let enc = VideoEncoder::new(&device, out.info.vendor_id, &cfg, ow, oh)?;
        let conv = Converter::new(&device, &ctx, sw, sh, ow, oh, fps)?;
        let cursor_r = if cfg.capture_cursor {
            CursorRenderer::new(&device).map_err(|e| log::warn!("cursor overlay disabled: {e:#}")).ok()
        } else {
            None
        };
        Ok((device, ctx, dup, enc, conv, cursor_r))
    })();
    let (device, ctx, mut dup, enc, mut conv, mut cursor_r) = match setup {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = init_tx.send(Err(e));
            bail!(msg);
        }
    };
    let desc = StreamDesc { kind: StreamKind::Video, params: enc.params.clone(), time_base: enc.time_base, title: "Video".into() };
    let _ = init_tx.send(Ok(VideoInfo { desc, encoder: enc.name.clone(), width: enc.width, height: enc.height }));
    if !go_rx.recv_timeout(Duration::from_secs(10)).unwrap_or(false) {
        return Ok(());
    }
    let (enc_w, enc_h) = (enc.width, enc.height);

    // Encoding runs on its own thread so a slow encoder call (Media
    // Foundation can block for tens of ms) never delays frame capture.
    let pool = enc.pool();
    let (enc_tx, enc_rx) = bounded::<(crate::venc::HwFrame, i64, bool)>(8);
    let enc_thread = {
        let shared = shared.clone();
        std::thread::Builder::new().name("gc-encode".into()).spawn(move || -> Result<()> {
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            }
            let mut enc = enc;
            let mut sink = |p: Packet| shared.on_packet(0, p);
            let mut res = Ok(());
            while let Ok((frame, pts, key)) = enc_rx.recv() {
                if let Err(e) = enc.submit(frame, pts, key, &mut sink) {
                    res = Err(e);
                    break;
                }
            }
            enc.flush(&mut sink);
            res
        })?
    };
    let mut queue_drops = 0u64;

    let bind = D3D11_BIND_FLAG(D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0);
    let dev2 = device.clone();
    let mut make_tex = move |w, h| create_texture(&dev2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, bind);
    let mut desktop: Option<ID3D11Texture2D> = None;
    let mut composed: Option<ID3D11Texture2D> = None;
    let mut input: Option<ID3D11Texture2D> = None;
    let mut cursor = CursorState::default();
    let mut dirty = false;

    let timer = Timer::new();
    let period = 1_000_000.0 / fps as f64;
    let tick_time = |k: i64| t0 + (k as f64 * period) as i64;
    let mut tick: i64 = ((clock::now_us() - t0) as f64 / period).ceil() as i64;
    let mut stat_t = Instant::now();
    let mut stat_frames = 0u32;
    let mut warmed_up = false;
    // Per-stage timing (poll, compose, convert, encode) for the periodic log.
    let mut prof = [(0u64, 0u64); 4];
    let mut prof_n = 0u64;
    let mut prof_t = Instant::now();
    let mut dropped_at_log = 0u64;
    let mut lap;
    let mark = |i: usize, lap: &mut Instant, prof: &mut [(u64, u64); 4]| {
        let us = lap.elapsed().as_micros() as u64;
        prof[i].0 += us;
        prof[i].1 = prof[i].1.max(us);
        *lap = Instant::now();
    };

    while !stop.load(Ordering::Relaxed) {
        timer.wait_until(tick_time(tick));
        lap = Instant::now();

        match dup.poll(&ctx, &mut desktop, &mut make_tex, &mut cursor)? {
            Poll::Changed { desktop: d } => {
                if d && (conv.in_w != dup.width || conv.in_h != dup.height) {
                    log::info!("desktop resized to {}x{}", dup.width, dup.height);
                    conv = Converter::new(&device, &ctx, dup.width, dup.height, enc_w, enc_h, fps)?;
                    composed = None;
                }
                dirty = true;
            }
            Poll::Idle | Poll::Lost => {}
        }
        mark(0, &mut lap, &mut prof);

        if dirty {
            if let Some(src) = desktop.as_ref() {
                let draw_cursor = cursor_r.is_some() && cursor.visible && cursor.shape.is_some();
                if draw_cursor {
                    if composed.is_none() {
                        composed = Some(make_tex(dup.width, dup.height)?);
                        conv.reset_inputs();
                    }
                    let c = composed.as_ref().unwrap();
                    unsafe { ctx.CopyResource(c, src) };
                    if let Err(e) = cursor_r.as_mut().unwrap().draw(&device, &ctx, c, dup.width, dup.height, &cursor) {
                        log::warn!("cursor draw failed, disabling: {e:#}");
                        cursor_r = None;
                    }
                    input = Some(c.clone());
                } else {
                    input = Some(src.clone());
                }
                dirty = false;
            }
        }

        // Screenshot requests are served from the clean desktop image.
        let pending: Vec<_> = std::mem::take(&mut *shared.shots.lock());
        if !pending.is_empty() {
            let shot = desktop.as_ref().ok_or_else(|| anyhow!("no frame yet")).and_then(|t| readback(&device, &ctx, t));
            for tx in pending {
                let _ = tx.send(match &shot {
                    Ok(s) => Ok(Shot { width: s.width, height: s.height, pitch: s.pitch, data: s.data.clone() }),
                    Err(e) => Err(anyhow!("{e:#}")),
                });
            }
        }

        mark(1, &mut lap, &mut prof);
        if let Some(inp) = input.as_ref() {
            let (frame, surf, slice) = pool.acquire()?;
            conv.convert(inp, &surf, slice).context("convert")?;
            mark(2, &mut lap, &mut prof);
            let force = shared.force_key.load(Ordering::Relaxed);
            match enc_tx.try_send((frame, tick, force)) {
                Ok(()) => {
                    if force {
                        shared.force_key.store(false, Ordering::Relaxed);
                    }
                }
                Err(crossbeam_channel::TrySendError::Full(_)) => {
                    queue_drops += 1;
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => break,
            }
            mark(3, &mut lap, &mut prof);
            prof_n += 1;
            stat_frames += 1;
        }

        tick += 1;
        let now_tick = ((clock::now_us() - t0) as f64 / period) as i64;
        if now_tick > tick + 1 {
            shared.dropped.fetch_add((now_tick - tick) as u64, Ordering::Relaxed);
            tick = now_tick;
        }
        if stat_t.elapsed() >= Duration::from_secs(1) {
            let fps_now = stat_frames as f64 / stat_t.elapsed().as_secs_f64();
            shared.fps_x100.store((fps_now * 100.0) as u64, Ordering::Relaxed);
            if !warmed_up {
                // Encoder start-up stalls are not real drops.
                warmed_up = true;
                shared.dropped.store(0, Ordering::Relaxed);
            }
            stat_t = Instant::now();
            stat_frames = 0;
        }
        if prof_t.elapsed() >= Duration::from_secs(10) && prof_n > 0 {
            let d = shared.dropped.load(Ordering::Relaxed);
            let f = |i: usize| format!("{:.1}/{:.1}", prof[i].0 as f64 / prof_n as f64 / 1000.0, prof[i].1 as f64 / 1000.0);
            log::info!(
                "video: {:.1} fps, dropped {} (encoder busy {}) | ms avg/max poll {} compose {} convert {} queue {}",
                prof_n as f64 / prof_t.elapsed().as_secs_f64(),
                d - dropped_at_log,
                queue_drops,
                f(0),
                f(1),
                f(2),
                f(3)
            );
            dropped_at_log = d;
            queue_drops = 0;
            prof = [(0, 0); 4];
            prof_n = 0;
            prof_t = Instant::now();
        }
    }
    drop(enc_tx);
    shared.fps_x100.store(0, Ordering::Relaxed);
    match enc_thread.join() {
        Ok(r) => r.context("encoder"),
        Err(_) => bail!("encoder thread panicked"),
    }
}
