//! Disk-backed storage for the replay buffer: packet payloads go to temporary
//! segment files and only a small index stays in memory.
//!
//! Each segment is opened with FILE_FLAG_DELETE_ON_CLOSE and shared (Arc) by
//! the packets stored in it, so Windows deletes the file as soon as the last
//! packet referring to it is pruned and no clip being saved still reads it —
//! and also when the app exits or crashes.

use crate::ffutil::{Packet, Payload};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x0000_2000;

/// A new segment is started after this many bytes (≈ 15 s at 32 Mbit/s).
const SEGMENT_BYTES: u64 = 64 << 20;

pub struct Segment {
    file: File,
    len: AtomicU64,
}

impl Segment {
    fn create(path: &Path) -> io::Result<Segment> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .attributes(FILE_ATTRIBUTE_NOT_CONTENT_INDEXED)
            .open(path)?;
        Ok(Segment { file, len: AtomicU64::new(0) })
    }

    /// Appends `data`; returns its offset. Only the buffer (under its lock)
    /// appends, while savers may read older ranges concurrently: all I/O
    /// uses explicit offsets, never the shared file cursor.
    fn append(&self, data: &[u8]) -> io::Result<u64> {
        let offset = self.len.load(Ordering::Acquire);
        let mut done = 0;
        while done < data.len() {
            done += self.file.seek_write(&data[done..], offset + done as u64)?;
        }
        self.len.store(offset + data.len() as u64, Ordering::Release);
        Ok(offset)
    }

    pub fn read_at(&self, offset: u64, dst: &mut [u8]) -> io::Result<()> {
        let mut done = 0;
        while done < dst.len() {
            let n = self.file.seek_read(&mut dst[done..], offset + done as u64)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }

    fn len(&self) -> u64 {
        self.len.load(Ordering::Acquire)
    }
}

pub struct DiskStore {
    dir: PathBuf,
    current: Option<Arc<Segment>>,
    next: u64,
    /// Set after a write error (e.g. disk full): packets then stay in RAM.
    failed: bool,
}

impl DiskStore {
    /// Default location: the user's temp folder.
    pub fn default_dir() -> PathBuf {
        std::env::temp_dir().join("GeniusClip")
    }

    pub fn new(dir: PathBuf) -> DiskStore {
        let _ = std::fs::create_dir_all(&dir);
        // Leftovers can only exist if a delete-on-close failed; files still
        // open in another instance refuse deletion and are kept.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "buf") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        DiskStore { dir, current: None, next: 0, failed: false }
    }

    /// Moves the packet's payload to disk. On failure the packet is returned
    /// unchanged, so the buffer keeps working from memory.
    pub fn store(&mut self, p: Arc<Packet>) -> Arc<Packet> {
        if self.failed {
            return p;
        }
        let Payload::Mem(bytes) = &p.data else { return p };
        match self.append(bytes) {
            Ok((seg, offset)) => Arc::new(Packet {
                data: Payload::Disk { seg, offset, len: bytes.len() as u32 },
                pts: p.pts,
                dts: p.dts,
                duration: p.duration,
                key: p.key,
                time_us: p.time_us,
            }),
            Err(e) => {
                log::error!("disk buffer write failed, keeping the buffer in memory: {e}");
                self.failed = true;
                self.current = None;
                p
            }
        }
    }

    fn append(&mut self, data: &[u8]) -> io::Result<(Arc<Segment>, u64)> {
        if self.current.as_ref().is_none_or(|s| s.len() >= SEGMENT_BYTES) {
            let name = format!("{}-{}.buf", std::process::id(), self.next);
            self.next += 1;
            self.current = Some(Arc::new(Segment::create(&self.dir.join(name))?));
        }
        let seg = self.current.clone().unwrap();
        let offset = seg.append(data)?;
        Ok((seg, offset))
    }

    /// Drops the open segment (it is deleted once its packets are gone).
    pub fn reset(&mut self) {
        self.current = None;
        self.failed = false;
    }
}
