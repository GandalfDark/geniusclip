//! User actions triggered by hotkeys, the tray menu or the UI.

use crate::i18n::t;
use crate::library::Kind;
use crate::overlay::{self, Toast};
use crate::settings::Settings;
use crate::state::AppState;
use geniusclip_engine::game::{self, AppInfo};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// A fullscreen app on the monitor being recorded (the game), if any.
fn fullscreen_on_capture_monitor(app: &AppHandle) -> Option<AppInfo> {
    let want = app.state::<AppState>().settings.read().engine.monitor.clone();
    // Cached: enumerating displays while a game runs can make it stutter.
    let mons = crate::monitors::list();
    let m = want.as_deref().and_then(|id| mons.iter().find(|m| m.id == id)).or_else(|| mons.iter().find(|m| m.primary))?;
    game::fullscreen_app(m.x, m.y, m.width, m.height)
}

/// The game a clip belongs to: a fullscreen app on the recorded monitor,
/// else the foreground app, else the last game seen during the replay window.
pub fn current_game(app: &AppHandle) -> AppInfo {
    let st = app.state::<AppState>();
    if let Some(a) = fullscreen_on_capture_monitor(app) {
        return a;
    }
    let fg = game::foreground_app();
    if let Some(a) = fg.as_ref().filter(|a| !a.is_desktop) {
        return a.clone();
    }
    let window = Duration::from_secs(st.settings.read().replay_seconds as u64);
    if let Some((a, seen)) = st.last_game.lock().as_ref() {
        if seen.elapsed() <= window {
            return a.clone();
        }
    }
    let lang = st.settings.read().lang();
    AppInfo { name: t(lang, "desktop").into(), exe: String::new(), is_desktop: true }
}

/// Names handed out recently. Files are written in the background, so a
/// second save within the same second would not see the first one on disk
/// yet and pick the same name.
static RESERVED: Mutex<Vec<(PathBuf, Instant)>> = Mutex::new(Vec::new());

fn target_path(settings: &Settings, root: &Path, game: &AppInfo, suffix: &str, ext: &str) -> PathBuf {
    let name = game::sanitize(&game.name);
    let dir = if settings.sort_by_game { root.join(&name) } else { root.to_path_buf() };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let base = format!("{name} {stamp}{suffix}");
    let mut reserved = RESERVED.lock();
    // Names are stamped to the second, so an old reservation can't collide.
    reserved.retain(|(_, at)| at.elapsed() < Duration::from_secs(600));
    let taken = |p: &PathBuf| p.exists() || reserved.iter().any(|(r, _)| r == p);
    let mut p = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while taken(&p) {
        p = dir.join(format!("{base} ({n}).{ext}"));
        n += 1;
    }
    reserved.push((p.clone(), Instant::now()));
    p
}

fn ensure_dir(p: &Path) -> anyhow::Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}

fn fail(app: &AppHandle, e: impl std::fmt::Display) {
    log::warn!("action failed: {e}");
    overlay::toast(app, Toast::error(&e.to_string()));
}

pub fn save_clip(app: &AppHandle) {
    save(app, None, false);
}

/// The short-clip hotkey: only the last few seconds (setting), e.g. a quick joke.
pub fn save_short_clip(app: &AppHandle) {
    let secs = app.state::<AppState>().settings.read().short_seconds;
    save(app, Some(secs), false);
}

/// A clip of the last `seconds` (auto clips). Nobody pressed a key, so
/// nothing is said when there is no replay to save from.
pub fn save_seconds(app: &AppHandle, seconds: u32) {
    save(app, Some(seconds), true);
}

fn save(app: &AppHandle, seconds: Option<u32>, quiet: bool) {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    let say = |what| {
        if !quiet {
            overlay::toast(app, Toast::simple(what));
        }
    };
    if !s.replay_enabled {
        say("replay-off-hint");
        return;
    }
    if crate::power::waiting_for_game() {
        say("replay-wait");
        return;
    }
    // Only the battery pause can meet a key press (the others are a dark or
    // locked screen); the replay is empty while paused.
    if crate::power::system_paused() {
        say("replay-paused");
        return;
    }
    let g = current_game(app);
    let path = target_path(&s, &s.clips_dir, &g, "", "mp4");
    let res = ensure_dir(&path).and_then(|_| {
        st.library.expect(&path, Kind::Clip, &g.name);
        st.engine.save_replay(path.clone(), seconds, "GeniusClip".into(), s.skip_saved)
    });
    match res {
        Ok(geniusclip_engine::SaveOutcome::Started) => {}
        Ok(geniusclip_engine::SaveOutcome::NothingNew) => say("already-saved"),
        Err(e) => fail(app, e),
    }
}

pub fn toggle_replay(app: &AppHandle) {
    let on = !app.state::<AppState>().settings.read().replay_enabled;
    set_replay(app, on);
    overlay::toast(app, Toast::simple(if on { "replay-on" } else { "replay-off" }));
}

