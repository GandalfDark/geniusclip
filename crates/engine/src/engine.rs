//! Engine orchestration: owns the capture/encode pipeline, the replay buffer,
//! live recording and screenshots, and reports events to the host app.

use crate::audio::AudioPipeline;
use crate::audio::LiveAudio;
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
use crate::dup::{rotate_bgra, Duplicator, Poll, Rotation};
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
use windows::core::Interface;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentThread, SetThreadPriority, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_HIGHEST, TIMER_ALL_ACCESS,
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
    /// Frames dropped during the last 10 seconds (0 when capture keeps up).
    pub dropped_recent: u64,
    /// Capture is paused (display off, locked, or on battery by choice).
    pub paused: bool,
    pub last_error: Option<String>,
    /// Noise suppression is on but its model failed to load.
    pub noise_unavailable: bool,
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

#[derive(Clone)]
struct Shot {
    width: u32,
    height: u32,
    pitch: usize,
    data: Vec<u8>,
    /// Turns the image upright (portrait displays), see `upright`.
    rotation: Rotation,
}

impl Shot {
    /// Rotates on the CPU, so it runs on the screenshot thread rather than
    /// on the capture thread.
    fn upright(self) -> Shot {
        if self.rotation == Rotation::None {
            return self;
        }
        let (data, width, height) = rotate_bgra(&self.data, self.width, self.height, self.pitch, self.rotation);
        Shot { width, height, pitch: width as usize * 4, data, rotation: Rotation::None }
    }
}

/// State shared between the pipeline threads and the API.
struct Shared {
    buffer: Mutex<Option<ReplayBuffer>>,
    denoise: Arc<DenoiseControl>,
    live_audio: Arc<LiveAudio>,
    recorder: Mutex<Option<(Recorder, Instant)>>,
    replay_on: AtomicBool,
    force_key: AtomicBool,
    shots: Mutex<Vec<Sender<Result<Shot>>>>,
    fps_x100: AtomicU64,
    dropped: AtomicU64,
    dropped_recent: AtomicU64,
    /// Repeat the last desktop frame instead of capturing (see `set_hold`).
    hold: AtomicBool,
    hold_card: Mutex<Option<crate::card::HoldCard>>,
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
    /// Why the video thread ended, if it failed. Per pipeline, so a thread
    /// of an abandoned start cannot report into the running one.
    failed: Arc<Mutex<Option<String>>>,
}

