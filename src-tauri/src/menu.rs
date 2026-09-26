//! In-game menu (default Alt+X): a transparent always-on-top window over the
//! game's monitor, like NVIDIA's overlay but without touching the game. The
//! page (src/routes/menu) animates in on `menu://open` and asks to be hidden
//! with the `menu_close` command after its closing animation.
//!
//! The window is excluded from screen capture, so clips and screenshots taken
//! while it is open show the game only.

use crate::overlay::{self, Toast};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};

pub const LABEL: &str = "menu";

static OPEN: AtomicBool = AtomicBool::new(false);
/// The window that was in front (the game), to give focus back on close.
static PREV: Mutex<isize> = Mutex::new(0);

pub fn toggle(app: &AppHandle) {
    if OPEN.load(Ordering::Relaxed) {
        // Let the page play its closing animation; it then calls menu_close.
        let _ = app.emit_to(LABEL, "menu://close", ());
    } else {
        open(app);
    }
}

fn open(app: &AppHandle) {
    // A game in true exclusive fullscreen would minimize if another window
    // took focus; say so instead.
    if unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN) {
        overlay::toast(app, Toast::simple("menu-unavailable"));
        return;
    }
    let fg = unsafe { GetForegroundWindow() };
    *PREV.lock() = fg.0 as isize;
    let win = match app.get_webview_window(LABEL) {
        Some(w) => w,
        None => match create(app) {
            Ok(w) => w,
            Err(e) => {
                log::error!("menu window: {e}");
                return;
            }
        },
    };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let r = unsafe {
        let _ = GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTOPRIMARY), &mut info);
        info.rcMonitor
    };
    let _ = win.set_position(PhysicalPosition::new(r.left, r.top));
    let _ = win.set_size(PhysicalSize::new((r.right - r.left) as u32, (r.bottom - r.top) as u32));
    let _ = win.show();
    let _ = win.set_focus();
    OPEN.store(true, Ordering::Relaxed);
    let _ = app.emit_to(LABEL, "menu://open", ());
}

fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("menu".into()))
        .title("GeniusClip")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .disable_drag_drop_handler()
        .build()?;
    if let Ok(h) = win.hwnd() {
        unsafe {
            let _ = SetWindowDisplayAffinity(HWND(h.0 as _), WDA_EXCLUDEFROMCAPTURE);
        }
    }
    // Clicking into another window (or Alt+Tab) closes the menu.
    let handle = app.clone();
    win.on_window_event(move |e| {
        if let tauri::WindowEvent::Focused(false) = e {
            if OPEN.load(Ordering::Relaxed) {
                hide(&handle, false);
            }
        }
    });
    Ok(win)
}

/// Hides the menu; with `restore_focus` the game gets keyboard and mouse back.
pub fn hide(app: &AppHandle, restore_focus: bool) {
    OPEN.store(false, Ordering::Relaxed);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    let prev = *PREV.lock();
    if restore_focus && prev != 0 {
        unsafe {
            let _ = SetForegroundWindow(HWND(prev as _));
        }
    }
}
