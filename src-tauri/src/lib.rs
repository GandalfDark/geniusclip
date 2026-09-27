mod actions;
mod brand;
mod commands;
mod hotkeys;
mod i18n;
mod library;
mod menu;
mod overlay;
mod power;
mod settings;
mod share;
mod sound;
mod state;
mod stats;
mod tray;
mod updates;

use geniusclip_engine::{Engine, EngineEvent};
use library::Library;
use overlay::Toast;
use parking_lot::{Mutex, RwLock};
use settings::Settings;
use state::AppState;
use std::path::Path;
use std::time::Duration;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

const MAIN: &str = "main";

pub fn show_main(app: &AppHandle) {
    show_main_at(app, None);
}

/// Shows the main window on a page ("gallery", "settings").
pub fn show_main_at(app: &AppHandle, route: Option<&str>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(r) = route {
            let _ = app.emit_to(MAIN, "app://navigate", format!("/{r}"));
        }
        return;
    }
    // Created on demand (and destroyed on close) so the WebView costs no
    // memory while the app sits in the tray. The page shows the window once
    // it has rendered, avoiding a white flash.
    let res = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App(route.unwrap_or("index.html").into()))
        .title("GeniusClip")
        .inner_size(1200.0, 780.0)
        .min_inner_size(980.0, 640.0)
        .decorations(false)
        // HTML5 drag-and-drop (dragging clips out to Telegram) needs the
        // webview's own file-drop handler off.
        .disable_drag_drop_handler()
        .center()
        .visible(false)
        .background_color(Color(20, 20, 22, 255))
        .build();
    match res {
        Ok(w) => {
            brand::apply(app);
            // Never keep playing the microphone back once the window is gone.
            let h = app.clone();
            w.on_window_event(move |e| {
                if matches!(e, tauri::WindowEvent::Destroyed) {
                    commands::stop_mic_test(&h);
                    // Closed while a hotkey field was listening.
                    if hotkeys::is_suspended() {
                        let h = h.clone();
                        std::thread::spawn(move || hotkeys::set_suspended(&h, false));
                    }
                }
            });
        }
        Err(e) => log::error!("cannot create main window: {e}"),
    }
}

pub fn emit_settings(app: &AppHandle) {
    let s = app.state::<AppState>().settings.read().clone();
    let _ = app.emit("settings://changed", &s);
}

pub fn sync_autostart(app: &AppHandle, on: bool) {
    // Dev builds must not register the debug binary for autostart.
    if cfg!(debug_assertions) {
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    let res = if on { al.enable() } else { al.disable() };
    if let Err(e) = res {
        log::warn!("autostart: {e}");
    }
}

/// Lets the webviews load files from the media folders (asset protocol).
///
/// Folders that are no longer configured stay allowed until the app restarts:
/// tauri's asset scope (2.11) can only add allowed or forbidden paths, never
/// remove one, and a forbidden path wins over any later allow, so forbidding
/// an old folder would break playback if the user picked it (or a folder
/// containing it) again.
pub fn allow_media_dirs(app: &AppHandle) {
    let st = app.state::<AppState>();
    let s = st.settings.read();
    let scope = app.asset_protocol_scope();
    for d in [&s.clips_dir, &s.screenshots_dir] {
        let _ = scope.allow_directory(d, true);
    }
    let _ = scope.allow_directory(st.library.thumbs_dir(), true);
}

/// `library://changed` payload, sent to every window when a library file is
/// added, changed or removed, so lists can be patched without a rescan:
///
/// `{ path: string, entry: MediaEntry | null, removed: boolean }`
///
/// - saved, trimmed or renamed-to: `entry` is the file's new listing (upsert
///   it by `path`), `removed` is false;
/// - deleted or renamed-from: `entry` is null, `removed` is true;
/// - `entry` null and `removed` false: the file could not be read; rescan.
///
/// A rename sends two events: the old path removed, then the new entry.
#[derive(Clone, serde::Serialize)]
struct LibraryChange<'a> {
    path: &'a Path,
    entry: Option<&'a library::Entry>,
    removed: bool,
}

pub fn emit_library_changed(app: &AppHandle, path: &Path, entry: Option<&library::Entry>, removed: bool) {
    let _ = app.emit("library://changed", LibraryChange { path, entry, removed });
}

