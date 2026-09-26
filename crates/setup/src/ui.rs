//! The installer window, drawn with Direct2D/DirectWrite. No WebView: it may
//! not be installed yet on a fresh Windows 10 (the NSIS setup brings it).
//!
//! Stages: Welcome (Install / Options) → Installing (progress) → Done (hotkey
//! hint, Launch) or Failed (Try again). Everything is laid out in DIPs on a
//! fixed 420×560 canvas and scaled by the render target's DPI.

use crate::i18n::{self, Strings, LANGS};
use crate::install::{self, Installed, Options};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::core::{w, Result, BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::Vector2;

const W: f32 = 420.0;
const H: f32 = 560.0;
const PAD: f32 = 32.0;
const BUTTON_Y: f32 = 444.0;
const LINK_Y: f32 = 506.0;
const MENU_W: f32 = 220.0;
const MENU_ITEM: f32 = 30.0;

const WM_INSTALL_DONE: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER: usize = 1;

static LOGO: &[u8] = include_bytes!("../assets/logo.png");

// The app's "Studio" palette (src/app.css).
const BG: u32 = 0x141416;
const PANEL: u32 = 0x1c1c21;
const LINE: u32 = 0x33333a;
const TEXT: u32 = 0xececef;
const TEXT2: u32 = 0x9a9aa4;
const TEXT3: u32 = 0x6c6c76;
const ACCENT: u32 = 0x9580ff;
const ACCENT2: u32 = 0xd77ce0;
const INK: u32 = 0x150c26;
const DANGER: u32 = 0xff5c5c;

fn rgb(c: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: ((c >> 16) & 255) as f32 / 255.0, g: ((c >> 8) & 255) as f32 / 255.0, b: (c & 255) as f32 / 255.0, a }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}
fn rr(r: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }
}
fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}
fn inside(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
/// The app's soft curve (motion.ts), close enough to cubic-bezier(.22,1,.36,1).
fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4)
}

struct Anim {
    from: f32,
    to: f32,
    start: Instant,
    ms: f32,
}

impl Anim {
    fn new(v: f32, ms: f32) -> Anim {
        Anim { from: v, to: v, start: Instant::now(), ms }
    }
    fn set(&mut self, to: f32) {
        if to != self.to {
            self.from = self.value();
            self.to = to;
            self.start = Instant::now();
        }
    }
    fn restart(&mut self, from: f32, to: f32) {
        self.from = from;
        self.to = to;
        self.start = Instant::now();
    }
    fn value(&self) -> f32 {
        let t = self.start.elapsed().as_secs_f32() * 1000.0 / self.ms;
        self.from + (self.to - self.from) * ease(t)
    }
    fn done(&self) -> bool {
        self.start.elapsed().as_secs_f32() * 1000.0 >= self.ms
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    Minimize,
    Lang,
    LangItem(usize),
    Primary,
    Options,
    ChangeDir,
    Autostart,
    Shortcut,
    Secondary,
    /// Anywhere outside the open language menu.
    Backdrop,
}

#[derive(Clone, PartialEq)]
enum Stage {
    Welcome,
    Installing,
    Done,
    Failed(String),
}

struct Fonts {
    title: IDWriteTextFormat,
    heading: IDWriteTextFormat,
    body: IDWriteTextFormat,
    label: IDWriteTextFormat,
    small: IDWriteTextFormat,
    button: IDWriteTextFormat,
    link: IDWriteTextFormat,
    chip: IDWriteTextFormat,
    menu: IDWriteTextFormat,
}

struct App {
    hwnd: HWND,
    dpi: f32,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    wic: IWICImagingFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    logo: Option<ID2D1Bitmap>,
    round: ID2D1StrokeStyle,
    display_family: HSTRING,
    text_family: HSTRING,
    fonts: Fonts,
    strings: Strings,
    lang: usize,
    lang_picked: bool,
    stage: Stage,
    stage_anim: Anim,
    opts: bool,
    opts_anim: Anim,
    menu: bool,
    menu_anim: Anim,
    dir: PathBuf,
    autostart: bool,
    shortcut: bool,
    installed: Option<Installed>,
    hotkey: String,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    hovers: Vec<(Hit, f32)>,
    progress: f32,
    install_started: Instant,
    finishing: bool,
    result: Arc<Mutex<Option<std::result::Result<(), String>>>>,
    last_frame: Instant,
    timer: bool,
}

pub fn run() -> Result<()> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("GeniusClipSetup");
        let wc = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);

        // Centre on the monitor under the cursor, sized for its DPI.
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let mon = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let k = dx as f32 / 96.0;
        let (pw, ph) = ((W * k).round() as i32, (H * k).round() as i32);
        let wa = info.rcWork;
        let (x, y) = (wa.left + (wa.right - wa.left - pw) / 2, wa.top + (wa.bottom - wa.top - ph) / 2);

        let app = Box::into_raw(Box::new(App::new()?));
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class,
            &HSTRING::from((*app).s("title")),
            WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            x,
            y,
            pw,
            ph,
            None,
            None,
            Some(hinst),
            Some(app as *const _),
        )?;
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, 4);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        // Content fades in once the window is up.
        (*app).stage_anim.restart(0.0, 1.0);
        (*app).kick();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(Box::from_raw(app));
    }
    Ok(())
}

