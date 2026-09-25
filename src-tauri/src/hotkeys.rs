use crate::state::AppState;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

type Action = fn(&AppHandle);

/// (Re)registers all hotkeys from settings. Failures (e.g. a combo already
/// taken by another app such as NVIDIA's overlay) are stored for the UI.
pub fn register_all(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let hk = app.state::<AppState>().settings.read().hotkeys.clone();
    let list: [(&str, String, Action); 4] = [
        ("saveClip", hk.save_clip, crate::actions::save_clip),
        ("toggleReplay", hk.toggle_replay, crate::actions::toggle_replay),
        ("screenshot", hk.screenshot, crate::actions::screenshot),
        ("toggleRecording", hk.toggle_recording, crate::actions::toggle_recording),
    ];
    let mut errors = Vec::new();
    for (name, accel, action) in list {
        if accel.trim().is_empty() {
            continue;
        }
        let res = gs.on_shortcut(accel.as_str(), move |app, _sc, ev| {
            if ev.state == ShortcutState::Pressed {
                let app = app.clone();
                std::thread::spawn(move || action(&app));
            }
        });
        if let Err(e) = res {
            log::warn!("hotkey {name} ({accel}) failed: {e}");
            errors.push(name.to_string());
        }
    }
    *app.state::<AppState>().hotkey_errors.lock() = errors;
}
