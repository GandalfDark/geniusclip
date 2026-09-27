//! Persistent user settings (JSON in the app config dir).

use geniusclip_engine::EngineConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    pub save_clip: String,
    pub toggle_replay: String,
    pub screenshot: String,
    pub toggle_recording: String,
    /// Saves only the last `short_seconds`; unset by default.
    pub save_short: String,
    /// Opens the in-game menu.
    pub toggle_menu: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        // Chosen to avoid the defaults of NVIDIA App (Alt+F1/F9/F10),
        // AMD Adrenalin (Ctrl+Shift+…), Xbox Game Bar (Win+Alt+…) and Steam (F12).
        Hotkeys {
            save_clip: "Alt+F8".into(),
            toggle_replay: "Alt+Shift+F8".into(),
            screenshot: "Alt+F6".into(),
            toggle_recording: "Alt+F7".into(),
            save_short: String::new(),
            toggle_menu: "Alt+KeyX".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlaySettings {
    pub enabled: bool,
    /// "top-right" | "top-left" | "bottom-right" | "bottom-left"
    pub corner: String,
    pub sound: bool,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        OverlaySettings { enabled: true, corner: "top-right".into(), sound: false }
    }
}

/// "First steps" checklist on the home page.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Onboarding {
    /// A clip was saved (set by the backend).
    pub clip_saved: bool,
    /// The in-game menu was opened (set by the backend).
    pub menu_opened: bool,
    /// Hidden by the user (set by the UI).
    pub dismissed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub engine: EngineConfig,
    pub replay_enabled: bool,
    pub replay_seconds: u32,
    /// Start a new clip where the previous one ended (no repeated footage).
    pub skip_saved: bool,
    /// Length of clips saved with the short-clip hotkey.
    pub short_seconds: u32,
    pub clips_dir: PathBuf,
    pub screenshots_dir: PathBuf,
    pub sort_by_game: bool,
    pub hotkeys: Hotkeys,
    pub autostart: bool,
    /// "auto" | "ru" | "en"
    pub language: String,
    /// Accent preset id used by the UI.
    pub accent: String,
    pub overlay: OverlaySettings,
    pub auto_update: bool,
    /// Laptops: no replay while running on battery.
    pub pause_on_battery: bool,
    pub onboarding: Onboarding,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: EngineConfig::default(),
            replay_enabled: true,
            replay_seconds: 300,
            skip_saved: true,
            short_seconds: 30,
            clips_dir: PathBuf::new(),
            screenshots_dir: PathBuf::new(),
            sort_by_game: true,
            hotkeys: Hotkeys::default(),
            autostart: true,
            language: "auto".into(),
            accent: "violet".into(),
            overlay: OverlaySettings::default(),
            auto_update: true,
            pause_on_battery: false,
            onboarding: Onboarding::default(),
        }
    }
}

pub fn path(app: &AppHandle) -> PathBuf {
    app.path().app_config_dir().unwrap_or_else(|_| PathBuf::from(".")).join("settings.json")
}

