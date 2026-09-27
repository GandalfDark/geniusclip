//! Global hotkeys via RegisterHotKey on a dedicated thread.
//!
//! Implemented directly instead of using tauri-plugin-global-shortcut: its
//! `global-hotkey` backend derives Windows hotkey ids as `mods << 16 | key`,
//! but Windows keeps only 16 bits of the id, so combos that differ only in
//! modifiers (Alt+F8 vs Alt+Shift+F8) collide and fire the wrong action.

use crate::state::AppState;
use parking_lot::Mutex;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY};

type Action = fn(&AppHandle);

const WM_REREGISTER: u32 = WM_APP + 1;

struct Worker {
    thread_id: u32,
    /// Pending registration request: (accelerators, reply channel).
    request: Mutex<Option<(Vec<String>, Sender<Vec<usize>>)>>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();

const ACTIONS: [(&str, Action); 6] = [
    ("saveClip", crate::actions::save_clip),
    ("toggleReplay", crate::actions::toggle_replay),
    ("screenshot", crate::actions::screenshot),
    ("toggleRecording", crate::actions::toggle_recording),
    ("saveShort", crate::actions::save_short_clip),
    ("toggleMenu", crate::menu::toggle),
];

fn worker(app: &AppHandle) -> &'static Worker {
    WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        let app = app.clone();
        std::thread::Builder::new()
            .name("gc-hotkeys".into())
            .spawn(move || unsafe {
                // Create this thread's message queue before announcing it.
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                let _ = tx.send(GetCurrentThreadId());
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    match msg.message {
                        WM_HOTKEY => {
                            let id = msg.wParam.0;
                            if let Some((_, action)) = ACTIONS.get(id.wrapping_sub(1)) {
                                let (app, action) = (app.clone(), *action);
                                std::thread::spawn(move || action(&app));
                            }
                        }
                        WM_REREGISTER => {
                            if let Some((accels, reply)) = WORKER.get().and_then(|w| w.request.lock().take()) {
                                let _ = reply.send(register(&accels));
                            }
                        }
                        _ => {}
                    }
                }
            })
            .expect("spawn hotkey thread");
        Worker { thread_id: rx.recv().unwrap_or(0), request: Mutex::new(None) }
    })
}

/// Registers the set on the calling (hotkey) thread; returns indexes that failed.
unsafe fn register(accels: &[String]) -> Vec<usize> {
    // Everything is released first: moving a combo from a later action to
    // an earlier one would otherwise fail while the later id still holds it.
    for i in 0..ACTIONS.len() {
        let _ = UnregisterHotKey(None, i as i32 + 1);
    }
    let mut failed = Vec::new();
    for (i, accel) in accels.iter().enumerate() {
        if accel.trim().is_empty() {
            continue;
        }
        let ok = parse(accel).is_some_and(|(mods, vk)| RegisterHotKey(None, i as i32 + 1, mods | MOD_NOREPEAT, vk).is_ok());
        if !ok {
            log::warn!("hotkey {} ({accel}) could not be registered", ACTIONS[i].0);
            failed.push(i);
        }
    }
    failed
}

/// One registration at a time: the worker has a single request slot, and
/// each run reads the settings (and suspension) once it holds this.
static REGISTER: Mutex<()> = Mutex::new(());

/// Hotkeys are released while the UI records a new combo.
struct Suspension {
    on: bool,
    /// Bumped on every request, so only the latest one registers and a stale
    /// safety timer does nothing.
    generation: u64,
}

static SUSPENSION: Mutex<Suspension> = Mutex::new(Suspension { on: false, generation: 0 });
/// Hotkeys come back on their own if the UI never resumes them.
const SUSPEND_MAX: Duration = Duration::from_secs(60);

/// (Re)registers all hotkeys from settings. Failures (a combo already taken
/// by another app, e.g. a GPU vendor overlay) are stored for the UI.
pub fn register_all(app: &AppHandle) {
    let _one_at_a_time = REGISTER.lock();
    register_locked(app);
}

/// `register_all` for a caller holding `REGISTER`.
fn register_locked(app: &AppHandle) {
    let suspended = is_suspended();
    let accels = if suspended {
        vec![String::new(); ACTIONS.len()]
    } else {
        let hk = app.state::<AppState>().settings.read().hotkeys.clone();
        vec![hk.save_clip, hk.toggle_replay, hk.screenshot, hk.toggle_recording, hk.save_short, hk.toggle_menu]
    };
    let w = worker(app);
    let (tx, rx) = channel();
    *w.request.lock() = Some((accels, tx));
    unsafe {
        let _ = PostThreadMessageW(w.thread_id, WM_REREGISTER, Default::default(), Default::default());
    }
    let failed = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    // While suspended nothing is registered, which says nothing about conflicts.
    if !suspended {
        *app.state::<AppState>().hotkey_errors.lock() = failed.into_iter().map(|i| ACTIONS[i].0.to_string()).collect();
    }
}

