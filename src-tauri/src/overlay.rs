//! Native on-screen notifications ("Clip saved") drawn with Direct2D into a
//! per-pixel-alpha layered window.
//!
//! Native instead of a WebView so it is instant and always ready (no page
//! load), costs almost no memory, and is click-through and never focused.
//! The window is only shown while a toast is animating: a permanently visible
//! topmost window would force DWM composition over fullscreen games.

use crate::i18n::t;
use crate::state::AppState;
use parking_lot::Mutex;
use std::sync::mpsc::channel;
use std::sync::OnceLock;
use std::time::Instant;
use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_SHOW_TOAST: u32 = WM_APP + 2;
const ENTER_MS: f32 = 220.0;
const LEAVE_AT_MS: f32 = 2800.0;
const LEAVE_MS: f32 = 240.0;
const TOTAL_MS: u128 = 3060;
/// How long a toast stays on screen; a newer one replaces it right away.
pub const SHOWN_FOR: std::time::Duration = std::time::Duration::from_millis(TOTAL_MS as u64);

const REC_RED: u32 = 0xff4f4f;
const MUTED: u32 = 0x686872;
const WARNING: u32 = 0xffb020;

#[derive(Clone, Default)]
pub struct Toast {
    /// clip | recording | recording-start | screenshot | replay-on | replay-off | replay-off-hint | disk-low | error
    pub kind: String,
    pub game: String,
    pub seconds: f64,
    pub message: String,
    /// disk-low: free MB and the drive.
    pub disk: Option<(u64, String)>,
}

impl Toast {
    pub fn simple(kind: &str) -> Toast {
        Toast { kind: kind.into(), ..Default::default() }
    }
    pub fn error(msg: &str) -> Toast {
        Toast { kind: "error".into(), message: msg.into(), ..Default::default() }
    }
    pub fn disk_low(free_mb: u64, drive: &str) -> Toast {
        Toast { kind: "disk-low".into(), disk: Some((free_mb, drive.into())), ..Default::default() }
    }
}

/// Everything the render thread needs, resolved on the caller's side.
struct Spec {
    title: String,
    sub: String,
    bar: D2D1_COLOR_F,
    right: bool,
    bottom: bool,
    mon: RECT,
}

struct Worker {
    thread_id: u32,
    /// The toast window; also serves as clipboard owner for the app.
    hwnd: isize,
    pending: Mutex<Option<Spec>>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();

fn rgb(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

/// Must match src/lib/accents.ts.
pub(crate) fn accent(id: &str) -> u32 {
    match id {
        "red" => 0xff5a36,
        "lime" => 0xc6f432,
        "cyan" => 0x38d2f0,
        "amber" => 0xffb020,
        "mono" => 0xe6e6ea,
        _ => 0x9580ff,
    }
}

fn fmt_duration(sec: f64) -> String {
    let s = sec.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn create(_app: &AppHandle) -> tauri::Result<()> {
    WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("gc-overlay".into())
            .spawn(move || unsafe { run(tx) })
            .expect("spawn overlay thread");
        let (thread_id, hwnd) = rx.recv().unwrap_or((0, 0));
        Worker { thread_id, hwnd, pending: Mutex::new(None) }
    });
    Ok(())
}

/// A window owned by this process that always exists (the toast window).
pub fn window() -> Option<HWND> {
    WORKER.get().filter(|w| w.hwnd != 0).map(|w| HWND(w.hwnd as *mut _))
}

pub fn toast(app: &AppHandle, toast: Toast) {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    if !s.overlay.enabled && toast.kind != "error" {
        return;
    }
    if s.overlay.sound && matches!(toast.kind.as_str(), "clip" | "recording" | "screenshot" | "recording-start") {
        crate::sound::play_saved();
    }
    let lang = s.lang();
    let title = t(lang, &format!("ov.{}", toast.kind)).to_string();
    let sub = match toast.kind.as_str() {
        "error" => toast.message.clone(),
        "replay-off-hint" => t(lang, "ov.replay-off-hint.sub").to_string(),
        "already-saved" => t(lang, "ov.already-saved.sub").to_string(),
        "replay-wait" => t(lang, "ov.replay-wait.sub").to_string(),
        "replay-paused" => t(lang, "ov.replay-paused.sub").to_string(),
        "copied" => t(lang, "ov.copied.sub").to_string(),
        "menu-unavailable" => t(lang, "ov.menu-unavailable.sub").to_string(),
        "disk-low" => {
            let (free, drive) = toast.disk.clone().unwrap_or_default();
            t(lang, "ov.disk-low.sub").replace("{free}", &crate::disk::fmt_size(lang, free)).replace("{drive}", &drive)
        }
        _ => {
            let mut parts = Vec::new();
            if toast.seconds > 0.0 {
                parts.push(fmt_duration(toast.seconds));
            }
            if !toast.game.is_empty() {
                parts.push(toast.game.clone());
            }
            parts.join(" · ")
        }
    };
    let bar = match toast.kind.as_str() {
        "error" | "recording-start" => REC_RED,
        "replay-off" | "replay-off-hint" | "already-saved" | "replay-wait" | "replay-paused" => MUTED,
        "disk-low" => WARNING,
        _ => accent(&s.accent),
    };

    // Cached: toasts show up in game, where enumerating displays can hitch it.
    let mons = crate::monitors::list();
    let mon = crate::monitors::pick(&mons, s.engine.monitor.as_deref())
        .map(|m| RECT { left: m.x, top: m.y, right: m.x + m.width as i32, bottom: m.y + m.height as i32 })
        .unwrap_or(RECT { left: 0, top: 0, right: 1920, bottom: 1080 });

    let spec = Spec {
        title,
        sub,
        bar: rgb(bar, 1.0),
        right: !s.overlay.corner.ends_with("left"),
        bottom: s.overlay.corner.starts_with("bottom"),
        mon,
    };
    if let Some(w) = WORKER.get() {
        *w.pending.lock() = Some(spec);
        unsafe {
            let _ = PostThreadMessageW(w.thread_id, WM_SHOW_TOAST, WPARAM(0), LPARAM(0));
        }
    }
}

// ---------------------------------------------------------------------------
// Render thread

struct Renderer {
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    scale: f32,
    title_fmt: Option<IDWriteTextFormat>,
    sub_fmt: Option<IDWriteTextFormat>,
    mem_dc: HDC,
    dib: HBITMAP,
    size: (i32, i32),
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, wp, lp)
}