/// Serializes changes to the engine's configuration. Each change reads the
/// settings once it holds this, so a slow pipeline restart can't finish
/// after a newer change and put the older state back.
static ENGINE_APPLY: Mutex<()> = Mutex::new(());

/// Applies the current capture settings and buffer length to the engine.
pub fn configure_engine(app: &AppHandle) {
    let _one_at_a_time = ENGINE_APPLY.lock();
    let st = app.state::<AppState>();
    let (cfg, secs) = {
        let s = st.settings.read();
        (s.engine.clone(), s.replay_seconds)
    };
    if let Err(e) = st.engine.configure(cfg, secs) {
        log::error!("configure: {e:#}");
    }
}

/// Applies the current replay switch to the engine.
pub fn apply_replay(app: &AppHandle) -> anyhow::Result<()> {
    let _one_at_a_time = ENGINE_APPLY.lock();
    let st = app.state::<AppState>();
    let on = st.settings.read().replay_enabled;
    st.engine.set_replay_enabled(on)
}

pub fn set_replay(app: &AppHandle, on: bool) {
    let st = app.state::<AppState>();
    {
        let mut s = st.settings.write();
        s.replay_enabled = on;
        let _ = s.save(app);
    }
    // Game tracking runs only with replay on.
    crate::wake_ticker();
    if on {
        // "Only in games" decides first whether capture starts at all.
        crate::power::sync(app);
    } else {
        forget_game();
    }
    if let Err(e) = apply_replay(app) {
        fail(app, e);
    }
    crate::tray::refresh(app);
    crate::emit_settings(app);
}

pub fn screenshot(app: &AppHandle) {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    let g = current_game(app);
    let path = target_path(&s, &s.screenshots_dir, &g, "", "png");
    let res = ensure_dir(&path).and_then(|_| {
        st.library.expect(&path, Kind::Screenshot, &g.name);
        st.engine.screenshot(path.clone())
    });
    if let Err(e) = res {
        fail(app, e);
    }
}

pub fn toggle_recording(app: &AppHandle) {
    let st = app.state::<AppState>();
    let res = if st.engine.is_recording() {
        st.engine.stop_recording()
    } else {
        let s = st.settings.read().clone();
        let g = current_game(app);
        let path = target_path(&s, &s.clips_dir, &g, " REC", "mp4");
        ensure_dir(&path).and_then(|_| {
            st.library.expect(&path, Kind::Recording, &g.name);
            st.engine.start_recording(path, "GeniusClip recording".into())
        })
    };
    if let Err(e) = res {
        fail(app, e);
    }
    crate::tray::refresh(app);
}

/// After a clip, recording or screenshot has been written: "first steps"
/// progress, and a warning when the clips drive is running out of space.
pub fn after_save(app: &AppHandle, kind: Kind) {
    if kind == Kind::Clip {
        Settings::mark_onboarding(app, |o| &mut o.clip_saved);
    }
    warn_if_disk_low(app);
}

/// When the low-space toast was last shown; it comes at most this often.
static DISK_WARNED: Mutex<Option<Instant>> = Mutex::new(None);
const DISK_WARN_EVERY: Duration = Duration::from_secs(10 * 60);

fn warn_if_disk_low(app: &AppHandle) {
    if DISK_WARNED.lock().is_some_and(|t| t.elapsed() < DISK_WARN_EVERY) {
        return;
    }
    let h = app.clone();
    // Off the engine's thread (the estimate enumerates monitors), and once
    // the "saved" toast has played: a new toast would cut it short.
    std::thread::spawn(move || {
        std::thread::sleep(overlay::SHOWN_FOR + Duration::from_millis(200));
        let settings = h.state::<AppState>().settings.read().clone();
        let space = match crate::disk::space(&settings) {
            Ok(s) if s.low => s,
            Ok(_) => return,
            Err(e) => {
                log::info!("disk space: {e:#}");
                return;
            }
        };
        {
            // Several saves in a row each get here: one toast.
            let mut warned = DISK_WARNED.lock();
            if warned.is_some_and(|t| t.elapsed() < DISK_WARN_EVERY) {
                return;
            }
            *warned = Some(Instant::now());
        }
        log::warn!("low disk space: {} MB free on {} (warning below {} MB)", space.free_mb, space.drive, space.low_mb);
        overlay::toast(&h, Toast::disk_low(space.free_mb, &space.drive));
    });
}

/// Whether `track_foreground` has anything to do: only clips from the replay
/// buffer use the last game, so there is none while replay is off or paused
/// (but it runs while "Only in games" waits: it is what spots the game).
pub fn tracking_foreground(app: &AppHandle) -> bool {
    app.state::<AppState>().settings.read().replay_enabled && !crate::power::system_paused()
}

/// What decides `track_foreground`'s answer, all of it cheap to read: the
/// foreground window (its rect too: a game going fullscreen on the recorded
/// monitor changes the answer), and the monitor it looks at.
#[derive(PartialEq)]
struct TrackKey {
    hwnd: isize,
    pid: u32,
    rect: (i32, i32, i32, i32),
    monitors: u64,
    monitor: Option<String>,
}