impl Settings {
    pub fn load(app: &AppHandle) -> Settings {
        let p = path(app);
        let file = std::fs::read(&p);
        // Only a file that isn't there: one that can't be read right now
        // (locked by a scanner) must not be overwritten below.
        let new_install = match &file {
            Ok(b) => is_new_install(b),
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        let mut s = match &file {
            Ok(b) => Settings::parse(b).unwrap_or_else(|e| {
                // One bad value (e.g. an enum variant from a newer version)
                // must not reset everything, clip folders included: keep a
                // copy of the file and every value that still fits.
                log::warn!("settings.json invalid: {e}; keeping the values that still parse");
                let _ = std::fs::copy(&p, p.with_file_name("settings.bak.json"));
                Settings::salvage(b)
            }),
            Err(_) => Settings::default(),
        };
        s.fill_defaults(app);
        if new_install && !ran_before(app, &s) {
            s.fit_hardware();
            // Saved right away: from now on these are the user's settings
            // (and the next start is no longer a first one).
            if let Err(e) = s.save(app) {
                log::warn!("settings.json: {e:#}");
            }
        }
        s
    }

    /// First start only: capture defaults this PC can keep up with. The
    /// app's defaults (native resolution, high quality, 5 minutes) suit a
    /// gaming PC; on a weak GPU they cost the game frames, and a long buffer
    /// in RAM crowds out a game on 8 GB.
    fn fit_hardware(&mut self) {
        let mons = crate::monitors::list();
        let Some(m) = crate::monitors::pick(&mons, self.engine.monitor.as_deref()) else {
            log::info!("first start: no monitor found; default capture settings");
            return;
        };
        let hw = Hardware {
            vendor_id: m.vendor_id,
            vram_mb: dedicated_vram_mb(&m.adapter, m.vendor_id),
            hw_encoder: has_hw_encoder(m.vendor_id),
            width: m.width,
            height: m.height,
            ram_mb: crate::commands::ram_total_mb(),
        };
        let weak = hw.apply(self);
        log::info!(
            "first start: {} ({:04x}, {} MB VRAM, {} encoder), {}x{}, {} MB RAM -> {:?}, {:?} quality, {} s replay{}",
            m.adapter,
            hw.vendor_id,
            hw.vram_mb.map_or("?".into(), |v| v.to_string()),
            if hw.hw_encoder { "hardware" } else { "Media Foundation" },
            hw.width,
            hw.height,
            hw.ram_mb,
            self.engine.resolution,
            self.engine.quality,
            self.replay_seconds,
            if weak { " (weak GPU)" } else { "" },
        );
    }

    /// An existing settings file. Users updating from a version without the
    /// "first steps" checklist already know the app: theirs starts hidden.
    fn parse(bytes: &[u8]) -> serde_json::Result<Settings> {
        let mut s: Settings = serde_json::from_slice(bytes)?;
        s.onboarding.dismissed |= from_older_version(bytes);
        Ok(s)
    }

    /// Defaults overlaid with each top-level value (and each engine, hotkey
    /// and overlay field) from `bytes` that still deserializes.
    fn salvage(bytes: &[u8]) -> Settings {
        use serde_json::Value;
        fn slot<'a>(m: &'a mut Value, path: &[&str]) -> &'a mut Value {
            path.iter().fold(m, |m, k| &mut m[*k])
        }
        /// Sets one value; puts the old one back if the result doesn't parse.
        fn try_set(merged: &mut Value, path: &[&str], v: Value) {
            let old = std::mem::replace(slot(merged, path), v);
            if serde_json::from_value::<Settings>(merged.clone()).is_err() {
                *slot(merged, path) = old;
            }
        }
        // See `parse`.
        let existing = Settings { onboarding: Onboarding { dismissed: from_older_version(bytes), ..Default::default() }, ..Default::default() };
        let Ok(Value::Object(file)) = serde_json::from_slice::<Value>(bytes) else { return existing };
        let Ok(mut merged) = serde_json::to_value(&existing) else { return existing };
        for (key, value) in file {
            match value {
                Value::Object(fields) if merged.get(&key).is_some_and(Value::is_object) => {
                    for (k, v) in fields {
                        try_set(&mut merged, &[key.as_str(), k.as_str()], v);
                    }
                }
                v => try_set(&mut merged, &[key.as_str()], v),
            }
        }
        serde_json::from_value(merged).unwrap_or(existing)
    }

    /// Settings coming from the UI: empty folders fall back to the defaults,
    /// lengths stay within what the UI offers.
    pub fn validate(&mut self, app: &AppHandle) {
        self.fill_defaults(app);
        let max = if self.engine.disk_buffer { 3600 } else { 1200 };
        self.replay_seconds = self.replay_seconds.clamp(60, max);
        self.short_seconds = self.short_seconds.clamp(10, 60);
    }

    fn fill_defaults(&mut self, app: &AppHandle) {
        let p = app.path();
        if self.clips_dir.as_os_str().is_empty() {
            self.clips_dir = p.video_dir().unwrap_or_else(|_| PathBuf::from(".")).join("GeniusClip");
        }
        if self.screenshots_dir.as_os_str().is_empty() {
            self.screenshots_dir = p.picture_dir().unwrap_or_else(|_| PathBuf::from(".")).join("GeniusClip");
        }
        self.replay_seconds = self.replay_seconds.clamp(10, 3600);
        self.short_seconds = self.short_seconds.clamp(5, 600);
        // Presets from before the "Studio" redesign map to the default.
        if !["violet", "red", "lime", "cyan", "amber", "mono"].contains(&self.accent.as_str()) {
            self.accent = "violet".into();
        }
    }

    pub fn save(&self, app: &AppHandle) -> anyhow::Result<()> {
        let p = path(app);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, p)?;
        Ok(())
    }

    /// Records a "first steps" milestone reached in the app; saves and tells
    /// the windows if it is new. Monotonic: `update_settings` never clears it.
    pub fn mark_onboarding(app: &AppHandle, step: fn(&mut Onboarding) -> &mut bool) {
        let st = app.state::<crate::state::AppState>();
        {
            let mut s = st.settings.write();
            let flag = step(&mut s.onboarding);
            if *flag {
                return;
            }
            *flag = true;
            if let Err(e) = s.save(app) {
                log::warn!("settings.json: {e:#}");
            }
        }
        crate::emit_settings(app);
    }

    /// Effective UI language ("ru" or "en").
    pub fn lang(&self) -> &'static str {
        crate::i18n::LANGS.iter().find(|l| **l == self.language).copied().unwrap_or_else(crate::i18n::system_lang)
    }
}

