//! IPC commands used by the UI.

use crate::library::{Entry, Kind};
use crate::settings::Settings;
use crate::state::AppState;
use crate::updates::{UpdateInfo, WhatsNew};
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

/// Error codes for the usual, user-caused failures; the UI translates them.
/// Any error string not starting with "err." is technical text, shown as is.
/// The technical details of a coded error go to the log.
pub mod code {
    /// The file no longer exists.
    pub const NOT_FOUND: &str = "err.not-found";
    /// Not a media file inside the clips or screenshots folder.
    pub const NOT_IN_LIBRARY: &str = "err.not-in-library";
    /// Rename: another file already has that name.
    pub const NAME_TAKEN: &str = "err.name-taken";
    /// The file is open in another program.
    pub const IN_USE: &str = "err.in-use";
    /// Not enough free disk space.
    pub const DISK_FULL: &str = "err.disk-full";
    /// Trimming failed for another reason.
    pub const TRIM_FAILED: &str = "err.trim-failed";
    /// The clips or screenshots folder can't be created (drive gone, no access).
    pub const FOLDER_UNAVAILABLE: &str = "err.folder-unavailable";
    /// The settings were applied but could not be written to disk.
    pub const SETTINGS_NOT_SAVED: &str = "err.settings-not-saved";
}

/// The code for a well-known file error, if it is one.
fn io_code(e: &std::io::Error) -> Option<&'static str> {
    use std::io::ErrorKind;
    match (e.kind(), e.raw_os_error()) {
        (ErrorKind::NotFound, _) => Some(code::NOT_FOUND),
        // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
        (ErrorKind::StorageFull, _) | (_, Some(39 | 112)) => Some(code::DISK_FULL),
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        (_, Some(32 | 33)) => Some(code::IN_USE),
        _ => None,
    }
}

/// The first I/O error in an error chain.
fn io_cause(e: &anyhow::Error) -> Option<&std::io::Error> {
    e.chain().find_map(|c| c.downcast_ref::<std::io::Error>())
}

/// Logs a failed operation on a library file and returns what the UI shows:
/// a code when the file is gone or open elsewhere or the disk is full, else
/// the technical message.
fn file_failed(what: &str, path: &Path, e: &dyn std::fmt::Display, cause: Option<&std::io::Error>) -> String {
    log::warn!("{what} {}: {e:#}", path.display());
    if let Some(c) = cause.and_then(io_code) {
        c.into()
    } else if !path.exists() {
        code::NOT_FOUND.into()
    } else if crate::library::is_locked(path) {
        code::IN_USE.into()
    } else {
        format!("{e:#}")
    }
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
    /// Notes of the update that was just installed, until dismissed.
    whats_new: Option<WhatsNew>,
    /// "Only in games" holds capture until a game shows up.
    waiting_for_game: bool,
    /// Codecs the GPU can encode, once checked.
    supported_codecs: Option<Vec<geniusclip_engine::Codec>>,
}

pub(crate) fn ram_total_mb() -> u64 {
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
            // Fresh for the settings page (outside games); also refreshes the cache.
            monitors: crate::monitors::refresh(),
            audio_outputs: geniusclip_engine::list_audio_devices(false).unwrap_or_default(),
            audio_inputs: geniusclip_engine::list_audio_devices(true).unwrap_or_default(),
            version: app.package_info().version.to_string(),
            hotkey_errors: st.hotkey_errors.lock().clone(),
            update: st.update.lock().clone(),
            whats_new: crate::updates::whats_new(&app),
            waiting_for_game: crate::power::waiting_for_game(),
            supported_codecs: st.codecs.lock().clone(),
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
    /// Also the size of a clip of the whole replay buffer.
    pub buffer_mb: u32,
}

/// Expected output size, bitrate and replay buffer memory for a config.
#[tauri::command]
pub async fn estimate(settings: Settings) -> CmdResult<Estimate> {
    blocking(move || estimate_for(&settings)).await
}

