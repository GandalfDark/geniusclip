//! "Report a problem": a zip on the Desktop with the logs, the settings and a
//! summary of the system, for attaching to a bug report. Reports end up in
//! public issues, so the user's profile path and name are taken out of every
//! file in it.

use crate::state::AppState;
use anyhow::Context;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use windows::core::{GUID, HSTRING};

/// Writes the report; returns its path.
pub fn create(app: &AppHandle) -> anyhow::Result<PathBuf> {
    use windows::Win32::UI::Shell::FOLDERID_Desktop;
    let desktop = known_folder(&FOLDERID_Desktop).context("Desktop folder")?;
    let now = chrono::Local::now();
    let stamp = now.format("%Y-%m-%d_%H-%M");
    let mut out = desktop.join(format!("GeniusClip-report-{stamp}.zip"));
    let mut n = 2;
    while out.exists() {
        out = desktop.join(format!("GeniusClip-report-{stamp} ({n}).zip"));
        n += 1;
    }

    let mut files = vec![("system.txt".to_string(), system_info(app))];
    match std::fs::read(crate::settings::path(app)) {
        Ok(b) => files.push(("settings.json".into(), String::from_utf8_lossy(&b).into_owned())),
        Err(e) => log::info!("report: settings.json: {e}"),
    }
    for p in log_files(app) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read(&p) {
            Ok(b) => files.push((name, String::from_utf8_lossy(&b).into_owned())),
            Err(e) => log::info!("report: {name}: {e}"),
        }
    }
    let privacy = Privacy::current();
    let files: Vec<(String, String)> = files.into_iter().map(|(name, text)| (name, privacy.clean(&text))).collect();

    // Written aside and moved into place: the Desktop never shows a
    // half-written zip.
    let part = out.with_extension("zip.part");
    let written = write_zip(&part, &files, &now).and_then(|_| Ok(std::fs::rename(&part, &out)?));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    log::info!("problem report saved ({} files)", files.len());
    Ok(out)
}

fn write_zip(path: &Path, files: &[(String, String)], now: &chrono::DateTime<chrono::Local>) -> anyhow::Result<()> {
    use chrono::{Datelike, Timelike};
    let time = zip::DateTime::from_date_and_time(now.year() as u16, now.month() as u8, now.day() as u8, now.hour() as u8, now.minute() as u8, now.second().min(58) as u8)
        .unwrap_or_default();
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).last_modified_time(time);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    for (name, text) in files {
        zip.start_file(name.as_str(), opts)?;
        zip.write_all(text.as_bytes())?;
    }
    zip.finish()?.sync_all()?;
    Ok(())
}

/// tauri-plugin-log's files (the current one and any rotated ones).
fn log_files(app: &AppHandle) -> Vec<PathBuf> {
    let Some(rd) = app.path().app_log_dir().ok().and_then(|d| std::fs::read_dir(d).ok()) else { return Vec::new() };
    let mut files: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("log"))).collect();
    files.sort();
    files
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        s.ok().map(PathBuf::from)
    }
}

// ---------------------------------------------------------------------------
// system.txt

fn system_info(app: &AppHandle) -> String {
    let st = app.state::<AppState>();
    let mut s = String::new();
    let _ = writeln!(s, "GeniusClip {}", app.package_info().version);
    let _ = writeln!(s, "FFmpeg {}", geniusclip_engine::ffmpeg_version());
    let _ = writeln!(s, "Created {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S %:z"));

    let _ = writeln!(s, "\n[System]");
    let _ = writeln!(s, "Windows: {}", windows_version());
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    let _ = writeln!(s, "CPU: {} ({threads} threads)", cpu_name().unwrap_or_else(|| "?".into()));
    let (total, avail) = memory_mb();
    let _ = writeln!(s, "RAM: {total} MB, {avail} MB available");
    let _ = writeln!(s, "Battery: {}", if crate::power::has_battery() { "yes" } else { "no" });

    let _ = writeln!(s, "\n[GPUs]");
    for g in gpus() {
        let _ = writeln!(s, "- {g}");
    }

    let _ = writeln!(s, "\n[Monitors]");
    match geniusclip_engine::list_monitors() {
        Ok(mons) => {
            for m in mons {
                let hz = refresh_hz(&m.id).map_or("?".into(), |f| f.to_string());
                let _ = writeln!(
                    s,
                    "- {} ({}): {}x{} @ {hz} Hz, HDR {}{}, at {},{}, on {}",
                    m.name,
                    m.id,
                    m.width,
                    m.height,
                    if m.hdr { "on" } else { "off" },
                    if m.primary { ", primary" } else { "" },
                    m.x,
                    m.y,
                    m.adapter
                );
            }
        }
        Err(e) => {
            let _ = writeln!(s, "error: {e:#}");
        }
    }

    for (title, capture) in [("Audio outputs", false), ("Audio inputs", true)] {
        let _ = writeln!(s, "\n[{title}]");
        match geniusclip_engine::list_audio_devices(capture) {
            Ok(devs) => {
                for d in devs {
                    let _ = writeln!(s, "- {}{}{}", d.name, if d.is_default { " (default)" } else { "" }, if d.bluetooth { " (Bluetooth)" } else { "" });
                }
            }
            Err(e) => {
                let _ = writeln!(s, "error: {e:#}");
            }
        }
    }

    let settings = st.settings.read().clone();
    let _ = writeln!(s, "\n[Clips drive]");
    match crate::disk::space(&settings) {
        Ok(d) => {
            let _ = writeln!(s, "{}: {} MB free of {} MB (low below {} MB)", d.drive, d.free_mb, d.total_mb, d.low_mb);
        }
        Err(e) => {
            let _ = writeln!(s, "error: {e:#}");
        }
    }

    let hotkey_errors = st.hotkey_errors.lock().clone();
    if !hotkey_errors.is_empty() {
        let _ = writeln!(s, "\n[Hotkeys not registered]\n{}", hotkey_errors.join(", "));
    }

    // Encoder, resolution, dropped frames, last error.
    let _ = writeln!(s, "\n[Capture status]");
    let status = st.engine.status();
    let _ = writeln!(s, "{}", serde_json::to_string_pretty(&status).unwrap_or_else(|e| e.to_string()));
    s
}

