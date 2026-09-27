//! Turns a finished recording into a regular MP4, in place.
//!
//! Recordings are written as fragmented MP4 — a header without samples, then
//! a `moof` (sample table) + `mdat` (media) pair about every second — so a
//! recording the app never got to finish still plays. The catch: a player
//! has to walk every fragment before it knows the duration or can seek. A
//! four-hour recording has ~15 000 of them, WebView2 fetches each one with a
//! separate request and takes minutes to open it, and video editors cope
//! badly with fragments too.
//!
//! Finishing writes one ordinary `moov` with every sample after the last
//! fragment. Then, without moving any media data, the first `moof` header is
//! overwritten with an `mdat` header that spans all the fragments and the
//! empty fragmented header becomes `free`: the file is `ftyp free mdat moov`.
//! The sample table is read from the fragments themselves: while recording
//! (just written, so from the OS cache), or all at once for a file that was
//! left fragmented.

use anyhow::{ensure, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// Sample flag: not a sync (key) sample.
const NON_SYNC: u32 = 0x0001_0000;
/// Largest `moov` or `moof` read into memory.
const MAX_HEADER: u64 = 256 << 20;

/// The sample table of a fragmented MP4, read as the file grows.
#[derive(Default)]
pub struct Index {
    /// Offset of the next top-level box to read.
    next: u64,
    header: Option<Header>,
    tracks: Vec<Track>,
    first_moof: Option<u64>,
    /// End of the last complete fragment.
    end: u64,
    /// A fragment's `moof` whose `mdat` isn't complete yet.
    pending: Option<(u64, Vec<u8>)>,
    /// An index of an interrupted finish follows the fragments.
    stopped: bool,
    /// Size of the largest fragment, for telling a torn end from damage.
    largest: u64,
}

/// The fragmented file's `moov`: offset and content.
struct Header {
    at: u64,
    moov: Vec<u8>,
    timescale: u32,
}

#[derive(Default)]
struct Track {
    id: u32,
    timescale: u32,
    defaults: Defaults,
    sizes: Vec<u32>,
    /// Sample durations as (count, delta) runs: the `stts` table.
    stts: Vec<(u32, u32)>,
    /// Composition offsets as runs, once one isn't zero: the `ctts` table.
    ctts: Vec<(u32, i32)>,
    /// Sync sample numbers (1-based).
    sync: Vec<u32>,
    chunks: Vec<Chunk>,
    /// Decode time of the first sample and after the last one.
    start: u64,
    time: u64,
}

#[derive(Clone, Copy, Default)]
struct Defaults {
    desc: u32,
    duration: u32,
    size: u32,
    flags: u32,
}

/// A run of samples stored back to back.
struct Chunk {
    offset: u64,
    count: u32,
    desc: u32,
}

impl Track {
    fn push(&mut self, size: u32, duration: u32, cts: i32, sync: bool) {
        let n = self.sizes.len() as u32;
        self.sizes.push(size);
        match self.stts.last_mut() {
            Some((c, d)) if *d == duration => *c += 1,
            _ => self.stts.push((1, duration)),
        }
        match self.ctts.last_mut() {
            Some((c, o)) if *o == cts => *c += 1,
            Some(_) => self.ctts.push((1, cts)),
            // Offsets stay implicit (zero) until one isn't.
            None if cts != 0 => {
                if n > 0 {
                    self.ctts.push((n, 0));
                }
                self.ctts.push((1, cts));
            }
            None => {}
        }
        if sync {
            self.sync.push(n + 1);
        }
        self.time += duration as u64;
    }

    /// A fragment starts at decode time `t`: a gap lengthens the sample
    /// before it, so later samples keep their times.
    fn align(&mut self, t: u64) {
        if self.sizes.is_empty() {
            self.start = t;
            self.time = t;
        } else if t > self.time {
            let gap = u32::try_from(t - self.time).unwrap_or(u32::MAX);
            if let Some(last) = self.stts.last_mut() {
                let d = last.1.saturating_add(gap);
                if last.0 == 1 {
                    last.1 = d;
                } else {
                    last.0 -= 1;
                    self.stts.push((1, d));
                }
            }
            self.time = t;
        }
    }
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    /// Reads the boxes completed since the last call.
    pub fn advance(&mut self, f: &mut File) -> Result<()> {
        let len = f.metadata()?.len();
        while !self.stopped {
            let at = self.next;
            let Some((typ, hdr, size)) = box_header(f, at, len)? else { break };
            let end = at.checked_add(size).context("bad box size")?;
            if end > len {
                break;
            }
            match &typ {
                b"moov" if self.header.is_none() => {
                    ensure!(self.first_moof.is_none(), "header after the fragments");
                    ensure!(size <= MAX_HEADER, "header too large");
                    let body = read_at(f, at + hdr, size - hdr)?;
                    self.read_header(at, body)?;
                }
                // The index of a finish that was cut short: written again.
                b"moov" => {
                    self.stopped = true;
                    break;
                }
                b"moof" => {
                    ensure!(self.header.is_some(), "fragment before the header");
                    ensure!(self.pending.is_none(), "fragment without media");
                    ensure!(size <= MAX_HEADER, "fragment header too large");
                    self.pending = Some((at, read_at(f, at + hdr, size - hdr)?));
                }
                b"mdat" => {
                    let (moof_at, moof) = self.pending.take().context("not a fragmented MP4")?;
                    // Its end never written (zeros after a power cut): the
                    // recording ends before it. Compressed media never
                    // ends in a run of zeros like that.
                    let check = (size - hdr).min(4096);
                    if check > 0 && read_at(f, end - check, check)?.iter().all(|&b| b == 0) {
                        self.stopped = true;
                        break;
                    }
                    self.add_fragment(moof_at, &moof, at + hdr, end)?;
                    self.first_moof.get_or_insert(moof_at);
                    self.largest = self.largest.max(end - moof_at);
                    self.end = end;
                }
                _ => {}
            }
            self.next = end;
        }
        Ok(())
    }

    fn read_header(&mut self, at: u64, moov: Vec<u8>) -> Result<()> {
        let mut timescale = 0;
        let mut tracks = Vec::new();
        let mut trex = Vec::new();
        for b in children(&moov)? {
            match &b.typ {
                b"mvhd" => timescale = header_timescale(b.body)?,
                b"trak" => {
                    let id = tkhd_id(find(b.body, b"tkhd")?.context("track without tkhd")?)?;
                    let mdia = find(b.body, b"mdia")?.context("track without mdia")?;
                    let ts = header_timescale(find(mdia, b"mdhd")?.context("track without mdhd")?)?;
                    let stsz = find_path(mdia, &[b"minf", b"stbl", b"stsz"])?;
                    // Samples in the header: not a recording in progress.
                    if let Some(s) = stsz {
                        let mut c = Cur::new(s);
                        c.skip(8)?;
                        ensure!(c.u32()? == 0, "already indexed");
                    }
                    ensure!(ts > 0, "track without timescale");
                    tracks.push(Track { id, timescale: ts, ..Default::default() });
                }
                b"mvex" => {
                    for t in children(b.body)? {
                        if t.typ == *b"trex" {
                            let mut c = Cur::new(t.body);
                            c.u32()?;
                            let id = c.u32()?;
                            let d = Defaults { desc: c.u32()?, duration: c.u32()?, size: c.u32()?, flags: c.u32()? };
                            trex.push((id, d));
                        }
                    }
                }
                _ => {}
            }
        }
        ensure!(timescale > 0 && !tracks.is_empty(), "no tracks");
        ensure!(!trex.is_empty(), "not a fragmented MP4");
        for t in &mut tracks {
            t.defaults = trex.iter().find(|(id, _)| *id == t.id).map(|(_, d)| *d).unwrap_or_default();
        }
        self.header = Some(Header { at, moov, timescale });
        self.tracks = tracks;
        Ok(())
    }

    /// Adds the samples of the fragment whose `moof` is at `moof_at` and whose
    /// media is `data_start..data_end`.
    fn add_fragment(&mut self, moof_at: u64, moof: &[u8], data_start: u64, data_end: u64) -> Result<()> {
        // Where a track fragment's data starts if its header doesn't say.
        let mut next_base = moof_at;
        for traf in children(moof)? {
            if traf.typ != *b"traf" {
                continue;
            }
            let (mut tfhd, mut tfdt, mut truns) = (None, None, Vec::new());
            for c in children(traf.body)? {
                match &c.typ {
                    b"tfhd" => tfhd = Some(c.body),
                    b"tfdt" => tfdt = Some(c.body),
                    b"trun" => truns.push(c.body),
                    _ => {}
                }
            }
            let mut c = Cur::new(tfhd.context("fragment without tfhd")?);
            let flags = c.u32()? & 0xFF_FFFF;
            let id = c.u32()?;
            let track = self.tracks.iter_mut().find(|t| t.id == id).context("fragment of an unknown track")?;
            let mut d = track.defaults;
            let base_offset = if flags & 0x01 != 0 { Some(c.u64()?) } else { None };
            if flags & 0x02 != 0 {
                d.desc = c.u32()?;
            }
            if flags & 0x08 != 0 {
                d.duration = c.u32()?;
            }
            if flags & 0x10 != 0 {
                d.size = c.u32()?;
            }
            if flags & 0x20 != 0 {
                d.flags = c.u32()?;
            }
            let base = match base_offset {
                Some(b) => b,
                None if flags & 0x2_0000 != 0 => moof_at,
                None => next_base,
            };
            if let Some(b) = tfdt {
                let mut c = Cur::new(b);
                let v = c.u32()? >> 24;
                track.align(if v == 1 { c.u64()? } else { c.u32()? as u64 });
            }
            let mut pos = base;
            for trun in truns {
                let mut c = Cur::new(trun);
                let tf = c.u32()? & 0xFF_FFFF;
                let count = c.u32()?;
                // Without an offset, a run follows the one before it.
                if tf & 0x01 != 0 {
                    pos = base.checked_add_signed(c.u32()? as i32 as i64).context("bad data offset")?;
                }
                let first_flags = if tf & 0x04 != 0 { Some(c.u32()?) } else { None };
                // Every sample reads its fields from the box or takes up
                // media bytes: a damaged count can't run on for billions.
                ensure!(tf & 0xF00 != 0 || d.size > 0, "fragment of empty samples");
                let start = pos;
                for i in 0..count {
                    let duration = if tf & 0x100 != 0 { c.u32()? } else { d.duration };
                    let size = if tf & 0x200 != 0 { c.u32()? } else { d.size };
                    let sflags = if tf & 0x400 != 0 {
                        c.u32()?
                    } else {
                        first_flags.filter(|_| i == 0).unwrap_or(d.flags)
                    };
                    // Unsigned in version 0, but never that large.
                    let cts = if tf & 0x800 != 0 { c.u32()? as i32 } else { 0 };
                    pos += size as u64;
                    ensure!(start >= data_start && pos <= data_end, "sample data outside its fragment");
                    track.push(size, duration, cts, sflags & NON_SYNC == 0);
                }
                if count > 0 {
                    track.chunks.push(Chunk { offset: start, count, desc: d.desc.max(1) });
                }
            }
            next_base = pos;
        }
        Ok(())
    }

    /// Writes the full index after the last complete fragment and makes the
    /// file a regular MP4. Anything after that fragment (the fragmented
    /// trailer, or the torn end of a recording that was cut off) is dropped.
    pub fn finish(self, f: &mut File) -> Result<()> {
        let header = self.header.as_ref().context("no header")?;
        let first = self.first_moof.context("nothing recorded")?;
        let at = self.end;
        // What follows the last fragment read is dropped: the fragmented
        // trailer, a fragment cut off mid-write, zeros left by a power cut.
        // Much more than that is damage with footage after it: kept.
        let tail = f.metadata()?.len() - at;
        ensure!(tail <= (64 << 20).max(4 * self.largest), "{} MB of unreadable data after the last fragment", tail >> 20);
        let moov = self.build(header)?;
        f.seek(SeekFrom::Start(at))?;
        if let Err(e) = f.write_all(&moov) {
            // Out of space: no half an index after the fragments.
            let _ = f.set_len(at);
            return Err(e.into());
        }
        f.set_len(at + moov.len() as u64)?;
        // The index is on disk before the fragments are hidden behind one
        // `mdat`: cut off before this point, the file is still fragmented
        // (and a later finish writes the index again).
        f.sync_data()?;
        let mut mdat = [0u8; 16];
        mdat[..4].copy_from_slice(&1u32.to_be_bytes());
        mdat[4..8].copy_from_slice(b"mdat");
        mdat[8..].copy_from_slice(&(at - first).to_be_bytes());
        write_at(f, first, &mdat)?;
        // On disk before the header goes: `free` without the `mdat` would
        // leave fragments no player reads.
        f.sync_data()?;
        write_at(f, header.at + 4, b"free")?;
        f.sync_data()?;
        Ok(())
    }

    fn build(&self, h: &Header) -> Result<Vec<u8>> {
        for t in &self.tracks {
            let chunked: u64 = t.chunks.iter().map(|c| c.count as u64).sum();
            ensure!(chunked == t.sizes.len() as u64, "track {}: samples and chunks disagree", t.id);
        }
        let movie = |t: &Track| rescale(t.time, t.timescale, h.timescale);
        let duration = self.tracks.iter().map(movie).max().unwrap_or(0);
        let mut out = Vec::new();
        for b in children(&h.moov)? {
            match &b.typ {
                b"mvhd" => out.extend(boxed(b"mvhd", &with_duration(b.body, 4, duration)?)),
                b"trak" => out.extend(self.trak(b.body, h.timescale)?),
                b"mvex" => {}
                _ => out.extend_from_slice(b.whole),
            }
        }
        Ok(boxed(b"moov", &out))
    }

    fn trak(&self, body: &[u8], movie_ts: u32) -> Result<Vec<u8>> {
        let id = tkhd_id(find(body, b"tkhd")?.context("track without tkhd")?)?;
        let t = self.tracks.iter().find(|t| t.id == id).context("unknown track")?;
        let lead = rescale(t.start, t.timescale, movie_ts);
        let media = rescale(t.time - t.start, t.timescale, movie_ts);
        let late = lead > 0;
        let mut out = Vec::new();
        for c in children(body)? {
            match &c.typ {
                b"tkhd" => out.extend(boxed(b"tkhd", &with_duration(c.body, 8, lead + media)?)),
                // A track that starts late gets its own edit list below.
                b"edts" if late => {}
                b"mdia" => {
                    if late {
                        out.extend(late_start(lead, media));
                    }
                    out.extend(boxed(b"mdia", &self.mdia(c.body, t)?));
                }
                _ => out.extend_from_slice(c.whole),
            }
        }
        Ok(boxed(b"trak", &out))
    }

    fn mdia(&self, body: &[u8], t: &Track) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for c in children(body)? {
            match &c.typ {
                b"mdhd" => out.extend(boxed(b"mdhd", &with_duration(c.body, 4, t.time - t.start)?)),
                b"minf" => {
                    let mut minf = Vec::new();
                    for m in children(c.body)? {
                        if m.typ == *b"stbl" {
                            minf.extend(boxed(b"stbl", &stbl(m.body, t)?));
                        } else {
                            minf.extend_from_slice(m.whole);
                        }
                    }
                    out.extend(boxed(b"minf", &minf));
                }
                _ => out.extend_from_slice(c.whole),
            }
        }
        Ok(out)
    }
}