/// The file was written by a version of the app without `onboarding`. The
/// app always writes every setting (`engine` included), while the installer
/// of a new install writes only its own few (autostart, language): such a
/// file is a new user's. Anything unreadable counts as an older file.
fn from_older_version(bytes: &[u8]) -> bool {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Object(m)) => m.contains_key("engine") && !m.get("onboarding").is_some_and(|o| o.is_object()),
        _ => true,
    }
}

/// A settings file the app hasn't written yet: only the few values the
/// installer writes (the app always writes `engine`, see `from_older_version`).
/// An unreadable file counts as an existing user's.
fn is_new_install(file: &[u8]) -> bool {
    match serde_json::from_slice::<serde_json::Value>(file) {
        Ok(serde_json::Value::Object(m)) => !m.contains_key("engine"),
        _ => false,
    }
}

/// Signs of an earlier start: older versions saved settings only once one
/// was changed, so a user who never did has no `engine` either, and must
/// keep the defaults they have been recording with. Every start creates the
/// clips folder, and the first window the WebView2 profile.
fn ran_before(app: &AppHandle, s: &Settings) -> bool {
    s.clips_dir.is_dir() || app.path().app_local_data_dir().is_ok_and(|d| d.join("EBWebView").is_dir())
}

// PCI vendor ids (as in `MonitorInfo::vendor_id`).
const VENDOR_NVIDIA: u32 = 0x10DE;
const VENDOR_AMD: u32 = 0x1002;
const VENDOR_INTEL: u32 = 0x8086;

/// What the first-start defaults depend on (the capture monitor's GPU).
struct Hardware {
    vendor_id: u32,
    /// Dedicated video memory; None when it couldn't be read.
    vram_mb: Option<u64>,
    /// NVENC or AMF is installed for this GPU; else FFmpeg falls back to
    /// Media Foundation, which encodes slowly or on the CPU.
    hw_encoder: bool,
    width: u32,
    height: u32,
    ram_mb: u64,
}

impl Hardware {
    /// A GPU that struggles to encode full-size video next to a game:
    /// integrated Intel graphics (Media Foundation only), an AMD APU or
    /// entry card with under 2 GB of its own memory, or no NVENC/AMF.
    fn weak_gpu(&self) -> bool {
        self.vendor_id == VENDOR_INTEL || (self.vendor_id == VENDOR_AMD && self.vram_mb.is_some_and(|v| v < 2048)) || !self.hw_encoder
    }

    /// Sets the first-start defaults; returns whether the GPU counts as weak.
    fn apply(&self, s: &mut Settings) -> bool {
        use geniusclip_engine::{Quality, Resolution};
        // By the short side, so a rotated screen counts like a landscape one.
        let short = self.width.min(self.height);
        let weak = self.weak_gpu();
        if weak {
            if short > 1080 {
                s.engine.resolution = Resolution::P1080;
            }
            s.engine.quality = Quality::Medium;
        } else if short >= 2160 {
            s.engine.resolution = Resolution::P1440;
        }
        // 8 GB modules show up as a little less (memory reserved by the
        // firmware or an integrated GPU).
        if self.ram_mb > 0 && self.ram_mb <= 8 * 1024 {
            s.replay_seconds = 120;
        }
        weak
    }
}

/// Dedicated memory of the adapter driving the capture monitor.
fn dedicated_vram_mb(adapter: &str, vendor_id: u32) -> Option<u64> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.ok()?;
    (0..)
        .map_while(|i| unsafe { factory.EnumAdapters1(i) }.ok())
        .filter_map(|a| unsafe { a.GetDesc1() }.ok())
        .find(|d| {
            let len = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
            d.VendorId == vendor_id && String::from_utf16_lossy(&d.Description[..len]) == adapter
        })
        .map(|d| d.DedicatedVideoMemory as u64 >> 20)
}

