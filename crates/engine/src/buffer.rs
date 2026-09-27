//! Replay buffer of encoded packets.
//!
//! Stores already-compressed packets (≈ bitrate × duration bytes), trimmed on
//! whole-GOP boundaries so a clip can always start on a keyframe. The bytes
//! live in RAM, or in temporary files (see `disk`) with only the index here.
//!
//! Packets of a stream arrive in time order (no B-frames), so time ranges are
//! found by binary search, and an index of video keyframes keeps each push
//! O(1) while nothing has to be dropped.

use crate::disk::DiskStore;
use crate::ffutil::{Packet, PacketRef, StreamDesc, StreamKind};
use std::collections::VecDeque;

pub struct ReplayBuffer {
    pub streams: Vec<StreamDesc>,
    queues: Vec<VecDeque<PacketRef>>,
    /// Times of the video keyframes held, oldest first; the first one is the
    /// oldest video packet (the video queue always starts on a keyframe).
    keys: VecDeque<i64>,
    bytes: usize,
    max_us: i64,
    disk: Option<DiskStore>,
    /// End of the last saved clip; the next clip can continue from here
    /// instead of repeating footage that is already saved.
    pub last_saved_end_us: Option<i64>,
}

/// Packets selected for one clip, per stream, in decode order.
pub struct ClipData {
    pub streams: Vec<StreamDesc>,
    pub packets: Vec<Vec<PacketRef>>,
    /// Where playback starts (t = 0 in the file). Video begins at the
    /// keyframe before it; those preroll frames are hidden via an MP4 edit list.
    pub origin_us: i64,
    pub end_us: i64,
}

impl ReplayBuffer {
    pub fn new(streams: Vec<StreamDesc>, max_seconds: u32, disk: Option<DiskStore>) -> Self {
        let n = streams.len();
        ReplayBuffer {
            streams,
            queues: (0..n).map(|_| VecDeque::new()).collect(),
            keys: VecDeque::new(),
            bytes: 0,
            max_us: max_seconds as i64 * 1_000_000,
            disk,
            last_saved_end_us: None,
        }
    }

    pub fn set_max_seconds(&mut self, s: u32) {
        self.max_us = s as i64 * 1_000_000;
        self.prune();
    }

    pub fn clear(&mut self) {
        let gone: Vec<PacketRef> = self.queues.iter_mut().flat_map(|q| q.drain(..)).collect();
        self.keys.clear();
        self.bytes = 0;
        if let Some(d) = &self.disk {
            d.release(gone);
            d.reset();
        }
        self.last_saved_end_us = None;
    }

    /// Time of the newest video packet (or any packet for audio-only).
    pub fn end_us(&self) -> Option<i64> {
        match self.video_index() {
            Some(v) => self.queues[v].back().map(|p| p.time_us),
            None => self.queues.iter().filter_map(|q| q.back()).map(|p| p.time_us).max(),
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    fn video_index(&self) -> Option<usize> {
        self.streams.iter().position(|s| s.kind == StreamKind::Video)
    }

    /// Seconds of video currently held.
    pub fn duration_us(&self) -> i64 {
        let Some(v) = self.video_index() else { return 0 };
        let q = &self.queues[v];
        match (q.front(), q.back()) {
            (Some(a), Some(b)) => b.time_us - a.time_us,
            _ => 0,
        }
    }

    pub fn push(&mut self, stream: usize, pkt: PacketRef) {
        let video = Some(stream) == self.video_index();
        // The first video packet kept must be a keyframe.
        if video && self.queues[stream].is_empty() && !pkt.key {
            return;
        }
        // Only queued: the disk writer thread moves the bytes later.
        if let Some(d) = &self.disk {
            d.store(&pkt);
        }
        if video && pkt.key {
            self.keys.push_back(pkt.time_us);
        }
        self.bytes += pkt.data.len();
        self.queues[stream].push_back(pkt);
        self.prune();
    }

    /// Removes the oldest packets of `stream` while `old` holds for them.
    fn drop_front(&mut self, stream: usize, gone: &mut Vec<PacketRef>, old: impl Fn(&Packet) -> bool) {
        let q = &mut self.queues[stream];
        while q.front().is_some_and(|p| old(p)) {
            let p = q.pop_front().unwrap();
            self.bytes -= p.data.len();
            gone.push(p);
        }
    }

    fn prune(&mut self) {
        let mut gone = Vec::new();
        match self.video_index() {
            None => {
                // Audio-only buffer: plain time window.
                let newest = self.queues.iter().filter_map(|q| q.back()).map(|p| p.time_us).max().unwrap_or(0);
                let min = newest - self.max_us;
                for i in 0..self.queues.len() {
                    self.drop_front(i, &mut gone, |p| p.time_us < min);
                }
            }
            Some(v) => {
                let Some(newest) = self.queues[v].back().map(|p| p.time_us) else { return };
                // Drop leading GOPs while the remainder still covers max_us.
                while let Some(&cut) = self.keys.get(1) {
                    if newest - cut < self.max_us {
                        break;
                    }
                    self.keys.pop_front();
                    self.drop_front(v, &mut gone, |p| p.time_us < cut);
                }
                let start = self.queues[v].front().map(|p| p.time_us).unwrap_or(newest);
                for i in 0..self.queues.len() {
                    if i != v {
                        self.drop_front(i, &mut gone, |p| p.time_us < start - 100_000);
                    }
                }
            }
        }
        // Dropping a packet can close (delete) its disk segment: not here,
        // under the buffer lock on a capture thread.
        if let Some(d) = &self.disk {
            d.release(gone);
        }
    }

    /// Selects the `seconds` before `until` (default: newest packet), or only
    /// what came after `since` when that is later. Packets are shared (Arc).
    pub fn snapshot(&self, seconds: u32, since: Option<i64>, until: Option<i64>) -> Option<ClipData> {
        let v = self.video_index();
        let end_us = until.or_else(|| self.end_us())?;
        let mut origin_us = end_us - seconds as i64 * 1_000_000;
        if let Some(s) = since {
            origin_us = origin_us.max(s + 1);
        }
        let start_us = match v {
            Some(_) => {
                let first_key = *self.keys.front()?;
                origin_us = origin_us.max(first_key);
                // The last keyframe at or before the origin (first_key is one).
                self.keys[self.keys.partition_point(|&t| t <= origin_us) - 1]
            }
            None => origin_us,
        };
        let packets = self
            .queues
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let from = if Some(i) == v { start_us } else { origin_us };
                let a = q.partition_point(|p| p.time_us < from);
                let b = q.partition_point(|p| p.time_us <= end_us).max(a);
                q.range(a..b).cloned().collect()
            })
            .collect();
        Some(ClipData { streams: self.streams.clone(), packets, origin_us, end_us })
    }
}
