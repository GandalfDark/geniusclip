//! Free space on the drive holding the clips: the UI's low-space warning and
//! a toast after saving, before a full disk starts failing saves.

use crate::settings::Settings;
use serde::Serialize;
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumePathNameW};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpace {
    /// The volume: "C:", or the root of a network share or mounted folder.
    pub drive: String,
    pub free_mb: u64,
    pub total_mb: u64,
    /// Less free space than this counts as low.
    pub low_mb: u64,
    pub low: bool,
}

/// Warn below this much even when replay clips are small.
const LOW_FLOOR_MB: u64 = 5 * 1024;

/// Space on the clips folder's drive, judged against the size of a clip of
/// the whole replay buffer (two of them should still fit).
pub fn space(settings: &Settings) -> anyhow::Result<DiskSpace> {
    // The folder itself may not exist yet (or any more): its drive counts.
    let dir = settings.clips_dir.ancestors().find(|a| a.is_dir()).ok_or_else(|| anyhow::anyhow!("clips folder unavailable: {}", settings.clips_dir.display()))?;
    let (free, total) = free_and_total(dir)?;
    let clip_mb = crate::commands::estimate_for(settings).buffer_mb as u64;
    Ok(judge(volume_name(dir), free, total, clip_mb))
}

fn judge(drive: String, free: u64, total: u64, clip_mb: u64) -> DiskSpace {
    let low_mb = LOW_FLOOR_MB.max(clip_mb * 2);
    let free_mb = free >> 20;
    DiskSpace { drive, free_mb, total_mb: total >> 20, low_mb, low: free_mb < low_mb }
}

/// Bytes available to this user (quotas included) and the volume's size.
fn free_and_total(dir: &Path) -> windows::core::Result<(u64, u64)> {
    let (mut free, mut total) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir), Some(&mut free), Some(&mut total), None) }?;
    Ok((free, total))
}

/// "C:" for `C:\Users\…`; the mount point for anything else.
fn volume_name(dir: &Path) -> String {
    let mut buf = [0u16; 1024];
    let root = match unsafe { GetVolumePathNameW(&HSTRING::from(dir), &mut buf) } {
        Ok(()) => String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())]),
        Err(_) => dir.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default(),
    };
    let trimmed = root.trim_end_matches('\\');
    if trimmed.is_empty() { root } else { trimmed.to_string() }
}

/// A size for people: "3,2 ГБ", "12 GB", "640 MB".
pub fn fmt_size(lang: &str, mb: u64) -> String {
    let (n, unit) = if mb >= 1024 {
        let gb = mb as f64 / 1024.0;
        (if gb < 10.0 { format!("{gb:.1}") } else { format!("{gb:.0}") }, "unit.gb")
    } else {
        (mb.to_string(), "unit.mb")
    };
    // Most supported languages write a decimal comma.
    let n = if matches!(lang, "en" | "zh-CN" | "ja" | "ko") { n } else { n.replace('.', ",") };
    format!("{n} {}", crate::i18n::t(lang, unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_means_less_than_two_full_clips_or_5_gb() {
        let gb = 1u64 << 30;
        // Short buffer: the 5 GB floor applies.
        let d = judge("C:".into(), 4 * gb, 100 * gb, 300);
        assert_eq!((d.free_mb, d.total_mb, d.low_mb, d.low), (4096, 102_400, 5120, true));
        assert!(!judge("C:".into(), 6 * gb, 100 * gb, 300).low);
        // 20-minute 4K buffer (~4 GB per clip): two of them.
        let d = judge("D:".into(), 6 * gb, 100 * gb, 4000);
        assert_eq!((d.low_mb, d.low), (8000, true));
    }

    #[test]
    fn volume_of_an_existing_folder() {
        let dir = std::env::temp_dir();
        let name = volume_name(&dir);
        assert!(!name.is_empty() && !name.ends_with('\\'), "{name}");
        assert!(dir.to_string_lossy().to_lowercase().starts_with(&name.to_lowercase()));
        let (free, total) = free_and_total(&dir).unwrap();
        assert!(total > 0 && free <= total);
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(fmt_size("en", 3277), "3.2 GB");
        assert_eq!(fmt_size("ru", 3277), "3,2 ГБ");
        assert_eq!(fmt_size("en", 13_000), "13 GB");
        assert_eq!(fmt_size("fr", 640), "640 Mo");
    }
}