unsafe fn pick_family(dw: &IDWriteFactory, candidates: &[&str]) -> HSTRING {
    let mut coll = None;
    if dw.GetSystemFontCollection(&mut coll, false).is_ok() {
        if let Some(coll) = coll {
            for c in candidates {
                let (mut idx, mut exists) = (0u32, BOOL(0));
                if coll.FindFamilyName(&HSTRING::from(*c), &mut idx, &mut exists).is_ok() && exists.as_bool() {
                    return HSTRING::from(*c);
                }
            }
        }
    }
    HSTRING::from("Segoe UI")
}

unsafe fn make_fonts(dw: &IDWriteFactory, display: &HSTRING, text: &HSTRING, locale: &str) -> Result<Fonts> {
    let loc = HSTRING::from(locale);
    let fmt = |family: &HSTRING, size: f32, weight: DWRITE_FONT_WEIGHT, center: bool, wrap: bool| -> Result<IDWriteTextFormat> {
        let f = dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, &loc)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        if center {
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        }
        if !wrap {
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = dw.CreateEllipsisTrimmingSign(&f)?;
            let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            f.SetTrimming(&trim, &sign)?;
        }
        Ok(f)
    };
    let body = fmt(text, 15.0, DWRITE_FONT_WEIGHT_NORMAL, false, true)?;
    body.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
    Ok(Fonts {
        title: fmt(display, 30.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, false, false)?,
        heading: fmt(display, 25.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, false, false)?,
        body,
        label: fmt(text, 14.5, DWRITE_FONT_WEIGHT_NORMAL, false, false)?,
        small: fmt(text, 13.0, DWRITE_FONT_WEIGHT_NORMAL, false, false)?,
        button: fmt(text, 15.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, false)?,
        link: fmt(text, 13.5, DWRITE_FONT_WEIGHT_MEDIUM, true, false)?,
        chip: fmt(text, 13.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, false)?,
        menu: fmt(text, 14.0, DWRITE_FONT_WEIGHT_NORMAL, false, false)?,
    })
}

