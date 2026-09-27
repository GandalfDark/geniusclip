//! In-game menu (default Alt+X): a transparent always-on-top window over the
//! game's monitor, like NVIDIA's overlay but without touching the game. The
//! page (src/routes/menu) animates in on `menu://open` and asks to be hidden
//! with the `menu_close` command after its closing animation.
//!
//! While the menu is open, video capture holds the last game frame (see
//! `Engine::set_hold`): the menu covers the whole screen, and a WebView window
//! excluded from capture comes out black in Desktop Duplication. The menu's
//! screenshot button briefly turns the page invisible and captures for real.

use crate::overlay::{self, Toast};
use crate::state::AppState;
use parking_lot::{Condvar, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow};
use std::time::{Duration, Instant};

pub const LABEL: &str = "menu";

static OPEN: AtomicBool = AtomicBool::new(false);
/// The page has started (menu_ready). A new WebView takes a moment to load
/// its page; until then focus bounces around and the page can't hear events.
static READY: AtomicBool = AtomicBool::new(false);
/// When the menu was last shown.
static OPENED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// Bumped on every open/hide, so a delayed unload can tell if it's stale.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Held through open, hide and the delayed unload/resume, so the unload
/// can't destroy the window between open() fetching it and showing it.
/// Never taken on the UI thread: open() waits for it while creating the window.
static LOCK: Mutex<()> = Mutex::new(());
/// The hidden menu's WebView (~100 MB) is freed after this long unused.
const UNLOAD_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Until then it is suspended (`set_asleep`), this long after being hidden.
const SLEEP_AFTER: Duration = Duration::from_secs(1);
/// The pending unload: when, and the generation of the hide that asked for
/// it. One timer thread serves every hide (a newer hide replaces it).
static UNLOAD: Mutex<Option<(Instant, u64)>> = Mutex::new(None);
static UNLOAD_CHANGED: Condvar = Condvar::new();
static UNLOAD_TIMER: OnceLock<()> = OnceLock::new();
/// The window that was in front (the game), to give focus back on close.
static PREV: Mutex<isize> = Mutex::new(0);
/// A destroy of the menu window was requested and it hasn't gone yet.
/// `destroy()` only queues it on the UI thread: until then the dying window
/// is still returned by `get_webview_window`, and showing it would leave the
/// menu flagged open with no page (presses ignored until the 5 s recovery).
static UNLOADING: AtomicBool = AtomicBool::new(false);
/// How long `open` waits for a dying window to go before giving up.
const DESTROY_WAIT: Duration = Duration::from_secs(2);

/// Freezes the video on the game, labelled "menu is open" (or resumes it).
pub fn hold(app: &AppHandle, on: bool) {
    let card = on.then(|| {
        let s = app.state::<AppState>().settings.read().clone();
        let lang = s.lang();
        let c = overlay::accent(&s.accent);
        geniusclip_engine::HoldCard {
            title: crate::i18n::t(lang, "menu.hold-title").into(),
            subtitle: crate::i18n::t(lang, "menu.hold-hint").into(),
            accent: [(c >> 16) as u8, (c >> 8) as u8, c as u8],
        }
    });
    app.state::<AppState>().engine.set_hold(card);
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// The page has loaded and listens to menu events.
pub fn ready() {
    READY.store(true, Ordering::Relaxed);
}

fn opened_within(d: Duration) -> bool {
    OPENED_AT.lock().is_some_and(|t| t.elapsed() < d)
}

pub fn toggle(app: &AppHandle) {
    if OPEN.load(Ordering::Relaxed) && !READY.load(Ordering::Relaxed) {
        // Still starting: the page opens itself once loaded, and a press now
        // would be lost (or close it right away). Give up on a page that
        // never came up.
        if opened_within(Duration::from_secs(5)) {
            return;
        }
        log::warn!("menu page did not start; recreating it");
        hide(app, false);
        if let Some(w) = app.get_webview_window(LABEL) {
            destroy(&w);
        }
    }
    if OPEN.load(Ordering::Relaxed) {
        let shown = app.get_webview_window(LABEL).is_some_and(|w| w.is_visible().unwrap_or(false));
        if shown {
            // Let the page play its closing animation; it then calls menu_close.
            let _ = app.emit_to(LABEL, "menu://close", ());
            return;
        }
        // Flagged open, but the window is gone or hidden: it is closed as far
        // as the user can tell, so reset and open it again.
        hide(app, false);
    }
    open(app);
}

fn open(app: &AppHandle) {
    // A game in true exclusive fullscreen would minimize if another window
    // took focus; say so instead.
    if unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN) {
        overlay::toast(app, Toast::simple("menu-unavailable"));
        return;
    }
    let _guard = LOCK.lock();
    let fg = unsafe { GetForegroundWindow() };
    *PREV.lock() = fg.0 as isize;
    // Freeze the video on the game before the menu shows up (the capture
    // loop checks once per frame).
    hold(app, true);
    std::thread::sleep(std::time::Duration::from_millis(40));
    let win = match window(app) {
        Ok(w) => w,
        Err(e) => {
            log::error!("menu window: {e}");
            // No menu after all: don't leave clips frozen on the card.
            hold(app, false);
            return;
        }
    };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let r = unsafe {
        let _ = GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTOPRIMARY), &mut info);
        info.rcMonitor
    };
    // Wake the page first: queued on the UI thread ahead of the show and of
    // menu://open, so the event never waits in a suspended renderer.
    set_asleep(&win, false);
    let _ = win.set_position(PhysicalPosition::new(r.left, r.top));
    let _ = win.set_size(PhysicalSize::new((r.right - r.left) as u32, (r.bottom - r.top) as u32));
    let _ = win.show();
    let _ = win.set_focus();
    *OPENED_AT.lock() = Some(Instant::now());
    OPEN.store(true, Ordering::Relaxed);
    GENERATION.fetch_add(1, Ordering::Relaxed);
    let _ = app.emit_to(LABEL, "menu://open", ());
    crate::settings::Settings::mark_onboarding(app, |o| &mut o.menu_opened);
}

