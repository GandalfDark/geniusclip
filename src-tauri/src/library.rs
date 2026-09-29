//! Clip library: scans the clip/screenshot folders, keeps a small index with
//! per-file metadata (kind, game, duration) and generates thumbnails.

use crate::settings::Settings;
use geniusclip_engine::media;
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Clip,
    Recording,
    Screenshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    kind: Kind,
    game: String,
    size: u64,
    modified: i64,
    duration: f64,
    width: u32,
    height: u32,
    /// Absent in indexes from older versions.
    #[serde(default)]
    favorite: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub game: String,
    pub kind: Kind,
    pub size: u64,
    /// Unix milliseconds.
    pub modified: i64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub favorite: bool,
}

/// What a file that is being written will be: kind, game, favorite.
type Pending = (Kind, String, bool);

pub struct Library {
    index_path: PathBuf,
    thumbs_dir: PathBuf,
    records: Mutex<HashMap<PathBuf, Record>>,
    /// Metadata for files that are being written right now.
    pending: Mutex<HashMap<PathBuf, Pending>>,
    /// One library.json writer at a time.
    persisting: Mutex<()>,
    thumb_jobs: Limiter,
}

/// At most this many thumbnails are decoded at once; the gallery asks for a
/// whole screen of them together.
const THUMB_JOBS: usize = 3;

/// A counting semaphore (std has none).
struct Limiter {
    busy: Mutex<usize>,
    freed: Condvar,
}

struct Slot<'a>(&'a Limiter);

impl Limiter {
    fn acquire(&self, max: usize) -> Slot<'_> {
        let mut busy = self.busy.lock();
        while *busy >= max {
            self.freed.wait(&mut busy);
        }
        *busy += 1;
        Slot(self)
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock() -= 1;
        self.0.freed.notify_one();
    }
}

fn file_stamp(p: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len(), modified))
}

fn is_video(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
}
fn is_image(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("jpg"))
}
/// A file type the library lists.
pub fn is_media(p: &Path) -> bool {
    is_video(p) || is_image(p)
}

impl Library {
    pub fn new(data_dir: &Path, cache_dir: &Path) -> Library {
        let index_path = data_dir.join("library.json");
        let records = std::fs::read(&index_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(PathBuf, Record)>>(&b).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();
        Library {
            index_path,
            thumbs_dir: cache_dir.join("thumbs"),
            records: Mutex::new(records),
            pending: Mutex::new(HashMap::new()),
            persisting: Mutex::new(()),
            thumb_jobs: Limiter { busy: Mutex::new(0), freed: Condvar::new() },
        }
    }

    fn persist(&self) {
        // Serialized, with the records read inside, so the newest state is
        // written last; through a temp file, so a crash or a concurrent write
        // never leaves a torn index (which would load as empty).
        let _one_at_a_time = self.persisting.lock();
        let v: Vec<(PathBuf, Record)> = self.records.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if let Some(d) = self.index_path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = self.index_path.with_extension("json.tmp");
        let res = serde_json::to_vec(&v)
            .map_err(std::io::Error::from)
            .and_then(|b| std::fs::write(&tmp, b))
            .and_then(|_| std::fs::rename(&tmp, &self.index_path));
        if let Err(e) = res {
            log::warn!("library.json: {e}");
        }
    }

    /// Remembers what a file that is about to be written is.
    pub fn expect(&self, path: &Path, kind: Kind, game: &str) {
        self.expect_as(path, kind, game, false);
    }

    /// `expect`, for a file that replaces a favorite (a trim that replaces
    /// the original keeps its star).
    pub fn expect_as(&self, path: &Path, kind: Kind, game: &str, favorite: bool) {
        self.pending.lock().insert(path.to_path_buf(), (kind, game.to_string(), favorite));
    }

    /// Records a finished file; returns its entry.
    pub fn add(&self, path: &Path) -> Option<Entry> {
        let (kind, game, favorite) = self.pending.lock().remove(path).unwrap_or_else(|| (guess_kind(path), String::new(), false));
        let mut rec = self.make_record(path, kind, &game)?;
        rec.favorite = favorite;
        self.records.lock().insert(path.to_path_buf(), rec.clone());
        self.persist();
        Some(entry(path, &rec))
    }

    fn make_record(&self, path: &Path, kind: Kind, game: &str) -> Option<Record> {
        let (size, modified) = file_stamp(path)?;
        let (duration, width, height) = if is_video(path) {
            media::probe(path).map(|i| (i.duration, i.width, i.height)).unwrap_or_default()
        } else {
            (0.0, 0, 0)
        };
        Some(Record { kind, game: game.to_string(), size, modified, duration, width, height, favorite: false })
    }

    /// Lists all clips/recordings/screenshots, newest first.
    pub fn scan(&self, settings: &Settings) -> Vec<Entry> {
        let mut files = Vec::new();
        for root in [&settings.clips_dir, &settings.screenshots_dir] {
            collect(root, root, 0, &mut files);
        }
        files.sort();
        files.dedup_by(|a, b| a.0 == b.0);
        let mut out = Vec::new();
        // New or changed files, with what the index knew about them.
        let mut stale = Vec::new();
        {
            let recs = self.records.lock();
            for (path, folder_game) in files {
                if path.to_string_lossy().ends_with(".part") {
                    continue;
                }
                let Some((size, modified)) = file_stamp(&path) else { continue };
                match recs.get(&path) {
                    Some(r) if r.size == size && r.modified == modified => out.push(entry(&path, r)),
                    old => {
                        let kind = old.map(|r| r.kind).unwrap_or_else(|| guess_kind(&path));
                        let game = old.map(|r| r.game.clone()).filter(|g| !g.is_empty()).unwrap_or(folder_game);
                        // A file changed on disk (edited elsewhere) keeps its star.
                        let favorite = old.is_some_and(|r| r.favorite);
                        stale.push((path, kind, game, favorite));
                    }
                }
            }
        }
        // Probing opens every new video with FFmpeg: not under the lock,
        // which saving a clip (add) needs too.
        let fresh: Vec<(PathBuf, Record)> = stale
            .into_iter()
            .filter_map(|(p, kind, game, favorite)| self.make_record(&p, kind, &game).map(|r| (p, Record { favorite, ..r })))
            .collect();
        let known: Vec<PathBuf> = self.records.lock().keys().cloned().collect();
        let gone: Vec<PathBuf> = known.into_iter().filter(|p| !p.exists()).collect();
        if self.merge_probed(fresh, gone, &mut out) {
            self.persist();
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified));
        out
    }