/// The sample table: the fragmented header's sample descriptions and the
/// tables built from the fragments.
fn stbl(body: &[u8], t: &Track) -> Result<Vec<u8>> {
    const TABLES: [&[u8; 4]; 9] = [b"stts", b"ctts", b"cslg", b"stss", b"stsz", b"stz2", b"stsc", b"stco", b"co64"];
    let mut out = Vec::new();
    for c in children(body)? {
        if !TABLES.contains(&&c.typ) {
            out.extend_from_slice(c.whole);
        }
    }
    let n = t.sizes.len() as u32;

    let mut b = be(t.stts.len() as u32);
    for &(c, d) in &t.stts {
        b.extend(be(c));
        b.extend(be(d));
    }
    out.extend(full(b"stts", 0, &b));

    if !t.ctts.is_empty() {
        let mut runs = t.ctts.clone();
        let covered: u32 = runs.iter().map(|r| r.0).sum();
        if covered < n {
            runs.push((n - covered, 0));
        }
        let mut b = be(runs.len() as u32);
        for &(c, o) in &runs {
            b.extend(be(c));
            b.extend(be(o as u32));
        }
        out.extend(full(b"ctts", u8::from(runs.iter().any(|r| r.1 < 0)), &b));
    }

    if (t.sync.len() as u32) < n {
        let mut b = be(t.sync.len() as u32);
        for &s in &t.sync {
            b.extend(be(s));
        }
        out.extend(full(b"stss", 0, &b));
    }

    let mut b = Vec::new();
    for (i, c) in t.chunks.iter().enumerate() {
        let same = t.chunks[..i].last().is_some_and(|p| p.count == c.count && p.desc == c.desc);
        if !same {
            b.extend(be(i as u32 + 1));
            b.extend(be(c.count));
            b.extend(be(c.desc));
        }
    }
    out.extend(full(b"stsc", 0, &[be((b.len() / 12) as u32), b].concat()));

    let uniform = t.sizes.first().filter(|&&s| t.sizes.iter().all(|&x| x == s));
    let mut b = be(uniform.copied().unwrap_or(0));
    b.extend(be(n));
    if uniform.is_none() {
        b.reserve(t.sizes.len() * 4);
        for &s in &t.sizes {
            b.extend(be(s));
        }
    }
    out.extend(full(b"stsz", 0, &b));

    let wide = t.chunks.iter().any(|c| c.offset > u32::MAX as u64);
    let mut b = be(t.chunks.len() as u32);
    for c in &t.chunks {
        if wide {
            b.extend(c.offset.to_be_bytes());
        } else {
            b.extend(be(c.offset as u32));
        }
    }
    out.extend(full(if wide { b"co64" } else { b"stco" }, 0, &b));
    Ok(out)
}

