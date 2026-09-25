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
        OverlaySettings { enabled: true, corner: "top-right".into(), sound: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub engine: EngineConfig,
    pub replay_enabled: bool,
    pub replay_seconds: u32,
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: EngineConfig::default(),
            replay_enabled: true,
            replay_seconds: 300,
            clips_dir: PathBuf::new(),
            screenshots_dir: PathBuf::new(),
            sort_by_game: true,
            hotkeys: Hotkeys::default(),
            autostart: true,
            language: "auto".into(),
            accent: "aurora".into(),
            overlay: OverlaySettings::default(),
            auto_update: true,
        }
    }
}

fn path(app: &AppHandle) -> PathBuf {
    app.path().app_config_dir().unwrap_or_else(|_| PathBuf::from(".")).join("settings.json")
}

impl Settings {
    pub fn load(app: &AppHandle) -> Settings {
        let mut s: Settings = std::fs::read(path(app))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| log::warn!("settings.json invalid: {e}")).ok())
            .unwrap_or_default();
        s.fill_defaults(app);
        s
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

    /// Effective UI language ("ru" or "en").
    pub fn lang(&self) -> &'static str {
        match self.language.as_str() {
            "ru" => "ru",
            "en" => "en",
            _ => crate::i18n::system_lang(),
        }
    }
}