fn media_saved(app: &AppHandle, path: &Path, kind: &str, seconds: f64) {
    let st = app.state::<AppState>();
    let entry = st.library.add(path);
    let game = entry.as_ref().map(|e| e.game.clone()).unwrap_or_default();
    let seconds = entry.as_ref().map(|e| e.duration).filter(|d| *d > 0.0).unwrap_or(seconds);
    overlay::toast(app, Toast { kind: kind.into(), game, seconds, ..Default::default() });
    emit_library_changed(app, path, entry.as_ref(), false);
    tray::refresh(app);
}

fn on_engine_event(app: &AppHandle, e: EngineEvent) {
    if app.try_state::<AppState>().is_none() {
        return;
    }
    match &e {
        EngineEvent::ClipSaved { path, seconds } => media_saved(app, path, "clip", *seconds),
        EngineEvent::RecordingSaved { path } => media_saved(app, path, "recording", 0.0),
        EngineEvent::ScreenshotSaved { path } => media_saved(app, path, "screenshot", 0.0),
        EngineEvent::RecordingStarted { .. } => overlay::toast(app, Toast::simple("recording-start")),
        EngineEvent::ClipFailed { error } | EngineEvent::RecordingFailed { error } | EngineEvent::ScreenshotFailed { error } => {
            overlay::toast(app, Toast::error(error))
        }
        EngineEvent::Error { message } => log::warn!("engine: {message}"),
        EngineEvent::Status(_) => {}
    }
    let _ = app.emit("engine://event", &e);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch just focuses the running app.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_drag::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = Settings::load(&handle);
            let _ = std::fs::create_dir_all(&settings.clips_dir);
            let _ = std::fs::create_dir_all(&settings.screenshots_dir);
            let library = Library::new(&app.path().app_data_dir()?, &app.path().app_cache_dir()?);

            let h = handle.clone();
            let engine = Engine::new(move |e| on_engine_event(&h, e));
            app.manage(AppState {
                engine,
                settings: RwLock::new(settings.clone()),
                library,
                last_game: Mutex::new(None),
                hotkey_errors: Mutex::new(Vec::new()),
                update: Mutex::new(None),
            });
            log::info!("GeniusClip {} (FFmpeg {})", app.package_info().version, geniusclip_engine::ffmpeg_version());

            allow_media_dirs(&handle);
            overlay::create(&handle)?;
            tray::create(&handle)?;
            hotkeys::register_all(&handle);
            power::start(&handle);
            sync_autostart(&handle, settings.autostart);

            // Start capture off the UI thread.
            let h = handle.clone();
            std::thread::spawn(move || {
                actions::configure_engine(&h);
                if h.state::<AppState>().settings.read().replay_enabled {
                    if let Err(e) = actions::apply_replay(&h) {
                        log::error!("replay start: {e:#}");
                    }
                }
                tray::refresh(&h);
            });

            // Once a second: remember the focused game (skipped while there
            // is no replay buffer), push status to the UI.
            let h = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                actions::track_foreground(&h);
                if h.get_webview_window(MAIN).is_some() {
                    let status = h.state::<AppState>().engine.status();
                    let _ = h.emit_to(MAIN, "engine://status", &status);
                }
            });

            // Started with Windows: stay in the tray. Unless this start comes
            // from an update the user installed, which relaunches with the
            // original arguments, `--autostart` included.
            let after_update = updates::take_show_after_update(&handle);
            if after_update || !std::env::args().any(|a| a == "--autostart") {
                show_main(&handle);
            }
            updates::spawn_periodic(handle.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_status,
            commands::estimate,
            commands::update_settings,
            commands::set_replay_enabled,
            commands::save_clip,
            commands::take_screenshot,
            commands::toggle_recording,
            commands::hotkey_errors,
            commands::list_media,
            commands::thumbnail,
            commands::delete_media,
            commands::rename_media,
            commands::trim_media,
            commands::clip_audio,
            commands::media_info,
            commands::open_path,
            commands::reveal_path,
            commands::open_media_dir,
            commands::check_update,
            commands::install_update,
            commands::close_main,
            commands::preview_toast,
            commands::copy_media,
            commands::mic_test,
            commands::menu_close,
            commands::menu_screenshot,
            commands::menu_ready,
            commands::system_stats,
            commands::open_in_app,
            commands::take_pending_open,
            commands::quit_app,
            commands::set_hotkeys_suspended,
        ])
        .build(tauri::generate_context!())
        .expect("error while building GeniusClip")
        .run(|app, event| match event {
            // Closing the last window keeps the app alive in the tray.
            RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
            RunEvent::Exit => {
                if let Some(st) = app.try_state::<AppState>() {
                    st.engine.shutdown();
                }
            }
            _ => {}
        });
}