/// An edit list that starts the track `lead` (movie timescale) late.
fn late_start(lead: u64, media: u64) -> Vec<u8> {
    let mut e = be(2);
    for (duration, time) in [(lead, -1i64), (media, 0)] {
        e.extend(duration.to_be_bytes());
        e.extend(time.to_be_bytes());
        e.extend([0, 1, 0, 0]);
    }
    boxed(b"edts", &full(b"elst", 1, &e))
}

/// What `finalize_file` found.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It was fragmented and is a regular MP4 now.
    Finished,
    /// It already was a regular MP4.
    Regular,
    /// Not a single complete fragment: nothing was recorded.
    Empty,
}

/// Finishes a recording that was left fragmented: the app was killed during
/// it, or an older version wrote it. Opened for reading and writing.
pub fn finalize_file(f: &mut File) -> Result<Outcome> {
    if finish_interrupted(f)? {
        return Ok(Outcome::Regular);
    }
    let mut index = Index::new();
    index.advance(f)?;
    if index.first_moof.is_none() {
        return Ok(Outcome::Empty);
    }
    index.finish(f)?;
    Ok(Outcome::Finished)
}

/// Whether the file is a recording that isn't finished: its first `moov`
/// is a fragmented header (with `mvex`). Reads only the first few boxes.
pub fn needs_finish(f: &mut File) -> Result<bool> {
    let len = f.metadata()?.len();
    let mut at = 0;
    for _ in 0..16 {
        let Some((typ, hdr, size)) = box_header(f, at, len)? else { return Ok(false) };
        if typ == *b"moov" {
            ensure!(size <= MAX_HEADER && at + size <= len, "bad header");
            return Ok(find(&read_at(f, at + hdr, size - hdr)?, b"mvex")?.is_some());
        }
        at = at.checked_add(size).context("bad box size")?;
    }
    Ok(false)
}

