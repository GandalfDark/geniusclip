//! IPC commands used by the UI.

use crate::library::{Entry, Kind};
use crate::settings::Settings;
use crate::state::AppState;
use crate::updates::UpdateInfo;
use geniusclip_engine::{media, AudioDevice, EngineStatus, MonitorEvent, MonitorInfo};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Runs blocking work (COM, D3D, FFmpeg, the engine lock) on a worker thread:
/// Tauri runs synchronous commands on the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    settings: Settings,
    status: EngineStatus,
    monitors: Vec<MonitorInfo>,
    audio_outputs: Vec<AudioDevice>,
    audio_inputs: Vec<AudioDevice>,
    version: String,
    hotkey_errors: Vec<String>,
    update: Option<UpdateInfo>,
    lang: String,
    /// Installed memory, for the buffer-size warning.
    ram_total_mb: u64,
    has_battery: bool,
}

fn ram_total_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.map(|_| m.ullTotalPhys / 1_048_576).unwrap_or(0)
}

#[tauri::command]
pub async fn get_snapshot(app: AppHandle) -> CmdResult<Snapshot> {
    blocking(move || {
        let st = app.state::<AppState>();
        let settings = st.settings.read().clone();
        let snapshot = Snapshot {
            lang: settings.lang().into(),
            ram_total_mb: ram_total_mb(),
            has_battery: crate::power::has_battery(),
            settings,
            status: st.engine.status(),
            monitors: geniusclip_engine::list_monitors().unwrap_or_default(),
            audio_outputs: geniusclip_engine::list_audio_devices(false).unwrap_or_default(),
            audio_inputs: geniusclip_engine::list_audio_devices(true).unwrap_or_default(),
            version: app.package_info().version.to_string(),
            hotkey_errors: st.hotkey_errors.lock().clone(),
            update: st.update.lock().clone(),
        };
        snapshot
    })
    .await
}