/// The vendor's encoder runtime that FFmpeg loads (the engine tries NVENC
/// on NVIDIA, AMF on AMD, else Media Foundation), installed with the driver.
fn has_hw_encoder(vendor_id: u32) -> bool {
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
    let dll = match vendor_id {
        VENDOR_NVIDIA => "nvEncodeAPI64.dll",
        VENDOR_AMD => "amfrt64.dll",
        _ => return false,
    };
    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    len > 0 && len < buf.len() && std::path::Path::new(&String::from_utf16_lossy(&buf[..len])).join(dll).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_start_defaults_follow_the_hardware() {
        use geniusclip_engine::{Quality, Resolution};
        let pick = |vendor_id, vram_mb, hw_encoder, (width, height), ram_mb| {
            let mut s = Settings::default();
            let weak = Hardware { vendor_id, vram_mb, hw_encoder, width, height, ram_mb }.apply(&mut s);
            (weak, s.engine.resolution, s.engine.quality, s.replay_seconds)
        };
        let def = Settings::default();
        let (res, q, secs) = (def.engine.resolution, def.engine.quality, def.replay_seconds);
        // A gaming PC keeps the defaults.
        assert_eq!(pick(VENDOR_NVIDIA, Some(8192), true, (2560, 1440), 32768), (false, res, q, secs));
        // 4K with any strong GPU records 1440p.
        assert_eq!(pick(VENDOR_NVIDIA, Some(8192), true, (3840, 2160), 32768), (false, Resolution::P1440, q, secs));
        // Integrated Intel: 1080p (when larger) at medium; 8 GB: two minutes.
        assert_eq!(pick(VENDOR_INTEL, Some(128), false, (3840, 2160), 8000), (true, Resolution::P1080, Quality::Medium, 120));
        assert_eq!(pick(VENDOR_INTEL, Some(128), false, (1920, 1080), 16384), (true, res, Quality::Medium, secs));
        // AMD: weak with little VRAM or without AMF; unknown VRAM isn't held against it.
        assert_eq!(pick(VENDOR_AMD, Some(512), true, (2560, 1440), 16384), (true, Resolution::P1080, Quality::Medium, secs));
        assert_eq!(pick(VENDOR_AMD, None, true, (2560, 1440), 16384), (false, res, q, secs));
        assert_eq!(pick(VENDOR_AMD, Some(8192), false, (1920, 1080), 16384), (true, res, Quality::Medium, secs));
        // A rotated 1080p screen is not "larger than 1080p".
        assert_eq!(pick(VENDOR_INTEL, None, false, (1080, 1920), 16384).1, res);
        // RAM unknown: no change.
        assert_eq!(pick(VENDOR_NVIDIA, Some(8192), true, (1920, 1080), 0).3, secs);
    }

    #[test]
    fn only_a_new_install_is_a_first_start() {
        // Written by the installer.
        assert!(is_new_install(br#"{"autostart": false, "language": "de"}"#));
        // Written by the app, any version.
        assert!(!is_new_install(br#"{"engine": {}, "replaySeconds": 120}"#));
        assert!(!is_new_install(&serde_json::to_vec(&Settings::default()).unwrap()));
        assert!(!is_new_install(b"not json"));
    }

    #[test]
    fn onboarding_starts_hidden_only_after_an_update() {
        assert_eq!(Settings::default().onboarding, Onboarding::default());
        // Written by an older version of the app.
        let old = Settings::parse(br#"{"engine": {}, "replaySeconds": 120}"#).unwrap();
        assert!(old.onboarding.dismissed && !old.onboarding.clip_saved);
        assert_eq!(old.replay_seconds, 120);
        // Written by the installer of a new install.
        let fresh = Settings::parse(br#"{"autostart": false, "language": "de"}"#).unwrap();
        assert_eq!((fresh.onboarding.dismissed, fresh.autostart), (false, false));
        let new = Settings::parse(br#"{"engine": {}, "onboarding": {"clipSaved": true}}"#).unwrap();
        assert_eq!(new.onboarding, Onboarding { clip_saved: true, menu_opened: false, dismissed: false });
        // A default Settings written out reads back the same.
        let round = Settings::parse(&serde_json::to_vec(&Settings::default()).unwrap()).unwrap();
        assert_eq!(round, Settings::default());
        // Salvaged files follow the same rule; unreadable ones count as older.
        assert!(Settings::salvage(br#"{"engine": {}, "replaySeconds": "bad"}"#).onboarding.dismissed);
        assert!(!Settings::salvage(br#"{"replaySeconds": "bad"}"#).onboarding.dismissed);
        assert!(Settings::salvage(b"not json").onboarding.dismissed);
        let kept = Settings::salvage(br#"{"engine": {}, "replaySeconds": "bad", "onboarding": {"menuOpened": true}}"#);
        assert_eq!(kept.onboarding, Onboarding { clip_saved: false, menu_opened: true, dismissed: false });
    }
}
