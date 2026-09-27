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
        let mut s = match std::fs::read(&p) {
            Ok(b) => Settings::parse(&b).unwrap_or_else(|e| {
                // One bad value (e.g. an enum variant from a newer version)
                // must not reset everything, clip folders included: keep a
                // copy of the file and every value that still fits.
                log::warn!("settings.json invalid: {e}; keeping the values that still parse");
                let _ = std::fs::copy(&p, p.with_file_name("settings.bak.json"));
                Settings::salvage(&b)
            }),
            Err(_) => Settings::default(),
        };
        s.fill_defaults(app);
        s
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

#[cfg(test)]
mod tests {
    use super::*;

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
