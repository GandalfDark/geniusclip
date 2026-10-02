//! Detects which application (game) is in the foreground, for naming and
//! sorting clips.

use serde::Serialize;
use std::path::Path;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// Display name used for folders and file names ("Counter-Strike 2").
    pub name: String,
    pub exe: String,
    /// True when the foreground is the Windows shell / desktop.
    pub is_desktop: bool,
}

const SHELL_EXES: &[&str] = &[
    "explorer.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchapp.exe",
    "lockapp.exe",
    "textinputhost.exe",
    "applicationframehost.exe",
    "geniusclip.exe",
];

/// Overlays that cover the whole screen on top of games (never the game itself).
const OVERLAY_EXES: &[&str] = &[
    "nvidia overlay.exe",
    "nvidia share.exe",
    "nvidia app.exe",
    "radeonsoftware.exe",
    "amdrsserv.exe",
    "gameoverlayui.exe",
    "discord.exe",
    "rtss.exe",
    "gamebar.exe",
    "gamebarftserver.exe",
    "overwolf.exe",
    "medal.exe",
];

/// Game launchers and stores, with the name clips get while one is in front
/// and no game was lately (Steam's windows belong to "Steam Client
/// WebHelper"). They are in front between games, never the game itself.
const LAUNCHER_EXES: &[(&str, &str)] = &[
    ("steamwebhelper.exe", "Steam"),
    ("steam.exe", "Steam"),
    ("epicgameslauncher.exe", "Epic Games"),
    ("battle.net.exe", "Battle.net"),
    ("eadesktop.exe", "EA app"),
    ("origin.exe", "Origin"),
    ("galaxyclient.exe", "GOG Galaxy"),
    ("riotclientux.exe", "Riot Client"),
    ("riotclientservices.exe", "Riot Client"),
    ("upc.exe", "Ubisoft Connect"),
    ("ubisoftconnect.exe", "Ubisoft Connect"),
];

fn launcher_name(exe: &str) -> Option<&'static str> {
    let exe = exe.to_lowercase();
    LAUNCHER_EXES.iter().find(|(e, _)| *e == exe).map(|(_, name)| *name)
}

/// A game launcher or store (Steam, Epic Games, Battle.net...).
pub fn is_launcher(app: &AppInfo) -> bool {
    launcher_name(&app.exe).is_some()
}

/// Fullscreen apps that aren't games: browsers, video players, slideshows,
/// calls. "Only in games" doesn't wake capture for them.
const NOT_GAMES: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "browser.exe",
    "opera.exe",
    "brave.exe",
    "vivaldi.exe",
    "arc.exe",
    "vlc.exe",
    "mpc-hc.exe",
    "mpc-hc64.exe",
    "mpc-be.exe",
    "mpc-be64.exe",
    "potplayer.exe",
    "potplayermini.exe",
    "potplayermini64.exe",
    "wmplayer.exe",
    "microsoft.media.player.exe",
    "video.ui.exe",
    "mpv.exe",
    "telegram.exe",
    "powerpnt.exe",
    "zoom.exe",
    "ms-teams.exe",
    "teams.exe",
    "obs64.exe",
];

/// Whether a fullscreen app counts as a game (not a browser or a player).
pub fn is_game(app: &AppInfo) -> bool {
    let exe = app.exe.to_lowercase();
    !app.is_desktop && !NOT_GAMES.contains(&exe.as_str()) && !OVERLAY_EXES.contains(&exe.as_str()) && !is_launcher(app)
}

/// Hosts whose file description is meaningless; the window title names the app.
const HOST_EXES: &[&str] = &["javaw.exe", "java.exe", "python.exe", "pythonw.exe", "electron.exe"];