impl Pipeline {
    fn is_alive(&self) -> bool {
        self.video.as_ref().is_some_and(|h| !h.is_finished())
    }
    /// Runs under the engine lock: audio threads (WASAPI can hang with the
    /// Windows audio service) get at most 2 s before they are left behind.
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
    /// Capture paused while nobody can see the screen (display off, locked).
    paused: bool,
    /// Video threads whose start timed out, told to stop. One may be stuck
    /// in a driver call holding a D3D device and an encoder session, so the
    /// supervisor starts no new one until they have ended.
    stray: Vec<JoinHandle<()>>,
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
            paused: false,
            stray: Vec::new(),
        }));
        let shared = Arc::new(Shared {
            buffer: Mutex::new(None),
            denoise: Arc::default(),
            live_audio: Arc::default(),
            recorder: Mutex::new(None),
            replay_on: AtomicBool::new(false),
            force_key: AtomicBool::new(false),
            shots: Mutex::new(Vec::new()),
            fps_x100: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            dropped_recent: AtomicU64::new(0),
            hold: AtomicBool::new(false),
            hold_card: Mutex::new(None),
        });
        let alive = Arc::new(AtomicBool::new(true));
        let mut me = Engine { state, shared, events, supervisor: None, alive, monitor: Mutex::new(None) };
        me.supervisor = Some(me.spawn_supervisor());
        me
    }

    /// Restarts a crashed pipeline (GPU reset, driver update…) after a pause,
    /// and keeps retrying, less and less often, while replay wants capture
    /// but it failed to start (monitor not listed yet after unlock, init
    /// timeout…). Also reports a recording that ended by itself.
    fn spawn_supervisor(&self) -> JoinHandle<()> {
        let (state, shared, events, alive) = (self.state.clone(), self.shared.clone(), self.events.clone(), self.alive.clone());
        std::thread::Builder::new()
            .name("gc-supervisor".into())
            .spawn(move || {
                let mut last_start = Instant::now() - Duration::from_secs(60);
                // Failed starts in a row: sets the back-off; the failure is
                // logged once until it works again.
                let mut failures = 0u32;
                while alive.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(500));
                    let mut st = state.lock();
                    if st.shutdown {
                        break;
                    }
                    // The recorder stops on a write error (disk full): report it
                    // now instead of showing "recording" until the user stops it.
                    let rec_ended = shared.recorder.lock().as_ref().is_some_and(|(r, _)| r.is_finished());
                    if rec_ended {
                        log::warn!("recording ended by itself");
                        finish_recording(&shared, &events);
                        // Capture only ran for the recording (see `stop_recording`).
                        if !st.replay_enabled || st.paused {
                            if let Some(p) = st.pipeline.take() {
                                p.shutdown();
                            }
                        }
                    }
                    let dead = st.pipeline.as_ref().is_some_and(|p| !p.is_alive());
                    let died = dead && last_start.elapsed() > Duration::from_secs(3);
                    if died {
                        if let Some(p) = st.pipeline.take() {
                            let err = p.failed.lock().take().unwrap_or_else(|| "capture stopped".into());
                            log::warn!("pipeline died: {err}; restarting");
                            events(EngineEvent::Error { message: err.clone() });
                            st.last_error = Some(err);
                            p.shutdown();
                        }
                        // The recording cannot go on in a new pipeline: save it.
                        finish_recording(&shared, &events);
                    }
                    st.stray.retain(|h| !h.is_finished());
                    let want = st.replay_enabled && !st.paused;
                    if st.pipeline.is_some() || !want {
                        failures = 0;
                    }
                    let retry = want
                        && st.pipeline.is_none()
                        && st.stray.is_empty()
                        && (died || last_start.elapsed() > retry_delay(failures));
                    if retry {
                        let outcome = match Launch::start(st.cfg.clone(), &shared) {
                            Err(e) => Some(Err(e)),
                            Ok(launch) => {
                                // Setting up takes a while (up to 10 s before it
                                // counts as hung): status and settings calls must
                                // not wait for it.
                                drop(st);
                                let init = launch.wait();
                                st = state.lock();
                                // Settings changed, capture paused or turned off, or
                                // an API call started capture meanwhile: moot.
                                let wanted = !st.shutdown
                                    && st.replay_enabled
                                    && !st.paused
                                    && st.pipeline.is_none()
                                    && st.cfg.same_pipeline(&launch.cfg);
                                match init {
                                    Some(Ok(info)) if wanted => Some(launch.complete(&mut st, &shared, &events, info)),
                                    Some(Ok(_)) => {
                                        drop(launch.cancel());
                                        None
                                    }
                                    Some(Err(e)) => {
                                        launch.join();
                                        wanted.then_some(Err(e))
                                    }
                                    None => {
                                        st.stray.push(launch.cancel());
                                        wanted.then(|| Err(anyhow!("video pipeline did not start")))
                                    }
                                }
                            }
                        };
                        match outcome {
                            Some(Ok(())) => {
                                if failures > 0 {
                                    log::info!("capture started");
                                }
                                failures = 0;
                                // A crash's reason stays visible; an earlier start error is stale now.
                                if !died {
                                    st.last_error = None;
                                }
                            }
                            Some(Err(e)) => {
                                if failures == 0 {
                                    log::warn!("capture failed to start, retrying with back-off: {e:#}");
                                } else {
                                    log::debug!("capture start retry failed: {e:#}");
                                }
                                failures += 1;
                                st.last_error = Some(format!("{e:#}"));
                            }
                            None => {}
                        }
                        last_start = Instant::now();
                    }
                    if died || retry || rec_ended {
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
        self.shared.live_audio.set(cfg.system_volume, cfg.mic_volume, cfg.mic_muted);
        if st.cfg.same_pipeline(&cfg) {
            st.cfg = cfg;
            return Ok(());
        }
        st.cfg = cfg;
        let res = if let Some(p) = st.pipeline.take() {
            p.shutdown();
            // A recording ends with its pipeline (see `start_pipeline`), so
            // capture only restarts if replay still needs it.
            finish_recording(&self.shared, &self.events);
            if st.replay_enabled && !st.paused {
                start_pipeline(&mut st, &self.shared, &self.events)
            } else {
                Ok(())
            }
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

    /// Stops capture while the display is off or the session is locked, and
    /// resumes it afterwards. Settings stay as they are; the replay buffer
    /// starts over (it would only hold a dark or lock screen). A recording in
    /// progress keeps running.
    pub fn set_paused(&self, paused: bool) -> Result<()> {
        let mut st = self.state.lock();
        if st.paused == paused || st.shutdown {
            return Ok(());
        }
        st.paused = paused;
        let recording = self.shared.recorder.lock().is_some();
        let res = if paused {
            if !recording {
                if let Some(p) = st.pipeline.take() {
                    p.shutdown();
                }
                log::info!("capture paused");
            }
            Ok(())
        } else if st.replay_enabled && st.pipeline.is_none() {
            log::info!("capture resumed");
            start_pipeline(&mut st, &self.shared, &self.events)
        } else {
            Ok(())
        };
        let r = self.after_change(&mut st, res);
        drop(st);
        self.emit_status();
        r
    }

    pub fn set_replay_enabled(&self, on: bool) -> Result<()> {
        let mut st = self.state.lock();
        st.replay_enabled = on;
        self.shared.replay_on.store(on, Ordering::Relaxed);
        let res = if on {
            if st.pipeline.is_none() && !st.paused {
                start_pipeline(&mut st, &self.shared, &self.events)
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
            dropped_recent: shared.dropped_recent.load(Ordering::Relaxed),
            paused: st.paused,
            last_error: st.last_error.clone(),
            noise_unavailable: st.cfg.noise_suppression && shared.denoise.failed(),
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
        // A pipeline that just died is about to be restarted, which would end
        // the recording at once: replace it now instead.
        if st.pipeline.as_ref().is_some_and(|p| !p.is_alive()) {
            if let Some(p) = st.pipeline.take() {
                p.shutdown();
            }
        }
        if st.pipeline.is_none() {
            let r = start_pipeline(&mut st, &self.shared, &self.events);
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
        if !finish_recording(&self.shared, &self.events) {
            bail!("not recording");
        }
        {
            // Capture keeps running only for an active (not paused) replay.
            let mut st = self.state.lock();
            if !st.replay_enabled || st.paused {
                if let Some(p) = st.pipeline.take() {
                    p.shutdown();
                }
            }
        }
        self.emit_status();
        Ok(())
    }

    /// With a card, video keeps repeating the last captured frame, dimmed
    /// and labelled with the card. The in-game menu covers the whole screen,
    /// so its time in a clip shows the game as it was when the menu opened
    /// instead of the menu. `None` resumes capture.
    pub fn set_hold(&self, card: Option<crate::card::HoldCard>) {
        let on = card.is_some();
        *self.shared.hold_card.lock() = card;
        self.shared.hold.store(on, Ordering::Relaxed);
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
            let res = shot.map(Shot::upright).and_then(|s| media::save_png_bgra(&path, s.width, s.height, s.pitch, &s.data));
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

/// Ends the live recording, if any; the file is finished in the background,
/// then `RecordingSaved` or `RecordingFailed` follows. False if none ran.
fn finish_recording(shared: &Shared, events: &EventSink) -> bool {
    let Some((rec, _)) = shared.recorder.lock().take() else { return false };
    let events = events.clone();
    let spawned = std::thread::Builder::new().name("gc-rec-finish".into()).spawn(move || match rec.stop() {
        Ok(path) => events(EngineEvent::RecordingSaved { path }),
        Err(e) => events(EngineEvent::RecordingFailed { error: format!("{e:#}") }),
    });
    // Unlikely; dropping the recorder still finishes the file.
    if let Err(e) = spawned {
        log::error!("finish recording: {e}");
    }
    true
}

/// Pause before the supervisor tries again after `failures` failed starts in a row.
fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(match failures {
        0 | 1 => 5,
        2 => 15,
        3 => 30,
        _ => 60,
    })
}

/// How long the video thread may take to set up (device, duplication,
/// encoder) before the start counts as failed.
const INIT_TIMEOUT: Duration = Duration::from_secs(10);

/// A video thread that is setting up and not yet part of a pipeline.
struct Launch {
    cfg: EngineConfig,
    t0: i64,
    stop: Arc<AtomicBool>,
    failed: Arc<Mutex<Option<String>>>,
    video: JoinHandle<()>,
    init_rx: Receiver<Result<VideoInfo>>,
    go_tx: Sender<bool>,
}

impl Launch {
    fn start(cfg: EngineConfig, shared: &Arc<Shared>) -> Result<Launch> {
        let t0 = clock::now_us();
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(Mutex::new(None));
        let (init_tx, init_rx) = bounded::<Result<VideoInfo>>(1);
        let (go_tx, go_rx) = bounded::<bool>(1);
        let video = {
            let (cfg, shared, stop, failed) = (cfg.clone(), shared.clone(), stop.clone(), failed.clone());
            std::thread::Builder::new().name("gc-video".into()).spawn(move || {
                if let Err(e) = video_thread(cfg, t0, shared, stop, init_tx, go_rx) {
                    log::error!("video pipeline: {e:#}");
                    *failed.lock() = Some(format!("{e:#}"));
                }
            })?
        };
        Ok(Launch { cfg, t0, stop, failed, video, init_rx, go_tx })
    }

    /// Waits for the video thread's setup; None when it did not finish in time.
    fn wait(&self) -> Option<Result<VideoInfo>> {
        match self.init_rx.recv_timeout(INIT_TIMEOUT) {
            Ok(r) => Some(r),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => Some(Err(anyhow!("video thread ended during setup"))),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                log::warn!("video setup did not finish within {} s; no new capture start until it ends", INIT_TIMEOUT.as_secs());
                None
            }
        }
    }

    /// After a failed setup: the thread is ending already.
    fn join(self) {
        let _ = self.video.join();
    }

    /// Tells the video thread not to start capturing; it ends as soon as its
    /// setup returns. The handle tells when that happened.
    fn cancel(self) -> JoinHandle<()> {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.go_tx.send(false);
        self.video
    }

    /// Adds audio and a fresh replay buffer, then lets video capture begin.
    fn complete(self, st: &mut State, shared: &Arc<Shared>, events: &EventSink, info: VideoInfo) -> Result<()> {
        // Packet times restart at the new pipeline's t0 and its streams may
        // differ, so a recording from the previous one cannot continue.
        finish_recording(shared, events);
        shared.dropped.store(0, Ordering::Relaxed);
        let cfg = st.cfg.clone();
        let mut streams = vec![info.desc.clone()];
        let audio = {
            let shared2 = shared.clone();
            let sink: crate::audio::AudioSink = Arc::new(move |i, p| shared2.on_packet(1 + i, p));
            match AudioPipeline::start(&cfg, self.t0, sink, shared.denoise.clone(), shared.live_audio.clone()) {
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
        let disk = if cfg.disk_buffer {
            DiskStore::new(DiskStore::default_dir()).map_err(|e| log::error!("disk buffer unavailable, using memory: {e}")).ok()
        } else {
            None
        };
        let old = shared.buffer.lock().replace(ReplayBuffer::new(streams.clone(), st.replay_seconds, disk));
        // Freed after unlocking: audio packets already wait for this lock.
        drop(old);
        let _ = self.go_tx.send(true);
        st.pipeline = Some(Pipeline { stop: self.stop, video: Some(self.video), audio, streams, info, failed: self.failed });
        Ok(())
    }
}

/// Starts capture while holding the engine lock (API calls, which report
/// the outcome to their caller).
fn start_pipeline(st: &mut State, shared: &Arc<Shared>, events: &EventSink) -> Result<()> {
    let launch = Launch::start(st.cfg.clone(), shared)?;
    match launch.wait() {
        Some(Ok(info)) => launch.complete(st, shared, events, info),
        Some(Err(e)) => {
            launch.join();
            Err(e)
        }
        None => {
            st.stray.push(launch.cancel());
            bail!("video pipeline did not start")
        }
    }
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

fn readback(device: &ID3D11Device, ctx: &ID3D11DeviceContext, tex: &ID3D11Texture2D, rotation: Rotation) -> Result<Shot> {
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
        Ok(Shot { width: d.Width, height: d.Height, pitch, data, rotation })
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
            return readback(&device, &ctx, desktop.as_ref().unwrap(), dup.rotation);
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
    let bind = D3D11_BIND_FLAG(D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0);
    let mut desktop: Option<ID3D11Texture2D> = None;
    let mut cursor = CursorState::default();
    let setup = (|| -> Result<_> {
        let out = find_output(cfg.monitor.as_deref())?;
        log::info!("capturing {} ({}x{}) on {}", out.info.name, out.info.width, out.info.height, out.info.adapter);
        let (device, ctx) = create_device(&out.adapter)?;
        let mut dup = Duplicator::new(&device, &out.output)?;
        // The encoder size is fixed for the whole pipeline: take it from a
        // real frame when one comes quickly (a new duplication hands out the
        // current image at once), so a driver whose mode size is in another
        // orientation than assumed still gets an upright video without bars.
        let dev2 = device.clone();
        let mut make = move |w, h| create_texture(&dev2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, bind);
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            if let Poll::Changed { desktop: true } = dup.poll(&ctx, &mut desktop, &mut make, &mut cursor)? {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        // The video is upright: a portrait display gives a portrait video.
        let (sw, sh) = dup.upright_size();
        if (sw, sh) != (out.info.width, out.info.height) {
            log::info!("desktop image {sw}x{sh} upright differs from the monitor's {}x{}", out.info.width, out.info.height);
        }
        let (ow, oh) = output_size(&cfg, sw, sh);
        let enc = VideoEncoder::new(&device, out.info.vendor_id, &cfg, ow, oh)?;
        let conv = Converter::new(&device, &ctx, dup.width, dup.height, dup.rotation, ow, oh, fps)?;
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
    let (enc_tx, enc_rx) = bounded::<EncodeJob>(8);
    let cut = Arc::new(AtomicBool::new(false));
    let handle = {
        let (shared, cut) = (shared.clone(), cut.clone());
        std::thread::Builder::new().name("gc-encode".into()).spawn(move || -> Result<()> {
            // Above normal, not highest: with a software fallback encoder the
            // game's threads must not starve; the queue has 8 frames of slack.
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
            }
            let mut enc = enc;
            let mut sink = |p: Packet| {
                if !cut.load(Ordering::Relaxed) {
                    shared.on_packet(0, p);
                }
            };
            let mut res = Ok(());
            while let Ok((frame, pts, key)) = enc_rx.recv() {
                match enc.submit(frame, pts, key, &mut sink) {
                    Ok(true) => {}
                    Ok(false) => {
                        shared.dropped.fetch_add(1, Ordering::Relaxed);
                        // A recording must still start on a keyframe: ask again.
                        if key {
                            shared.force_key.store(true, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        res = Err(e);
                        break;
                    }
                }
            }
            enc.flush(&mut sink);
            res
        })?
    };
    let mut encoder = EncodeThread { tx: Some(enc_tx), handle: Some(handle), cut };
    let mut queue_drops = 0u64;

    let dev2 = device.clone();
    let mut make_tex = move |w, h| create_texture(&dev2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, bind);
    let mut desktop_raw = None;
    let mut composed: Option<ID3D11Texture2D> = None;
    let mut input: Option<ID3D11Texture2D> = None;
    // The labelled copy of the last frame while holding.
    let mut held: Option<ID3D11Texture2D> = None;
    // Setup may have grabbed the first frame already.
    let mut dirty = desktop.is_some();

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
    // Drops per second over the last 10 s, for a "dropping now" indicator.
    let mut recent: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
    let mut dropped_at_sec = 0u64;
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

        // Holding needs a frame to repeat; without one, capture as usual.
        let hold = shared.hold.load(Ordering::Relaxed) && input.is_some();
        if hold && held.is_none() {
            let t = make_tex(conv.in_w, conv.in_h)?;
            unsafe { ctx.CopyResource(&t, input.as_ref().unwrap()) };
            if let Some(card) = shared.hold_card.lock().clone() {
                if let Err(e) = crate::card::draw(&t, conv.in_w, conv.in_h, conv.rotation, &card) {
                    log::warn!("hold card: {e:#}");
                }
            }
            held = Some(t);
        } else if !hold && held.is_some() {
            held = None;
            conv.reset_inputs();
        }
        if !hold {
            match dup.poll(&ctx, &mut desktop, &mut make_tex, &mut cursor)? {
                Poll::Changed { desktop: d } => {
                    // Resolution or orientation changed (the video keeps its size).
                    if d && !conv.fits(dup.width, dup.height, dup.rotation) {
                        log::info!("desktop changed to {}x{} rotation {:?}", dup.width, dup.height, dup.rotation);
                        conv = Converter::new(&device, &ctx, dup.width, dup.height, dup.rotation, enc_w, enc_h, fps)?;
                        composed = None;
                    }
                    dirty = true;
                }
                Poll::Idle | Poll::Lost => {}
            }
            // A restarted duplication copies into a new desktop texture; the
            // converter's cached view would keep the old one alive (VRAM).
            let raw = desktop.as_ref().map(|t| t.as_raw());
            if raw != desktop_raw {
                desktop_raw = raw;
                conv.reset_inputs();
            }
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
                    // The pointer is placed by how the image itself is turned,
                    // whether or not the converter manages to turn it upright.
                    if let Err(e) = cursor_r.as_mut().unwrap().draw(&device, &ctx, c, &cursor, dup.rotation) {
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
            let shot = desktop.as_ref().ok_or_else(|| anyhow!("no frame yet")).and_then(|t| readback(&device, &ctx, t, dup.rotation));
            for tx in pending {
                let _ = tx.send(match &shot {
                    Ok(s) => Ok(s.clone()),
                    Err(e) => Err(anyhow!("{e:#}")),
                });
            }
        }

        mark(1, &mut lap, &mut prof);
        if let Some(inp) = held.as_ref().or(input.as_ref()) {
            let tx = encoder.tx.as_ref().unwrap();
            // Only this thread sends, so a full queue stays full until the
            // encoder takes a frame: this one would be dropped, skip its GPU work.
            let sent = if tx.is_full() {
                // A dead encoder never takes one, and the queue keeps
                // reporting full after it is gone: end so capture restarts.
                if encoder.is_finished() {
                    break;
                }
                false
            } else {
                let (frame, surf, slice) = pool.acquire()?;
                conv.convert(inp, &surf, slice).context("convert")?;
                mark(2, &mut lap, &mut prof);
                // Taken, not just read: a request arriving while this frame
                // is sent (start_recording, encoder refusal) stays pending.
                let force = shared.force_key.swap(false, Ordering::Relaxed);
                match tx.try_send((frame, tick, force)) {
                    Ok(()) => true,
                    Err(e) => {
                        if force {
                            shared.force_key.store(true, Ordering::Relaxed);
                        }
                        if e.is_disconnected() {
                            break;
                        }
                        false
                    }
                }
            };
            if !sent {
                queue_drops += 1;
                shared.dropped.fetch_add(1, Ordering::Relaxed);
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
            let d = shared.dropped.load(Ordering::Relaxed);
            recent.push_back(d.saturating_sub(dropped_at_sec));
            dropped_at_sec = d;
            if recent.len() > 10 {
                recent.pop_front();
            }
            shared.dropped_recent.store(recent.iter().sum(), Ordering::Relaxed);
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
    shared.fps_x100.store(0, Ordering::Relaxed);
    let res = encoder.finish();
    if stop.load(Ordering::Relaxed) {
        res
    } else {
        // Left the loop because the encoder is gone.
        res.and_then(|()| Err(anyhow!("encoder stopped")))
    }
}

type EncodeJob = (crate::venc::HwFrame, i64, bool);

/// How long a stopping pipeline waits for its encoder to flush.
const ENCODER_STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// The encoder thread of one pipeline. Every way out of the capture loop
/// (errors included) stops it and waits for its last packets: they carry
/// this pipeline's times and must not land in the next pipeline's buffer
/// or recording.
struct EncodeThread {
    tx: Option<Sender<EncodeJob>>,
    handle: Option<JoinHandle<Result<()>>>,
    /// Set when the thread is left behind: its later packets are discarded.
    cut: Arc<AtomicBool>,
}

impl EncodeThread {
    fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    fn finish(&mut self) -> Result<()> {
        // Closing the queue ends the encoder's loop; it flushes, then returns.
        self.tx = None;
        let Some(h) = self.handle.take() else { return Ok(()) };
        let until = Instant::now() + ENCODER_STOP_TIMEOUT;
        while !h.is_finished() {
            if Instant::now() >= until {
                self.cut.store(true, Ordering::Relaxed);
                bail!("encoder did not stop within {} s", ENCODER_STOP_TIMEOUT.as_secs());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        match h.join() {
            Ok(r) => r.context("encoder"),
            Err(_) => bail!("encoder thread panicked"),
        }
    }
}

impl Drop for EncodeThread {
    fn drop(&mut self) {
        if let Err(e) = self.finish() {
            log::warn!("{e:#}");
        }
    }
}
