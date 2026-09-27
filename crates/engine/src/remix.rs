//! Audio track tools for saved clips: list tracks, extract them for preview,
//! and trim with per-track volume (video is copied, audio re-encoded).

use crate::audio::{AudioEncoder, BLOCK, RATE};
use crate::ffutil::*;
use crate::media::{dict_get, Input};
use crate::mux::Muxer;
use anyhow::{bail, Result};
use ffmpeg_sys_next as ff;
use std::collections::VecDeque;
use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::ptr;

/// Titles of the audio tracks, in file order.
pub fn audio_tracks(path: &Path) -> Result<Vec<String>> {
    let input = Input::open(path)?;
    let mut out = Vec::new();
    unsafe {
        for &st in input.streams() {
            if (*(*st).codecpar).codec_type == ff::AVMediaType::AVMEDIA_TYPE_AUDIO {
                out.push(dict_get((*st).metadata, "handler_name").unwrap_or_default());
            }
        }
    }
    Ok(out)
}

/// GeniusClip's own layout: a mixed track followed by game and mic tracks.
/// For it, the mix is rebuilt from the separate tracks.
pub fn is_mix_layout(titles: &[String]) -> bool {
    titles.len() == 3 && titles[1] == "Game" && titles[2] == "Mic"
}

/// Copies each audio track into its own .m4a (no re-encode) for preview.
pub fn extract_tracks(path: &Path, out_dir: &Path, stem: &str) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let input = Input::open(path)?;
    unsafe {
        let mut muxers: Vec<(usize, Muxer, PathBuf)> = Vec::new();
        for (i, &st) in input.streams().iter().enumerate() {
            let par = (*st).codecpar;
            if (*par).codec_type != ff::AVMediaType::AVMEDIA_TYPE_AUDIO {
                continue;
            }
            let dst = out_dir.join(format!("{stem}_{}.m4a", muxers.len()));
            let desc = StreamDesc { kind: StreamKind::Audio, params: CodecParams::from_params(par)?, time_base: (*st).time_base, title: String::new() };
            muxers.push((i, Muxer::create(&dst, &[desc], false, "")?, dst));
        }
        let pkt = AvPacket::new();
        while ff::av_read_frame(input.0, pkt.0) >= 0 {
            let si = (*pkt.0).stream_index as usize;
            if let Some((_, m, _)) = muxers.iter_mut().find(|(i, _, _)| *i == si) {
                let tb = (*input.streams()[si]).time_base;
                let p = Packet::from_av(pkt.0, tb);
                m.write(0, &p, 0)?;
            }
            ff::av_packet_unref(pkt.0);
        }
        let mut paths = Vec::new();
        for (_, m, dst) in muxers {
            m.finish()?;
            paths.push(dst);
        }
        Ok(paths)
    }
}

/// Decoded samples of one input track on the 48 kHz timeline.
struct Source {
    stream: usize,
    dec: CodecCtx,
    tb: ff::AVRational,
    /// Sample index of `buf[0]`, once known.
    first: Option<i64>,
    /// Interleaved stereo.
    buf: VecDeque<f32>,
    eof: bool,
}

impl Source {
    fn end(&self) -> Option<i64> {
        self.first.map(|f| f + (self.buf.len() / 2) as i64)
    }

    unsafe fn push_frame(&mut self, f: *const ff::AVFrame) {
        let f = &*f;
        let n = f.nb_samples as usize;
        let ch = f.ch_layout.nb_channels.max(1) as usize;
        let idx = if f.pts == ff::AV_NOPTS_VALUE { self.end().unwrap_or(0) } else { rescale(f.pts, self.tb, q(1, RATE as i32)) };
        match self.end() {
            None => self.first = Some(idx),
            // Fill small gaps with silence; overlaps are trimmed.
            Some(e) if idx > e => self.buf.extend(std::iter::repeat(0.0).take(((idx - e) as usize).min(RATE as usize) * 2)),
            _ => {}
        }
        let l = std::slice::from_raw_parts(f.data[0] as *const f32, n);
        let r = if ch > 1 { std::slice::from_raw_parts(f.data[1] as *const f32, n) } else { l };
        for i in 0..n {
            self.buf.push_back(l[i]);
            self.buf.push_back(r[i]);
        }
    }