impl App {
    fn new() -> Result<App> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let round = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let display_family = pick_family(&dw, &["Segoe UI Variable Display", "Segoe UI"]);
            let text_family = pick_family(&dw, &["Segoe UI Variable Text", "Segoe UI"]);
            let lang = i18n::system_lang();
            let fonts = make_fonts(&dw, &display_family, &text_family, LANGS[lang].locale)?;
            let installed = install::installed();
            let dir = installed.as_ref().map(|i| i.dir.clone()).unwrap_or_else(install::default_dir);
            Ok(App {
                hwnd: HWND::default(),
                dpi: 96.0,
                d2d,
                dw,
                wic,
                rt: None,
                logo: None,
                round,
                display_family,
                text_family,
                fonts,
                strings: Strings::load(),
                lang,
                lang_picked: false,
                stage: Stage::Welcome,
                stage_anim: Anim::new(0.0, 420.0),
                opts: false,
                opts_anim: Anim::new(0.0, 300.0),
                menu: false,
                menu_anim: Anim::new(0.0, 180.0),
                dir,
                autostart: true,
                shortcut: true,
                installed,
                hotkey: install::save_hotkey(),
                hover: None,
                pressed: None,
                hovers: Vec::new(),
                progress: 0.0,
                install_started: Instant::now(),
                finishing: false,
                result: Arc::new(Mutex::new(None)),
                last_frame: Instant::now(),
                timer: false,
            })
        }
    }

    fn s(&self, key: &str) -> String {
        self.strings.get(self.lang, key)
    }

    // ---------------------------------------------------------------- layout

    fn measure(&self, s: &str, f: &IDWriteTextFormat) -> f32 {
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&wide(s), f, 10_000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }

    fn primary_label(&self) -> String {
        match &self.stage {
            Stage::Done => self.s("launch"),
            Stage::Failed(_) => self.s("retry"),
            _ => match &self.installed {
                Some(i) => match install::compare_installed(&i.version) {
                    -1 => self.s("update"),
                    0 => self.s("reinstall"),
                    _ => self.s("install"),
                },
                None => self.s("install"),
            },
        }
    }

    fn lang_pill(&self) -> D2D_RECT_F {
        let w = self.measure(LANGS[self.lang].name, &self.fonts.small) + 40.0;
        rect(W - PAD - w, 61.0, w, 30.0)
    }

    fn link_rect(&self, label: &str) -> D2D_RECT_F {
        let w = self.measure(label, &self.fonts.link) + 36.0;
        rect((W - w) / 2.0, LINK_Y, w, 28.0)
    }

    /// Clickable areas, topmost last.
    fn hits(&self) -> Vec<(Hit, D2D_RECT_F)> {
        let mut v = Vec::new();
        if self.stage != Stage::Installing {
            v.push((Hit::Close, rect(W - 44.0, 8.0, 36.0, 32.0)));
        }
        v.push((Hit::Minimize, rect(W - 82.0, 8.0, 36.0, 32.0)));
        match &self.stage {
            Stage::Welcome => {
                v.push((Hit::Lang, self.lang_pill()));
                v.push((Hit::Primary, rect(PAD, BUTTON_Y, W - 2.0 * PAD, 52.0)));
                v.push((Hit::Options, self.link_rect(&self.s("options"))));
                if self.opts {
                    v.push((Hit::ChangeDir, rect(PAD, 276.0, W - 2.0 * PAD, 42.0)));
                    v.push((Hit::Autostart, rect(PAD, 330.0, W - 2.0 * PAD, 32.0)));
                    v.push((Hit::Shortcut, rect(PAD, 366.0, W - 2.0 * PAD, 32.0)));
                }
                if self.menu {
                    v.push((Hit::Backdrop, rect(0.0, 0.0, W, H)));
                    let x = W - PAD - MENU_W;
                    for i in 0..LANGS.len() {
                        v.push((Hit::LangItem(i), rect(x + 6.0, 104.0 + i as f32 * MENU_ITEM, MENU_W - 12.0, MENU_ITEM)));
                    }
                }
            }
            Stage::Installing => {}
            Stage::Done | Stage::Failed(_) => {
                v.push((Hit::Primary, rect(PAD, BUTTON_Y, W - 2.0 * PAD, 52.0)));
                v.push((Hit::Secondary, self.link_rect(&self.s("close"))));
            }
        }
        v
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits().into_iter().rev().find(|(_, r)| inside(r, x, y)).map(|(h, _)| h)
    }

    fn level(&self, hit: Hit) -> f32 {
        self.hovers.iter().find(|(h, _)| *h == hit).map(|(_, l)| *l).unwrap_or(0.0)
    }

    // ------------------------------------------------------------ animation

    fn kick(&mut self) {
        if !self.timer {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER, 15, None);
            }
            self.timer = true;
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn animating(&self) -> bool {
        !self.stage_anim.done()
            || !self.opts_anim.done()
            || !self.menu_anim.done()
            || self.stage == Stage::Installing
            || self.hovers.iter().any(|(h, l)| (l - if Some(*h) == self.hover { 1.0 } else { 0.0 }).abs() > 0.01)
    }

    fn step(&mut self) {
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.1);
        self.last_frame = Instant::now();
        let k = (dt * 14.0).min(1.0);
        for (hit, _) in self.hits() {
            if !self.hovers.iter().any(|(h, _)| *h == hit) {
                self.hovers.push((hit, 0.0));
            }
        }
        let hover = self.hover;
        for (h, l) in self.hovers.iter_mut() {
            let target = if Some(*h) == hover { 1.0 } else { 0.0 };
            *l += (target - *l) * k;
        }
        if self.stage == Stage::Installing {
            let t = self.install_started.elapsed().as_secs_f32();
            // Silent NSIS reports no progress: approach 90% on a curve,
            // finish when the setup process exits.
            let target = if self.finishing { 1.0 } else { 0.9 * (1.0 - (-t / 2.5).exp()) };
            self.progress += (target - self.progress) * (dt * 6.0).min(1.0);
            if self.finishing && self.progress > 0.995 {
                self.set_stage(Stage::Done);
            }
        }
    }

    fn set_stage(&mut self, stage: Stage) {
        self.stage = stage;
        self.stage_anim.restart(0.0, 1.0);
        self.menu = false;
        self.menu_anim.set(0.0);
        self.hover = None;
        self.kick();
    }

    // -------------------------------------------------------------- actions

    fn click(&mut self, hit: Hit) {
        unsafe {
            match hit {
                Hit::Close | Hit::Secondary => {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                Hit::Minimize => {
                    let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
                }
                Hit::Lang => {
                    self.menu = !self.menu;
                    self.menu_anim.set(if self.menu { 1.0 } else { 0.0 });
                }
                Hit::Backdrop => {
                    self.menu = false;
                    self.menu_anim.set(0.0);
                }
                Hit::LangItem(i) => {
                    self.menu = false;
                    self.menu_anim.set(0.0);
                    if i != self.lang {
                        self.lang = i;
                        self.lang_picked = true;
                        if let Ok(f) = make_fonts(&self.dw, &self.display_family, &self.text_family, LANGS[i].locale) {
                            self.fonts = f;
                        }
                        let _ = SetWindowTextW(self.hwnd, &HSTRING::from(self.s("title")));
                        self.hovers.clear();
                    }
                }
                Hit::Options => {
                    self.opts = !self.opts;
                    self.opts_anim.set(if self.opts { 1.0 } else { 0.0 });
                }
                Hit::ChangeDir => {
                    if let Some(dir) = self.pick_folder() {
                        self.dir = install::normalize_dir(dir);
                    }
                }
                Hit::Autostart => self.autostart = !self.autostart,
                Hit::Shortcut => self.shortcut = !self.shortcut,
                Hit::Primary => match self.stage {
                    Stage::Welcome | Stage::Failed(_) => self.start_install(),
                    Stage::Done => {
                        install::launch(&self.dir);
                        let _ = DestroyWindow(self.hwnd);
                        return;
                    }
                    Stage::Installing => {}
                },
            }
        }
        self.kick();
    }

    fn start_install(&mut self) {
        let opts = Options {
            dir: self.dir.clone(),
            autostart: self.autostart,
            shortcut: self.shortcut,
            language: self.lang_picked.then(|| LANGS[self.lang].code),
        };
        self.progress = 0.0;
        self.finishing = false;
        self.install_started = Instant::now();
        self.set_stage(Stage::Installing);
        let hwnd = self.hwnd.0 as isize;
        let result = self.result.clone();
        std::thread::spawn(move || {
            let r = install::run(&opts);
            *result.lock().unwrap() = Some(r);
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_INSTALL_DONE, WPARAM(0), LPARAM(0));
            }
        });
    }

    fn install_done(&mut self) {
        let result = self.result.lock().unwrap().take();
        match result {
            Some(Ok(())) => self.finishing = true,
            Some(Err(e)) => self.set_stage(Stage::Failed(e)),
            None => {}
        }
        self.kick();
    }

    fn pick_folder(&self) -> Option<PathBuf> {
        unsafe {
            let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let opts = dlg.GetOptions().ok()?;
            dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
            let _ = dlg.SetTitle(&HSTRING::from(self.s("pickFolder")));
            if let Some(parent) = self.dir.parent() {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(parent.as_os_str()), None) {
                    let _ = dlg.SetFolder(&item);
                }
            }
            dlg.Show(Some(self.hwnd)).ok()?;
            let item = dlg.GetResult().ok()?;
            let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s.map(PathBuf::from)
        }
    }

    // -------------------------------------------------------------- drawing

    fn ensure_target(&mut self) -> Result<ID2D1HwndRenderTarget> {
        if let Some(rt) = &self.rt {
            return Ok(rt.clone());
        }
        unsafe {
            let mut rc = RECT::default();
            GetClientRect(self.hwnd, &mut rc)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: self.dpi,
                dpiY: self.dpi,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let rt = self.d2d.CreateHwndRenderTarget(&props, &hprops)?;
            // Logo: PNG → premultiplied BGRA → D2D bitmap.
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromMemory(LOGO)?;
            let decoder = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let frame = decoder.GetFrame(0)?;
            let conv = self.wic.CreateFormatConverter()?;
            conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
            self.logo = rt.CreateBitmapFromWicBitmap(&conv, None).ok();
            self.rt = Some(rt.clone());
            Ok(rt)
        }
    }

    fn paint(&mut self) {
        self.step();
        if let Err(e) = self.draw() {
            if e.code() == D2DERR_RECREATE_TARGET {
                self.rt = None;
                self.logo = None;
            }
        }
        if !self.animating() && self.timer {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER);
            }
            self.timer = false;
        }
    }

    fn draw(&mut self) -> Result<()> {
        let rt = self.ensure_target()?;
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&rgb(BG, 1.0)));
            let p = Painter { app: self, rt: &rt };
            p.glow()?;
            p.chrome()?;
            let a = self.stage_anim.value();
            let dy = (1.0 - a) * 10.0;
            match &self.stage {
                Stage::Welcome => p.welcome(a, dy)?,
                Stage::Installing => p.installing(a, dy)?,
                Stage::Done => p.done(a, dy)?,
                Stage::Failed(msg) => p.failed(msg, a, dy)?,
            }
            p.menu()?;
            rt.EndDraw(None, None)
        }
    }
}