unsafe fn run(ready: std::sync::mpsc::Sender<(u32, isize)>) {
    let mut msg = MSG::default();
    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);

    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let class = w!("GeniusClipToast");
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        class,
        w!("GeniusClip toast"),
        WS_POPUP,
        0,
        0,
        1,
        1,
        None,
        None,
        Some(hinst.into()),
        None,
    )
    .unwrap_or_default();
    // Toasts never end up in clips or screenshots.
    let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);

    let mut renderer = Renderer::new().map_err(|e| log::error!("overlay renderer: {e}")).ok();
    let _ = ready.send((GetCurrentThreadId(), hwnd.0 as isize));

    let mut active: Option<(Spec, Instant)> = None;
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        match msg.message {
            WM_SHOW_TOAST => {
                if let Some(spec) = WORKER.get().and_then(|w| w.pending.lock().take()) {
                    active = Some((spec, Instant::now()));
                    SetTimer(Some(hwnd), 1, 15, None);
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
                    if renderer.is_none() {
                        // Creation failed before; the GPU may be back.
                        renderer = Renderer::new().ok();
                    }
                    if let Some((spec, t0)) = active.as_ref() {
                        if let Err(e) = draw(&mut renderer, hwnd, spec, t0.elapsed().as_millis() as f32) {
                            log::warn!("overlay draw: {e}");
                        }
                    }
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
            WM_TIMER => {
                let done = match active.as_ref() {
                    Some((spec, t0)) if renderer.is_some() && t0.elapsed().as_millis() < TOTAL_MS => {
                        let _ = draw(&mut renderer, hwnd, spec, t0.elapsed().as_millis() as f32);
                        false
                    }
                    _ => true,
                };
                if done {
                    let _ = KillTimer(Some(hwnd), 1);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    active = None;
                }
            }
            _ => {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// Draws a frame. After a GPU reset or driver update the render target is
/// lost for good (D2DERR_RECREATE_TARGET): the renderer is rebuilt, and the
/// next frame draws with the new one.
unsafe fn draw(renderer: &mut Option<Renderer>, hwnd: HWND, spec: &Spec, t_ms: f32) -> windows::core::Result<()> {
    let Some(r) = renderer.as_mut() else { return Ok(()) };
    let res = r.frame(hwnd, spec, t_ms);
    if res.as_ref().is_err_and(|e| e.code() == D2DERR_RECREATE_TARGET) {
        log::info!("overlay: render target lost; recreating");
        *renderer = Renderer::new().map_err(|e| log::error!("overlay renderer: {e}")).ok();
        return Ok(());
    }
    res
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

fn rr(l: f32, t: f32, r: f32, b: f32, rad: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: l, top: t, right: r, bottom: b }, radiusX: rad, radiusY: rad }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Renderer {
    unsafe fn new() -> windows::core::Result<Self> {
        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d.CreateDCRenderTarget(&props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let mem_dc = CreateCompatibleDC(None);
        Ok(Renderer { dw, rt, scale: 0.0, title_fmt: None, sub_fmt: None, mem_dc, dib: HBITMAP::default(), size: (0, 0) })
    }

    unsafe fn format(&self, family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32) -> windows::core::Result<IDWriteTextFormat> {
        let f = self.dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        let sign = self.dw.CreateEllipsisTrimmingSign(&f)?;
        let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        f.SetTrimming(&trim, &sign)?;
        Ok(f)
    }

    unsafe fn ensure(&mut self, scale: f32, w: i32, h: i32) -> windows::core::Result<()> {
        if (self.scale - scale).abs() > f32::EPSILON || self.title_fmt.is_none() {
            self.title_fmt = Some(self.format(w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.0 * scale)?);
            // Timecodes in a monospace face, like the app.
            self.sub_fmt = Some(self.format(w!("Consolas"), DWRITE_FONT_WEIGHT_NORMAL, 12.5 * scale)?);
            self.scale = scale;
        }
        if self.size != (w, h) {
            if !self.dib.is_invalid() {
                let _ = DeleteObject(self.dib.into());
            }
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            self.dib = CreateDIBSection(Some(self.mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            SelectObject(self.mem_dc, self.dib.into());
            self.size = (w, h);
        }
        Ok(())
    }

    unsafe fn frame(&mut self, hwnd: HWND, s: &Spec, t_ms: f32) -> windows::core::Result<()> {
        // Scale for the target monitor's DPI.
        let hmon = MonitorFromPoint(POINT { x: s.mon.left + 1, y: s.mon.top + 1 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let k = dx as f32 / 96.0;

        let pad = 14.0 * k; // room for the shadow
        let has_sub = !s.sub.is_empty();
        let (cw, ch) = (300.0 * k, if has_sub { 58.0 } else { 42.0 } * k);
        let (ww, wh) = ((cw + pad * 2.0).ceil() as i32, (ch + pad * 2.0).ceil() as i32);
        self.ensure(k, ww, wh)?;

        // Short slide + fade, like a system notification.
        let p = ease_out(t_ms / ENTER_MS);
        let q = 1.0 - ((t_ms - LEAVE_AT_MS) / LEAVE_MS).clamp(0.0, 1.0);
        let alpha = (p * q).clamp(0.0, 1.0);
        let dir = if s.right { 1.0 } else { -1.0 };
        let slide = dir * ((1.0 - p) * 16.0 + (1.0 - q) * 10.0) * k;

        let margin = 18.0 * k;
        let card_x = if s.right { s.mon.right as f32 - margin - cw } else { s.mon.left as f32 + margin };
        let card_y = if s.bottom { s.mon.bottom as f32 - margin - ch - 48.0 * k } else { s.mon.top as f32 + margin };
        let win_pos = POINT { x: (card_x - pad + slide).round() as i32, y: (card_y - pad).round() as i32 };

        let rt = &self.rt;
        rt.BindDC(self.mem_dc, &RECT { left: 0, top: 0, right: ww, bottom: wh })?;
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

        let (l, tp, r, b) = (pad, pad, pad + cw, pad + ch);
        let radius = 8.0 * k;

        // Tight shadow for separation from bright game scenes.
        let shadow = rt.CreateSolidColorBrush(&rgb(0x000000, 0.08), None)?;
        for i in 0..6 {
            let g = i as f32 * 1.5 * k;
            rt.FillRoundedRectangle(&rr(l - g * 0.5, tp - g * 0.2 + 3.0 * k, r + g * 0.5, b + g + 3.0 * k, radius + g), &shadow);
        }

        let bg = rt.CreateSolidColorBrush(&rgb(0x1c1c20, 0.97), None)?;
        let card = rr(l, tp, r, b, radius);
        rt.FillRoundedRectangle(&card, &bg);
        let border = rt.CreateSolidColorBrush(&rgb(0x33333a, 1.0), None)?;
        rt.DrawRoundedRectangle(&rr(l + 0.5, tp + 0.5, r - 0.5, b - 0.5, radius), &border, 1.0 * k.max(1.0), None);

        // Accent bar.
        let bar = rt.CreateSolidColorBrush(&s.bar, None)?;
        let inset = 11.0 * k;
        rt.FillRoundedRectangle(&rr(l + 12.0 * k, tp + inset, l + 15.0 * k, b - inset, 1.5 * k), &bar);

        let text_l = l + 26.0 * k;
        let text_r = r - 16.0 * k;
        let white = rt.CreateSolidColorBrush(&rgb(0xececef, 1.0), None)?;
        let muted = rt.CreateSolidColorBrush(&rgb(0x9a9aa4, 1.0), None)?;
        let title_rect = if has_sub {
            D2D_RECT_F { left: text_l, top: tp + 9.0 * k, right: text_r, bottom: tp + 30.0 * k }
        } else {
            D2D_RECT_F { left: text_l, top: tp, right: text_r, bottom: b }
        };
        if let Some(f) = &self.title_fmt {
            rt.DrawText(&wide(&s.title), f, &title_rect, &white, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
        if has_sub {
            if let Some(f) = &self.sub_fmt {
                let rect = D2D_RECT_F { left: text_l, top: tp + 30.0 * k, right: text_r, bottom: tp + 49.0 * k };
                rt.DrawText(&wide(&s.sub), f, &rect, &muted, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
        }

        rt.EndDraw(None, None)?;

        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: (alpha * 255.0) as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(&win_pos),
            Some(&SIZE { cx: ww, cy: wh }),
            Some(self.mem_dc),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )?;
        Ok(())
    }
}

impl Drop for Renderer {
    // Renderers are rebuilt after a GPU reset; free the GDI objects.
    fn drop(&mut self) {
        unsafe {
            // The DC first: a bitmap still selected into it can't be deleted.
            let _ = DeleteDC(self.mem_dc);
            if !self.dib.is_invalid() {
                let _ = DeleteObject(self.dib.into());
            }
        }
    }
}