pub(crate) fn estimate_for(settings: &Settings) -> Estimate {
    // Cached: this also runs after every save (the low-disk check), in game.
    let mons = crate::monitors::list();
    let m = crate::monitors::pick(&mons, settings.engine.monitor.as_deref());
    let (sw, sh) = m.map(|m| (m.width, m.height)).unwrap_or((1920, 1080));
    let (w, h) = geniusclip_engine::output_size(&settings.engine, sw, sh);
    let bps = geniusclip_engine::target_bitrate(&settings.engine, w, h, settings.engine.fps);
    let audio_bps = geniusclip_engine::audio_bitrate(&settings.engine);
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
    let mut new = settings;
    new.validate(app);
    for d in [&new.clips_dir, &new.screenshots_dir] {
        std::fs::create_dir_all(d).map_err(|e| {
            log::warn!("media folder {}: {e}", d.display());
            code::FOLDER_UNAVAILABLE.to_string()
        })?;
    }
    // The UI sends whole snapshots, debounced: values the backend owns may
    // have changed since it took one, so those come from the current state.
    // Merged, compared and saved under the write lock, which `set_replay`
    // and `mark_onboarding` save under too: an older snapshot can neither
    // undo their change nor overwrite their file.
    let (old, saved) = {
        let mut cur = st.settings.write();
        // Replay is switched by the hotkey, the tray and the in-game menu;
        // the UI changes it only through `set_replay_enabled`, never here.
        new.replay_enabled = cur.replay_enabled;
        // "First steps" reached meanwhile (a clip saved while the UI held
        // older settings) stay reached: the UI never clears them.
        new.onboarding.clip_saved |= cur.onboarding.clip_saved;
        new.onboarding.menu_opened |= cur.onboarding.menu_opened;
        let old = std::mem::replace(&mut *cur, new.clone());
        // Reported only after the side effects below: the new values are
        // live either way, and the next update compares against them, so
        // skipping the side effects here would lose them for good (resending
        // the same values would be a no-op). Resending does retry the save.
        (old, new.save(app))
    };

    if old.engine.monitor != new.engine.monitor {
        // Picked from a list the page just enumerated: look again rather than
        // trust a cache that may predate a display change.
        crate::monitors::invalidate();
    }
    if old.engine != new.engine || old.replay_seconds != new.replay_seconds {
        let h = app.clone();
        // Pipeline restarts take a moment; answer the UI right away. The
        // latest settings are applied, one change at a time.
        std::thread::spawn(move || crate::actions::configure_engine(&h));
    }
    if old.hotkeys != new.hotkeys {
        crate::hotkeys::register_all(app);
    }
    if old.pause_on_battery != new.pause_on_battery {
        crate::power::refresh();
    }
    if old.perf_overlay != new.perf_overlay {
        crate::perf::sync();
    }
    if old.auto_clips != new.auto_clips {
        let h = app.clone();
        std::thread::spawn(move || crate::autoclip::sync(&h));
    }
    if old.games_only != new.games_only {
        let h = app.clone();
        std::thread::spawn(move || crate::actions::check_games_gate(&h));
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
    if let Err(e) = saved {
        log::error!("settings.json: {e:#}");
        let full = io_cause(&e).and_then(io_code) == Some(code::DISK_FULL);
        return Err(if full { code::DISK_FULL } else { code::SETTINGS_NOT_SAVED }.into());
    }
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
    // Recorded here, before any await, in the order the UI asked: the worker
    // threads below can start in any order, and only the latest request's
    // worker changes the registration.
    let generation = crate::hotkeys::request_suspended(&app, suspended);
    blocking(move || {
        crate::hotkeys::apply_suspended(&app, generation);
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
    let denied = || code::NOT_IN_LIBRARY.to_string();
    // A drive path, nothing else: not a network (UNC) path, which opening
    // would connect to, no ".." and no ":" past the drive (a stream name).
    use std::path::{Component, Prefix};
    let plain = p.components().enumerate().all(|(i, c)| match c {
        Component::Prefix(pre) => i == 0 && matches!(pre.kind(), Prefix::Disk(_)),
        Component::RootDir => i == 1,
        Component::Normal(n) => !n.to_string_lossy().contains(':'),
        _ => false,
    });
    if !plain || !p.is_absolute() || !crate::library::is_media(p) {
        return Err(denied());
    }
    let (clips, shots) = {
        let s = st.settings.read();
        (s.clips_dir.clone(), s.screenshots_dir.clone())
    };
    let Ok(file) = std::fs::canonicalize(p) else {
        // Gone (deleted or moved outside the app). Said only for paths in
        // the media folders, so it can't be used to probe the whole disk.
        let listed = [&clips, &shots].iter().any(|d| p.starts_with(d));
        return Err(if listed { code::NOT_FOUND.into() } else { denied() });
    };
    if !file.is_file() {
        return Err(denied());
    }
    let inside = [&clips, &shots].iter().any(|d| std::fs::canonicalize(d).is_ok_and(|d| file.starts_with(d)));
    if inside {
        Ok(())
    } else {
        Err(denied())
    }
}

/// Async: the recycle bin can take a while on a slow or network drive.
#[tauri::command]
pub async fn delete_media(app: AppHandle, path: PathBuf) -> CmdResult<()> {
    blocking(move || {
        let st = app.state::<AppState>();
        check_media_path(&st, &path)?;
        trash::delete(&path).map_err(|e| file_failed("delete", &path, &e, None))?;
        st.library.forget(&path);
        crate::emit_library_changed(&app, &path, None, true);
        Ok(())
    })
    .await?
}

/// Async: renaming on a slow or network drive must not hold the UI thread.
#[tauri::command]
pub async fn rename_media(app: AppHandle, path: PathBuf, name: String) -> CmdResult<PathBuf> {
    blocking(move || rename_media_now(&app, path, name)).await?
}

fn rename_media_now(app: &AppHandle, path: PathBuf, name: String) -> CmdResult<PathBuf> {
    let st = app.state::<AppState>();
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
        return Err(code::NAME_TAKEN.into());
    }
    std::fs::rename(&path, &to).map_err(|e| file_failed("rename", &path, &e, Some(&e)))?;
    st.library.rename(&path, &to);
    crate::emit_library_changed(app, &path, None, true);
    let entry = st.library.entry(&to);
    crate::emit_library_changed(app, &to, entry.as_ref(), false);
    Ok(to)
}

#[derive(Serialize, Clone)]
struct ClipAudioProgress<'a> {
    path: &'a Path,
    /// 0..1
    done: f32,
}

#[derive(Serialize)]
pub struct ClipAudio {
    /// Track titles in file order.
    titles: Vec<String>,
    /// GeniusClip layout: track 0 is the mix of the separate tracks after it
    /// (game, mic, Discord voices).
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
        // A long recording takes a while: the editor shows how far along.
        let mut progress = |done: f32| {
            let _ = app.emit("clip-audio://progress", ClipAudioProgress { path: &path, done });
        };
        let (files, peaks) = app
            .state::<AppState>()
            .library
            .clip_audio(&path, titles.len(), skip, &mut progress)
            .map_err(|e| file_failed("clip audio", &path, &e, io_cause(&e)))?;
        Ok(ClipAudio { titles, mix, files, peaks })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
/// `speed` below 1 (0.5, 0.25) also slows the result down (see `remix::slow_down`).
pub async fn trim_media(app: AppHandle, path: PathBuf, start: f64, end: f64, replace: bool, gains: Option<Vec<f32>>, speed: Option<f64>) -> CmdResult<Option<Entry>> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        check_media_path(&st, &path)?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let lang = st.settings.read().lang();
        // Slowed down: 2 or 4 times as long.
        let factor = speed.filter(|s| *s > 0.0 && *s < 0.99).map(|s| (1.0 / s).round().clamp(2.0, 4.0) as u32);
        let suffix = crate::i18n::t(lang, if factor.is_some() { "slow_suffix" } else { "trim_suffix" });
        let mut out = path.with_file_name(format!("{stem} ({suffix}).mp4"));
        let mut n = 2;
        while out.exists() {
            out = path.with_file_name(format!("{stem} ({suffix} {n}).mp4"));
            n += 1;
        }
        // From the index; a folder scan (only for a file not indexed yet)
        // runs on a copy of the settings, not under their lock.
        let (kind, game) = st.library.info(&path).unwrap_or_else(|| {
            let settings = st.settings.read().clone();
            let meta = st.library.scan(&settings).into_iter().find(|e| e.path == path);
            meta.map(|e| (e.kind, e.game)).unwrap_or((Kind::Clip, String::new()))
        });
        // A trim that replaces the original keeps its star; a copy starts without.
        let favorite = replace && st.library.is_favorite(&path);
        let mut trimmed = match gains.filter(|g| g.iter().any(|&x| (x - 1.0).abs() > 0.005)) {
            Some(g) => geniusclip_engine::remix::trim_with_gains(&path, &out, start, end, &g),
            None => media::trim(&path, &out, start, end),
        };
        if let (Ok(()), Some(k)) = (&trimmed, factor) {
            let slow = out.with_extension("slow.mp4");
            trimmed = geniusclip_engine::remix::slow_down(&out, &slow, k).and_then(|()| {
                std::fs::remove_file(&out)?;
                std::fs::rename(&slow, &out)?;
                Ok(())
            });
            if trimmed.is_err() {
                let _ = std::fs::remove_file(&slow);
            }
        }
        if let Err(e) = trimmed {
            // Checked before the partial output is removed, which frees the space again.
            let full = crate::library::disk_nearly_full(&out) || io_cause(&e).and_then(io_code) == Some(code::DISK_FULL);
            let _ = std::fs::remove_file(&out);
            log::warn!("trim {}: {e:#}", path.display());
            return Err(if full { code::DISK_FULL } else { code::TRIM_FAILED }.into());
        }
        let final_path = if replace {
            if let Err(e) = trash::delete(&path) {
                // The original stays as it was (usually open in another
                // program): drop the trimmed copy rather than leave it
                // behind under a name the user never asked for.
                let _ = std::fs::remove_file(&out);
                return Err(file_failed("trim: delete original", &path, &e, None));
            }
            st.library.forget(&path);
            match std::fs::rename(&out, &path) {
                Ok(()) => path.clone(),
                Err(e) => {
                    // The original is in the recycle bin already: keep the
                    // trimmed copy under its own name instead of losing it.
                    log::warn!("trim: replace {}: {e}", path.display());
                    crate::emit_library_changed(&app, &path, None, true);
                    out
                }
            }
        } else {
            out
        };
        st.library.expect_as(&final_path, kind, &game, favorite);
        let entry = st.library.add(&final_path);
        crate::emit_library_changed(&app, &final_path, entry.as_ref(), false);
        Ok(entry)
    })
    .await
    .map_err(err)?
}