/// The menu window to show: the existing one, or a new one once a window
/// being destroyed is really gone. Waits, so never on the UI thread (which
/// delivers the destruction); `open` runs on a hotkey thread.
fn window(app: &AppHandle) -> anyhow::Result<WebviewWindow> {
    if UNLOADING.load(Ordering::SeqCst) {
        // Tauri drops the window from its list right before the Destroyed
        // handlers run: once it is gone here, a new one can take its label.
        let until = Instant::now() + DESTROY_WAIT;
        while app.get_webview_window(LABEL).is_some() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if app.get_webview_window(LABEL).is_some() {
            // The next press uses whatever window there is then.
            UNLOADING.store(false, Ordering::SeqCst);
            anyhow::bail!("the previous menu window is still closing");
        }
    }
    match app.get_webview_window(LABEL) {
        Some(w) => Ok(w),
        None => Ok(create(app)?),
    }
}

/// Destroys the menu window (asynchronously, see `UNLOADING`).
fn destroy(w: &WebviewWindow) {
    UNLOADING.store(true, Ordering::SeqCst);
    if let Err(e) = w.destroy() {
        log::warn!("menu window destroy: {e}");
        UNLOADING.store(false, Ordering::SeqCst);
    }
}

fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    READY.store(false, Ordering::Relaxed);
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
    // Clicking into another window (or Alt+Tab) closes the menu. Focus also
    // bounces while the WebView starts and when a game grabs it back right
    // after the menu shows: only a real switch to another app counts.
    let handle = app.clone();
    win.on_window_event(move |e| match e {
        tauri::WindowEvent::Focused(false) if OPEN.load(Ordering::Relaxed) => {
            // Off the UI thread: hide() may wait for an open() in progress.
            let h = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                let settling = !READY.load(Ordering::Relaxed) || opened_within(Duration::from_millis(700));
                if OPEN.load(Ordering::Relaxed) && !settling && !foreground_is_ours() {
                    hide(&h, false);
                }
            });
        }
        tauri::WindowEvent::Destroyed => {
            READY.store(false, Ordering::Relaxed);
            UNLOADING.store(false, Ordering::SeqCst);
        }
        _ => {}
    });
    Ok(win)
}

/// The foreground window belongs to GeniusClip (the menu itself).
fn foreground_is_ours() -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid == std::process::id()
}

