//! App icon in the current accent color (tray + main window). The icons are
//! prepared from assets/logo/*.png by scripts/make-icon.mjs.

use crate::state::AppState;
use tauri::image::Image;
use tauri::{AppHandle, Manager};

fn png(accent: &str) -> &'static [u8] {
    match accent {
        "red" => include_bytes!("../icons/accent/red.png"),
        "lime" => include_bytes!("../icons/accent/lime.png"),
        "cyan" => include_bytes!("../icons/accent/cyan.png"),
        "amber" => include_bytes!("../icons/accent/amber.png"),
        "mono" => include_bytes!("../icons/accent/mono.png"),
        _ => include_bytes!("../icons/accent/violet.png"),
    }
}

pub fn icon(app: &AppHandle) -> Option<Image<'static>> {
    let accent = app.state::<AppState>().settings.read().accent.clone();
    Image::from_bytes(png(&accent)).map_err(|e| log::warn!("icon: {e}")).ok()
}

/// Re-applies the accent-colored icon to the tray and the main window.
pub fn apply(app: &AppHandle) {
    let Some(img) = icon(app) else { return };
    if let Some(tray) = app.tray_by_id(crate::tray::ID) {
        let _ = tray.set_icon(Some(img.clone()));
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_icon(img);
    }
}