/// Stars or unstars a clip or screenshot.
#[tauri::command]
pub async fn set_favorite(app: AppHandle, path: PathBuf, on: bool) -> CmdResult<Entry> {
    blocking(move || {
        let st = app.state::<AppState>();
        check_media_path(&st, &path)?;
        let entry = st
            .library
            .set_favorite(&path, on)
            .or_else(|| {
                // Not indexed yet, or changed on disk: index it first, on a
                // copy of the settings (not under their lock).
                let settings = st.settings.read().clone();
                st.library.scan(&settings);
                st.library.set_favorite(&path, on)
            })
            .ok_or_else(|| code::NOT_FOUND.to_string())?;
        crate::emit_library_changed(&app, &path, Some(&entry), false);
        Ok(entry)
    })
    .await?
}

/// Free space on the drive holding the clips folder.
#[tauri::command]
pub async fn disk_space(app: AppHandle) -> CmdResult<crate::disk::DiskSpace> {
    blocking(move || {
        let settings = app.state::<AppState>().settings.read().clone();
        crate::disk::space(&settings).map_err(err)
    })
    .await?
}

/// Hides the "what's new" notes of the installed update for good.
#[tauri::command]
pub fn dismiss_whats_new(app: AppHandle) -> CmdResult<()> {
    crate::updates::dismiss_whats_new(&app).map_err(err)
}

