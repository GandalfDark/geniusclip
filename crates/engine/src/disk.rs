//! Disk-backed storage for the replay buffer: packet payloads go to temporary
//! segment files and only a small index stays in memory.
//!
//! All file work happens on one writer thread. Packets enter the buffer with
//! their bytes in memory and are moved to disk shortly after (see `Payload`),
//! so a slow disk never stalls the encoder or audio threads, which push under
//! the buffer lock. Pruned packets are dropped on that thread too: dropping
//! the last packet of a segment closes, and so deletes, its file.
//!
//! Each segment is opened with FILE_FLAG_DELETE_ON_CLOSE and shared (Arc) by
//! the packets stored in it, so Windows deletes the file as soon as the last
//! packet referring to it is pruned and no clip being saved still reads it —
//! and also when the app exits or crashes.

use crate::ffutil::PacketRef;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x0000_2000;

/// A new segment is started after this many bytes (≈ 15 s at 32 Mbit/s).
const SEGMENT_BYTES: u64 = 64 << 20;
/// After a write error the disk is tried again this much later.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Jobs queued for the writer (≈ 10 s of packets at 240 fps). When full,
/// new packets simply stay in memory.
const QUEUE: usize = 4096;

/// Segment numbers are unique per process, not per store: segments of an
/// earlier pipeline can still be open (a clip being saved from them).
static NEXT_SEGMENT: AtomicU64 = AtomicU64::new(0);

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

    /// Appends `data`; returns its offset. Only the writer thread appends,
    /// while savers may read older ranges concurrently: all I/O uses
    /// explicit offsets, never the shared file cursor.
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

enum Job {
    Store(PacketRef),
    Release(Vec<PacketRef>),
    Reset,
}

/// The replay buffer's handle to the writer thread, which ends once this is
/// dropped and its queue is done.
pub struct DiskStore {
    tx: Sender<Job>,
}

impl DiskStore {
    /// Default location: the user's temp folder.
    pub fn default_dir() -> PathBuf {
        std::env::temp_dir().join("GeniusClip")
    }

    pub fn new(dir: PathBuf) -> io::Result<DiskStore> {
        let (tx, rx) = bounded(QUEUE);
        std::thread::Builder::new()
            .name("gc-disk".into())
            .spawn(move || Writer { dir, current: None, retry_at: None }.run(rx))?;
        Ok(DiskStore { tx })
    }

    /// Queues the packet's payload for the disk; until it is written (or if
    /// the writer is far behind) it is read from memory.
    pub fn store(&self, p: &PacketRef) {
        let _ = self.tx.try_send(Job::Store(p.clone()));
    }

    /// Hands packets the buffer let go of to the writer thread to drop (on a
    /// full queue they are dropped here after all).
    pub fn release(&self, packets: Vec<PacketRef>) {
        if !packets.is_empty() {
            let _ = self.tx.try_send(Job::Release(packets));
        }
    }

    /// Drops the open segment (it is deleted once its packets are gone) and
    /// tries the disk again if it had failed.
    pub fn reset(&self) {
        let _ = self.tx.try_send(Job::Reset);
    }
}

struct Writer {
    dir: PathBuf,
    current: Option<Arc<Segment>>,
    /// Set after a write error (e.g. disk full): packets stay in RAM until
    /// then, so the buffer keeps working without filling the disk further.
    retry_at: Option<Instant>,
}

impl Writer {
    fn run(mut self, rx: Receiver<Job>) {
        let _ = std::fs::create_dir_all(&self.dir);
        // Leftovers can only exist if a delete-on-close failed; files still
        // open (an earlier pipeline, another instance) refuse deletion.
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "buf") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        for job in rx {
            match job {
                Job::Store(p) => self.store(&p),
                Job::Release(packets) => drop(packets),
                Job::Reset => {
                    self.current = None;
                    self.retry_at = None;
                }
            }
        }
    }

    /// Moves the packet's payload to disk. On failure it stays in memory,
    /// so the buffer keeps working.
    fn store(&mut self, p: &PacketRef) {
        // Pruned before its turn came (only this job holds it): skip.
        if Arc::strong_count(p) == 1 || self.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        match p.data.move_to_disk(|bytes| self.append(bytes)) {
            Ok(()) => {
                if self.retry_at.take().is_some() {
                    log::info!("disk buffer writes work again");
                }
            }
            Err(e) => {
                log::error!("disk buffer write failed, keeping new packets in memory for a minute: {e}");
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
                self.current = None;
            }
        }
    }

    fn append(&mut self, data: &[u8]) -> io::Result<(Arc<Segment>, u64)> {
        if self.current.as_ref().is_none_or(|s| s.len() >= SEGMENT_BYTES) {
            let n = NEXT_SEGMENT.fetch_add(1, Ordering::Relaxed);
            let name = format!("{}-{n}.buf", std::process::id());
            self.current = Some(Arc::new(Segment::create(&self.dir.join(name))?));
        }
        let seg = self.current.clone().unwrap();
        let offset = seg.append(data)?;
        Ok((seg, offset))
    }
}