    /// The locked end of a scan: indexes the records probed outside the lock
    /// and forgets the files found gone; lists the entries in `out`. Returns
    /// whether the index changed.
    fn merge_probed(&self, fresh: Vec<(PathBuf, Record)>, gone: Vec<PathBuf>, out: &mut Vec<Entry>) -> bool {
        let mut changed = false;
        let mut recs = self.records.lock();
        for (path, rec) in fresh {
            // The file as it is now, checked under the lock: deletes, renames
            // and replace-trims move the file before they update the index
            // (forget, rename, add), so a file deleted or replaced during the
            // probe is caught here and never comes back into the index.
            let Some((size, modified)) = file_stamp(&path) else { continue };
            // add() may have indexed the same file meanwhile, with better metadata.
            let rec = match recs.get(&path) {
                Some(r) if r.size == size && r.modified == modified => r.clone(),
                // Changed during the probe (a replace-trim, a copy still being
                // written): what was probed is outdated. Left out; add() or
                // the next scan lists it.
                _ if rec.size != size || rec.modified != modified => continue,
                _ => {
                    recs.insert(path.clone(), rec.clone());
                    changed = true;
                    rec
                }
            };
            out.push(entry(&path, &rec));
        }
        // Forget files that disappeared (unless written again meanwhile).
        for p in gone {
            if !p.exists() && recs.remove(&p).is_some() {
                changed = true;
            }
        }
        changed
    }

    /// What the index knows about a file (kind, game), without a folder scan.
    pub fn info(&self, path: &Path) -> Option<(Kind, String)> {
        self.records.lock().get(path).map(|r| (r.kind, r.game.clone()))
    }

    pub fn is_favorite(&self, path: &Path) -> bool {
        self.records.lock().get(path).is_some_and(|r| r.favorite)
    }

    /// Stars or unstars an indexed file; returns its new listing. None when
    /// the index is not up to date for the file (scan first).
    pub fn set_favorite(&self, path: &Path, on: bool) -> Option<Entry> {
        let (size, modified) = file_stamp(path)?;
        let entry = {
            let mut recs = self.records.lock();
            let r = recs.get_mut(path).filter(|r| r.size == size && r.modified == modified)?;
            r.favorite = on;
            entry(path, r)
        };
        self.persist();
        Some(entry)
    }

    /// The listing of an indexed file, if the index is up to date for it.
    pub fn entry(&self, path: &Path) -> Option<Entry> {
        let (size, modified) = file_stamp(path)?;
        let recs = self.records.lock();
        recs.get(path).filter(|r| r.size == size && r.modified == modified).map(|r| entry(path, r))
    }

