//! Click-through, never-focused notification window shown over games.
//!
//! The window is only visible while a toast is on screen: a permanently
//! visible topmost window would force DWM composition over fullscreen games
//! and cost them independent-flip (latency).

use crate::state::AppState;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;

const LABEL: &str = "overlay";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 110.0;
const MARGIN: f64 = 20.0;
const SHOW_MS: u64 = 3200;

static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    /// clip | recording | recording-start | screenshot | replay-on | replay-off | replay-off-hint | error
    pub kind: String,
    pub game: String,
    pub seconds: f64,
    pub message: String,
    pub lang: String,
    pub accent: String,
    pub corner: String,
}

impl Toast {
    pub fn simple(kind: &str) -> Toast {
        Toast { kind: kind.into(), ..Default::default() }
    }
    pub fn error(msg: &str) -> Toast {
        Toast { kind: "error".into(), message: msg.into(), ..Default::default() }
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay".into()))
        .title("GeniusClip overlay")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .build()?;
    w.set_ignore_cursor_events(true)?;
    if let Ok(hwnd) = w.hwnd() {
        unsafe {
            let h = HWND(hwnd.0);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0) as isize);
        }
    }
    Ok(())
}

/// Physical rect of the monitor being captured.
fn target_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32, f64)> {
    let st = app.state::<AppState>();
    let want = st.settings.read().engine.monitor.clone();
    let mons = geniusclip_engine::list_monitors().ok()?;
    let m = want
        .as_deref()
        .and_then(|id| mons.iter().find(|m| m.id == id))
        .or_else(|| mons.iter().find(|m| m.primary))
        .or(mons.first())?;
    let scale = app
        .available_monitors()
        .ok()
        .and_then(|ms| ms.into_iter().find(|tm| tm.position().x == m.x && tm.position().y == m.y).map(|tm| tm.scale_factor()))
        .unwrap_or(1.0);
    Some((m.x, m.y, m.width as i32, m.height as i32, scale))
}

pub fn toast(app: &AppHandle, mut toast: Toast) {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    if !s.overlay.enabled && toast.kind != "error" {
        return;
    }
    toast.lang = s.lang().into();
    toast.accent = s.accent.clone();
    toast.corner = s.overlay.corner.clone();
    if s.overlay.sound && matches!(toast.kind.as_str(), "clip" | "recording" | "screenshot" | "recording-start") {
        crate::sound::play_saved();
    }

    let Some(w) = app.get_webview_window(LABEL) else { return };
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = app.emit_to(LABEL, "overlay://toast", &toast);

    if let (Some((mx, my, mw, mh, scale)), Ok(hwnd)) = (target_rect(app), w.hwnd()) {
        let (w_px, h_px) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
        let m = (MARGIN * scale) as i32;
        let x = if s.overlay.corner.ends_with("left") { mx + m } else { mx + mw - w_px - m };
        let y = if s.overlay.corner.starts_with("bottom") { my + mh - h_px - m * 3 } else { my + m };
        let _ = w.set_size(PhysicalSize::new(w_px as u32, h_px as u32));
        let _ = w.set_position(PhysicalPosition::new(x, y));
        unsafe {
            let h = HWND(hwnd.0);
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), x, y, w_px, h_px, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
    }

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SHOW_MS));
        if GENERATION.load(Ordering::SeqCst) == gen {
            if let Some(w) = app.get_webview_window(LABEL) {
                let _ = w.hide();
            }
        }
    });
}