    /// Takes BLOCK stereo samples starting at `pos` (silence where missing).
    fn take(&mut self, pos: i64, out: &mut [f32]) {
        out.fill(0.0);
        let Some(first) = self.first else { return };
        // Drop anything before pos.
        if first < pos {
            let drop = (((pos - first) * 2) as usize).min(self.buf.len());
            self.buf.drain(..drop);
            self.first = Some(first + (drop / 2) as i64);
        }
        let first = self.first.unwrap();
        let offset = ((first - pos).max(0) * 2) as usize;
        let avail = self.buf.len().min(out.len().saturating_sub(offset));
        for i in 0..avail {
            out[offset + i] = self.buf[i];
        }
        let consumed = (pos + BLOCK as i64 - first).max(0) as usize * 2;
        let consumed = consumed.min(self.buf.len());
        self.buf.drain(..consumed);
        self.first = Some(first + (consumed / 2) as i64);
    }

    /// True when samples up to `until` are available (or the input ended).
    fn ready(&self, until: i64) -> bool {
        self.eof || self.end().is_some_and(|e| e >= until)
    }
}

/// Lossless video trim with re-encoded audio at the given per-track gains
/// (1.0 = unchanged, 0.0 = muted). With GeniusClip's mix layout, the mix
/// track is rebuilt as game × gains[1] + mic × gains[2].
pub fn trim_with_gains(input_path: &Path, output: &Path, start: f64, end: f64, gains: &[f32]) -> Result<()> {
    if end <= start {
        bail!("empty selection");
    }
    let input = Input::open(input_path)?;
    let start_us = (start * 1e6) as i64;
    let end_us = (end * 1e6) as i64;
    let start_s = (start * RATE as f64).round() as i64;
    let end_s = (end * RATE as f64).round() as i64;
    unsafe {
        let mut video: Option<(usize, StreamDesc)> = None;
        let mut sources: Vec<Source> = Vec::new();
        let mut titles = Vec::new();
        let mut bitrates = Vec::new();
        for (i, &st) in input.streams().iter().enumerate() {
            let par = (*st).codecpar;
            match (*par).codec_type {
                ff::AVMediaType::AVMEDIA_TYPE_VIDEO if video.is_none() => {
                    video = Some((i, StreamDesc { kind: StreamKind::Video, params: CodecParams::from_params(par)?, time_base: (*st).time_base, title: String::new() }));
                }
                ff::AVMediaType::AVMEDIA_TYPE_AUDIO => {
                    let codec = ff::avcodec_find_decoder((*par).codec_id);
                    if codec.is_null() {
                        bail!("no audio decoder");
                    }
                    let dec = CodecCtx(ff::avcodec_alloc_context3(codec));
                    check(ff::avcodec_parameters_to_context(dec.0, par), "parameters_to_context")?;
                    (*dec.0).pkt_timebase = (*st).time_base;
                    check(ff::avcodec_open2(dec.0, codec, ptr::null_mut()), "open audio decoder")?;
                    titles.push(dict_get((*st).metadata, "handler_name").unwrap_or_default());
                    bitrates.push(if (*par).bit_rate > 0 { (*par).bit_rate } else { 160_000 });
                    sources.push(Source { stream: i, dec, tb: (*st).time_base, first: None, buf: VecDeque::new(), eof: false });
                }
                _ => {}
            }
        }
        let Some((vidx, vdesc)) = video else { bail!("no video stream") };
        let remix = is_mix_layout(&titles);
        let gain = |k: usize| gains.get(k).copied().unwrap_or(1.0).clamp(0.0, 4.0);

        let mut encoders = Vec::new();
        let mut descs = vec![vdesc];
        for (k, t) in titles.iter().enumerate() {
            // A one-off export: the default coder's quality is worth its time.
            let enc = AudioEncoder::new(bitrates[k].clamp(96_000, 256_000), false)?;
            descs.push(StreamDesc { kind: StreamKind::Audio, params: enc.params.clone(), time_base: enc.time_base, title: t.clone() });
            encoders.push(enc);
        }
        let comment = dict_get((*input.0).metadata, "comment").unwrap_or_default();
        let mut mux = Muxer::create(output, &descs, false, &comment)?;

        ff::av_seek_frame(input.0, -1, start_us * ff::AV_TIME_BASE as i64 / 1_000_000, ff::AVSEEK_FLAG_BACKWARD as c_int);
        let pkt = AvPacket::new();
        let frame = AvFrame::new();
        let mut seen_key = false;
        let mut video_done = false;
        let mut pos = start_s;
        let n = sources.len();
        let mut blocks = vec![vec![0f32; BLOCK * 2]; n];
        let mut outs = vec![vec![0f32; BLOCK * 2]; n];

        // Mixes and encodes every block whose input samples are all available.
        let mut produce = |sources: &mut Vec<Source>, mux: &mut Muxer, encoders: &mut Vec<AudioEncoder>, pos: &mut i64| -> Result<()> {
            while *pos < end_s && sources.iter().all(|s| s.ready(*pos + BLOCK as i64)) {
                for (k, s) in sources.iter_mut().enumerate() {
                    s.take(*pos, &mut blocks[k]);
                }
                for k in 0..n {
                    let (src, g) = if remix && k == 0 { (None, 0.0) } else { (Some(k), gain(k)) };
                    if let Some(src) = src {
                        for i in 0..BLOCK * 2 {
                            outs[k][i] = (blocks[src][i] * g).clamp(-1.0, 1.0);
                        }
                    }
                }
                if remix {
                    let (g1, g2) = (gain(1), gain(2));
                    for i in 0..BLOCK * 2 {
                        outs[0][i] = (blocks[1][i] * g1 + blocks[2][i] * g2).clamp(-1.0, 1.0);
                    }
                }
                for (k, enc) in encoders.iter_mut().enumerate() {
                    let mut res = Ok(());
                    enc.encode(&outs[k], *pos - start_s, &mut |p| {
                        if res.is_ok() {
                            res = mux.write(1 + k, &p, 0);
                        }
                    })?;
                    res?;
                }
                *pos += BLOCK as i64;
            }
            Ok(())
        };

        while ff::av_read_frame(input.0, pkt.0) >= 0 {
            let si = (*pkt.0).stream_index as usize;
            if si == vidx {
                let p = Packet::from_av(pkt.0, descs[0].time_base);
                ff::av_packet_unref(pkt.0);
                if p.time_us > end_us {
                    video_done = true;
                } else if seen_key || p.key {
                    seen_key = true;
                    mux.write(0, &p, start_us)?;
                }
            } else if let Some(s) = sources.iter_mut().find(|s| s.stream == si) {
                if ff::avcodec_send_packet(s.dec.0, pkt.0) >= 0 {
                    while ff::avcodec_receive_frame(s.dec.0, frame.0) >= 0 {
                        s.push_frame(frame.0);
                        ff::av_frame_unref(frame.0);
                    }
                }
                ff::av_packet_unref(pkt.0);
            } else {
                ff::av_packet_unref(pkt.0);
            }
            produce(&mut sources, &mut mux, &mut encoders, &mut pos)?;
            if video_done && pos >= end_s {
                break;
            }
        }
        // Drain decoders, then finish the audio up to the end with silence.
        for s in sources.iter_mut() {
            ff::avcodec_send_packet(s.dec.0, ptr::null());
            while ff::avcodec_receive_frame(s.dec.0, frame.0) >= 0 {
                s.push_frame(frame.0);
                ff::av_frame_unref(frame.0);
            }
            s.eof = true;
        }
        produce(&mut sources, &mut mux, &mut encoders, &mut pos)?;
        for (k, enc) in encoders.iter_mut().enumerate() {
            let mut res = Ok(());
            enc.flush(&mut |p| {
                if res.is_ok() {
                    res = mux.write(1 + k, &p, 0);
                }
            });
            res?;
        }
        mux.finish()?;
    }
    Ok(())
}