fn file_description(path: &str) -> Option<String> {
    unsafe {
        let wpath = HSTRING::from(path);
        let size = GetFileVersionInfoSizeW(&wpath, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(&wpath, None, size, buf.as_mut_ptr() as *mut _).ok()?;

        // Use the first language/codepage from the translation table.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let (lang, cp) = if VerQueryValueW(buf.as_ptr() as *const _, &HSTRING::from("\\VarFileInfo\\Translation"), &mut ptr, &mut len)
            .as_bool()
            && len >= 4
        {
            let t = std::slice::from_raw_parts(ptr as *const u16, 2);
            (t[0], t[1])
        } else {
            (0x0409, 0x04b0)
        };
        for key in ["FileDescription", "ProductName"] {
            let q = HSTRING::from(format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\{key}"));
            if VerQueryValueW(buf.as_ptr() as *const _, &q, &mut ptr, &mut len).as_bool() && len > 1 {
                // `len` is not reliable across files: stop at the first NUL.
                let raw = std::slice::from_raw_parts(ptr as *const u16, len as usize);
                let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                let s = String::from_utf16_lossy(&raw[..end]);
                let s: String = s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize]).trim().to_string()
}

pub fn foreground_app() -> Option<AppInfo> {
    app_for_window(unsafe { GetForegroundWindow() })
}

/// A game in front that fills its monitor (fullscreen or borderless).
pub struct FullscreenGame {
    pub app: AppInfo,
    pub pid: u32,
    /// The monitor it fills, in desktop coordinates.
    pub monitor: RECT,
}

/// The focused window, when it is a game filling its monitor.
pub fn foreground_fullscreen_game() -> Option<FullscreenGame> {
    fullscreen_game(unsafe { GetForegroundWindow() })
}

/// The window, when it is a game filling its monitor.
pub fn fullscreen_game(hwnd: HWND) -> Option<FullscreenGame> {
    unsafe {
        if hwnd.0.is_null() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if hmon.is_invalid() || !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).ok()?;
        let m = mi.rcMonitor;
        let near = |a: i32, b: i32| (a - b).abs() <= 1;
        if !(near(r.left, m.left) && near(r.top, m.top) && near(r.right, m.right) && near(r.bottom, m.bottom)) {
            return None;
        }
        let app = app_for_window(hwnd).filter(is_game)?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        Some(FullscreenGame { app, pid, monitor: m })
    }
}

/// The top-most visible window that exactly fills the given screen rect
/// (a fullscreen or borderless-fullscreen game), if any.
pub fn fullscreen_app(x: i32, y: i32, w: u32, h: u32) -> Option<AppInfo> {
    struct Search {
        want: RECT,
        found: Option<AppInfo>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
        let s = &mut *(lp.0 as *mut Search);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return true.into();
        }
        // Click-through / non-activating / tool windows are overlays (vendor
        // overlays, notifications, our own toast), not games.
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let overlay_bits = WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0;
        if ex & overlay_bits != 0 || (ex & WS_EX_LAYERED.0 != 0 && ex & WS_EX_TRANSPARENT.0 != 0) {
            return true.into();
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return true.into();
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return true.into();
        }
        // Fullscreen/borderless games match the monitor exactly; maximized
        // regular windows overhang it by their frame, so they don't count.
        let near = |a: i32, b: i32| (a - b).abs() <= 1;
        if near(r.left, s.want.left) && near(r.top, s.want.top) && near(r.right, s.want.right) && near(r.bottom, s.want.bottom) {
            // A launcher filling the screen (Steam's Big Picture) isn't a game either.
            if let Some(a) = app_for_window(hwnd).filter(|a| !a.is_desktop && !is_launcher(a) && !OVERLAY_EXES.contains(&a.exe.to_lowercase().as_str())) {
                s.found = Some(a);
                return false.into(); // stop: windows are enumerated top-most first
            }
        }
        true.into()
    }
    let mut s = Search { want: RECT { left: x, top: y, right: x + w as i32, bottom: y + h as i32 }, found: None };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut s as *mut Search as isize));
    }
    s.found
}

fn app_for_window(hwnd: HWND) -> Option<AppInfo> {
    unsafe {
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(proc);
        if !ok {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        let exe = Path::new(&full).file_name()?.to_string_lossy().to_string();
        let lower = exe.to_lowercase();
        if SHELL_EXES.contains(&lower.as_str()) {
            return Some(AppInfo { name: "Desktop".into(), exe, is_desktop: true });
        }
        let stem = Path::new(&exe).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let name = if let Some(n) = launcher_name(&lower) {
            Some(n.to_string())
        } else if HOST_EXES.contains(&lower.as_str()) {
            Some(window_title(hwnd)).filter(|t| !t.is_empty())
        } else {
            // Without a description (Valve's cs2.exe, dota2.exe) the window
            // title names the game, unless it's long (a document's name).
            file_description(&full).or_else(|| Some(window_title(hwnd)).filter(|t| !t.is_empty() && t.chars().count() <= 40))
        }
        .unwrap_or(stem);
        Some(AppInfo { name, exe, is_desktop: false })
    }
}

/// Makes a string safe for use as a Windows file/folder name.
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if s.chars().count() > 60 {
        s = s.chars().take(60).collect();
    }
    s = s.trim_end_matches(['.', ' ']).to_string();
    // Device names are reserved with any extension too ("NUL.txt").
    let stem = s.split('.').next().unwrap_or("").trim_end().to_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem[3..].chars().all(|c| c.is_ascii_digit() || "¹²³".contains(c)));
    if s.is_empty() || device {
        s = format!("_{s}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(exe: &str) -> AppInfo {
        AppInfo { name: exe.into(), exe: exe.into(), is_desktop: false }
    }

    #[test]
    fn launchers_are_not_games() {
        for exe in ["steamwebhelper.exe", "Steam.exe", "EpicGamesLauncher.exe", "Battle.net.exe"] {
            assert!(is_launcher(&app(exe)) && !is_game(&app(exe)), "{exe}");
        }
        assert_eq!(launcher_name("SteamWebHelper.exe"), Some("Steam"));
        assert!(is_game(&app("cs2.exe")) && !is_launcher(&app("cs2.exe")));
        assert!(!is_game(&app("chrome.exe")));
    }
}
