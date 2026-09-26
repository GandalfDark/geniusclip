//! Pauses capture while nobody can see the screen:
//! - display off: open audio streams would otherwise keep Windows from going
//!   to sleep on its own, and the GPU keeps encoding a dark screen;
//! - session locked: capture fails on the secure desktop anyway, and the
//!   pipeline would keep restarting.
//! Capture resumes when the display is back on and the session unlocked.

use crate::state::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::UI::WindowsAndMessaging::*;

static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;

fn apply() {
    let Some(app) = APP.get() else { return };
    let paused = DISPLAY_OFF.load(Ordering::Relaxed) || LOCKED.load(Ordering::Relaxed);
    let app = app.clone();
    // Stopping/starting capture takes a moment; keep the message loop free.
    std::thread::spawn(move || {
        if let Err(e) = app.state::<AppState>().engine.set_paused(paused) {
            log::warn!("resume capture: {e:#}");
        }
    });
}

extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_POWERBROADCAST if wp.0 == PBT_POWERSETTINGCHANGE as usize => {
                let s = &*(lp.0 as *const POWERBROADCAST_SETTING);
                if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
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
