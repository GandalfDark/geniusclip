//! Auto-update via tauri-plugin-updater (signed NSIS installers published
//! as GitHub releases).

use crate::state::AppState;
use serde::Serialize;
use std::time::Duration;
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

/// Downloads, stops capture (finalizing any recording) and runs the installer.
/// On Windows the installer replaces the app and restarts it.
pub async fn install(app: &AppHandle) -> anyhow::Result<()> {
    let Some(update) = app.updater()?.check().await? else { anyhow::bail!("no update available") };
    let handle = app.clone();
    let mut downloaded = 0usize;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk;
                let _ = handle.emit("update://progress", (downloaded, total));
            },
            || {},
        )
        .await?;
    app.state::<AppState>().engine.shutdown();
    if let Err(e) = update.install(bytes) {
        // Capture is already shut down and can't be started again in this
        // process: restart the current version rather than keep running
        // without recording anything.
        log::error!("update install failed: {e:#}");
    }
    app.restart();
}

pub fn spawn_periodic(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio_sleep(Duration::from_secs(15)).await;
        loop {
            if app.state::<AppState>().settings.read().auto_update {
                if let Err(e) = check(&app).await {
                    log::info!("update check failed: {e:#}");
                }
            }
            tokio_sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}
