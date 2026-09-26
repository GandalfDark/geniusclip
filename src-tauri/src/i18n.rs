//! The few strings the backend shows itself (tray menu, overlay texts).
//! Everything else is translated in the UI.

use windows::Win32::Globalization::GetUserDefaultUILanguage;

pub fn system_lang() -> &'static str {
    // Primary language id 0x19 = Russian; also use Russian for other CIS UI languages.
    let primary = unsafe { GetUserDefaultUILanguage() } & 0x3ff;
    match primary {
        0x19 | 0x22 | 0x23 | 0x3f | 0x40 | 0x43 | 0x28 | 0x42 => "ru", // ru, uk, be, kk, ky, uz, tg, tk
        _ => "en",
    }
}

pub fn t(lang: &str, key: &str) -> &'static str {
    let ru = lang == "ru";
    match key {
        "open" => if ru { "Открыть GeniusClip" } else { "Open GeniusClip" },
        "save_clip" => if ru { "Сохранить клип" } else { "Save clip" },
        "replay_on" => if ru { "Повтор: включён" } else { "Replay: on" },
        "replay_off" => if ru { "Повтор: выключен" } else { "Replay: off" },
        "record_start" => if ru { "Начать запись" } else { "Start recording" },
        "record_stop" => if ru { "Остановить запись" } else { "Stop recording" },
        "screenshot" => if ru { "Скриншот" } else { "Screenshot" },
        "open_folder" => if ru { "Открыть папку клипов" } else { "Open clips folder" },
        "copy_last" => if ru { "Скопировать последний клип" } else { "Copy last clip" },
        "ov.copied" => if ru { "Клип скопирован" } else { "Clip copied" },
        "ov.copied.sub" => if ru { "Вставьте в чат: Ctrl+V" } else { "Paste into a chat: Ctrl+V" },
        "quit" => if ru { "Выход" } else { "Quit" },
        "desktop" => if ru { "Рабочий стол" } else { "Desktop" },
        "ov.clip" => if ru { "Клип сохранён" } else { "Clip saved" },
        "ov.recording" => if ru { "Запись сохранена" } else { "Recording saved" },
        "ov.recording-start" => if ru { "Запись началась" } else { "Recording started" },
        "ov.screenshot" => if ru { "Скриншот сохранён" } else { "Screenshot saved" },
        "ov.replay-on" => if ru { "Повтор включён" } else { "Replay on" },
        "ov.replay-off" | "ov.replay-off-hint" => if ru { "Повтор выключен" } else { "Replay off" },
        "ov.replay-off-hint.sub" => if ru { "Включите его, чтобы сохранять клипы" } else { "Turn it on to save clips" },
        "ov.error" => if ru { "Не получилось" } else { "Something went wrong" },
        "ov.already-saved" => if ru { "Уже сохранено" } else { "Already saved" },
        "ov.already-saved.sub" => if ru { "Новых моментов пока нет" } else { "Nothing new since the last clip" },
        _ => "",
    }
}