/// Hides the menu; with `restore_focus` the game gets keyboard and mouse back.
/// Not for the UI thread (see `LOCK`).
pub fn hide(app: &AppHandle, restore_focus: bool) {
    let _guard = LOCK.lock();
    OPEN.store(false, Ordering::Relaxed);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    // The page stops the player and stats polling while hidden.
    let _ = app.emit_to(LABEL, "menu://hidden", ());
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    // Still this hide, not a newer open (which holds its own frame).
    let current = move || GENERATION.load(Ordering::Relaxed) == generation && !OPEN.load(Ordering::Relaxed);
    // Resume capture once the window is off the screen.
    let h = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        {
            let _guard = LOCK.lock();
            if current() {
                hold(&h, false);
            }
        }
        // Then, once the page has handled menu://hidden (and the reply to
        // menu_close), put it to sleep until the next open or the unload.
        // Not while it is still starting: its first open depends on it.
        std::thread::sleep(SLEEP_AFTER);
        let _guard = LOCK.lock();
        if current() && READY.load(Ordering::Relaxed) && !UNLOADING.load(Ordering::SeqCst) {
            if let Some(w) = h.get_webview_window(LABEL) {
                set_asleep(&w, true);
            }
        }
    });
    schedule_unload(app, generation);
    let prev = *PREV.lock();
    if restore_focus && prev != 0 {
        // On the UI thread, queued after the hide above.
        let _ = app.run_on_main_thread(move || unsafe {
            let _ = SetForegroundWindow(HWND(prev as _));
        });
    }
}

/// Suspends the hidden menu's WebView (`asleep`) or wakes it before showing.
///
/// Suspended, the page runs no timers, animations or rendering and its
/// renderer's memory can be reused by the game; the low memory target trims
/// what stays resident. Best effort: older WebView2 runtimes lack these
/// interfaces, and a failed call leaves the page running as before. Runs on
/// the UI thread (WebView2's), after anything already queued for the window.
fn set_asleep(w: &WebviewWindow, asleep: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, ICoreWebView2_3, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows_core_webview2::Interface;
    let res = w.with_webview(move |pw| unsafe {
        let controller = pw.controller();
        let Ok(webview) = controller.CoreWebView2() else { return };
        let v3 = webview.cast::<ICoreWebView2_3>().ok();
        let v19 = webview.cast::<ICoreWebView2_19>().ok();
        if asleep {
            // The window is already hidden, but WebView2 still counts itself
            // visible, and TrySuspend refuses a visible WebView.
            let _ = controller.SetIsVisible(false);
            if let Some(v19) = &v19 {
                let _ = v19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
            }
            if let Some(v3) = &v3 {
                let done = webview2_com::TrySuspendCompletedHandler::create(Box::new(|res, suspended| {
                    // Refused while e.g. media plays: it just stays awake.
                    if res.is_err() || !suspended {
                        log::debug!("menu webview not suspended: {res:?}");
                    }
                    Ok(())
                }));
                if let Err(e) = v3.TrySuspend(&done) {
                    log::debug!("menu webview suspend: {e}");
                }
            }
        } else {
            if let Some(v3) = &v3 {
                let _ = v3.Resume();
            }
            if let Some(v19) = &v19 {
                let _ = v19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
            }
            let _ = controller.SetIsVisible(true);
        }
    });
    if let Err(e) = res {
        log::debug!("menu webview: {e}");
    }
}

/// Frees the menu's WebView after `UNLOAD_AFTER`, unless it is opened (or
/// hidden again, which sets a new deadline) before then.
fn schedule_unload(app: &AppHandle, generation: u64) {
    *UNLOAD.lock() = Some((Instant::now() + UNLOAD_AFTER, generation));
    UNLOAD_CHANGED.notify_one();
    UNLOAD_TIMER.get_or_init(|| {
        let app = app.clone();
        if let Err(e) = std::thread::Builder::new().name("gc-menu-unload".into()).spawn(move || unload_timer(app)) {
            log::warn!("menu unload timer: {e}");
        }
    });
}

fn unload_timer(app: AppHandle) {
    let mut pending = UNLOAD.lock();
    loop {
        match *pending {
            None => UNLOAD_CHANGED.wait(&mut pending),
            Some((at, _)) if Instant::now() < at => {
                UNLOAD_CHANGED.wait_until(&mut pending, at);
            }
            Some((_, generation)) => {
                *pending = None;
                // LOCK is taken with UNLOAD released: hide() takes them the
                // other way round.
                MutexGuard::unlocked(&mut pending, || {
                    let _guard = LOCK.lock();
                    // Still that hide, not a newer open or hide.
                    if GENERATION.load(Ordering::Relaxed) == generation && !OPEN.load(Ordering::Relaxed) {
                        if let Some(w) = app.get_webview_window(LABEL) {
                            destroy(&w);
                        }
                    }
                });
            }
        }
    }
}
