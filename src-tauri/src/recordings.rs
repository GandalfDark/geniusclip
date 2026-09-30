//! Recordings left fragmented, finished in the background after startup.
//!
//! A recording is written as `<name> REC.mp4.part` and becomes a regular MP4
//! when it stops (see the engine's `finalize`). If the app is killed while
//! recording (Windows shuts down, a crash), the `.part` stays behind: it
//! plays, but the gallery doesn't list it. And recordings made before 0.1.6
//! were left fragmented, which makes a long one take minutes to open in the
//! built-in player.

use crate::state::AppState;
use geniusclip_engine::finalize;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_SHARE_DELETE, FILE_SHARE_READ};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};

/// Finishes the clips folder's fragmented recordings, once, a little after startup.
pub fn spawn(app: AppHandle) {
    let spawned = std::thread::Builder::new().name("gc-recordings".into()).spawn(move || {
        // Startup (capture, the window, the first listing) goes first.
        std::thread::sleep(Duration::from_secs(15));
        // Low CPU and disk priority: a game may be loading from the same disk.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        }
        let dir = app.state::<AppState>().settings.read().clips_dir.clone();
        for path in candidates(&dir) {
            if is_recording(&path) {
                repair(&app, &path);
            } else {
                drop_orphan(&path);
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("recordings thread: {e}");
    }
}

/// Recordings in the clips folder and its game folders.
fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(root) {
        dirs.extend(rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()));
    }
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            // Recordings, and clips whose writing was cut off (`.part`), named
            // the app's way: the folder may be one other programs use too
            // (a browser's unfinished download is `name.mp4.part`).
            if !(name.ends_with(" rec.mp4") || name.ends_with(".mp4.part")) || !has_timestamp(&name) {
                continue;
            }
            // Only in the cloud (OneDrive): reading would download it.
            let cloud = FILE_ATTRIBUTE_OFFLINE.0 | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0;
            if e.metadata().is_ok_and(|m| m.file_attributes() & cloud == 0) {
                out.push(e.path());
            }
        }
    }
    out
}

fn is_recording(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.ends_with(" rec.mp4") || name.ends_with(" rec.mp4.part")
}

/// Whether a file name holds the date and time the app puts in every name
/// ("2026-09-27 21-58-04").
fn has_timestamp(name: &str) -> bool {
    let b = name.as_bytes();
    let digit = |i: usize| b[i].is_ascii_digit();
    b.windows(19).enumerate().any(|(i, _)| {
        (0..19).all(|k| match k {
            4 | 7 | 13 | 16 => b[i + k] == b'-',
            10 => b[i + k] == b' ',
            _ => digit(i + k),
        })
    })
}

/// A clip's `.part` left by a crash: without its index it can't play, and
/// it goes to the Recycle Bin. One being written right now is open and skipped.
fn drop_orphan(path: &Path) {
    let Ok(f) = OpenOptions::new().read(true).write(true).share_mode(0).open(path) else { return };
    drop(f);
    match trash::delete(path) {
        Ok(()) => log::info!("removed unfinished clip {}", path.display()),
        Err(e) => log::warn!("unfinished clip {}: {e}", path.display()),
    }
}

fn repair(app: &AppHandle, path: &Path) {
    let part = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("part"));
    // A finished recording is only read (a few box headers) to tell.
    if !part && !File::open(path).is_ok_and(|mut f| finalize::needs_finish(&mut f).unwrap_or(false)) {
        return;
    }
    // Others may read, rename or delete it meanwhile, nobody may write it:
    // a recording in progress (its writer has it open) is skipped.
    let share = FILE_SHARE_READ.0 | FILE_SHARE_DELETE.0;
    let Ok(mut f) = OpenOptions::new().read(true).write(true).share_mode(share).open(path) else { return };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let modified = f.metadata().and_then(|m| m.modified()).ok();
    let started = Instant::now();
    let result = finalize::finalize_file(&mut f);
    // Whatever happened, an old recording must not jump to the top of the gallery.
    if let Some(m) = modified {
        let _ = f.set_modified(m);
    }
    drop(f);
    match result {
        Ok(finalize::Outcome::Empty) => {
            // Cut off in its first second: nothing to keep.
            log::info!("recording {name}: nothing recorded");
            if part {
                let _ = trash::delete(path);
            }
            return;
        }
        Ok(outcome) => log::info!("recording {name}: {outcome:?} in {:.1?}", started.elapsed()),
        // Fragmented, it still plays: a `.part` is listed as it is.
        Err(e) => {
            log::warn!("recording {name} stays fragmented: {e:#}");
            if !part {
                return;
            }
        }
    }
    let path = if part {
        let dst = free_name(&path.with_extension(""));
        if let Err(e) = std::fs::rename(path, &dst) {
            log::warn!("recording {name}: {e}");
            return;
        }
        log::info!("recovered recording {}", dst.display());
        dst
    } else {
        path.to_path_buf()
    };
    // Listed (or relisted) by the next scan, which keeps the game and star.
    crate::emit_library_changed(app, &path, None, false);
}

/// `path`, or `name (2) REC.mp4`… if it is taken (still ending in " REC",
/// which marks a recording).
fn free_name(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let base = stem.strip_suffix(" REC").unwrap_or(&stem).to_string();
    (2..)
        .map(|i| path.with_file_name(format!("{base} ({i}) REC.mp4")))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::has_timestamp;

    #[test]
    fn only_the_apps_names() {
        assert!(has_timestamp("cs2 2026-09-27 21-25-00 rec.mp4.part"));
        assert!(has_timestamp("brawlhalla 2026-09-27 00-48-38 (обрезка).mp4.part"));
        assert!(!has_timestamp("movie.mp4.part"));
        assert!(!has_timestamp("2026-09-27.mp4.part"));
    }
}
