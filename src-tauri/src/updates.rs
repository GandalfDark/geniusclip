//! Auto-update via tauri-plugin-updater (signed NSIS installers published
//! as GitHub releases).

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// The notes of an installed update, shown once the new version runs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    pub version: String,
    pub notes: String,
}

pub async fn check(app: &AppHandle) -> anyhow::Result<Option<UpdateInfo>> {
    let update = app.updater()?.check().await?;
    let info = update.map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    *app.state::<AppState>().update.lock() = info.clone();
    if let Some(i) = &info {
        let _ = app.emit("update://available", i);
    }
    Ok(info)
}

/// `update://progress` events at most this often: download chunks arrive
/// every few KB, and each event re-renders the UI.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Downloads, stops capture (finalizing any recording) and runs the installer.
/// On Windows the installer replaces the app and restarts it.
pub async fn install(app: &AppHandle) -> anyhow::Result<()> {
    let Some(update) = app.updater()?.check().await? else { anyhow::bail!("no update available") };
    let handle = app.clone();
    let mut downloaded = 0usize;
    let mut last_sent: Option<Instant> = None;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk;
                let done = total.is_some_and(|t| downloaded as u64 >= t);
                if done || last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    last_sent = Some(Instant::now());
                    let _ = handle.emit("update://progress", (downloaded, total));
                }
            },
            || {},
        )
        .await?;
    // The installer (or the restart below) relaunches with this process's
    // arguments; see `take_show_after_update`.
    mark_show_after_update(app);
    save_whats_new(app, &WhatsNew { version: update.version.clone(), notes: update.body.clone().unwrap_or_default() });
    app.state::<AppState>().engine.shutdown();
    if let Err(e) = update.install(bytes) {
        // Capture is already shut down and can't be started again in this
        // process: restart the current version rather than keep running
        // without recording anything.
        log::error!("update install failed: {e:#}");
    }
    app.restart();
}

/// A marker that the user started an update from the main window.
fn show_after_update_marker(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok().map(|d| d.join("show-after-update"))
}

fn mark_show_after_update(app: &AppHandle) {
    let Some(p) = show_after_update_marker(app) else { return };
    let res = p.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&p, b""));
    if let Err(e) = res {
        log::warn!("update marker: {e}");
    }
}

/// The marker only counts this long after the install began, so an install
/// that never relaunched doesn't open the window at some later boot.
const MARKER_FRESH: Duration = Duration::from_secs(10 * 60);

/// This start follows an update the user installed: show the main window
/// even with `--autostart`. The updater relaunches the app with the
/// arguments it was running with (tauri-plugin-updater passes them to the
/// NSIS installer as `/ARGS`), so an app started with Windows would come
/// back hidden in the tray. Consumes the marker.
pub fn take_show_after_update(app: &AppHandle) -> bool {
    let Some(p) = show_after_update_marker(app) else { return false };
    let Ok(written) = std::fs::metadata(&p).and_then(|m| m.modified()) else { return false };
    let _ = std::fs::remove_file(&p);
    SystemTime::now().duration_since(written).is_ok_and(|age| age < MARKER_FRESH)
}

fn whats_new_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("whats-new.json"))
}

fn save_whats_new(app: &AppHandle, notes: &WhatsNew) {
    let Some(p) = whats_new_file(app) else { return };
    let res = p
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| serde_json::to_vec(notes).map_err(std::io::Error::from))
        .and_then(|b| std::fs::write(&p, b));
    if let Err(e) = res {
        log::warn!("whats-new.json: {e}");
    }
}

/// The notes saved before the running version was installed. Notes of any
/// other version (an install that failed and restarted the old one) are
/// ignored rather than deleted: the old version may still be on its way
/// out while the installer runs.
pub fn whats_new(app: &AppHandle) -> Option<WhatsNew> {
    let notes: WhatsNew = serde_json::from_slice(&std::fs::read(whats_new_file(app)?).ok()?).ok()?;
    same_version(&notes.version, &app.package_info().version.to_string()).then_some(notes)
}

fn same_version(a: &str, b: &str) -> bool {
    a.trim().trim_start_matches('v') == b.trim().trim_start_matches('v')
}

pub fn dismiss_whats_new(app: &AppHandle) -> std::io::Result<()> {
    let Some(p) = whats_new_file(app) else { return Ok(()) };
    match std::fs::remove_file(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        res => res,
    }
}

pub fn spawn_periodic(app: AppHandle) {
    // A plain sleeping thread: sleeping in spawn_blocking held a thread of
    // the async runtime's blocking pool for six hours at a time.
    let res = std::thread::Builder::new().name("gc-updates".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        loop {
            if app.state::<AppState>().settings.read().auto_update {
                if let Err(e) = tauri::async_runtime::block_on(check(&app)) {
                    log::info!("update check failed: {e:#}");
                }
            }
            std::thread::sleep(Duration::from_secs(6 * 3600));
        }
    });
    if let Err(e) = res {
        log::warn!("update checks: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_without_a_v_prefix() {
        assert!(same_version("v0.1.4", "0.1.4"));
        assert!(same_version("0.1.4", "0.1.4"));
        assert!(!same_version("0.1.4", "0.1.3"));
        let json = serde_json::to_string(&WhatsNew { version: "0.1.4".into(), notes: "- Faster".into() }).unwrap();
        assert_eq!(json, r#"{"version":"0.1.4","notes":"- Faster"}"#);
    }
}