fn memory_mb() -> (u64, u64) {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    match unsafe { GlobalMemoryStatusEx(&mut m) } {
        Ok(()) => (m.ullTotalPhys >> 20, m.ullAvailPhys >> 20),
        Err(_) => (0, 0),
    }
}

/// Registry values under HKEY_LOCAL_MACHINE.
fn reg_string(key: &str, value: &str) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let mut buf = [0u16; 512];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    let res = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut bytes))
    };
    if res != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]).trim().to_string()).filter(|s| !s.is_empty())
}

fn reg_dword(key: &str, value: &str) -> Option<u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let mut v = 0u32;
    let mut bytes = 4u32;
    let res = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_DWORD, None, Some((&mut v as *mut u32).cast()), Some(&mut bytes))
    };
    (res == ERROR_SUCCESS).then_some(v)
}

/// E.g. "Windows 11 Pro 23H2 (build 22631.4317), x86_64".
fn windows_version() -> String {
    const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build: u32 = reg_string(NT, "CurrentBuildNumber").and_then(|b| b.parse().ok()).unwrap_or(0);
    let ubr = reg_dword(NT, "UBR").unwrap_or(0);
    let product = reg_string(NT, "ProductName").unwrap_or_else(|| "Windows".into());
    // ProductName still says "Windows 10" on Windows 11.
    let product = if build >= 22000 { product.replacen("Windows 10", "Windows 11", 1) } else { product };
    let release = reg_string(NT, "DisplayVersion").or_else(|| reg_string(NT, "ReleaseId")).unwrap_or_default();
    format!("{product} {release} (build {build}.{ubr}), {}", std::env::consts::ARCH)
}

fn cpu_name() -> Option<String> {
    reg_string(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString")
}

/// Hardware adapters with their driver version and memory.
fn gpus() -> Vec<String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return vec!["error: DXGI unavailable".into()] };
    let mut out = Vec::new();
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
        // Microsoft Basic Render Driver.
        if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let name = String::from_utf16_lossy(&d.Description[..d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len())]);
        // The user-mode driver version, as Device Manager shows it.
        let driver = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .map(|v| {
                let v = v as u64;
                format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
            })
            .unwrap_or_else(|_| "?".into());
        out.push(format!("{name}: driver {driver}, {} MB VRAM, PCI {:04x}:{:04x}", d.DedicatedVideoMemory >> 20, d.VendorId, d.DeviceId));
    }
    out
}

/// The current refresh rate of a display (GDI device name, `\\.\DISPLAY1`).
fn refresh_hz(device: &str) -> Option<u32> {
    use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    let ok = unsafe { EnumDisplaySettingsW(&HSTRING::from(device), ENUM_CURRENT_SETTINGS, &mut dm) }.as_bool();
    // 0 and 1 mean "the hardware's default".
    (ok && dm.dmDisplayFrequency > 1).then_some(dm.dmDisplayFrequency)
}

// ---------------------------------------------------------------------------
// Privacy

/// What identifies the user in logs and settings: the profile folder (in the
/// spellings paths get in text) and the account name.
struct Privacy {
    profiles: Vec<String>,
    names: Vec<String>,
}

