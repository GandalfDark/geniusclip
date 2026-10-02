//! The in-game overlay of frame rate and hardware load (Settings → In-game
//! stats, or its hotkey). Drawn natively with Direct2D into a click-through
//! layered window, like the toasts, over the game in front only.
//!
//! A topmost window over a fullscreen game makes Windows compose the screen
//! (a little more latency), so it is off by default and shows only while a
//! game fills its monitor. It is kept out of clips and screenshots.

use crate::fps::{self, Access, Rate};
use crate::i18n::t;
use crate::overlay::accent;
use crate::state::AppState;
use crate::stats::{self, Stats, User};
use geniusclip_engine::game::{self, FullscreenGame};
use std::sync::mpsc::channel;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::{w, Interface};
use windows_numerics::Vector2;
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

const WM_PERF_SYNC: u32 = WM_APP + 3;
/// Redrawn twice a second (frame rate); hardware is read once a second.
const TICK_MS: u32 = 500;
const STATS_EVERY: Duration = Duration::from_millis(950);
/// The focused window is looked at again this often even if it stays the
/// same (a game switching to fullscreen after it starts).
const RECHECK_GAME: Duration = Duration::from_secs(3);
/// Away from games this long: the frame rate session, counters and the
/// renderer are let go of.
const IDLE_RELEASE: Duration = Duration::from_secs(10);
const ACCESS_RECHECK: Duration = Duration::from_secs(10);

const TEXT: u32 = 0xe8e8ea;
const MUTED: u32 = 0x8b8b93;
const FPS_COLOR: u32 = 0x9fe1cb;
const REC_RED: u32 = 0xff4f4f;
const IDLE_GREY: u32 = 0x686872;

static THREAD: OnceLock<u32> = OnceLock::new();

pub fn start(app: &AppHandle) {
    let (tx, rx) = channel();
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("gc-perf".into()).spawn(move || unsafe { run(app, tx) });
    if spawned.is_ok() {
        if let Ok(id) = rx.recv() {
            let _ = THREAD.set(id);
        }
    }
}

/// The settings changed: show, hide or restyle now.
pub fn sync() {
    if let Some(&id) = THREAD.get() {
        unsafe {
            let _ = PostThreadMessageW(id, WM_PERF_SYNC, WPARAM(0), LPARAM(0));
        }
    }
}

/// The hotkey: shows or hides the overlay.
pub fn toggle(app: &AppHandle) {
    let on = {
        let st = app.state::<AppState>();
        let mut s = st.settings.write();
        s.perf_overlay.enabled = !s.perf_overlay.enabled;
        let _ = s.save(app);
        s.perf_overlay.enabled
    };
    crate::emit_settings(app);
    sync();
    // Away from a game nothing would show: say what happened.
    if game::foreground_fullscreen_game().is_none() {
        crate::overlay::toast(app, crate::overlay::Toast::simple(if on { "perf-on" } else { "perf-off" }));
    }
}

/// What a frame shows.
struct View {
    style: String,
    corner: String,
    show_fps: bool,
    show_gpu: bool,
    show_cpu: bool,
    show_clock: bool,
    /// None: no access to frame rates (Settings asks for it).
    rate: Option<Option<Rate>>,
    frames: Vec<f32>,
    stats: Option<Stats>,
    dot: u32,
    time: String,
    ms: &'static str,
    gb: &'static str,
    ram: &'static str,
}

struct Loop {
    app: AppHandle,
    hwnd: HWND,
    renderer: Option<Renderer>,
    timer: bool,
    shown: bool,
    game: (HWND, Option<FullscreenGame>, Instant),
    away_since: Option<Instant>,
    stats: Option<(Stats, Instant)>,
    access: Option<(Access, Instant)>,
    /// The process the frame rate session failed for, and when.
    fps_failed: Option<(u32, Instant)>,
    topmost_at: Instant,
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, wp, lp)
}