/// Drawing helpers over one frame.
struct Painter<'a> {
    app: &'a App,
    rt: &'a ID2D1HwndRenderTarget,
}

impl Painter<'_> {
    unsafe fn solid(&self, c: u32, a: f32) -> Result<ID2D1SolidColorBrush> {
        self.rt.CreateSolidColorBrush(&rgb(c, a), None)
    }

    unsafe fn gradient(&self, x0: f32, x1: f32, a: f32) -> Result<ID2D1LinearGradientBrush> {
        let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: rgb(ACCENT, a) }, D2D1_GRADIENT_STOP { position: 1.0, color: rgb(ACCENT2, a) }];
        let coll = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        self.rt.CreateLinearGradientBrush(&D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(x0, 0.0), endPoint: pt(x1, 0.0) }, None, &coll)
    }

    unsafe fn text(&self, s: &str, f: &IDWriteTextFormat, r: D2D_RECT_F, c: u32, a: f32) -> Result<()> {
        let b = self.solid(c, a)?;
        self.rt.DrawText(&wide(s), f, &r, &b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        Ok(())
    }

    unsafe fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: u32, a: f32, w: f32) -> Result<()> {
        let b = self.solid(c, a)?;
        self.rt.DrawLine(pt(x0, y0), pt(x1, y1), &b, w, &self.app.round);
        Ok(())
    }

    unsafe fn chevron(&self, cx: f32, cy: f32, up: f32, c: u32, a: f32) -> Result<()> {
        // up: 0 = pointing down, 1 = pointing up.
        let d = 3.5 * (1.0 - 2.0 * up);
        self.line(cx - 4.5, cy - d * 0.5, cx, cy + d * 0.5, c, a, 1.6)?;
        self.line(cx, cy + d * 0.5, cx + 4.5, cy - d * 0.5, c, a, 1.6)
    }

    unsafe fn check(&self, cx: f32, cy: f32, s: f32, c: u32, a: f32, w: f32) -> Result<()> {
        self.line(cx - 0.42 * s, cy + 0.02 * s, cx - 0.12 * s, cy + 0.32 * s, c, a, w)?;
        self.line(cx - 0.12 * s, cy + 0.32 * s, cx + 0.45 * s, cy - 0.3 * s, c, a, w)
    }

    unsafe fn glow(&self) -> Result<()> {
        let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: rgb(ACCENT, 0.24) }, D2D1_GRADIENT_STOP { position: 1.0, color: rgb(ACCENT, 0.0) }];
        let coll = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: pt(W - 30.0, -20.0), gradientOriginOffset: pt(0.0, 0.0), radiusX: 300.0, radiusY: 260.0 };
        let b = self.rt.CreateRadialGradientBrush(&props, None, &coll)?;
        self.rt.FillRectangle(&rect(0.0, 0.0, W, H), &b);
        // Hairline frame, like the app's panels.
        let border = self.solid(LINE, 0.9)?;
        self.rt.DrawRectangle(&rect(0.5, 0.5, W - 1.0, H - 1.0), &border, 1.0, None);
        Ok(())
    }

    unsafe fn chrome(&self) -> Result<()> {
        for (hit, r) in self.app.hits() {
            let l = self.app.level(hit);
            match hit {
                Hit::Minimize | Hit::Close => {
                    if l > 0.01 {
                        let bg = self.solid(if hit == Hit::Close { DANGER } else { TEXT }, 0.10 * l)?;
                        self.rt.FillRoundedRectangle(&rr(r, 8.0), &bg);
                    }
                    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    let c = if l > 0.5 { TEXT } else { TEXT2 };
                    if hit == Hit::Close {
                        self.line(cx - 5.0, cy - 5.0, cx + 5.0, cy + 5.0, c, 1.0, 1.4)?;
                        self.line(cx + 5.0, cy - 5.0, cx - 5.0, cy + 5.0, c, 1.0, 1.4)?;
                    } else {
                        self.line(cx - 5.5, cy, cx + 5.5, cy, c, 1.0, 1.4)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    unsafe fn logo(&self, a: f32, dy: f32) -> Result<()> {
        if let Some(bmp) = &self.app.logo {
            self.rt.DrawBitmap(bmp, Some(&rect(PAD, 44.0 + dy, 64.0, 64.0)), a, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
        Ok(())
    }

    unsafe fn primary(&self, label: &str, a: f32, dy: f32) -> Result<()> {
        let r = rect(PAD, BUTTON_Y + dy, W - 2.0 * PAD, 52.0);
        let g = self.gradient(r.left, r.right, a)?;
        self.rt.FillRoundedRectangle(&rr(r, 12.0), &g);
        let l = self.app.level(Hit::Primary);
        if self.app.pressed == Some(Hit::Primary) && self.app.hover == Some(Hit::Primary) {
            let dark = self.solid(0x000000, 0.12 * a)?;
            self.rt.FillRoundedRectangle(&rr(r, 12.0), &dark);
        } else if l > 0.01 {
            let light = self.solid(0xffffff, 0.12 * l * a)?;
            self.rt.FillRoundedRectangle(&rr(r, 12.0), &light);
        }
        self.text(label, &self.app.fonts.button, r, INK, a)
    }

    unsafe fn link(&self, hit: Hit, label: &str, chevron: Option<f32>, a: f32, dy: f32) -> Result<()> {
        let r = self.app.link_rect(label);
        let r = rect(r.left, r.top + dy, r.right - r.left, r.bottom - r.top);
        let l = self.app.level(hit);
        let c = if l > 0.5 { TEXT } else { TEXT2 };
        match chevron {
            Some(up) => {
                let tw = self.app.measure(label, &self.app.fonts.link);
                let x = (W - tw - 16.0) / 2.0;
                self.text(label, &self.app.fonts.link, rect(x, r.top, tw, r.bottom - r.top), c, a)?;
                self.chevron(x + tw + 10.0, (r.top + r.bottom) / 2.0, up, c, a)?;
            }
            None => self.text(label, &self.app.fonts.link, r, c, a)?,
        }
        Ok(())
    }

    unsafe fn checkbox(&self, hit: Hit, y: f32, label: &str, on: bool, a: f32) -> Result<()> {
        let b = rect(PAD, y + 6.0, 20.0, 20.0);
        if on {
            let g = self.gradient(b.left, b.right, a)?;
            self.rt.FillRoundedRectangle(&rr(b, 6.0), &g);
            self.check(PAD + 10.0, y + 16.0, 12.0, INK, a, 2.0)?;
        } else {
            let l = self.app.level(hit);
            let br = self.solid(if l > 0.5 { TEXT3 } else { LINE }, a)?;
            self.rt.DrawRoundedRectangle(&rr(rect(b.left + 0.75, b.top + 0.75, 18.5, 18.5), 5.5), &br, 1.5, None);
        }
        self.text(label, &self.app.fonts.label, rect(PAD + 32.0, y, W - 2.0 * PAD - 32.0, 32.0), TEXT, a)
    }

    unsafe fn welcome(&self, a: f32, dy: f32) -> Result<()> {
        let app = self.app;
        self.logo(a, dy)?;
        // Language pill.
        let r = app.lang_pill();
        let r = rect(r.left, r.top + dy, r.right - r.left, r.bottom - r.top);
        let l = app.level(Hit::Lang).max(if app.menu { 1.0 } else { 0.0 });
        let bg = self.solid(PANEL, a)?;
        self.rt.FillRoundedRectangle(&rr(r, 8.0), &bg);
        let br = self.solid(if l > 0.5 { TEXT3 } else { LINE }, a)?;
        self.rt.DrawRoundedRectangle(&rr(rect(r.left + 0.5, r.top + 0.5, r.right - r.left - 1.0, 29.0), 8.0), &br, 1.0, None);
        self.text(LANGS[app.lang].name, &app.fonts.small, rect(r.left + 12.0, r.top, r.right - r.left - 36.0, 30.0), TEXT2, a)?;
        self.chevron(r.right - 16.0, r.top + 15.0, app.menu_anim.value(), TEXT2, a)?;

        self.text("GeniusClip", &app.fonts.title, rect(PAD, 124.0 + dy, W - 2.0 * PAD, 44.0), TEXT, a)?;
        self.text(&app.s("tagline"), &app.fonts.body, rect(PAD, 170.0 + dy, W - 2.0 * PAD, 44.0), TEXT2, a)?;
        let note = match &app.installed {
            Some(i) => app.s("installed").replace("{v}", &i.version),
            None => app.s("version").replace("{v}", install::VERSION),
        };
        self.text(&note, &app.fonts.small, rect(PAD, 214.0 + dy, W - 2.0 * PAD, 22.0), TEXT3, a)?;

        // Options, sliding open under the text.
        let o = app.opts_anim.value();
        if o > 0.01 {
            let oa = a * o;
            let oy = dy - (1.0 - o) * 10.0;
            self.text(&app.s("folder"), &app.fonts.small, rect(PAD, 252.0 + oy, 200.0, 22.0), TEXT3, oa)?;
            let fr = rect(PAD, 276.0 + oy, W - 2.0 * PAD, 42.0);
            let panel = self.solid(PANEL, oa)?;
            self.rt.FillRoundedRectangle(&rr(fr, 10.0), &panel);
            let lvl = app.level(Hit::ChangeDir);
            let border = self.solid(if lvl > 0.5 { TEXT3 } else { LINE }, oa)?;
            self.rt.DrawRoundedRectangle(&rr(rect(fr.left + 0.5, fr.top + 0.5, fr.right - fr.left - 1.0, 41.0), 10.0), &border, 1.0, None);
            let change = app.s("change");
            let cw = app.measure(&change, &app.fonts.small);
            self.text(&change, &app.fonts.small, rect(fr.right - cw - 14.0, fr.top, cw, 42.0), ACCENT, oa)?;
            let path = app.dir.display().to_string();
            self.text(&path, &app.fonts.small, rect(fr.left + 14.0, fr.top, fr.right - fr.left - cw - 42.0, 42.0), TEXT2, oa)?;
            self.checkbox(Hit::Autostart, 330.0 + oy, &app.s("autostart"), app.autostart, oa)?;
            self.checkbox(Hit::Shortcut, 366.0 + oy, &app.s("shortcut"), app.shortcut, oa)?;
        }

        self.primary(&app.primary_label(), a, dy)?;
        self.link(Hit::Options, &app.s("options"), Some(o), a, dy)
    }

    unsafe fn installing(&self, a: f32, dy: f32) -> Result<()> {
        let app = self.app;
        self.logo(1.0, 0.0)?;
        let updating = app.installed.as_ref().is_some_and(|i| install::compare_installed(&i.version) < 0);
        let title = app.s(if updating { "updating" } else { "installing" });
        self.text(&title, &app.fonts.heading, rect(PAD, 124.0 + dy, W - 2.0 * PAD, 40.0), TEXT, a)?;
        self.text(&app.dir.display().to_string(), &app.fonts.small, rect(PAD, 166.0 + dy, W - 2.0 * PAD, 22.0), TEXT3, a)?;

        let pct = format!("{}%", (app.progress * 100.0).round() as i32);
        let pw = app.measure(&pct, &app.fonts.small);
        self.text(&format!("GeniusClip {}", install::VERSION), &app.fonts.small, rect(PAD, 440.0, 240.0, 22.0), TEXT3, a)?;
        self.text(&pct, &app.fonts.small, rect(W - PAD - pw, 440.0, pw + 2.0, 22.0), TEXT2, a)?;
        let track = rect(PAD, 470.0, W - 2.0 * PAD, 8.0);
        let tb = self.solid(0x26262c, a)?;
        self.rt.FillRoundedRectangle(&rr(track, 4.0), &tb);
        let fw = (track.right - track.left) * app.progress.clamp(0.02, 1.0);
        let fill = rect(track.left, track.top, fw, 8.0);
        let g = self.gradient(track.left, track.right, a)?;
        self.rt.FillRoundedRectangle(&rr(fill, 4.0), &g);
        // A soft highlight sweeping along the filled part.
        let t = app.install_started.elapsed().as_secs_f32();
        let sx = track.left + ((t * 0.8) % 1.4 - 0.2) * fw;
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: rgb(0xffffff, 0.0) },
            D2D1_GRADIENT_STOP { position: 0.5, color: rgb(0xffffff, 0.35 * a) },
            D2D1_GRADIENT_STOP { position: 1.0, color: rgb(0xffffff, 0.0) },
        ];
        let coll = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let shine = self.rt.CreateLinearGradientBrush(&D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(sx - 40.0, 0.0), endPoint: pt(sx + 40.0, 0.0) }, None, &coll)?;
        self.rt.PushAxisAlignedClip(&fill, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        self.rt.FillRectangle(&fill, &shine);
        self.rt.PopAxisAlignedClip();
        Ok(())
    }

    unsafe fn done(&self, a: f32, dy: f32) -> Result<()> {
        let app = self.app;
        // Success mark in place of the logo.
        let (cx, cy) = (PAD + 32.0, 76.0 + dy);
        let fill = self.solid(ACCENT, 0.14 * a)?;
        self.rt.FillEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: 31.0, radiusY: 31.0 }, &fill);
        let g = self.gradient(cx - 31.0, cx + 31.0, a)?;
        self.rt.DrawEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: 30.0, radiusY: 30.0 }, &g, 2.0, None);
        self.check(cx, cy, 26.0, ACCENT, a, 3.0)?;

        self.text(&app.s("done"), &app.fonts.heading, rect(PAD, 124.0 + dy, W - 2.0 * PAD, 40.0), TEXT, a)?;
        self.text(&app.s("hint"), &app.fonts.body, rect(PAD, 170.0 + dy, W - 2.0 * PAD, 24.0), TEXT2, a)?;
        // Hotkey as key chips, like on the app's home screen.
        let mut x = PAD;
        let y = 204.0 + dy;
        let parts = hotkey_parts(&app.hotkey);
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                self.text("+", &app.fonts.chip, rect(x, y, 18.0, 34.0), TEXT3, a)?;
                x += 18.0;
            }
            let w = app.measure(part, &app.fonts.chip).max(14.0) + 22.0;
            let r = rect(x, y, w, 34.0);
            let bg = self.solid(PANEL, a)?;
            self.rt.FillRoundedRectangle(&rr(r, 8.0), &bg);
            let br = self.solid(LINE, a)?;
            self.rt.DrawRoundedRectangle(&rr(rect(r.left + 0.5, r.top + 0.5, w - 1.0, 33.0), 8.0), &br, 1.0, None);
            // Key "depth": a slightly darker bottom edge.
            let edge = self.solid(0x000000, 0.25 * a)?;
            self.rt.FillRectangle(&rect(r.left + 6.0, r.bottom - 2.0, w - 12.0, 1.5), &edge);
            self.text(part, &app.fonts.chip, r, TEXT, a)?;
            x += w;
        }
        self.primary(&app.primary_label(), a, dy)?;
        self.link(Hit::Secondary, &app.s("close"), None, a, dy)
    }

    unsafe fn failed(&self, msg: &str, a: f32, dy: f32) -> Result<()> {
        let app = self.app;
        let (cx, cy) = (PAD + 32.0, 76.0 + dy);
        let fill = self.solid(DANGER, 0.12 * a)?;
        self.rt.FillEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: 31.0, radiusY: 31.0 }, &fill);
        let ring = self.solid(DANGER, a)?;
        self.rt.DrawEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: 30.0, radiusY: 30.0 }, &ring, 2.0, None);
        self.line(cx, cy - 11.0, cx, cy + 3.0, DANGER, a, 3.0)?;
        self.line(cx, cy + 10.5, cx, cy + 10.6, DANGER, a, 3.4)?;

        self.text(&app.s("failed"), &app.fonts.heading, rect(PAD, 124.0 + dy, W - 2.0 * PAD, 40.0), TEXT, a)?;
        self.text(msg, &app.fonts.body, rect(PAD, 170.0 + dy, W - 2.0 * PAD, 200.0), TEXT2, a)?;
        self.primary(&app.primary_label(), a, dy)?;
        self.link(Hit::Secondary, &app.s("close"), None, a, dy)
    }

    unsafe fn menu(&self) -> Result<()> {
        let app = self.app;
        let m = app.menu_anim.value();
        if m < 0.01 || app.stage != Stage::Welcome {
            return Ok(());
        }
        let x = W - PAD - MENU_W;
        let y = 98.0 - (1.0 - m) * 6.0;
        let h = LANGS.len() as f32 * MENU_ITEM + 12.0;
        for i in 1..6 {
            let g = i as f32 * 2.0;
            let sh = self.solid(0x000000, 0.06 * m)?;
            self.rt.FillRoundedRectangle(&rr(rect(x - g * 0.5, y + g * 0.6, MENU_W + g, h + g * 0.6), 12.0 + g * 0.5), &sh);
        }
        let panel = rect(x, y, MENU_W, h);
        let bg = self.solid(0x1f1f25, m)?;
        self.rt.FillRoundedRectangle(&rr(panel, 12.0), &bg);
        let br = self.solid(LINE, m)?;
        self.rt.DrawRoundedRectangle(&rr(rect(x + 0.5, y + 0.5, MENU_W - 1.0, h - 1.0), 12.0), &br, 1.0, None);
        for (i, lang) in LANGS.iter().enumerate() {
            let r = rect(x + 6.0, y + 6.0 + i as f32 * MENU_ITEM, MENU_W - 12.0, MENU_ITEM);
            let l = app.level(Hit::LangItem(i));
            if l > 0.01 {
                let hb = self.solid(0xffffff, 0.07 * l * m)?;
                self.rt.FillRoundedRectangle(&rr(r, 7.0), &hb);
            }
            let current = i == app.lang;
            self.text(lang.name, &app.fonts.menu, rect(r.left + 10.0, r.top, r.right - r.left - 40.0, MENU_ITEM), if current { TEXT } else { TEXT2 }, m)?;
            if current {
                self.check(r.right - 16.0, r.top + MENU_ITEM / 2.0, 12.0, ACCENT, m, 1.8)?;
            }
        }
        Ok(())
    }
}