/// True if the file has no fragments (already regular). A finish that was
/// cut off after hiding the fragments only lacks the last step then: the
/// fragmented header is still `moov`, which is done here.
fn finish_interrupted(f: &mut File) -> Result<bool> {
    let len = f.metadata()?.len();
    let mut at = 0;
    let mut moovs = Vec::new();
    for _ in 0..64 {
        let Some((typ, hdr, size)) = box_header(f, at, len)? else { break };
        match &typ {
            b"moof" => return Ok(false),
            b"moov" => moovs.push((at, hdr, size)),
            _ => {}
        }
        at = at.checked_add(size).context("bad box size")?;
        if at >= len {
            break;
        }
    }
    // Unreadable boxes (zeros after a power cut): left to the scan.
    if at != len || moovs.is_empty() {
        return Ok(false);
    }
    // Only the header, nothing after it: left to the scan (nothing recorded).
    let &(pos, hdr, size) = moovs.last().unwrap();
    ensure!(size <= MAX_HEADER, "header too large");
    if find(&read_at(f, pos + hdr, size - hdr)?, b"mvex")?.is_some() {
        return Ok(false);
    }
    // The fragmented header (with `mvex`) before the full index.
    for &(pos, hdr, size) in &moovs[..moovs.len() - 1] {
        ensure!(size <= MAX_HEADER, "header too large");
        if find(&read_at(f, pos + hdr, size - hdr)?, b"mvex")?.is_some() {
            write_at(f, pos + 4, b"free")?;
            f.sync_data()?;
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Boxes

struct Child<'a> {
    typ: [u8; 4],
    body: &'a [u8],
    whole: &'a [u8],
}

fn children(data: &[u8]) -> Result<Vec<Child<'_>>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < data.len() {
        let rest = &data[p..];
        ensure!(rest.len() >= 8, "truncated box");
        let typ: [u8; 4] = rest[4..8].try_into().unwrap();
        let (size, hdr) = match u32::from_be_bytes(rest[..4].try_into().unwrap()) {
            0 => (rest.len() as u64, 8),
            1 => {
                ensure!(rest.len() >= 16, "truncated box");
                (u64::from_be_bytes(rest[8..16].try_into().unwrap()), 16)
            }
            n => (n as u64, 8),
        };
        ensure!(size >= hdr as u64 && size <= rest.len() as u64, "bad size of box {}", String::from_utf8_lossy(&typ));
        let size = size as usize;
        out.push(Child { typ, body: &rest[hdr..size], whole: &rest[..size] });
        p += size;
    }
    Ok(out)
}

