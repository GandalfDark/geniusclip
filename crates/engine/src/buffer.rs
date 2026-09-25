//! In-memory replay buffer of encoded packets.
//!
//! Stores already-compressed packets (≈ bitrate × duration bytes), trimmed on
//! whole-GOP boundaries so a clip can always start on a keyframe.

use crate::ffutil::{PacketRef, StreamDesc, StreamKind};
use std::collections::VecDeque;

pub struct ReplayBuffer {
    pub streams: Vec<StreamDesc>,
    queues: Vec<VecDeque<PacketRef>>,
    bytes: usize,
    max_us: i64,
}

/// Packets selected for one clip, per stream, in decode order.
pub struct ClipData {
    pub streams: Vec<StreamDesc>,
    pub packets: Vec<Vec<PacketRef>>,
    /// Clip start on the engine timeline (first video keyframe).
    pub start_us: i64,
    pub end_us: i64,
}

impl ReplayBuffer {
    pub fn new(streams: Vec<StreamDesc>, max_seconds: u32) -> Self {
        let n = streams.len();
        ReplayBuffer { streams, queues: (0..n).map(|_| VecDeque::new()).collect(), bytes: 0, max_us: max_seconds as i64 * 1_000_000 }
    }

    pub fn set_max_seconds(&mut self, s: u32) {
        self.max_us = s as i64 * 1_000_000;
        self.prune();
    }

    pub fn clear(&mut self) {
        self.queues.iter_mut().for_each(|q| q.clear());
        self.bytes = 0;
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
        // The first video packet kept must be a keyframe.
        if Some(stream) == self.video_index() && self.queues[stream].is_empty() && !pkt.key {
            return;
        }
        self.bytes += pkt.data.len();
        self.queues[stream].push_back(pkt);
        self.prune();
    }

    fn prune(&mut self) {
        let Some(v) = self.video_index() else {
            // Audio-only buffer: plain time window.
            let newest = self.queues.iter().filter_map(|q| q.back()).map(|p| p.time_us).max().unwrap_or(0);
            for q in &mut self.queues {
                while q.front().is_some_and(|p| p.time_us < newest - self.max_us) {
                    self.bytes -= q.pop_front().unwrap().data.len();
                }
            }
            return;
        };
        let newest = match self.queues[v].back() {
            Some(p) => p.time_us,
            None => return,
        };
        // Drop leading GOPs while the remainder still covers max_us.
        loop {
            let q = &self.queues[v];
            let Some(next_key) = q.iter().skip(1).find(|p| p.key) else { break };
            if newest - next_key.time_us < self.max_us {
                break;
            }
            let cut = next_key.time_us;
            let q = &mut self.queues[v];
            while q.front().is_some_and(|p| p.time_us < cut) {
                self.bytes -= q.pop_front().unwrap().data.len();
            }
        }
        let start = self.queues[v].front().map(|p| p.time_us).unwrap_or(newest);
        for (i, q) in self.queues.iter_mut().enumerate() {
            if i == v {
                continue;
            }
            while q.front().is_some_and(|p| p.time_us < start - 100_000) {
                self.bytes -= q.pop_front().unwrap().data.len();
            }
        }
    }

    /// Selects the last `seconds` (starting at the keyframe at or before
    /// `now - seconds`). Packets are shared (Arc), so this is cheap.
    pub fn snapshot(&self, seconds: u32) -> Option<ClipData> {
        let v = self.video_index();
        let end_us = match v {
            Some(v) => self.queues[v].back()?.time_us,
            None => self.queues.iter().filter_map(|q| q.back()).map(|p| p.time_us).max()?,
        };
        let want = end_us - seconds as i64 * 1_000_000;
        let start_us = match v {
            Some(v) => {
                let q = &self.queues[v];
                q.iter().rev().find(|p| p.key && p.time_us <= want).or_else(|| q.iter().find(|p| p.key))?.time_us
            }
            None => want,
        };
        let packets = self
            .queues
            .iter()
            .enumerate()
            .map(|(i, q)| {
                q.iter()
                    .filter(|p| if Some(i) == v { p.time_us >= start_us } else { p.time_us >= start_us && p.time_us <= end_us })
                    .cloned()
                    .collect()
            })
            .collect();
        Some(ClipData { streams: self.streams.clone(), packets, start_us, end_us })
    }
}
