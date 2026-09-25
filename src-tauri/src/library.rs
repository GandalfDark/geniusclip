//! Clip library: scans the clip/screenshot folders, keeps a small index with
//! per-file metadata (kind, game, duration) and generates thumbnails.

use crate::settings::Settings;
use geniusclip_engine::media;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
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

impl Library {
    pub fn new(data_dir: &Path, cache_dir: &Path) -> Library {
        let index_path = data_dir.join("library.json");
        let records = std::fs::read(&index_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(PathBuf, Record)>>(&b).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();
        Library { index_path, thumbs_dir: cache_dir.join("thumbs"), records: Mutex::new(records), pending: Mutex::new(HashMap::new()) }
    }

    fn persist(&self) {
        let v: Vec<(PathBuf, Record)> = self.records.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if let Some(d) = self.index_path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec(&v) {
            let _ = std::fs::write(&self.index_path, b);
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
        let mut changed = false;
        {
            let mut recs = self.records.lock();
            for (path, folder_game) in files {
                if path.to_string_lossy().ends_with(".part") {
                    continue;
                }
                let Some((size, modified)) = file_stamp(&path) else { continue };
                let rec = match recs.get(&path) {
                    Some(r) if r.size == size && r.modified == modified => r.clone(),
                    old => {
                        let kind = old.map(|r| r.kind).unwrap_or_else(|| guess_kind(&path));
                        let game = old.map(|r| r.game.clone()).filter(|g| !g.is_empty()).unwrap_or(folder_game);
                        let Some(r) = self.make_record(&path, kind, &game) else { continue };
                        recs.insert(path.clone(), r.clone());
                        changed = true;
                        r
                    }
                };
                out.push(entry(&path, &rec));
            }
            // Forget files that disappeared.
            let before = recs.len();
            recs.retain(|p, _| p.exists());
            changed |= recs.len() != before;
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
        let out = self.thumbs_dir.join(format!("{:016x}.jpg", h.finish()));
        if !out.exists() {
            std::fs::create_dir_all(&self.thumbs_dir)?;
            let at = if is_video(path) { 1.0 } else { 0.0 };
            media::thumbnail(path, &out, 480, at)?;
        }
        Ok(out)
    }

    pub fn thumbs_dir(&self) -> &Path {
        &self.thumbs_dir
    }
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
        game: r.game.clone(),
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