/// Async: the engine lock is held while the pipeline restarts.
#[tauri::command]
pub async fn get_status(app: AppHandle) -> CmdResult<EngineStatus> {
    blocking(move || app.state::<AppState>().engine.status()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    width: u32,
    height: u32,
    bitrate_kbps: u32,
    buffer_mb: u32,
}

/// Expected output size, bitrate and replay buffer memory for a config.
#[tauri::command]
pub async fn estimate(settings: Settings) -> CmdResult<Estimate> {
    blocking(move || estimate_for(&settings)).await
}

fn estimate_for(settings: &Settings) -> Estimate {
    let mons = geniusclip_engine::list_monitors().unwrap_or_default();
    let m = settings
        .engine
        .monitor
        .as_deref()
        .and_then(|id| mons.iter().find(|m| m.id == id))
        .or_else(|| mons.iter().find(|m| m.primary))
        .or(mons.first());
    let (sw, sh) = m.map(|m| (m.width, m.height)).unwrap_or((1920, 1080));
    let (w, h) = geniusclip_engine::output_size(&settings.engine, sw, sh);
    let bps = geniusclip_engine::target_bitrate(&settings.engine, w, h, settings.engine.fps);
    let audio_bps: i64 = match (settings.engine.system_audio, settings.engine.mic) {
        (true, true) if settings.engine.separate_tracks => 480_000,
        (false, false) => 0,
        _ => 192_000,
    };
    let bytes = (bps + audio_bps) as f64 / 8.0 * settings.replay_seconds as f64;
    Estimate { width: w, height: h, bitrate_kbps: (bps / 1000) as u32, buffer_mb: (bytes / 1_048_576.0).round() as u32 }
}

#[tauri::command]
pub async fn update_settings(app: AppHandle, settings: Settings) -> CmdResult<Settings> {
    blocking(move || apply_settings(&app, settings)).await?
}

/// One settings update at a time, so each compares against the previous one.
static UPDATE: Mutex<()> = Mutex::new(());

fn apply_settings(app: &AppHandle, settings: Settings) -> CmdResult<Settings> {
    let _one_at_a_time = UPDATE.lock();
    let st = app.state::<AppState>();
    let old = st.settings.read().clone();
    let mut new = settings;
    new.validate(app);
    for d in [&new.clips_dir, &new.screenshots_dir] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    *st.settings.write() = new.clone();
    new.save(app).map_err(err)?;

    if old.engine != new.engine || old.replay_seconds != new.replay_seconds {
        let h = app.clone();
        // Pipeline restarts take a moment; answer the UI right away. The
        // latest settings are applied, one change at a time.
        std::thread::spawn(move || crate::actions::configure_engine(&h));
    }
    if old.replay_enabled != new.replay_enabled {
        crate::actions::set_replay(app, new.replay_enabled);
    }
    if old.hotkeys != new.hotkeys {
        crate::hotkeys::register_all(app);
    }
    if old.pause_on_battery != new.pause_on_battery {
        crate::power::refresh();
    }
    if old.autostart != new.autostart {
        crate::sync_autostart(app, new.autostart);
    }
    if old.clips_dir != new.clips_dir || old.screenshots_dir != new.screenshots_dir {
        crate::allow_media_dirs(app);
    }
    if old.accent != new.accent {
        crate::brand::apply(app);
    }
    crate::tray::refresh(app);
    crate::emit_settings(app);
    Ok(new)
}

/// Async: turning replay on starts the capture pipeline (D3D, encoder, WASAPI).
#[tauri::command]
pub async fn set_replay_enabled(app: AppHandle, on: bool) -> CmdResult<()> {
    blocking(move || crate::actions::set_replay(&app, on)).await
}

/// Releases all global hotkeys while the UI records a combo (so pressing one
/// that is already in use records it instead of triggering it), or registers
/// them again. They come back on their own after a minute. Returns the
/// actions whose hotkeys could not be registered, like `hotkey_errors`.
#[tauri::command]
pub async fn set_hotkeys_suspended(app: AppHandle, suspended: bool) -> CmdResult<Vec<String>> {
    blocking(move || {
        crate::hotkeys::set_suspended(&app, suspended);
        app.state::<AppState>().hotkey_errors.lock().clone()
    })
    .await
}

#[tauri::command]
pub fn save_clip(app: AppHandle) {
    std::thread::spawn(move || crate::actions::save_clip(&app));
}

#[tauri::command]
pub fn take_screenshot(app: AppHandle) {
    std::thread::spawn(move || crate::actions::screenshot(&app));
}

#[tauri::command]
pub fn toggle_recording(app: AppHandle) {
    std::thread::spawn(move || crate::actions::toggle_recording(&app));
}

#[tauri::command]
pub fn hotkey_errors(st: State<'_, AppState>) -> Vec<String> {
    st.hotkey_errors.lock().clone()
}

#[tauri::command]
pub async fn list_media(app: AppHandle) -> Vec<Entry> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let s = st.settings.read().clone();
        st.library.scan(&s)
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub async fn thumbnail(app: AppHandle, path: PathBuf) -> CmdResult<PathBuf> {
    blocking(move || {
        let st = app.state::<AppState>();
        check_media_path(&st, &path)?;
        st.library.thumbnail(&path).map_err(err)
    })
    .await?
}

/// Paths from the webviews: only existing media files inside the configured
/// media folders. Not the folders themselves (deleting one would trash every
/// clip), not other files (opening an .exe runs it), and not URLs or other
/// strings FFmpeg would read as a protocol.
fn check_media_path(st: &AppState, p: &Path) -> CmdResult<()> {
    let denied = || "path is outside the media folders".to_string();
    // Absolute means a drive or UNC path, never "proto:…".
    if !p.is_absolute() || !crate::library::is_media(p) {
        return Err(denied());
    }
    let file = std::fs::canonicalize(p).map_err(|_| denied())?;
    if !file.is_file() {
        return Err(denied());
    }
    let s = st.settings.read();
    let inside = [&s.clips_dir, &s.screenshots_dir].iter().any(|d| std::fs::canonicalize(d).is_ok_and(|d| file.starts_with(d)));
    if inside {
        Ok(())
    } else {
        Err(denied())
    }
}

#[tauri::command]
pub fn delete_media(st: State<'_, AppState>, path: PathBuf) -> CmdResult<()> {
    check_media_path(&st, &path)?;
    trash::delete(&path).map_err(err)?;
    st.library.forget(&path);
    Ok(())
}

#[tauri::command]
pub fn rename_media(st: State<'_, AppState>, path: PathBuf, name: String) -> CmdResult<PathBuf> {
    check_media_path(&st, &path)?;
    let clean = geniusclip_engine::game::sanitize(&name);
    let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let to = path.with_file_name(format!("{clean}.{ext}"));
    if to == path {
        return Ok(to);
    }
    // Windows paths ignore case: for a case-only rename ("clip" to "Clip")
    // the new name finds the file itself, which is not a conflict.
    let same_file = || std::fs::canonicalize(&to).ok() == std::fs::canonicalize(&path).ok();
    if to.exists() && !same_file() {
        return Err("a file with this name already exists".into());
    }
    std::fs::rename(&path, &to).map_err(err)?;
    st.library.rename(&path, &to);
    Ok(to)
}

#[derive(Serialize)]
pub struct ClipAudio {
    /// Track titles in file order.
    titles: Vec<String>,
    /// GeniusClip layout: track 0 is the game+mic mix of tracks 1 and 2.
    mix: bool,
    /// One playable file per track, for the preview.
    files: Vec<PathBuf>,
    /// Waveform per track (peak 0..1 per slice); empty for the mix track.
    peaks: Vec<Vec<f32>>,
}

#[tauri::command]
pub async fn clip_audio(app: AppHandle, path: PathBuf) -> CmdResult<ClipAudio> {
    tauri::async_runtime::spawn_blocking(move || {
        check_media_path(&app.state::<AppState>(), &path)?;
        let titles = geniusclip_engine::remix::audio_tracks(&path).map_err(err)?;
        let mix = geniusclip_engine::remix::is_mix_layout(&titles);
        if titles.is_empty() {
            return Ok(ClipAudio { titles, mix, files: Vec::new(), peaks: Vec::new() });
        }
        // The mix track is rebuilt from the game and mic tracks, so it is never shown.
        let skip: &[usize] = if mix { &[0] } else { &[] };
        let (files, peaks) = app.state::<AppState>().library.clip_audio(&path, titles.len(), skip).map_err(err)?;
        Ok(ClipAudio { titles, mix, files, peaks })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn trim_media(app: AppHandle, path: PathBuf, start: f64, end: f64, replace: bool, gains: Option<Vec<f32>>) -> CmdResult<Option<Entry>> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        check_media_path(&st, &path)?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let lang = st.settings.read().lang();
        let suffix = crate::i18n::t(lang, "trim_suffix");
        let mut out = path.with_file_name(format!("{stem} ({suffix}).mp4"));
        let mut n = 2;
        while out.exists() {
            out = path.with_file_name(format!("{stem} ({suffix} {n}).mp4"));
            n += 1;
        }
        let meta = st.library.scan(&st.settings.read().clone()).into_iter().find(|e| e.path == path);
        let (kind, game) = meta.map(|e| (e.kind, e.game)).unwrap_or((Kind::Clip, String::new()));
        match gains.filter(|g| g.iter().any(|&x| (x - 1.0).abs() > 0.005)) {
            Some(g) => geniusclip_engine::remix::trim_with_gains(&path, &out, start, end, &g),
            None => media::trim(&path, &out, start, end),
        }
        .map_err(|e| {
            let _ = std::fs::remove_file(&out);
            err(e)
        })?;
        let final_path = if replace {
            trash::delete(&path).map_err(err)?;
            std::fs::rename(&out, &path).map_err(err)?;
            st.library.forget(&path);
            path.clone()
        } else {
            out
        };
        st.library.expect(&final_path, kind, &game);
        Ok(st.library.add(&final_path))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn media_info(app: AppHandle, path: PathBuf) -> CmdResult<media::MediaInfo> {
    blocking(move || {
        check_media_path(&app.state::<AppState>(), &path)?;
        media::probe(&path).map_err(err)
    })
    .await?
}

#[tauri::command]
pub fn open_path(st: State<'_, AppState>, path: PathBuf) -> CmdResult<()> {
    check_media_path(&st, &path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)
}

#[tauri::command]
pub fn reveal_path(st: State<'_, AppState>, path: PathBuf) -> CmdResult<()> {
    check_media_path(&st, &path)?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err)
}

#[tauri::command]
pub fn open_media_dir(st: State<'_, AppState>, screenshots: bool) -> CmdResult<()> {
    let s = st.settings.read();
    let d = if screenshots { &s.screenshots_dir } else { &s.clips_dir };
    let _ = std::fs::create_dir_all(d);
    tauri_plugin_opener::open_path(d, None::<&str>).map_err(err)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    crate::updates::check(&app).await.map_err(err)
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> CmdResult<()> {
    crate::updates::install(&app).await.map_err(err)
}

/// Closing the main window destroys it (frees the WebView's memory); the app
/// keeps running in the tray.
#[tauri::command]
pub fn close_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.destroy();
    }
}

/// Puts the files on the clipboard so they can be pasted into a chat.
#[tauri::command]
pub fn copy_media(st: State<'_, AppState>, paths: Vec<PathBuf>) -> CmdResult<()> {
    for p in &paths {
        check_media_path(&st, p)?;
    }
    let owner = crate::overlay::window().ok_or("no window")?;
    crate::share::copy_files(owner, &paths).map_err(err)
}

/// Hides the in-game menu (after its closing animation) and gives the game focus back.
// Async: hide() may wait for an open() in progress, which needs the UI thread.
#[tauri::command]
pub async fn menu_close(app: AppHandle) {
    let _ = tauri::async_runtime::spawn_blocking(move || crate::menu::hide(&app, true)).await;
}

/// Screenshot from the in-game menu: the page has made itself invisible, so
/// capture briefly resumes to take the game as it is now, then holds again.
#[tauri::command]
pub async fn menu_screenshot(app: AppHandle) {
    let wait = |ms| tauri::async_runtime::spawn_blocking(move || std::thread::sleep(std::time::Duration::from_millis(ms)));
    crate::menu::hold(&app, false);
    let _ = wait(120).await;
    crate::actions::screenshot(&app);
    let _ = wait(100).await;
    if crate::menu::is_open() {
        crate::menu::hold(&app, true);
    }
}

/// The menu page has loaded (see `menu::ready`).
#[tauri::command]
pub fn menu_ready() {
    crate::menu::ready();
}

/// Async: GPU performance counters can take a moment.
#[tauri::command]
pub async fn system_stats() -> crate::stats::Stats {
    blocking(crate::stats::snapshot).await.unwrap_or_default()
}

static PENDING_OPEN: parking_lot::Mutex<Option<PathBuf>> = parking_lot::Mutex::new(None);

/// From the in-game menu: show the main window on a page, optionally with a
/// clip open in the viewer (picked up by the gallery via `take_pending_open`).
// Async: it may create the main window, which deadlocks when done from a
// synchronous command on Windows.
#[tauri::command]
pub async fn open_in_app(app: AppHandle, route: String, path: Option<PathBuf>) {
    *PENDING_OPEN.lock() = path;
    crate::menu::hide(&app, false);
    crate::show_main_at(&app, Some(&route));
}

#[tauri::command]
pub fn take_pending_open() -> Option<PathBuf> {
    PENDING_OPEN.lock().take()
}

/// The latest mic_test request. Requests run on worker threads, so each one
/// applies this (under MIC_TEST) rather than its own argument: a quick on
/// then off can't end with the microphone left playing.
static MIC_WANTED: AtomicBool = AtomicBool::new(false);
static MIC_TEST: Mutex<()> = Mutex::new(());

/// Stops the microphone check, including one that is still starting.
pub fn stop_mic_test(app: &AppHandle) {
    MIC_WANTED.store(false, Ordering::SeqCst);
    app.state::<AppState>().engine.stop_mic_monitor();
}

/// Microphone check: plays the mic back with the current noise suppression.
/// Emits `mic://level` [before, after] and `mic://stopped` (error or null).
#[tauri::command]
pub async fn mic_test(app: AppHandle, on: bool) -> CmdResult<()> {
    MIC_WANTED.store(on, Ordering::SeqCst);
    blocking(move || {
        let _one_at_a_time = MIC_TEST.lock();
        let st = app.state::<AppState>();
        if !MIC_WANTED.load(Ordering::SeqCst) {
            st.engine.stop_mic_monitor();
            return Ok(());
        }
        let h = app.clone();
        st.engine
            .start_mic_monitor(move |e| match e {
                MonitorEvent::Level { before, after } => {
                    let _ = h.emit("mic://level", (before, after));
                }
                MonitorEvent::Stopped { error } => {
                    let _ = h.emit("mic://stopped", error);
                }
            })
            .map_err(err)?;
        // Stopped (or the window closed) while this was starting.
        if !MIC_WANTED.load(Ordering::SeqCst) {
            st.engine.stop_mic_monitor();
        }
        Ok(())
    })
    .await?
}

/// Shows a sample toast so the user can check the overlay position.
#[tauri::command]
pub fn preview_toast(app: AppHandle) {
    let game = crate::actions::current_game(&app).name;
    crate::overlay::toast(&app, crate::overlay::Toast { kind: "clip".into(), game, seconds: 300.0, ..Default::default() });
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
