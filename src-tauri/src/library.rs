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
use std::time::UNIX_EPOCH;

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
}

pub struct Library {
    index_path: PathBuf,
    thumbs_dir: PathBuf,
    records: Mutex<HashMap<PathBuf, Record>>,
    /// Metadata for files that are being written right now.
    pending: Mutex<HashMap<PathBuf, (Kind, String)>>,
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
        self.pending.lock().insert(path.to_path_buf(), (kind, game.to_string()));
    }

    /// Records a finished file; returns its entry.
    pub fn add(&self, path: &Path) -> Option<Entry> {
        let (kind, game) = self.pending.lock().remove(path).unwrap_or_else(|| (guess_kind(path), String::new()));
        let rec = self.make_record(path, kind, &game)?;
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
        Some(Record { kind, game: game.to_string(), size, modified, duration, width, height })
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
                        stale.push((path, kind, game));
                    }
                }
            }
        }
        // Probing opens every new video with FFmpeg: not under the lock,
        // which saving a clip (add) needs too.
        let fresh: Vec<(PathBuf, Record)> =
            stale.into_iter().filter_map(|(p, kind, game)| self.make_record(&p, kind, &game).map(|r| (p, r))).collect();
        let known: Vec<PathBuf> = self.records.lock().keys().cloned().collect();
        let gone: Vec<PathBuf> = known.into_iter().filter(|p| !p.exists()).collect();
        let mut changed = !fresh.is_empty();
        {
            let mut recs = self.records.lock();
            for (path, rec) in fresh {
                // add() may have indexed the same file meanwhile, with better metadata.
                let rec = match recs.get(&path) {
                    Some(r) if r.size == rec.size && r.modified == rec.modified => r.clone(),
                    _ => {
                        recs.insert(path.clone(), rec.clone());
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
        }
        if changed {
            self.persist();
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified));
        out
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
    /// Only the most recently opened clip is kept in the cache.
    pub fn clip_audio(&self, path: &Path, count: usize, skip: &[usize]) -> anyhow::Result<(Vec<PathBuf>, Vec<Vec<f32>>)> {
        let (size, modified) = file_stamp(path).ok_or_else(|| anyhow::anyhow!("file not found"))?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (path, size, modified).hash(&mut h);
        let stem = format!("{:016x}", h.finish());
        let dir = self.thumbs_dir.join("tracks");
        let files: Vec<PathBuf> = (0..count).map(|k| dir.join(format!("{stem}_{k}.m4a"))).collect();
        let peaks_path = dir.join(format!("{stem}.peaks.json"));
        if files.iter().all(|p| p.exists()) {
            if let Some(peaks) = std::fs::read(&peaks_path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                return Ok((files, peaks));
            }
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let files = geniusclip_engine::remix::extract_tracks(path, &dir, &stem)?;
        let peaks = geniusclip_engine::remix::peaks(path, PEAK_BUCKETS, skip)?;
        let _ = std::fs::write(&peaks_path, serde_json::to_vec(&peaks)?);
        Ok((files, peaks))
    }

    pub fn thumbs_dir(&self) -> &Path {
        &self.thumbs_dir
    }
}

/// Waveform resolution: enough for a full-width timeline on a 4K screen.
const PEAK_BUCKETS: usize = 1200;

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