fn find<'a>(data: &'a [u8], typ: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    Ok(children(data)?.into_iter().find(|c| c.typ == *typ).map(|c| c.body))
}

fn find_path<'a>(mut data: &'a [u8], path: &[&[u8; 4]]) -> Result<Option<&'a [u8]>> {
    for typ in path {
        match find(data, typ)? {
            Some(d) => data = d,
            None => return Ok(None),
        }
    }
    Ok(Some(data))
}

/// Type, header length and size of the top-level box at `at`, if its header
/// is in the file.
fn box_header(f: &mut File, at: u64, len: u64) -> Result<Option<([u8; 4], u64, u64)>> {
    if len < at.saturating_add(8) {
        return Ok(None);
    }
    let mut h = [0u8; 16];
    let n = if len >= at.saturating_add(16) { 16 } else { 8 };
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(&mut h[..n])?;
    let typ: [u8; 4] = h[4..8].try_into().unwrap();
    let (size, hdr) = match u32::from_be_bytes(h[..4].try_into().unwrap()) {
        // Zeros where a box should start: a power cut's unwritten end.
        0 => return Ok(None),
        1 if n < 16 => return Ok(None),
        1 => (u64::from_be_bytes(h[8..16].try_into().unwrap()), 16),
        s => (s as u64, 8),
    };
    ensure!(size >= hdr, "bad box size at {at}");
    Ok(Some((typ, hdr, size)))
}

