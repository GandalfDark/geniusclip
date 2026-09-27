use crate::i18n::t;
use crate::state::AppState;
use parking_lot::Mutex;
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

/// Menu items in order; a separator follows the ones in `SEPARATOR_AFTER`.
const ITEMS: [&str; 8] = ["open", "save_clip", "toggle_replay", "toggle_recording", "screenshot", "copy_last", "open_folder", "quit"];
const SEPARATOR_AFTER: [&str; 2] = ["open", "copy_last"];

/// The texts the tray should show now: one per `ITEMS` entry, and the tooltip.
fn texts(app: &AppHandle) -> (Vec<String>, String) {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    let lang = s.lang();
    let recording = st.engine.is_recording();
    let replay = t(lang, if s.replay_enabled { "replay_on" } else { "replay_off" });
    let items = ITEMS
        .iter()
        .map(|id| match *id {
            "save_clip" => label(t(lang, "save_clip"), &s.hotkeys.save_clip),
            "toggle_replay" => label(replay, &s.hotkeys.toggle_replay),
            "toggle_recording" => label(t(lang, if recording { "record_stop" } else { "record_start" }), &s.hotkeys.toggle_recording),
            "screenshot" => label(t(lang, "screenshot"), &s.hotkeys.screenshot),
            id => t(lang, id).to_string(),
        })
        .collect();
    (items, format!("GeniusClip — {replay}"))
}

/// The tray's items and what they show. Every change is a round trip to the
/// UI thread, so a refresh only touches what differs (usually nothing: it
/// runs after every saved clip).
struct Shown {
    items: Vec<(MenuItem<Wry>, String)>,
    tooltip: String,
}

/// Also keeps refreshes one at a time, so `Shown` matches the screen.
/// Not taken on the UI thread (refresh waits for it while holding this).
static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let (labels, tooltip) = texts(app);
    let menu = Menu::new(app)?;
    let mut items = Vec::new();
    for (id, text) in ITEMS.iter().zip(labels) {
        let item = MenuItem::with_id(app, *id, &text, true, None::<&str>)?;
        menu.append(&item)?;
        if SEPARATOR_AFTER.contains(id) {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        items.push((item, text));
    }
    TrayIconBuilder::with_id(ID)
        .icon(crate::brand::icon(app).or_else(|| app.default_window_icon().cloned()).expect("app icon"))
        .tooltip(&tooltip)
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
    *SHOWN.lock() = Some(Shown { items, tooltip });
    Ok(())
}

/// Brings the tray's texts (language, hotkeys, replay and recording state)
/// up to date. Not for the UI thread (see `SHOWN`).
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(ID) else { return };
    let (labels, tooltip) = texts(app);
    let mut shown = SHOWN.lock();
    let Some(shown) = shown.as_mut() else { return };
    for ((item, current), want) in shown.items.iter_mut().zip(labels) {
        if *current != want && item.set_text(&want).is_ok() {
            *current = want;
        }
    }
    if shown.tooltip != tooltip && tray.set_tooltip(Some(&tooltip)).is_ok() {
        shown.tooltip = tooltip;
    }
}