/// Zips logs, settings and a system summary to the Desktop for a bug
/// report, shows it in Explorer and returns its path.
#[tauri::command]
pub async fn make_report(app: AppHandle) -> CmdResult<String> {
    let path = blocking({
        let app = app.clone();
        move || crate::report::create(&app)
    })
    .await?
    .map_err(|e| {
        log::warn!("problem report: {e:#}");
        match io_cause(&e).and_then(io_code) {
            Some(code::DISK_FULL) => code::DISK_FULL.to_string(),
            _ => err(e),
        }
    })?;
    // Explorer is driven through COM: on the UI thread, like `reveal_path`.
    let shown = path.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = tauri_plugin_opener::reveal_item_in_dir(&shown) {
            log::warn!("reveal report: {e}");
        }
    });
    Ok(path.to_string_lossy().into_owned())
}

/// The built-in player couldn't play a clip (e.g. a codec WebView2 can't
/// decode on this PC): logged with the file's format for problem reports.
#[tauri::command]
pub async fn log_playback_error(app: AppHandle, path: PathBuf, code: u32, message: String) {
    let _ = blocking(move || {
        let format = check_media_path(&app.state::<AppState>(), &path)
            .ok()
            .and_then(|_| media::probe(&path).ok())
            .map(|i| format!("{} {}x{} {:.0} fps, {} audio tracks", i.video_codec, i.width, i.height, i.fps, i.audio_tracks))
            .unwrap_or_else(|| "unknown format".into());
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let message: String = message.chars().take(200).collect();
        log::warn!("built-in player failed (MediaError {code}: {message}) on {name}: {format}");
    })
    .await;
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
    // Off the async workers (there are only a few): it may wait for the engine.
    let h = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || crate::actions::screenshot(&h)).await;
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
    blocking(|| crate::stats::snapshot(crate::stats::User::Menu)).await.unwrap_or_default()
}