    pub fn forget(&self, path: &Path) {
        self.records.lock().remove(path);
        self.persist();
    }

    pub fn rename(&self, from: &Path, to: &Path) {
        let mut recs = self.records.lock();
        if let Some(r) = recs.remove(from) {
            recs.insert(to.to_path_buf(), r);
        }
        drop(recs);
        self.persist();
    }

    /// Returns a cached JPEG thumbnail, creating it if needed.
    pub fn thumbnail(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let (size, modified) = file_stamp(path).ok_or_else(|| anyhow::anyhow!("file not found"))?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (path, size, modified).hash(&mut h);
        let name = format!("{:016x}", h.finish());
        let out = self.thumbs_dir.join(format!("{name}.jpg"));
        if out.exists() {
            return Ok(out);
        }
        let _slot = self.thumb_jobs.acquire(THUMB_JOBS);
        // Made by another request while this one waited.
        if out.exists() {
            return Ok(out);
        }
        std::fs::create_dir_all(&self.thumbs_dir)?;
        let at = if is_video(path) { 1.0 } else { 0.0 };
        // Written aside and moved into place: a half-written JPEG must never
        // be served (or taken as done by the check above).
        static TMP: AtomicU64 = AtomicU64::new(0);
        let tmp = self.thumbs_dir.join(format!("{name}.{}.tmp.jpg", TMP.fetch_add(1, Ordering::Relaxed)));
        let res = media::thumbnail(path, &tmp, 480, at).and_then(|_| Ok(std::fs::rename(&tmp, &out)?));
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            // Another request may have finished the same thumbnail first.
            if !out.exists() {
                return Err(e);
            }
        }
        Ok(out)
    }

    /// Extracts the clip's audio tracks into separate files for the trim
    /// preview and computes their waveforms (empty for skipped tracks).
    /// Each clip gets its own folder under `tracks`; the most recently used
    /// few are kept (see `TRACK_SETS_KEPT`).
    pub fn clip_audio(&self, path: &Path, count: usize, skip: &[usize], progress: &mut dyn FnMut(f32)) -> anyhow::Result<(Vec<PathBuf>, Vec<Vec<f32>>)> {
        let (size, modified) = file_stamp(path).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (path, size, modified).hash(&mut h);
        let stem = format!("{:016x}", h.finish());
        let root = self.thumbs_dir.join("tracks");
        let dir = root.join(&stem);
        if let Some(cached) = read_tracks(&dir, count, skip) {
            return Ok(cached);
        }
        // Every request extracts into a folder of its own, moved into place
        // once complete: overlapping requests (a quick switch between clips,
        // or the same clip twice) must never overwrite or wipe files another
        // request has already handed to the UI.
        static REQUEST: AtomicU64 = AtomicU64::new(0);
        let tmp = root.join(format!("{stem}.{}.{}.tmp", std::process::id(), REQUEST.fetch_add(1, Ordering::Relaxed)));
        let made = (|| -> anyhow::Result<(Vec<PathBuf>, Vec<Vec<f32>>)> {
            let (files, peaks) = geniusclip_engine::remix::prepare_tracks(path, &tmp, "track", PEAK_BUCKETS, skip, progress)?;
            std::fs::write(tmp.join(PEAKS_FILE), serde_json::to_vec(&peaks)?)?;
            Ok((files, peaks))
        })();
        let (files, peaks) = match made {
            Ok(v) => v,
            Err(e) => {
                // Checked before the partial files are removed, which frees the space again.
                let full = disk_nearly_full(&tmp);
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(if full { anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::StorageFull)).context(format!("{e:#}")) } else { e });
            }
        };
        if std::fs::rename(&tmp, &dir).is_err() {
            // Another request for the same clip finished first: use its files.
            if let Some(done) = read_tracks(&dir, count, skip) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Ok(done);
            }
            // A broken leftover (e.g. a prune that could not finish): replace it.
            let _ = std::fs::remove_dir_all(&dir);
            if let Err(e) = std::fs::rename(&tmp, &dir) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(anyhow::Error::from(e).context("tracks folder"));
            }
        }
        prune_tracks(&root);
        let files = files.iter().filter_map(|f| f.file_name()).map(|n| dir.join(n)).collect();
        Ok((files, peaks))
    }

    pub fn thumbs_dir(&self) -> &Path {
        &self.thumbs_dir
    }
}

/// Waveform resolution: enough for a full-width timeline on a 4K screen.
const PEAK_BUCKETS: usize = 1200;
const PEAKS_FILE: &str = "peaks.json";
/// Track folders kept: the clip being edited and the one before it, which
/// the UI may still be playing while the next one loads.
const TRACK_SETS_KEPT: usize = 2;
/// Temp folders older than this are leftovers from a crash.
const STALE_TMP: Duration = Duration::from_secs(3600);

