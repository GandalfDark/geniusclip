//! Pauses capture while nobody can see the screen:
//! - display off: open audio streams would otherwise keep Windows from going
//!   to sleep on its own, and the GPU keeps encoding a dark screen;
//! - session locked: capture fails on the secure desktop anyway, and the
//!   pipeline would keep restarting.
//! - on battery, if the user chose so (laptops).
//! Capture resumes when all of that is over.

use crate::state::AppState;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
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

fn should_pause(app: &AppHandle) -> bool {
    let battery = ON_BATTERY.load(Ordering::Relaxed) && app.state::<AppState>().settings.read().pause_on_battery;
    DISPLAY_OFF.load(Ordering::Relaxed) || LOCKED.load(Ordering::Relaxed) || battery
}

/// Whether capture is (or is about to be) paused, without the engine lock.
pub fn is_paused() -> bool {
    APP.get().is_some_and(should_pause)
}

fn apply() {
    let Some(app) = APP.get() else { return };
    let app = app.clone();
    // Stopping/starting capture takes a moment; keep the message loop free.
    // The state is read inside the serialized section, so of several quick
    // events (lock, then unlock) the last one to run applies the latest
    // state instead of an older thread finishing last and re-pausing.
    std::thread::spawn(move || {
        let _one_at_a_time = APPLY.lock();
        if let Err(e) = app.state::<AppState>().engine.set_paused(should_pause(&app)) {
            log::warn!("resume capture: {e:#}");
        }
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
        // A hidden top-level window (never shown) receives both notifications.
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