/// Waveform of every audio track: the peak level (0..1) in each of
/// `buckets` equal slices of the file. Tracks listed in `skip` are left empty.
pub fn peaks(path: &Path, buckets: usize, skip: &[usize]) -> Result<Vec<Vec<f32>>> {
    let input = Input::open(path)?;
    unsafe {
        let duration = (*input.0).duration as f64 / ff::AV_TIME_BASE as f64;
        if !(duration > 0.0) || buckets == 0 {
            bail!("unknown duration");
        }
        let per_bucket = duration * RATE as f64 / buckets as f64;
        // (stream, decoder, time base, output slot)
        let mut decs: Vec<(usize, CodecCtx, ff::AVRational, usize)> = Vec::new();
        let mut out = Vec::new();
        for (i, &st) in input.streams().iter().enumerate() {
            let par = (*st).codecpar;
            if (*par).codec_type != ff::AVMediaType::AVMEDIA_TYPE_AUDIO {
                continue;
            }
            let k = out.len();
            out.push(Vec::new());
            if skip.contains(&k) {
                continue;
            }
            let codec = ff::avcodec_find_decoder((*par).codec_id);
            if codec.is_null() {
                continue;
            }
            let dec = CodecCtx(ff::avcodec_alloc_context3(codec));
            check(ff::avcodec_parameters_to_context(dec.0, par), "parameters_to_context")?;
            (*dec.0).pkt_timebase = (*st).time_base;
            check(ff::avcodec_open2(dec.0, codec, ptr::null_mut()), "open audio decoder")?;
            out[k] = vec![0f32; buckets];
            decs.push((i, dec, (*st).time_base, k));
        }
        let pkt = AvPacket::new();
        let frame = AvFrame::new();
        let mut next = vec![0i64; out.len()];
        let take = |f: *const ff::AVFrame, tb: ff::AVRational, slot: &mut Vec<f32>, next: &mut i64| {
            let f = &*f;
            if f.format != ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int {
                return;
            }
            let n = f.nb_samples as usize;
            let idx = if f.pts == ff::AV_NOPTS_VALUE { *next } else { rescale(f.pts, tb, q(1, RATE as i32)) };
            *next = idx + n as i64;
            for c in 0..(f.ch_layout.nb_channels.clamp(1, 2) as usize) {
                let s = std::slice::from_raw_parts(f.data[c] as *const f32, n);
                for (j, v) in s.iter().enumerate() {
                    let b = ((idx + j as i64).max(0) as f64 / per_bucket) as usize;
                    if let Some(p) = slot.get_mut(b) {
                        *p = p.max(v.abs());
                    }
                }
            }
        };
        while ff::av_read_frame(input.0, pkt.0) >= 0 {
            let si = (*pkt.0).stream_index as usize;
            if let Some((_, dec, tb, k)) = decs.iter().find(|d| d.0 == si) {
                if ff::avcodec_send_packet(dec.0, pkt.0) >= 0 {
                    while ff::avcodec_receive_frame(dec.0, frame.0) >= 0 {
                        take(frame.0, *tb, &mut out[*k], &mut next[*k]);
                        ff::av_frame_unref(frame.0);
                    }
                }
            }
            ff::av_packet_unref(pkt.0);
        }
        for (_, dec, tb, k) in &decs {
            ff::avcodec_send_packet(dec.0, ptr::null());
            while ff::avcodec_receive_frame(dec.0, frame.0) >= 0 {
                take(frame.0, *tb, &mut out[*k], &mut next[*k]);
                ff::av_frame_unref(frame.0);
            }
        }
        for p in out.iter_mut().flat_map(|t| t.iter_mut()) {
            *p = (p.min(1.0) * 1000.0).round() / 1000.0;
        }
        Ok(out)
    }
}
