//! What the installer actually does: runs the embedded NSIS setup silently,
//! then applies the choices made in the window.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{w, PCWSTR};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

pub const VERSION: &str = env!("GC_VERSION");
/// The NSIS setup from `tauri build`; empty in demo builds.
static PAYLOAD: &[u8] = include_bytes!(env!("GC_SETUP_PAYLOAD_PATH"));

const PRODUCT: &str = "GeniusClip";
const EXE: &str = "GeniusClip.exe";
const UNINSTALL_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\GeniusClip");

pub fn demo() -> bool {
    PAYLOAD.is_empty()
}

pub struct Installed {
    pub version: String,
    pub dir: PathBuf,
}

fn reg_string(key: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key, value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) };
    if r.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// The current per-user installation, as recorded by the NSIS setup.
pub fn installed() -> Option<Installed> {
    let version = reg_string(UNINSTALL_KEY, w!("DisplayVersion"))?;
    let dir = reg_string(UNINSTALL_KEY, w!("InstallLocation"))?;
    Some(Installed { version, dir: PathBuf::from(dir.trim_matches('"')) })
}

pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
    base.join(PRODUCT)
}

/// A folder picked by the user: install into a GeniusClip subfolder of it,
/// unless it already is one.
pub fn normalize_dir(picked: PathBuf) -> PathBuf {
    if picked.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(PRODUCT)) {
        picked
    } else {
        picked.join(PRODUCT)
    }
}

fn parse_version(v: &str) -> Vec<u32> {
    v.split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect()
}

/// -1: `installed` is older than this installer, 0: same, 1: newer.
pub fn compare_installed(installed: &str) -> i32 {
    match parse_version(installed).cmp(&parse_version(VERSION)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn settings_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("com.geniusclip.app").join("settings.json"))
}

fn read_settings() -> serde_json::Map<String, serde_json::Value> {
    settings_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// The "save clip" hotkey to show on the last screen ("Alt+F8").
pub fn save_hotkey() -> String {
    read_settings()
        .get("hotkeys")
        .and_then(|h| h.get("saveClip"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Alt+F8")
        .to_string()
}

pub struct Options {
    pub dir: PathBuf,
    pub autostart: bool,
    pub shortcut: bool,
    /// Set when the language was picked in the installer (not auto-detected).
    pub language: Option<&'static str>,
}

pub fn run(opts: &Options) -> Result<(), String> {
    if demo() {
        std::thread::sleep(std::time::Duration::from_millis(3500));
        return Ok(());
    }
    let setup = std::env::temp_dir().join(format!("GeniusClip-{VERSION}-{}-setup.exe", std::process::id()));
    std::fs::write(&setup, PAYLOAD).map_err(|e| format!("{}: {e}", setup.display()))?;
    // NSIS: /S = silent (closes a running GeniusClip itself); /D= must be
    // last and unquoted, even with spaces.
    let status = Command::new(&setup).arg("/S").raw_arg(format!("/D={}", opts.dir.display())).status();
    let _ = std::fs::remove_file(&setup);
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => return Err(format!("setup exit code {}", s.code().unwrap_or(-1))),
        Err(e) => return Err(e.to_string()),
    }
    if !opts.dir.join(EXE).is_file() {
        return Err(format!("{EXE} not found in {}", opts.dir.display()));
    }
    if !opts.shortcut {
        if let Some(desktop) = known_folder_desktop() {
            let _ = std::fs::remove_file(desktop.join(format!("{PRODUCT}.lnk")));
        }
    }
    write_settings(opts);
    Ok(())
}

fn known_folder_desktop() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// Records the installer choices in the app's settings (other settings of an
/// existing installation are kept).
fn write_settings(opts: &Options) {
    let Some(path) = settings_path() else { return };
    let mut s = read_settings();
    s.insert("autostart".into(), opts.autostart.into());
    if let Some(lang) = opts.language {
        s.insert("language".into(), lang.into());
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&serde_json::Value::Object(s)) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn launch(dir: &Path) {
    if !demo() {
        let _ = Command::new(dir.join(EXE)).current_dir(dir).spawn();
    }
}