impl Privacy {
    fn current() -> Privacy {
        use windows::Win32::UI::Shell::FOLDERID_Profile;
        let profile = std::env::var("USERPROFILE")
            .ok()
            .filter(|p| !p.is_empty())
            .or_else(|| known_folder(&FOLDERID_Profile).map(|p| p.to_string_lossy().into_owned()));
        let mut names: Vec<String> = std::env::var("USERNAME").ok().into_iter().collect();
        // The profile folder can be named differently (a renamed or Microsoft account).
        if let Some(n) = profile.as_deref().and_then(|p| Path::new(p).file_name()) {
            names.push(n.to_string_lossy().into_owned());
        }
        Privacy::new(profile.as_deref(), names)
    }

    fn new(profile: Option<&str>, names: Vec<String>) -> Privacy {
        let mut profiles = Vec::new();
        if let Some(p) = profile.map(|p| p.trim_end_matches(['\\', '/'])).filter(|p| p.len() > 3) {
            // As written in JSON and Rust's debug output, as is, and with slashes.
            profiles.push(p.replace('\\', r"\\"));
            profiles.push(p.to_string());
            profiles.push(p.replace('\\', "/"));
        }
        let mut names: Vec<String> = names.into_iter().map(|n| n.trim().to_string()).filter(|n| n.chars().count() >= 2).collect();
        // Longer first: "ivan.petrov" before "ivan".
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
        Privacy { profiles, names }
    }

    fn clean(&self, text: &str) -> String {
        let mut s = text.to_string();
        for p in &self.profiles {
            s = replace_word(&s, p, "%USERPROFILE%");
        }
        for n in &self.names {
            s = replace_word(&s, n, "<user>");
        }
        s
    }
}

/// `text` with every whole-word, case-insensitive occurrence of `find`
/// replaced: a user named "max" must not turn "maximum" into "<user>imum".
fn replace_word(text: &str, find: &str, with: &str) -> String {
    let find: Vec<char> = find.chars().collect();
    let (Some(&first), Some(&last)) = (find.first(), find.last()) else { return text.to_string() };
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut prev = None;
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let rest = &text[i..];
        if same(c, first) && !(word(Some(first)) && word(prev)) {
            let mut chars = rest.char_indices();
            if find.iter().all(|&f| chars.next().is_some_and(|(_, c)| same(c, f))) {
                let len = chars.next().map_or(rest.len(), |(j, _)| j);
                if !(word(Some(last)) && word(rest[len..].chars().next())) {
                    out.push_str(with);
                    prev = rest[..len].chars().last();
                    i += len;
                    continue;
                }
            }
        }
        out.push(c);
        prev = Some(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_words_only_any_case() {
        assert_eq!(replace_word("max maximum Max_1 MAX.", "max", "<user>"), "<user> maximum Max_1 <user>.");
        assert_eq!(replace_word("Иван, иван!", "Иван", "<user>"), "<user>, <user>!");
        assert_eq!(replace_word("", "x", "y"), "");
        assert_eq!(replace_word("abc", "", "y"), "abc");
    }

    #[test]
    fn profile_and_name_are_removed_everywhere() {
        let p = Privacy::new(Some(r"C:\Users\Gandalf"), vec!["Gandalf".into(), "gandalf".into(), "G".into()]);
        assert_eq!(p.names, vec!["Gandalf".to_string()]);
        let log = r#"clips at c:\users\GANDALF\Videos\GeniusClip; "clipsDir": "C:\\Users\\Gandalf\\Videos", url file:///C:/Users/Gandalf/x.mp4, D:\Clips\Gandalf, Gandalfian"#;
        assert_eq!(
            p.clean(log),
            r#"clips at %USERPROFILE%\Videos\GeniusClip; "clipsDir": "%USERPROFILE%\\Videos", url file:///%USERPROFILE%/x.mp4, D:\Clips\<user>, Gandalfian"#
        );
        // Settings stay valid JSON.
        let json = r#"{"clipsDir":"C:\\Users\\Gandalf\\Videos\\GeniusClip"}"#;
        let v: serde_json::Value = serde_json::from_str(&p.clean(json)).unwrap();
        assert_eq!(v["clipsDir"], r"%USERPROFILE%\Videos\GeniusClip");
    }

    #[test]
    fn system_details_are_readable() {
        let v = windows_version();
        assert!(v.starts_with("Windows") && !v.contains("build 0."), "{v}");
        assert!(cpu_name().is_some());
        // No assertion on hardware: CI machines may have no GPU or display.
        eprintln!("{v} | {:?} | {:?} | {:?}", cpu_name(), gpus(), refresh_hz(r"\\.\DISPLAY1"));
    }

    #[test]
    fn zip_holds_every_file() {
        let dir = std::env::temp_dir().join(format!("geniusclip-test-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.zip");
        let files = vec![("system.txt".to_string(), "GeniusClip 1.0\n".to_string()), ("GeniusClip.log".to_string(), "x".repeat(10_000))];
        write_zip(&path, &files, &chrono::Local::now()).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("system.txt").unwrap(), &mut text).unwrap();
        assert_eq!(text, "GeniusClip 1.0\n");
        assert!(std::fs::metadata(&path).unwrap().len() < 5_000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