fn track_key(app: &AppHandle) -> TrackKey {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};
    let hwnd = unsafe { GetForegroundWindow() };
    let mut pid = 0u32;
    let mut r = RECT::default();
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = GetWindowRect(hwnd, &mut r);
    }
    TrackKey {
        hwnd: hwnd.0 as isize,
        pid,
        rect: (r.left, r.top, r.right, r.bottom),
        monitors: crate::monitors::generation(),
        monitor: app.state::<AppState>().settings.read().engine.monitor.clone(),
    }
}

/// A lookup: what it was made for and when, the app in front, and whether
/// a game fills the recorded monitor.
struct Tracked {
    key: TrackKey,
    at: Instant,
    found: Option<AppInfo>,
    in_game: bool,
}

/// The last `track_foreground` lookup.
static TRACKED: Mutex<Option<Tracked>> = Mutex::new(None);

/// A game on another monitor than the focused window can close without the
/// focus changing: the lookup is redone this often anyway.
const TRACK_REFRESH: Duration = Duration::from_secs(10);

/// Called every second: remembers the last game in focus, and runs the
/// "Only in games" check.
pub fn track_foreground(app: &AppHandle) {
    if !tracking_foreground(app) {
        return;
    }
    // Walking every window, opening the game's process and reading its
    // version info each second is wasted work while the same window stays in
    // front: the lookup is redone only when the foreground window changes.
    let key = track_key(app);
    let (found, in_game) = {
        let mut tracked = TRACKED.lock();
        match tracked.as_ref() {
            // The periodic refresh is only for "Only in games": naming clips
            // after a game that closed meanwhile is harmless.
            Some(t) if t.key == key && (t.at.elapsed() < TRACK_REFRESH || !games_only(app)) => (t.found.clone(), t.in_game),
            _ => {
                let fullscreen = fullscreen_on_capture_monitor(app);
                let in_game = fullscreen.as_ref().is_some_and(game::is_game);
                let found = fullscreen.or_else(|| game::foreground_app().filter(|a| !a.is_desktop));
                *tracked = Some(Tracked { key, at: Instant::now(), found: found.clone(), in_game });
                (found, in_game)
            }
        }
    };
    if let Some(a) = found {
        *app.state::<AppState>().last_game.lock() = Some((a, Instant::now()));
    }
    games_gate(app, in_game);
}

/// When "Only in games" last saw a game on the recorded monitor.
static GAME_SEEN: Mutex<Option<Instant>> = Mutex::new(None);

/// "Only in games": capture waits while no game (a fullscreen app that isn't
/// a browser or a player) is on the recorded monitor. After one leaves it
/// goes on for the replay's length: a quick alt-tab keeps the game in the
/// replay, and past that the replay would hold none of it anyway.
fn games_gate(app: &AppHandle, in_game: bool) {
    crate::power::set_waiting_for_game(games_waiting_for(app, in_game));
}

fn games_only(app: &AppHandle) -> bool {
    app.state::<AppState>().settings.read().games_only
}

/// Whether "Only in games" holds capture, given whether a game is on the
/// recorded monitor right now.
fn games_waiting_for(app: &AppHandle, in_game: bool) -> bool {
    let st = app.state::<AppState>();
    let (on, secs) = {
        let s = st.settings.read();
        (s.games_only, s.replay_seconds)
    };
    on && {
        let mut seen = GAME_SEEN.lock();
        if in_game {
            *seen = Some(Instant::now());
        }
        seen.is_none_or(|t| t.elapsed() > Duration::from_secs(secs as u64))
    }
}

/// The same with a fresh look at the recorded monitor, for when the tick
/// loop isn't checking (replay off, a pause ending, startup).
pub fn games_waiting(app: &AppHandle) -> bool {
    if !games_only(app) {
        return false;
    }
    let in_game = fullscreen_on_capture_monitor(app).as_ref().is_some_and(game::is_game);
    games_waiting_for(app, in_game)
}

/// The replay that held the last game seen is gone (a pause, replay off):
/// no grace after it any more.
pub fn forget_game() {
    *GAME_SEEN.lock() = None;
}

/// The "Only in games" check right away (after the setting changed).
pub fn check_games_gate(app: &AppHandle) {
    *TRACKED.lock() = None;
    crate::power::set_waiting_for_game(games_waiting(app));
}

/// Copies the newest clip or recording to the clipboard (tray action).
pub fn copy_last_clip(app: &AppHandle) {
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    let newest = st.library.scan(&s).into_iter().find(|e| e.kind != Kind::Screenshot);
    let Some(e) = newest else { return };
    match crate::overlay::window().ok_or_else(|| anyhow::anyhow!("no window")).and_then(|w| crate::share::copy_files(w, &[e.path])) {
        Ok(()) => overlay::toast(app, Toast::simple("copied")),
        Err(err) => fail(app, err),
    }
}
