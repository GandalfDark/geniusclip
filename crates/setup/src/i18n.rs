//! Installer strings: the `setup.*` keys of the app's backend locale files
//! (src-tauri/locales), so everything is translated in one place.

use std::collections::HashMap;
use windows::Win32::Globalization::GetUserDefaultUILanguage;

pub struct Lang {
    pub code: &'static str,
    /// Name of the language in itself.
    pub name: &'static str,
    /// DirectWrite locale: picks the right glyph shapes for CJK text.
    pub locale: &'static str,
}

/// Same order as LANGS in src-tauri/src/i18n.rs.
pub const LANGS: [Lang; 14] = [
    Lang { code: "ru", name: "Русский", locale: "ru-RU" },
    Lang { code: "en", name: "English", locale: "en-US" },
    Lang { code: "kk", name: "Қазақша", locale: "kk-KZ" },
    Lang { code: "uk", name: "Українська", locale: "uk-UA" },
    Lang { code: "de", name: "Deutsch", locale: "de-DE" },
    Lang { code: "fr", name: "Français", locale: "fr-FR" },
    Lang { code: "es", name: "Español", locale: "es-ES" },
    Lang { code: "pt-BR", name: "Português (Brasil)", locale: "pt-BR" },
    Lang { code: "pl", name: "Polski", locale: "pl-PL" },
    Lang { code: "tr", name: "Türkçe", locale: "tr-TR" },
    Lang { code: "it", name: "Italiano", locale: "it-IT" },
    Lang { code: "zh-CN", name: "简体中文", locale: "zh-CN" },
    Lang { code: "ja", name: "日本語", locale: "ja-JP" },
    Lang { code: "ko", name: "한국어", locale: "ko-KR" },
];
const EN: usize = 1;

const FILES: [&str; 14] = [
    include_str!("../../../src-tauri/locales/ru.json"),
    include_str!("../../../src-tauri/locales/en.json"),
    include_str!("../../../src-tauri/locales/kk.json"),
    include_str!("../../../src-tauri/locales/uk.json"),
    include_str!("../../../src-tauri/locales/de.json"),
    include_str!("../../../src-tauri/locales/fr.json"),
    include_str!("../../../src-tauri/locales/es.json"),
    include_str!("../../../src-tauri/locales/pt-BR.json"),
    include_str!("../../../src-tauri/locales/pl.json"),
    include_str!("../../../src-tauri/locales/tr.json"),
    include_str!("../../../src-tauri/locales/it.json"),
    include_str!("../../../src-tauri/locales/zh-CN.json"),
    include_str!("../../../src-tauri/locales/ja.json"),
    include_str!("../../../src-tauri/locales/ko.json"),
];

pub struct Strings {
    tables: Vec<HashMap<String, String>>,
}

impl Strings {
    pub fn load() -> Strings {
        Strings { tables: FILES.iter().map(|f| serde_json::from_str(f).unwrap_or_default()).collect() }
    }

    /// `setup.<key>` in the language, falling back to English.
    pub fn get(&self, lang: usize, key: &str) -> String {
        let key = format!("setup.{key}");
        self.tables[lang].get(&key).or_else(|| self.tables[EN].get(&key)).cloned().unwrap_or(key)
    }
}

/// The Windows UI language mapped to a supported one (mirrors the app).
pub fn system_lang() -> usize {
    let code = match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
        0x19 => "ru",
        0x3f => "kk",
        0x22 => "uk",
        0x07 => "de",
        0x0c => "fr",
        0x0a => "es",
        0x16 => "pt-BR",
        0x15 => "pl",
        0x1f => "tr",
        0x10 => "it",
        0x04 => "zh-CN",
        0x11 => "ja",
        0x12 => "ko",
        0x23 | 0x43 | 0x40 | 0x28 | 0x42 | 0x2c | 0x2b | 0x37 => "ru",
        _ => "en",
    };
    LANGS.iter().position(|l| l.code == code).unwrap_or(EN)
}