/// "Control+Shift+KeyK" → ["Ctrl", "Shift", "K"] (same as the app's hotkeyParts).
fn hotkey_parts(accel: &str) -> Vec<String> {
    accel
        .split('+')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let p = p.trim();
            if let Some(k) = p.strip_prefix("Key").filter(|k| k.len() == 1) {
                k.to_string()
            } else if let Some(d) = p.strip_prefix("Digit").filter(|d| d.len() == 1) {
                d.to_string()
            } else {
                match p {
                    "Control" => "Ctrl".into(),
                    "Super" => "Win".into(),
                    other => other.into(),
                }
            }
        })
        .collect()
}

unsafe fn app_of(hwnd: HWND) -> Option<&'static mut App> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    p.as_mut()
}

fn dips(app: &App, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xffff) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
    (x * 96.0 / app.dpi, y * 96.0 / app.dpi)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            let app = cs.lpCreateParams as *mut App;
            (*app).hwnd = hwnd;
            (*app).dpi = GetDpiForWindow(hwnd) as f32;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let Some(app) = app_of(hwnd) else { return DefWindowProcW(hwnd, msg, wp, lp) };
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                app.paint();
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_TIMER => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_SIZE => {
                if let Some(rt) = &app.rt {
                    let size = D2D_SIZE_U { width: (lp.0 & 0xffff) as u32, height: ((lp.0 >> 16) & 0xffff) as u32 };
                    let _ = rt.Resize(&size);
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                app.dpi = (wp.0 & 0xffff) as f32;
                if let Some(rt) = &app.rt {
                    rt.SetDpi(app.dpi, app.dpi);
                }
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                app.kick();
                LRESULT(0)
            }
            WM_NCHITTEST => {
                // Empty areas drag the window; controls get mouse input.
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut p);
                let (x, y) = (p.x as f32 * 96.0 / app.dpi, p.y as f32 * 96.0 / app.dpi);
                if app.menu || app.hit_at(x, y).is_some() {
                    LRESULT(HTCLIENT as isize)
                } else {
                    LRESULT(HTCAPTION as isize)
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = dips(app, lp);
                let hit = app.hit_at(x, y).filter(|h| *h != Hit::Backdrop);
                if hit != app.hover {
                    app.hover = hit;
                    app.kick();
                }
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
                LRESULT(0)
            }
            WM_MOUSELEAVE | WM_NCMOUSEMOVE => {
                if app.hover.is_some() {
                    app.hover = None;
                    app.kick();
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xffff) as u32 == HTCLIENT && app.hover.is_some() {
                    if let Ok(c) = LoadCursorW(None, IDC_HAND) {
                        SetCursor(Some(c));
                        return LRESULT(1);
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = dips(app, lp);
                app.pressed = app.hit_at(x, y);
                SetCapture(hwnd);
                app.kick();
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = dips(app, lp);
                let pressed = app.pressed.take();
                if let Some(hit) = pressed.filter(|p| app.hit_at(x, y) == Some(*p)) {
                    app.click(hit);
                }
                app.kick();
                LRESULT(0)
            }
            WM_KEYDOWN => {
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_RETURN if app.stage != Stage::Installing && !app.menu => app.click(Hit::Primary),
                    VK_ESCAPE if app.menu => app.click(Hit::Backdrop),
                    VK_ESCAPE if app.stage != Stage::Installing => app.click(Hit::Close),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_INSTALL_DONE => {
                app.install_done();
                LRESULT(0)
            }
            WM_CLOSE => {
                // Never leave a half-finished installation behind.
                if app.stage != Stage::Installing {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