/// A complete track folder: its files and waveforms. Marks it as just used.
/// A finished set: a file for every track but the skipped ones (an empty
/// path stands for those), and the waveforms.
fn read_tracks(dir: &Path, count: usize, skip: &[usize]) -> Option<(Vec<PathBuf>, Vec<Vec<f32>>)> {
    let files: Vec<PathBuf> = (0..count).map(|k| if skip.contains(&k) { PathBuf::new() } else { dir.join(format!("track_{k}.m4a")) }).collect();
    if !files.iter().all(|p| p.as_os_str().is_empty() || p.is_file()) {
        return None;
    }
    let peaks_path = dir.join(PEAKS_FILE);
    let peaks = serde_json::from_slice(&std::fs::read(&peaks_path).ok()?).ok()?;
    // The waveform file's time orders folders for pruning.
    let _ = std::fs::File::options().write(true).open(&peaks_path).and_then(|f| f.set_modified(SystemTime::now()));
    Some((files, peaks))
}

/// Removes all but the most recently used track folders, loose files from
/// older versions (which kept tracks directly in `tracks`) and temp folders
/// left by a crash. Temp folders of requests still running are left alone.
fn prune_tracks(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let mut sets = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            let _ = std::fs::remove_file(&p);
        } else if p.extension().is_some_and(|x| x == "tmp") {
            let age = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a > STALE_TMP) {
                let _ = std::fs::remove_dir_all(&p);
            }
        } else {
            let used = std::fs::metadata(p.join(PEAKS_FILE)).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
            sets.push((used, p));
        }
    }
    sets.sort_by_key(|(used, _)| std::cmp::Reverse(*used));
    for (_, p) in sets.into_iter().skip(TRACK_SETS_KEPT) {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// Less free space than this counts as a full disk: after a write fails for
/// lack of space, a few MB can still show as free.
const NEARLY_FULL: u64 = 64 << 20;

/// The volume holding `p` (or its nearest existing parent) is almost full.
pub fn disk_nearly_full(p: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let Some(dir) = p.ancestors().find(|a| a.is_dir()) else { return false };
    let mut free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(&windows::core::HSTRING::from(dir), Some(&mut free), None, None) }.is_ok();
    ok && free < NEARLY_FULL
}

/// Another program has the file open without allowing it to be renamed or
/// deleted (a player, an editor, an upload in progress).
pub fn is_locked(p: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const DELETE: u32 = 0x0001_0000;
    // Asks for exactly what a rename or delete needs, sharing everything
    // else: only a handle that denies deletion makes this fail.
    std::fs::OpenOptions::new()
        .access_mode(DELETE)
        .share_mode(7) // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        .open(p)
        .is_err_and(|e| matches!(e.raw_os_error(), Some(32 | 33))) // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
}

fn guess_kind(p: &Path) -> Kind {
    if is_image(p) {
        Kind::Screenshot
    } else if p.file_stem().is_some_and(|s| s.to_string_lossy().ends_with(" REC")) {
        Kind::Recording
    } else {
        Kind::Clip
    }
}

fn entry(path: &Path, r: &Record) -> Entry {
    Entry {
        path: path.to_path_buf(),
        name: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        // Older versions could store version-info garbage (NULs) in names.
        game: r.game.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "),
        kind: r.kind,
        size: r.size,
        modified: r.modified,
        duration: r.duration,
        width: r.width,
        height: r.height,
        favorite: r.favorite,
    }
}