static PENDING_OPEN: parking_lot::Mutex<Option<PathBuf>> = parking_lot::Mutex::new(None);

/// From the in-game menu: show the main window on a page, optionally with a
/// clip open in the viewer (picked up by the gallery via `take_pending_open`).
// Async: it may create the main window, which deadlocks when done from a
// synchronous command on Windows.
#[tauri::command]
pub async fn open_in_app(app: AppHandle, route: String, path: Option<PathBuf>) {
    // A page of the app, never an address of any other kind.
    if !["", "gallery", "settings"].contains(&route.as_str()) {
        return;
    }
    *PENDING_OPEN.lock() = path;
    // On a blocking thread: both wait for the UI thread, which would tie up
    // one of the few async workers meanwhile.
    let _ = tauri::async_runtime::spawn_blocking(move || {
        crate::menu::hide(&app, false);
        crate::show_main_at(&app, Some(&route));
    })
    .await;
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
/// Async: finding the game walks the windows and reads the exe's details.
#[tauri::command]
pub async fn preview_toast(app: AppHandle) {
    let _ = blocking(move || {
        let game = crate::actions::current_game(&app).name;
        crate::overlay::toast(&app, crate::overlay::Toast { kind: "clip".into(), game, seconds: 300.0, ..Default::default() });
    })
    .await;
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_codes() {
        use std::io::{Error, ErrorKind};
        assert_eq!(io_code(&Error::from_raw_os_error(32)), Some(code::IN_USE));
        assert_eq!(io_code(&Error::from_raw_os_error(112)), Some(code::DISK_FULL));
        assert_eq!(io_code(&Error::from(ErrorKind::NotFound)), Some(code::NOT_FOUND));
        assert_eq!(io_code(&Error::from_raw_os_error(5)), None);
        let chained = anyhow::Error::from(Error::from(ErrorKind::StorageFull)).context("extract");
        assert_eq!(io_cause(&chained).and_then(io_code), Some(code::DISK_FULL));
        assert!([code::NOT_FOUND, code::IN_USE, code::DISK_FULL, code::NAME_TAKEN].iter().all(|c| c.starts_with("err.")));
    }
}

/// Auto clips: whether each game is installed, set up and sending its state.
/// Async: it looks through the Steam libraries.
#[tauri::command]
pub async fn autoclip_status() -> CmdResult<crate::autoclip::Status> {
    blocking(crate::autoclip::status).await
}

/// Whether the in-game stats can show frame rates (see `fps::access`).
#[tauri::command]
pub async fn perf_access() -> CmdResult<crate::fps::Access> {
    blocking(crate::fps::access).await
}

/// Asks Windows (as administrator) to let this user read frame rates.
/// False when the user declined Windows' prompt.
#[tauri::command]
pub async fn grant_perf_access() -> CmdResult<bool> {
    let granted = tauri::async_runtime::spawn_blocking(crate::fps::grant).await.map_err(err)?.map_err(|e| {
        log::warn!("fps access: {e:#}");
        err(e)
    })?;
    crate::perf::sync();
    Ok(granted)
}