pub fn is_suspended() -> bool {
    SUSPENSION.lock().on
}

/// Asks for all hotkeys to be released (`true`) so the UI can record a combo
/// that is already in use instead of triggering it, or registered again.
/// Only records the wish and returns its generation; `apply_suspended` does
/// the (slow, cross-thread) registration. Call it in request order.
pub fn request_suspended(app: &AppHandle, on: bool) -> u64 {
    let generation = {
        let mut s = SUSPENSION.lock();
        s.on = on;
        s.generation += 1;
        s.generation
    };
    if on {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SUSPEND_MAX);
            // Resumes only if nothing was asked since: counts as a new request.
            let resumed = {
                let mut s = SUSPENSION.lock();
                (s.generation == generation && s.on).then(|| {
                    s.on = false;
                    s.generation += 1;
                    s.generation
                })
            };
            if let Some(generation) = resumed {
                log::warn!("hotkeys were suspended too long; registering them again");
                apply_suspended(&app, generation);
            }
        });
    }
    generation
}

/// Registers or releases the hotkeys for request `generation`. A newer
/// request has its own call coming, so an older one does nothing: requests
/// applied out of order can't leave the hotkeys in an older state.
pub fn apply_suspended(app: &AppHandle, generation: u64) {
    let _one_at_a_time = REGISTER.lock();
    if SUSPENSION.lock().generation == generation {
        register_locked(app);
    }
}

/// Parses "Alt+Shift+F8" / "Control+KeyK" (KeyboardEvent.code names).
fn parse(accel: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut vk = None;
    for part in accel.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "control" | "ctrl" | "commandorcontrol" => mods |= MOD_CONTROL,
            "super" | "win" | "meta" => mods |= MOD_WIN,
            _ => vk = Some(code_to_vk(part)?),
        }
    }
    Some((mods, vk?))
}

fn code_to_vk(code: &str) -> Option<u32> {
    let c = code.strip_prefix("Key").filter(|s| s.len() == 1).unwrap_or(code);
    if c.len() == 1 {
        let ch = c.chars().next()?.to_ascii_uppercase();
        if ch.is_ascii_alphanumeric() {
            return Some(ch as u32);
        }
    }
    if let Some(d) = code.strip_prefix("Digit").and_then(|d| d.parse::<u32>().ok()) {
        return Some(0x30 + d);
    }
    if let Some(n) = code.strip_prefix("Numpad").and_then(|d| d.parse::<u32>().ok()) {
        return Some(VK_NUMPAD0.0 as u32 + n);
    }
    if let Some(n) = code.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(VK_F1.0 as u32 + n - 1);
    }
    let vk = match code {
        "Space" => VK_SPACE,
        "Tab" => VK_TAB,
        "Enter" => VK_RETURN,
        "Insert" => VK_INSERT,
        "Delete" => VK_DELETE,
        "Home" => VK_HOME,
        "End" => VK_END,
        "PageUp" => VK_PRIOR,
        "PageDown" => VK_NEXT,
        "ArrowUp" => VK_UP,
        "ArrowDown" => VK_DOWN,
        "ArrowLeft" => VK_LEFT,
        "ArrowRight" => VK_RIGHT,
        "PrintScreen" => VK_SNAPSHOT,
        "Pause" => VK_PAUSE,
        "ScrollLock" => VK_SCROLL,
        "Backquote" => VK_OEM_3,
        "Minus" => VK_OEM_MINUS,
        "Equal" => VK_OEM_PLUS,
        "BracketLeft" => VK_OEM_4,
        "BracketRight" => VK_OEM_6,
        "Backslash" => VK_OEM_5,
        "Semicolon" => VK_OEM_1,
        "Quote" => VK_OEM_7,
        "Comma" => VK_OEM_COMMA,
        "Period" => VK_OEM_PERIOD,
        "Slash" => VK_OEM_2,
        "NumpadAdd" => VK_ADD,
        "NumpadSubtract" => VK_SUBTRACT,
        "NumpadMultiply" => VK_MULTIPLY,
        "NumpadDivide" => VK_DIVIDE,
        "NumpadDecimal" => VK_DECIMAL,
        _ => return None,
    };
    Some(vk.0 as u32)
}