fn read_at(f: &mut File, at: u64, len: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0; usize::try_from(len)?];
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn write_at(f: &mut File, at: u64, data: &[u8]) -> Result<()> {
    f.seek(SeekFrom::Start(at))?;
    f.write_all(data)?;
    Ok(())
}

fn be(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn boxed(typ: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 16);
    if let Ok(size) = u32::try_from(content.len() + 8) {
        out.extend(size.to_be_bytes());
        out.extend(typ);
    } else {
        out.extend(1u32.to_be_bytes());
        out.extend(typ);
        out.extend((content.len() as u64 + 16).to_be_bytes());
    }
    out.extend(content);
    out
}

fn full(typ: &[u8; 4], version: u8, content: &[u8]) -> Vec<u8> {
    let mut body = vec![version, 0, 0, 0];
    body.extend(content);
    boxed(typ, &body)
}

/// `mvhd`/`mdhd`: version, creation and modification time, timescale, duration.
fn header_timescale(body: &[u8]) -> Result<u32> {
    let mut c = Cur::new(body);
    let v = c.u32()? >> 24;
    c.skip(if v == 1 { 16 } else { 8 })?;
    c.u32()
}

/// `tkhd` has the track id where `mvhd` has the timescale.
fn tkhd_id(body: &[u8]) -> Result<u32> {
    header_timescale(body)
}

