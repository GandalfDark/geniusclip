//! Pauses capture while nobody can see the screen:
//! - display off: open audio streams would otherwise keep Windows from going
//!   to sleep on its own, and the GPU keeps encoding a dark screen;
//! - session locked: capture fails on the secure desktop anyway, and the
//!   pipeline would keep restarting.
//! - on battery, if the user chose so (laptops);
//! - "Only in games", while no game is on the recorded monitor (the check
//!   itself is `actions::games_gate`).
//! Capture resumes when all of that is over.
//!
//! The same hidden window also hears display changes, which drop the cached
//! monitor list (see `monitors`).

use crate::state::AppState;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, RegisterPowerSettingNotification, POWERBROADCAST_SETTING, SYSTEM_POWER_STATUS};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_CONSOLE_DISPLAY_STATE};
use windows::Win32::UI::WindowsAndMessaging::*;

static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
static ON_BATTERY: AtomicBool = AtomicBool::new(false);
static WAITING_FOR_GAME: AtomicBool = AtomicBool::new(false);
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;

/// Whether the PC has a battery (the setting is only shown on laptops).
pub fn has_battery() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    // BatteryFlag 128 = no system battery, 255 = unknown.
    unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && s.BatteryFlag != 128 && s.BatteryFlag != 255
}

fn on_battery_now() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && s.ACLineStatus == 0
}

/// Serializes `apply` runs; each one reads the flags only once it holds this.
static APPLY: Mutex<()> = Mutex::new(());

/// Re-evaluates after the "pause on battery" setting changed.
pub fn refresh() {
    apply();
}

fn paused_by_system(app: &AppHandle) -> bool {
    let battery = ON_BATTERY.load(Ordering::Relaxed) && app.state::<AppState>().settings.read().pause_on_battery;
    DISPLAY_OFF.load(Ordering::Relaxed) || LOCKED.load(Ordering::Relaxed) || battery
}

fn should_pause(app: &AppHandle) -> bool {
    paused_by_system(app) || WAITING_FOR_GAME.load(Ordering::Relaxed)
}

/// Whether the display, the lock or the battery pause capture ("Only in
/// games" aside), without the engine lock.
pub fn system_paused() -> bool {
    APP.get().is_some_and(paused_by_system)
}

/// Whether "Only in games" holds capture until a game shows up.
pub fn waiting_for_game() -> bool {
    WAITING_FOR_GAME.load(Ordering::Relaxed)
}

/// Serializes changes of the "waiting for a game" flag with their events, so
/// the windows get them in the order they happened.
static WAITING_LOCK: Mutex<()> = Mutex::new(());

/// "Only in games": pauses capture (and empties the replay, which then holds
/// no game any more) or lets it run again.
pub fn set_waiting_for_game(waiting: bool) {
    if store_waiting(waiting) {
        apply();
    }
}

/// Sets the flag; true if it changed (the caller applies it).
fn store_waiting(waiting: bool) -> bool {
    let _in_order = WAITING_LOCK.lock();
    if WAITING_FOR_GAME.swap(waiting, Ordering::Relaxed) == waiting {
        return false;
    }
    log::info!("{}", if waiting { "no game: capture waits for one" } else { "game: capture runs" });
    if let Some(app) = APP.get() {
        // A recording made while waiting filled the replay with the desktop.
        if !waiting {
            app.state::<AppState>().engine.clear_replay();
        }
        let _ = app.emit("capture://waiting", waiting);
    }
    true
}

/// Applies the pause state now, on this thread (at startup and before replay
/// is turned on, so capture doesn't start only to stop a second later).
pub fn sync(app: &AppHandle) {
    let _one_at_a_time = APPLY.lock();
    set_paused(app);
}

fn set_paused(app: &AppHandle) {
    let st = app.state::<AppState>();
    if paused_by_system(app) {
        // Capture starts over after this pause, with a new replay: the game
        // seen before it gives no grace.
        crate::actions::forget_game();
    } else {
        // "Only in games" isn't checked during a pause: before capture
        // resumes, it looks at the monitor.
        store_waiting(crate::actions::games_waiting(app));
    }
    let paused = should_pause(app);
    if let Err(e) = st.engine.set_paused(paused) {
        log::warn!("resume capture: {e:#}");
    }
    if paused && WAITING_FOR_GAME.load(Ordering::Relaxed) {
        st.engine.clear_replay();
    }
}

fn apply() {
    let Some(app) = APP.get() else { return };
    let app = app.clone();
    // Game tracking stops while paused and starts again after.
    crate::wake_ticker();
    // Stopping/starting capture takes a moment; keep the message loop free.
    // The state is read inside the serialized section, so of several quick
    // events (lock, then unlock) the last one to run applies the latest
    // state instead of an older thread finishing last and re-pausing.
    std::thread::spawn(move || {
        let _one_at_a_time = APPLY.lock();
        set_paused(&app);
    });
}

extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_POWERBROADCAST if wp.0 == PBT_POWERSETTINGCHANGE as usize => {
                let s = &*(lp.0 as *const POWERBROADCAST_SETTING);
                if s.PowerSetting == GUID_ACDC_POWER_SOURCE {
                    // 0 = AC, 1 = battery, 2 = short-term (UPS).
                    let battery = s.Data[0] == 1;
                    if ON_BATTERY.swap(battery, Ordering::Relaxed) != battery {
                        apply();
                    }
                } else if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
                    // 0 = off, 1 = on, 2 = dimmed.
                    let off = s.Data[0] == 0;
                    if DISPLAY_OFF.swap(off, Ordering::Relaxed) != off {
                        apply();
                    }
                }
                LRESULT(1)
            }
            // Broadcast to every top-level window, hidden ones included, when
            // a display is added, removed or changes mode: the cached monitor
            // list is stale.
            WM_DISPLAYCHANGE => {
                crate::monitors::invalidate();
                DefWindowProcW(h, m, wp, lp)
            }
            WM_WTSSESSION_CHANGE => {
                match wp.0 {
                    WTS_SESSION_LOCK => {
                        LOCKED.store(true, Ordering::Relaxed);
                        apply();
                    }
                    WTS_SESSION_UNLOCK => {
                        LOCKED.store(false, Ordering::Relaxed);
                        apply();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }
}

pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let _ = std::thread::Builder::new().name("gc-power".into()).spawn(|| unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("GeniusClipPower");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        // A hidden top-level window (never shown) receives these notifications
        // and display changes (a message-only window would miss broadcasts).
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("GeniusClip power"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst.into()), None) else {
            log::warn!("power notifications unavailable");
            return;
        };
        if RegisterPowerSettingNotification(HANDLE(hwnd.0), &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE).is_err() {
            log::warn!("display state notifications unavailable");
        }
        ON_BATTERY.store(on_battery_now(), Ordering::Relaxed);
        let _ = RegisterPowerSettingNotification(HANDLE(hwnd.0), &GUID_ACDC_POWER_SOURCE, DEVICE_NOTIFY_WINDOW_HANDLE);
        apply();
        if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).is_err() {
            log::warn!("session notifications unavailable");
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}
