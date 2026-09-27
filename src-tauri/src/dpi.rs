//! Keeps a window's WebView at its monitor's scale.
//!
//! With monitors at different scales (e.g. a 4K screen at 150% next to 100%
//! ones), a window can be created with one monitor's scale and shown on
//! another: Windows places new windows on the monitor the app was launched
//! from, then we center or move them. WebView2 can keep the first scale,
//! which draws the page enlarged and blurry. After a window shows up and
//! whenever its scale changes, the WebView's rasterization scale is set to
//! the window's actual DPI.

use tauri::{WebviewWindow, WindowEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForWindow;

/// Sets the WebView's scale to the window's monitor scale if they differ.
pub fn sync(w: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller3;
    use windows_core_webview2::Interface;
    let Ok(hwnd) = w.hwnd() else { return };
    let dpi = unsafe { GetDpiForWindow(HWND(hwnd.0 as _)) };
    if dpi == 0 {
        return;
    }
    let scale = dpi as f64 / 96.0;
    let res = w.with_webview(move |pw| unsafe {
        let controller = pw.controller();
        let _ = controller.NotifyParentWindowPositionChanged();
        let Ok(c3) = controller.cast::<ICoreWebView2Controller3>() else { return };
        let mut current = 0.0;
        if c3.RasterizationScale(&mut current).is_ok() && (current - scale).abs() > 0.01 {
            log::info!("webview scale {current} → {scale} (window dpi {dpi})");
            let _ = c3.SetRasterizationScale(scale);
        }
    });
    if let Err(e) = res {
        log::debug!("webview scale: {e}");
    }
}

/// Re-syncs on every scale change of the window, and once shortly after it
/// was created (it is shown by its page once rendered).
pub fn watch(w: &WebviewWindow) {
    let handle = w.clone();
    w.on_window_event(move |e| {
        if matches!(e, WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Focused(true)) {
            // Off the event handler: `hwnd()`/`with_webview` go through the UI thread.
            let w = handle.clone();
            std::thread::spawn(move || sync(&w));
        }
    });
    let w = w.clone();
    std::thread::spawn(move || {
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if w.is_visible().unwrap_or(false) {
                break;
            }
        }
        sync(&w);
    });
}