/// `mvhd`, `mdhd` or `tkhd` with its duration set; `mid` is the size of the
/// fields between the times and the duration. A duration too long for
/// version 0 turns the box into version 1.
fn with_duration(body: &[u8], mid: usize, duration: u64) -> Result<Vec<u8>> {
    let v = body.first().copied().context("empty box")?;
    let (times, dur) = if v == 1 { (16, 8) } else { (8, 4) };
    let at = 4 + times + mid;
    ensure!(body.len() >= at + dur, "short box");
    let mut out = Vec::with_capacity(body.len() + 12);
    if v == 1 {
        out.extend_from_slice(&body[..at]);
        out.extend(duration.to_be_bytes());
    } else if let Ok(d) = u32::try_from(duration) {
        out.extend_from_slice(&body[..at]);
        out.extend(d.to_be_bytes());
    } else {
        out.push(1);
        out.extend_from_slice(&body[1..4]);
        for i in 0..2 {
            out.extend((u32::from_be_bytes(body[4 + i * 4..8 + i * 4].try_into().unwrap()) as u64).to_be_bytes());
        }
        out.extend_from_slice(&body[12..12 + mid]);
        out.extend(duration.to_be_bytes());
    }
    out.extend_from_slice(&body[at + dur..]);
    Ok(out)
}

fn rescale(v: u64, from: u32, to: u32) -> u64 {
    ((v as u128 * to as u128 + from as u128 / 2) / from.max(1) as u128) as u64
}

struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn new(d: &'a [u8]) -> Self {
        Cur { d, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(self.d.len() - self.p >= n, "truncated box");
        let s = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
}
