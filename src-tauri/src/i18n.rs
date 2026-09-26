//! The few strings the backend shows itself (tray menu, overlay texts, file
//! names). Translations live in src-tauri/locales/<code>.json; everything
//! else is translated in the UI (src/lib/locales).

use std::collections::HashMap;
use std::sync::OnceLock;
use windows::Win32::Globalization::GetUserDefaultUILanguage;

/// Supported languages; must match LANGS in src/lib/i18n.ts.
pub const LANGS: [&str; 14] = ["ru", "en", "kk", "uk", "de", "fr", "es", "pt-BR", "pl", "tr", "it", "zh-CN", "ja", "ko"];

const FILES: [&str; 14] = [
    include_str!("../locales/ru.json"),
    include_str!("../locales/en.json"),
    include_str!("../locales/kk.json"),
    include_str!("../locales/uk.json"),
    include_str!("../locales/de.json"),
    include_str!("../locales/fr.json"),
    include_str!("../locales/es.json"),
    include_str!("../locales/pt-BR.json"),
    include_str!("../locales/pl.json"),
    include_str!("../locales/tr.json"),
    include_str!("../locales/it.json"),
    include_str!("../locales/zh-CN.json"),
    include_str!("../locales/ja.json"),
    include_str!("../locales/ko.json"),
];

fn tables() -> &'static [HashMap<String, String>] {
    static TABLES: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
    TABLES.get_or_init(|| FILES.iter().map(|f| serde_json::from_str(f).expect("valid locale JSON")).collect())
}

/// The Windows UI language mapped to a supported one.
pub fn system_lang() -> &'static str {
    let primary = unsafe { GetUserDefaultUILanguage() } & 0x3ff;
    match primary {
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
        // Other CIS languages: be, uz, ky, tg, tk, az, hy, ka.
        0x23 | 0x43 | 0x40 | 0x28 | 0x42 | 0x2c | 0x2b | 0x37 => "ru",
        _ => "en",
    }
}

/// Translation of `key`, falling back to English.
pub fn t(lang: &str, key: &str) -> &'static str {
    let tables = tables();
    let idx = LANGS.iter().position(|l| *l == lang).unwrap_or(1);
    tables[idx].get(key).or_else(|| tables[1].get(key)).map(String::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_has_every_key() {
        let tables = tables();
        for (i, table) in tables.iter().enumerate() {
            for key in tables[0].keys() {
                assert!(table.get(key).is_some_and(|s| !s.is_empty()), "{} is missing {key}", LANGS[i]);
            }
        }
    }
}