unsafe fn run(app: AppHandle, ready: std::sync::mpsc::Sender<u32>) {
    let mut msg = MSG::default();
    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let class = w!("GeniusClipPerf");
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        class,
        w!("GeniusClip stats"),
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
    let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    let _ = ready.send(GetCurrentThreadId());

    let now = Instant::now();
    let mut l = Loop {
        app,
        hwnd,
        renderer: None,
        timer: false,
        shown: false,
        game: (HWND::default(), None, now - RECHECK_GAME),
        away_since: None,
        stats: None,
        access: None,
        fps_failed: None,
        topmost_at: now,
    };
    l.sync();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        match msg.message {
            WM_PERF_SYNC => {
                // Settings may have changed what is shown or the access.
                l.access = None;
                l.stats = None;
                l.sync();
            }
            WM_TIMER => l.tick(),
            _ => {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Loop {
    fn enabled(&self) -> bool {
        self.app.state::<AppState>().settings.read().perf_overlay.enabled
    }

    unsafe fn sync(&mut self) {
        if self.enabled() {
            if !self.timer {
                SetTimer(Some(self.hwnd), 1, TICK_MS, None);
                self.timer = true;
            }
            self.tick();
        } else {
            if self.timer {
                let _ = KillTimer(Some(self.hwnd), 1);
                self.timer = false;
            }
            self.hide();
            self.release();
        }
    }

    unsafe fn hide(&mut self) {
        if self.shown {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            self.shown = false;
        }
    }

    fn release(&mut self) {
        fps::stop();
        stats::release(User::Overlay);
        self.stats = None;
        self.renderer = None;
        self.away_since = None;
    }

    unsafe fn tick(&mut self) {
        if !self.enabled() {
            return self.sync();
        }
        let fg = GetForegroundWindow();
        if fg != self.game.0 || self.game.2.elapsed() >= RECHECK_GAME {
            self.game = (fg, game::fullscreen_game(fg), Instant::now());
        }
        let Some((pid, monitor)) = self.game.1.as_ref().map(|g| (g.pid, g.monitor)) else {
            self.hide();
            let since = *self.away_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= IDLE_RELEASE && (self.renderer.is_some() || self.stats.is_some()) {
                self.release();
                self.away_since = Some(since);
            }
            return;
        };
        self.away_since = None;
        let view = self.view(pid);
        if let Err(e) = self.draw(&view, monitor) {
            log::warn!("stats overlay draw: {e}");
            return;
        }
        if !self.shown || self.topmost_at.elapsed() >= Duration::from_secs(1) {
            // Games take the top spot back when they go fullscreen.
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
            self.topmost_at = Instant::now();
            self.shown = true;
        }
    }

    fn view(&mut self, pid: u32) -> View {
        let st = self.app.state::<AppState>();
        let (p, lang, accent_id) = {
            let s = st.settings.read();
            (s.perf_overlay.clone(), s.lang(), s.accent.clone())
        };
        let wants_fps = p.fps || p.style == "fps";
        let rate = if wants_fps {
            let access = match self.access {
                Some((a, at)) if at.elapsed() < ACCESS_RECHECK => a,
                _ => {
                    let a = fps::access();
                    self.access = Some((a, Instant::now()));
                    a
                }
            };
            if access == Access::Granted {
                let failed_lately = self.fps_failed.is_some_and(|(f, at)| f == pid && at.elapsed() < Duration::from_secs(30));
                if !failed_lately {
                    if let Err(e) = fps::follow(pid) {
                        log::warn!("fps: {e:#}");
                        self.fps_failed = Some((pid, Instant::now()));
                    }
                }
                Some(fps::rate())
            } else {
                None
            }
        } else {
            fps::stop();
            None
        };
        let frames = if p.style == "panel" && p.fps { fps::frame_times(120) } else { Vec::new() };
        let stats = if p.style != "fps" && (p.gpu || p.cpu) {
            match &self.stats {
                Some((s, at)) if at.elapsed() < STATS_EVERY => Some(s.clone()),
                _ => {
                    let s = stats::snapshot(User::Overlay);
                    self.stats = Some((s.clone(), Instant::now()));
                    Some(s)
                }
            }
        } else {
            None
        };
        let status = st.engine.status();
        let dot = if status.recording {
            REC_RED
        } else if status.running && status.replay_enabled && !status.paused {
            accent(&accent_id)
        } else {
            IDLE_GREY
        };
        View {
            style: p.style,
            corner: p.corner,
            show_fps: p.fps,
            show_gpu: p.gpu,
            show_cpu: p.cpu,
            show_clock: p.clock,
            rate,
            frames,
            stats,
            dot,
            time: chrono::Local::now().format("%H:%M").to_string(),
            ms: t(lang, "perf.ms"),
            gb: t(lang, "perf.gb"),
            ram: t(lang, "perf.ram"),
        }
    }

    unsafe fn draw(&mut self, v: &View, monitor: RECT) -> windows::core::Result<()> {
        if self.renderer.is_none() {
            self.renderer = Some(Renderer::new()?);
        }
        let r = self.renderer.as_mut().unwrap();
        let res = r.frame(self.hwnd, v, monitor);
        if res.as_ref().is_err_and(|e| e.code() == D2DERR_RECREATE_TARGET) {
            log::info!("stats overlay: render target lost; recreating");
            self.renderer = None;
            return Ok(());
        }
        res
    }
}

// ---------------------------------------------------------------------------
// Drawing

fn rgb(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

fn rr(l: f32, t: f32, r: f32, b: f32, rad: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: l, top: t, right: r, bottom: b }, radiusX: rad, radiusY: rad }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// A line of text with some ranges in other colors (UTF-16 offsets).
struct Line {
    text: String,
    colored: Vec<(usize, usize, u32)>,
}

impl Line {
    fn new() -> Self {
        Line { text: String::new(), colored: Vec::new() }
    }

    fn push(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn push_colored(&mut self, s: &str, color: u32) {
        let start = self.text.encode_utf16().count();
        self.text.push_str(s);
        self.colored.push((start, s.encode_utf16().count(), color));
    }

    fn sep(&mut self) {
        if !self.text.is_empty() {
            self.text.push_str(" · ");
        }
    }
}

fn fps_text(rate: &Option<Option<Rate>>) -> Option<(String, Option<Rate>)> {
    match rate {
        None => None,
        Some(Some(r)) if r.fps > 0.0 => Some((format!("{:.0}", r.fps), Some(*r))),
        Some(_) => Some(("—".into(), None)),
    }
}

struct Renderer {
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    scale: f32,
    small: Option<IDWriteTextFormat>,
    mid: Option<IDWriteTextFormat>,
    big: Option<IDWriteTextFormat>,
    mem_dc: HDC,
    dib: HBITMAP,
    /// The DIB's pixels (premultiplied BGRA, top-down).
    bits: *mut std::ffi::c_void,
    size: (i32, i32),
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
        Ok(Renderer { dw, rt, scale: 0.0, small: None, mid: None, big: None, mem_dc, dib: HBITMAP::default(), bits: std::ptr::null_mut(), size: (0, 0) })
    }

    unsafe fn format(&self, size: f32) -> windows::core::Result<IDWriteTextFormat> {
        let f = self.dw.CreateTextFormat(w!("Consolas"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }

    unsafe fn fonts(&mut self, k: f32) -> windows::core::Result<()> {
        if (self.scale - k).abs() > f32::EPSILON || self.small.is_none() {
            self.small = Some(self.format(12.5 * k)?);
            self.mid = Some(self.format(16.0 * k)?);
            self.big = Some(self.format(22.0 * k)?);
            self.scale = k;
        }
        Ok(())
    }

    unsafe fn surface(&mut self, w: i32, h: i32) -> windows::core::Result<()> {
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
            self.dib = CreateDIBSection(Some(self.mem_dc), &bmi, DIB_RGB_COLORS, &mut self.bits, None, 0)?;
            SelectObject(self.mem_dc, self.dib.into());
            self.size = (w, h);
        }
        Ok(())
    }

    /// A text layout with its colored ranges, and its size.
    unsafe fn layout(&self, line: &Line, format: &IDWriteTextFormat) -> windows::core::Result<(IDWriteTextLayout, f32, f32)> {
        let layout = self.dw.CreateTextLayout(&wide(&line.text), format, 4000.0, 400.0)?;
        for &(start, len, color) in &line.colored {
            let brush = self.rt.CreateSolidColorBrush(&rgb(color, 1.0), None)?;
            layout.SetDrawingEffect(&brush.cast::<windows::core::IUnknown>()?, DWRITE_TEXT_RANGE { startPosition: start as u32, length: len as u32 })?;
        }
        let mut m = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut m)?;
        Ok((layout, m.widthIncludingTrailingWhitespace, m.height))
    }

    unsafe fn frame(&mut self, hwnd: HWND, v: &View, mon: RECT) -> windows::core::Result<()> {
        let hmon = MonitorFromPoint(POINT { x: mon.left + 1, y: mon.top + 1 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let k = dx as f32 / 96.0;
        self.fonts(k)?;
        let small = self.small.clone().unwrap();
        let mid = self.mid.clone().unwrap();
        let big = self.big.clone().unwrap();

        // Content first (its size decides the window's), drawn below.
        enum Item {
            Text(IDWriteTextLayout, f32, f32),
            Graph(Vec<f32>, f32),
            Gap(f32),
        }
        let pad_x = if v.style == "fps" { 6.0 * k } else { 8.0 * k };
        let pad_y = if v.style == "panel" { 6.0 * k } else { 2.0 * k };
        let mut items: Vec<Item> = Vec::new();
        match v.style.as_str() {
            "fps" => {
                // The number alone, a size up.
                let mut line = Line::new();
                let text = fps_text(&v.rate).map(|(s, _)| s).unwrap_or_else(|| "—".into());
                line.push_colored(&text, FPS_COLOR);
                let (lay, w, h) = self.layout(&line, &mid)?;
                items.push(Item::Text(lay, w, h));
            }
            "panel" => {
                if v.show_fps {
                    let mut head = Line::new();
                    match fps_text(&v.rate) {
                        Some((n, r)) => {
                            head.push_colored(&n, FPS_COLOR);
                            let (lay, w, h) = self.layout(&head, &big)?;
                            items.push(Item::Text(lay, w, h));
                            let mut sub = Line::new();
                            sub.push("FPS");
                            if let Some(r) = r {
                                sub.push(&format!(" · {:.1} {} · 1% {:.0}", r.frame_ms, v.ms, r.low_1));
                            }
                            let (lay, w, h) = self.layout(&sub, &small)?;
                            items.push(Item::Text(lay, w, h));
                        }
                        None => {
                            head.push("FPS —");
                            let (lay, w, h) = self.layout(&head, &small)?;
                            items.push(Item::Text(lay, w, h));
                        }
                    }
                    if v.frames.len() > 2 {
                        items.push(Item::Gap(2.0 * k));
                        items.push(Item::Graph(v.frames.clone(), 20.0 * k));
                        items.push(Item::Gap(2.0 * k));
                    }
                }
                if let Some(s) = &v.stats {
                    if v.show_gpu {
                        if let Some(g) = s.gpu {
                            let mut l = Line::new();
                            l.push(&format!("GPU {g:.0}%"));
                            if let Some(t) = s.gpu_temp {
                                l.push(&format!(" · {t}°"));
                            }
                            let (lay, w, h) = self.layout(&l, &small)?;
                            items.push(Item::Text(lay, w, h));
                        }
                        if let Some(vr) = s.vram_used_gb {
                            let mut l = Line::new();
                            l.push(&format!("VRAM {vr:.1} {}", v.gb));
                            let (lay, w, h) = self.layout(&l, &small)?;
                            items.push(Item::Text(lay, w, h));
                        }
                    }
                    if v.show_cpu {
                        let mut l = Line::new();
                        l.push(&format!("CPU {:.0}% · {} {:.0} {}", s.cpu, v.ram, s.ram_used_gb, v.gb));
                        let (lay, w, h) = self.layout(&l, &small)?;
                        items.push(Item::Text(lay, w, h));
                    }
                }
                if v.show_clock {
                    let mut l = Line::new();
                    l.push_colored("●", v.dot);
                    l.push(&format!(" {}", v.time));
                    let (lay, w, h) = self.layout(&l, &small)?;
                    items.push(Item::Text(lay, w, h));
                }
            }
            _ => {
                let mut l = Line::new();
                if v.show_fps {
                    match fps_text(&v.rate) {
                        Some((n, r)) => {
                            l.push_colored(&n, FPS_COLOR);
                            l.push(" FPS");
                            if let Some(r) = r {
                                l.push(&format!(" · {:.1} {}", r.frame_ms, v.ms));
                            }
                        }
                        None => l.push("FPS —"),
                    }
                }
                if let Some(s) = &v.stats {
                    if v.show_gpu {
                        if let Some(g) = s.gpu {
                            l.sep();
                            l.push(&format!("GPU {g:.0}%"));
                            if let Some(t) = s.gpu_temp {
                                l.push(&format!(" {t}°"));
                            }
                        }
                    }
                    if v.show_cpu {
                        l.sep();
                        l.push(&format!("CPU {:.0}%", s.cpu));
                    }
                }
                if v.show_clock {
                    l.sep();
                    l.push(&v.time);
                    l.push(" ");
                    l.push_colored("●", v.dot);
                }
                if l.text.is_empty() {
                    l.push("—");
                }
                let (lay, w, h) = self.layout(&l, &small)?;
                items.push(Item::Text(lay, w, h));
            }
        }

        let content_w = items
            .iter()
            .map(|i| match i {
                Item::Text(_, w, _) => *w,
                Item::Graph(..) => 132.0 * k,
                Item::Gap(_) => 0.0,
            })
            .fold(0.0f32, f32::max);
        let content_h: f32 = items
            .iter()
            .map(|i| match i {
                Item::Text(_, _, h) => *h,
                Item::Graph(_, h) => *h,
                Item::Gap(g) => *g,
            })
            .sum();
        let (cw, ch) = (content_w + pad_x * 2.0, content_h + pad_y * 2.0);
        let (ww, wh) = (cw.ceil() as i32 + 2, ch.ceil() as i32 + 2);
        self.surface(ww, wh)?;

        let margin = 10.0 * k;
        let right = v.corner.ends_with("right");
        let bottom = v.corner.starts_with("bottom");
        let x = if right { mon.right as f32 - margin - cw } else { mon.left as f32 + margin };
        let y = if bottom { mon.bottom as f32 - margin - ch } else { mon.top as f32 + margin };

        let rt = &self.rt;
        rt.BindDC(self.mem_dc, &RECT { left: 0, top: 0, right: ww, bottom: wh })?;
        rt.BeginDraw();
        rt.Clear(Some(&rgb(0, 0.0)));
        // A lighter backdrop under the lone number.
        let bg = rt.CreateSolidColorBrush(&rgb(0x0a0a0c, if v.style == "fps" { 0.45 } else { 0.64 }), None)?;
        rt.FillRoundedRectangle(&rr(1.0, 1.0, 1.0 + cw, 1.0 + ch, if v.style == "panel" { 8.0 } else { 6.0 } * k), &bg);
        let text = rt.CreateSolidColorBrush(&rgb(TEXT, 1.0), None)?;
        let muted = rt.CreateSolidColorBrush(&rgb(MUTED, 1.0), None)?;
        let graph = rt.CreateSolidColorBrush(&rgb(FPS_COLOR, 1.0), None)?;
        let mut top = 1.0 + pad_y;
        let left = 1.0 + pad_x;
        for item in &items {
            match item {
                Item::Text(lay, _, h) => {
                    rt.DrawTextLayout(Vector2 { X: left, Y: top }, lay, &text, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    top += h;
                }
                Item::Graph(frames, h) => {
                    // Frame times, newest on the right; spikes are hitches.
                    let avg = frames.iter().sum::<f32>() / frames.len() as f32;
                    let peak = frames.iter().copied().fold(0.0f32, f32::max).max(avg * 2.0).max(1.0);
                    let width = 132.0 * k;
                    let step = width / (frames.len() - 1) as f32;
                    let base = top + h;
                    rt.DrawLine(Vector2 { X: left, Y: base }, Vector2 { X: left + width, Y: base }, &muted, 0.5 * k, None);
                    let point = |i: usize, ft: f32| Vector2 { X: left + i as f32 * step, Y: base - (ft / peak).min(1.0) * (h - 1.0) };
                    for i in 1..frames.len() {
                        rt.DrawLine(point(i - 1, frames[i - 1]), point(i, frames[i]), &graph, 1.2 * k, None);
                    }
                    top += h;
                }
                Item::Gap(g) => top += g,
            }
        }
        rt.EndDraw(None, None)?;

        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(&POINT { x: x.round() as i32 - 1, y: y.round() as i32 - 1 }),
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
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.mem_dc);
            if !self.dib.is_invalid() {
                let _ = DeleteObject(self.dib.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws each look with sample numbers over a game-like color into
    /// target/perf-<look>.bmp, to look at:
    ///   cargo test -p geniusclip render_looks -- --ignored
    #[test]
    #[ignore]
    fn render_looks() {
        unsafe {
            let hinst = GetModuleHandleW(None).unwrap();
            let class = w!("GeniusClipPerfTest");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() });
            let hwnd = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW, class, w!("test"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst.into()), None).unwrap();
            let mon = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
            let frames: Vec<f32> = (0..120).map(|i| if i == 84 { 21.0 } else { 6.6 + (i % 9) as f32 * 0.09 }).collect();
            for style in ["line", "panel", "fps"] {
                let v = View {
                    style: style.into(),
                    corner: "top-left".into(),
                    show_fps: true,
                    show_gpu: true,
                    show_cpu: true,
                    show_clock: true,
                    rate: Some(Some(Rate { fps: 143.6, frame_ms: 6.96, low_1: 118.2 })),
                    frames: frames.clone(),
                    stats: Some(Stats { cpu: 35.2, ram_used_gb: 18.4, ram_total_gb: 32.0, gpu: Some(62.0), gpu_temp: Some(71), vram_used_gb: Some(7.1) }),
                    dot: 0x9580ff,
                    time: "21:34".into(),
                    ms: "мс",
                    gb: "ГБ",
                    ram: "ОЗУ",
                };
                let mut r = Renderer::new().unwrap();
                r.frame(hwnd, &v, mon).unwrap();
                let (w, h) = r.size;
                let px = std::slice::from_raw_parts(r.bits as *const u8, (w * h * 4) as usize);
                // Over a dark teal "game", as 24-bit BMP (bottom-up rows).
                let (bg_r, bg_g, bg_b) = (0x2bu32, 0x3du32, 0x48u32);
                let row = ((w * 3 + 3) / 4 * 4) as usize;
                let mut out = Vec::with_capacity(54 + row * h as usize);
                let size = 54 + row * h as usize;
                out.extend_from_slice(b"BM");
                out.extend_from_slice(&(size as u32).to_le_bytes());
                out.extend_from_slice(&[0; 4]);
                out.extend_from_slice(&54u32.to_le_bytes());
                out.extend_from_slice(&40u32.to_le_bytes());
                out.extend_from_slice(&w.to_le_bytes());
                out.extend_from_slice(&h.to_le_bytes());
                out.extend_from_slice(&1u16.to_le_bytes());
                out.extend_from_slice(&24u16.to_le_bytes());
                out.extend_from_slice(&[0; 24]);
                for y in (0..h as usize).rev() {
                    let start = out.len();
                    for x in 0..w as usize {
                        let i = (y * w as usize + x) * 4;
                        let a = px[i + 3] as u32;
                        let mix = |c: u8, bg: u32| (c as u32 + bg * (255 - a) / 255).min(255) as u8;
                        out.extend_from_slice(&[mix(px[i], bg_b), mix(px[i + 1], bg_g), mix(px[i + 2], bg_r)]);
                    }
                    out.resize(start + row, 0);
                }
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../target/perf-{style}.bmp"));
                std::fs::write(&path, out).unwrap();
                println!("{} ({w}x{h})", path.display());
            }
        }
    }
}