/// Collects media files up to one folder deep (the per-game folders).
fn collect(root: &Path, dir: &Path, depth: u32, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if depth == 0 {
                collect(root, &p, 1, out);
            }
        } else if is_video(&p) || is_image(&p) {
            let game = if depth == 1 {
                dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
            } else {
                String::new()
            };
            let _ = root;
            out.push((p, game));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("geniusclip-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A complete one-track folder, last used `age` seconds ago.
    fn track_set(root: &Path, name: &str, age: u64) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("track_0.m4a"), b"x").unwrap();
        std::fs::write(d.join(PEAKS_FILE), b"[[0.5]]").unwrap();
        let f = std::fs::File::options().write(true).open(d.join(PEAKS_FILE)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age)).unwrap();
        d
    }

    #[test]
    fn prune_keeps_recent_sets_and_running_requests() {
        let root = scratch("prune");
        let newest = track_set(&root, "a", 10);
        let previous = track_set(&root, "b", 20);
        let oldest = track_set(&root, "c", 30);
        let running = root.join("d.1.2.tmp");
        std::fs::create_dir_all(&running).unwrap();
        let legacy = root.join("0123456789abcdef_0.m4a");
        std::fs::write(&legacy, b"x").unwrap();
        prune_tracks(&root);
        assert!(newest.exists() && previous.exists());
        assert!(!oldest.exists());
        assert!(running.exists());
        assert!(!legacy.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_tracks_needs_every_file_and_marks_use() {
        let root = scratch("read");
        let d = track_set(&root, "a", 600);
        assert!(read_tracks(&d, 2, &[]).is_none());
        let (files, peaks) = read_tracks(&d, 1, &[]).unwrap();
        assert_eq!(files, vec![d.join("track_0.m4a")]);
        assert_eq!(peaks, vec![vec![0.5]]);
        let used = std::fs::metadata(d.join(PEAKS_FILE)).unwrap().modified().unwrap();
        assert!(used.elapsed().unwrap() < Duration::from_secs(60));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn favorites_survive_reloads_renames_rescans_and_replacing() {
        let root = scratch("fav");
        let (data, cache) = (root.join("data"), root.join("cache"));
        let shot = root.join("a.png");
        std::fs::write(&shot, b"x").unwrap();
        let lib = Library::new(&data, &cache);
        lib.expect(&shot, Kind::Screenshot, "Game");
        assert!(!lib.add(&shot).unwrap().favorite);
        assert!(lib.set_favorite(&shot, true).unwrap().favorite);
        // Renamed, then loaded again from library.json.
        let renamed = root.join("b.png");
        std::fs::rename(&shot, &renamed).unwrap();
        lib.rename(&shot, &renamed);
        let lib = Library::new(&data, &cache);
        assert!(lib.entry(&renamed).unwrap().favorite);
        // Changed on disk: the rescan keeps the star.
        std::fs::write(&renamed, b"xyz").unwrap();
        let settings = Settings { clips_dir: root.clone(), screenshots_dir: root.clone(), ..Default::default() };
        let listed = lib.scan(&settings);
        assert!(listed.iter().find(|e| e.path == renamed).unwrap().favorite);
        // Replaced by a trimmed version.
        let keep = lib.is_favorite(&renamed);
        lib.forget(&renamed);
        lib.expect_as(&renamed, Kind::Screenshot, "Game", keep);
        assert!(lib.add(&renamed).unwrap().favorite);
        // A file not indexed yet can't be starred without a scan.
        let other = root.join("c.png");
        std::fs::write(&other, b"x").unwrap();
        assert!(lib.set_favorite(&other, true).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_deleted_or_replaced_during_a_scan_are_not_indexed() {
        let root = scratch("race");
        let lib = Library::new(&root.join("data"), &root.join("cache"));
        let (a, b, c) = (root.join("a.png"), root.join("b.png"), root.join("c.png"));
        for p in [&a, &b, &c] {
            std::fs::write(p, b"x").unwrap();
        }
        let probed: Vec<(PathBuf, Record)> = [&a, &b, &c].into_iter().map(|p| (p.clone(), lib.make_record(p, Kind::Screenshot, "").unwrap())).collect();
        // While the probe ran: a deleted, b replaced by a different file.
        std::fs::remove_file(&a).unwrap();
        std::fs::write(&b, b"longer").unwrap();
        let mut out = Vec::new();
        assert!(lib.merge_probed(probed, Vec::new(), &mut out));
        assert_eq!(out.iter().map(|e| &e.path).collect::<Vec<_>>(), vec![&c]);
        assert!(lib.info(&a).is_none() && lib.info(&b).is_none() && lib.info(&c).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn indexes_from_before_favorites_still_load() {
        let root = scratch("oldindex");
        std::fs::write(
            root.join("library.json"),
            r#"[["C:\\x.png",{"kind":"screenshot","game":"","size":1,"modified":0,"duration":0.0,"width":0,"height":0}]]"#,
        )
        .unwrap();
        let lib = Library::new(&root, &root);
        assert_eq!(lib.records.lock().len(), 1);
        assert!(!lib.is_favorite(Path::new("C:\\x.png")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn locked_files_are_detected() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = scratch("lock");
        let f = root.join("clip.mp4");
        std::fs::write(&f, b"x").unwrap();
        assert!(!is_locked(&f));
        // Shared for reading only, like a player that denies deletion.
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(&f).unwrap();
        assert!(is_locked(&f));
        drop(held);
        assert!(!is_locked(&f));
        let _ = std::fs::remove_dir_all(&root);
    }
}
