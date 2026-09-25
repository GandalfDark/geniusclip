//! User actions triggered by hotkeys, the tray menu or the UI.

use crate::i18n::t;
use crate::library::Kind;
use crate::overlay::{self, Toast};
use crate::settings::Settings;
use crate::state::AppState;
use geniusclip_engine::game::{self, AppInfo};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// The game a clip belongs to: the foreground app, or the last game seen
/// while the replay window was recording if the user is on the desktop / in our UI.
pub fn current_game(app: &AppHandle) -> AppInfo {
    let st = app.state::<AppState>();
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

fn target_path(settings: &Settings, root: &Path, game: &AppInfo, suffix: &str, ext: &str) -> PathBuf {
    let name = game::sanitize(&game.name);
    let dir = if settings.sort_by_game { root.join(&name) } else { root.to_path_buf() };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    let base = format!("{name} {stamp}{suffix}");
    let mut p = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{base} ({n}).{ext}"));
        n += 1;
    }
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
    let st = app.state::<AppState>();
    let s = st.settings.read().clone();
    if !s.replay_enabled {
        overlay::toast(app, Toast::simple("replay-off-hint"));
        return;
    }
    let g = current_game(app);
    let path = target_path(&s, &s.clips_dir, &g, "", "mp4");
    let res = ensure_dir(&path).and_then(|_| {
        st.library.expect(&path, Kind::Clip, &g.name);
        st.engine.save_replay(path.clone(), None, "GeniusClip".into())
    });
    if let Err(e) = res {
        fail(app, e);
    }
}

pub fn toggle_replay(app: &AppHandle) {
    let on = !app.state::<AppState>().settings.read().replay_enabled;
    set_replay(app, on);
    overlay::toast(app, Toast::simple(if on { "replay-on" } else { "replay-off" }));
}

pub fn set_replay(app: &AppHandle, on: bool) {
    let st = app.state::<AppState>();
    {
        let mut s = st.settings.write();
        s.replay_enabled = on;
        let _ = s.save(app);
    }
    if let Err(e) = st.engine.set_replay_enabled(on) {
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

/// Called every second: remembers the last game in focus.
pub fn track_foreground(app: &AppHandle) {
    if let Some(a) = game::foreground_app().filter(|a| !a.is_desktop) {
        *app.state::<AppState>().last_game.lock() = Some((a, Instant::now()));
    }
}
