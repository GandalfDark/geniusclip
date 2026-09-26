//! IPC commands used by the UI.

use crate::library::{Entry, Kind};
use crate::settings::Settings;
use crate::state::AppState;
use crate::updates::UpdateInfo;
use geniusclip_engine::{media, AudioDevice, EngineStatus, MonitorInfo};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
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
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, st: State<'_, AppState>) -> Snapshot {
    let settings = st.settings.read().clone();
    Snapshot {
        lang: settings.lang().into(),
        settings,
        status: st.engine.status(),
        monitors: geniusclip_engine::list_monitors().unwrap_or_default(),
        audio_outputs: geniusclip_engine::list_audio_devices(false).unwrap_or_default(),
        audio_inputs: geniusclip_engine::list_audio_devices(true).unwrap_or_default(),
        version: app.package_info().version.to_string(),
        hotkey_errors: st.hotkey_errors.lock().clone(),
        update: st.update.lock().clone(),
    }
}

#[tauri::command]
pub fn get_status(st: State<'_, AppState>) -> EngineStatus {
    st.engine.status()
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
pub fn estimate(settings: Settings) -> Estimate {
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
pub fn update_settings(app: AppHandle, st: State<'_, AppState>, settings: Settings) -> CmdResult<Settings> {
    let old = st.settings.read().clone();
    let mut new = settings;
    new.replay_seconds = new.replay_seconds.clamp(10, 3600);
    for d in [&new.clips_dir, &new.screenshots_dir] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    *st.settings.write() = new.clone();
    new.save(&app).map_err(err)?;

    if old.engine != new.engine || old.replay_seconds != new.replay_seconds {
        let (cfg, secs) = (new.engine.clone(), new.replay_seconds);
        let h = app.clone();
        // Pipeline restarts take a moment; keep the UI responsive.
        std::thread::spawn(move || {
            let _ = h.state::<AppState>().engine.configure(cfg, secs);
        });
    }
    if old.replay_enabled != new.replay_enabled {
        crate::actions::set_replay(&app, new.replay_enabled);
    }
    if old.hotkeys != new.hotkeys {
        crate::hotkeys::register_all(&app);
    }
    if old.autostart != new.autostart {
        crate::sync_autostart(&app, new.autostart);
    }
    if old.clips_dir != new.clips_dir || old.screenshots_dir != new.screenshots_dir {
        crate::allow_media_dirs(&app);
    }
    if old.accent != new.accent {
        crate::brand::apply(&app);
    }
    crate::tray::refresh(&app);
    crate::emit_settings(&app);
    Ok(new)
}

#[tauri::command]
pub fn set_replay_enabled(app: AppHandle, on: bool) {
    crate::actions::set_replay(&app, on);
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
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().library.thumbnail(&path).map_err(err))
        .await
        .map_err(err)?
}

/// Only files inside the configured media folders may be modified from the UI.
fn check_media_path(st: &AppState, p: &Path) -> CmdResult<()> {
    let s = st.settings.read();
    let canon = |x: &Path| std::fs::canonicalize(x).unwrap_or_else(|_| x.to_path_buf());
    let p = canon(p);
    if [&s.clips_dir, &s.screenshots_dir].iter().any(|d| p.starts_with(canon(d))) {
        Ok(())
    } else {
        Err("path is outside the media folders".into())
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
    if to.exists() {
        return Err("a file with this name already exists".into());
    }
    std::fs::rename(&path, &to).map_err(err)?;
    st.library.rename(&path, &to);
    Ok(to)
}

#[tauri::command]
pub async fn trim_media(app: AppHandle, path: PathBuf, start: f64, end: f64, replace: bool) -> CmdResult<Option<Entry>> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        check_media_path(&st, &path)?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let lang = st.settings.read().lang();
        let suffix = if lang == "ru" { "обрезка" } else { "trim" };
        let mut out = path.with_file_name(format!("{stem} ({suffix}).mp4"));
        let mut n = 2;
        while out.exists() {
            out = path.with_file_name(format!("{stem} ({suffix} {n}).mp4"));
            n += 1;
        }
        let meta = st.library.scan(&st.settings.read().clone()).into_iter().find(|e| e.path == path);
        let (kind, game) = meta.map(|e| (e.kind, e.game)).unwrap_or((Kind::Clip, String::new()));
        media::trim(&path, &out, start, end).map_err(err)?;
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
pub fn media_info(path: PathBuf) -> CmdResult<media::MediaInfo> {
    media::probe(&path).map_err(err)
}

#[tauri::command]
pub fn open_path(path: PathBuf) -> CmdResult<()> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)
}

#[tauri::command]
pub fn reveal_path(path: PathBuf) -> CmdResult<()> {
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
