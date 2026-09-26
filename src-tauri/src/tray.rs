use crate::i18n::t;
use crate::state::AppState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

pub const ID: &str = "main";

fn label(text: &str, hotkey: &str) -> String {
    if hotkey.is_empty() {
        text.to_string()
    } else {
        format!("{text}\t{hotkey}")
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    let lang = s.lang();
    let recording = st.engine.is_recording();
    let item = |id: &str, text: String| MenuItem::with_id(app, id, text, true, None::<&str>);
    Menu::with_items(
        app,
        &[
            &item("open", t(lang, "open").into())?,
            &PredefinedMenuItem::separator(app)?,
            &item("save_clip", label(t(lang, "save_clip"), &s.hotkeys.save_clip))?,
            &item(
                "toggle_replay",
                label(t(lang, if s.replay_enabled { "replay_on" } else { "replay_off" }), &s.hotkeys.toggle_replay),
            )?,
            &item(
                "toggle_recording",
                label(t(lang, if recording { "record_stop" } else { "record_start" }), &s.hotkeys.toggle_recording),
            )?,
            &item("screenshot", label(t(lang, "screenshot"), &s.hotkeys.screenshot))?,
            &item("copy_last", t(lang, "copy_last").into())?,
            &PredefinedMenuItem::separator(app)?,
            &item("open_folder", t(lang, "open_folder").into())?,
            &item("quit", t(lang, "quit").into())?,
        ],
    )
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(ID)
        .icon(crate::brand::icon(app).or_else(|| app.default_window_icon().cloned()).expect("app icon"))
        .tooltip("GeniusClip")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| {
            let app = app.clone();
            let id = ev.id().0.clone();
            std::thread::spawn(move || match id.as_str() {
                "open" => crate::show_main(&app),
                "save_clip" => crate::actions::save_clip(&app),
                "toggle_replay" => crate::actions::toggle_replay(&app),
                "toggle_recording" => crate::actions::toggle_recording(&app),
                "screenshot" => crate::actions::screenshot(&app),
                "copy_last" => crate::actions::copy_last_clip(&app),
                "open_folder" => {
                    let dir = app.state::<AppState>().settings.read().clips_dir.clone();
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = tauri_plugin_opener::open_path(&dir, None::<&str>);
                }
                "quit" => app.exit(0),
                _ => {}
            });
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                crate::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    refresh(app);
    Ok(())
}

pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(ID) else { return };
    if let Ok(menu) = build_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
    let st = app.state::<AppState>();
    let lang = st.settings.read().lang();
    let on = st.settings.read().replay_enabled;
    let _ = tray.set_tooltip(Some(format!("GeniusClip — {}", t(lang, if on { "replay_on" } else { "replay_off" }))));
}
