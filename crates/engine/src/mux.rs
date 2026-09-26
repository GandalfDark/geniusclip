//! MP4 writing: replay clips (from a buffer snapshot) and live recordings.

use crate::buffer::ClipData;
use crate::ffutil::*;
use anyhow::{bail, Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use ffmpeg_sys_next as ff;
use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread::JoinHandle;

const fn mktag(a: u8, b: u8, c: u8, d: u8) -> u32 {
    a as u32 | (b as u32) << 8 | (c as u32) << 16 | (d as u32) << 24
}

pub struct Muxer {
    oc: *mut ff::AVFormatContext,
    src_tb: Vec<ff::AVRational>,
    tmp: PathBuf,
    dst: PathBuf,
    header_written: bool,
    pkt: AvPacket,
}

unsafe impl Send for Muxer {}

impl Muxer {
    /// Opens `<dst>.part` for writing; `finish` renames it into place.
    pub fn create(dst: &Path, streams: &[StreamDesc], fragmented: bool, comment: &str) -> Result<Self> {
        let tmp = dst.with_extension("mp4.part");
        unsafe {
            let mut oc = ptr::null_mut();
            let fname = path_cstr(&tmp);
            check(
                ff::avformat_alloc_output_context2(&mut oc, ptr::null(), cstr("mp4").as_ptr(), fname.as_ptr()),
                "avformat_alloc_output_context2",
            )?;
            let mut me = Muxer { oc, src_tb: Vec::new(), tmp: tmp.clone(), dst: dst.to_path_buf(), header_written: false, pkt: AvPacket::new() };
            let mut first_audio = true;
            for s in streams {
                let st = ff::avformat_new_stream(oc, ptr::null());
                if st.is_null() {
                    bail!("avformat_new_stream");
                }
                check(ff::avcodec_parameters_copy((*st).codecpar, s.params.as_ptr()), "avcodec_parameters_copy")?;
                let par = &mut *(*st).codecpar;
                par.codec_tag = if par.codec_id == ff::AVCodecID::AV_CODEC_ID_HEVC { mktag(b'h', b'v', b'c', b'1') } else { 0 };
                (*st).time_base = s.time_base;
                if s.kind == StreamKind::Audio {
                    ff::av_dict_set(&mut (*st).metadata, cstr("handler_name").as_ptr(), cstr(&s.title).as_ptr(), 0);
                    (*st).disposition = if first_audio { ff::AV_DISPOSITION_DEFAULT as c_int } else { 0 };
                    first_audio = false;
                }
                me.src_tb.push(s.time_base);
            }
            ff::av_dict_set(&mut (*oc).metadata, cstr("comment").as_ptr(), cstr(comment).as_ptr(), 0);

            check(ff::avio_open(&mut (*oc).pb, fname.as_ptr(), ff::AVIO_FLAG_WRITE as c_int), "avio_open")
                .with_context(|| format!("cannot write {}", tmp.display()))?;
            let mut opts = ptr::null_mut();
            let flags = if fragmented { "+frag_keyframe+empty_moov+default_base_moof" } else { "+faststart" };
            ff::av_dict_set(&mut opts, cstr("movflags").as_ptr(), cstr(flags).as_ptr(), 0);
            let r = ff::avformat_write_header(oc, &mut opts);
            ff::av_dict_free(&mut opts);
            check(r, "avformat_write_header")?;
            me.header_written = true;
            Ok(me)
        }
    }

    /// Writes a packet, shifting its timestamps so `origin_us` becomes t=0.
    pub fn write(&mut self, stream: usize, p: &Packet, origin_us: i64) -> Result<()> {
        unsafe {
            let tb = self.src_tb[stream];
            let off = rescale(origin_us, US, tb);
            let pkt = self.pkt.0;
            check(ff::av_new_packet(pkt, p.data.len() as c_int), "av_new_packet")?;
            if let Err(e) = p.data.read_into(std::slice::from_raw_parts_mut((*pkt).data, p.data.len())) {
                ff::av_packet_unref(pkt);
                return Err(anyhow::Error::new(e).context("read buffered packet"));
            }
            (*pkt).pts = p.pts - off;
            (*pkt).dts = p.dts - off;
            (*pkt).duration = p.duration;
            (*pkt).flags = if p.key { ff::AV_PKT_FLAG_KEY as c_int } else { 0 };
            (*pkt).stream_index = stream as c_int;
            let st = *(*self.oc).streams.add(stream);
            ff::av_packet_rescale_ts(pkt, tb, (*st).time_base);
            let r = ff::av_interleaved_write_frame(self.oc, pkt);
            ff::av_packet_unref(pkt);
            check(r, "av_interleaved_write_frame")?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<PathBuf> {
        unsafe {
            check(ff::av_write_trailer(self.oc), "av_write_trailer")?;
        }
        self.close();
        std::fs::rename(&self.tmp, &self.dst).context("rename clip")?;
        Ok(self.dst.clone())
    }

    fn close(&mut self) {
        unsafe {
            if !self.oc.is_null() {
                if !(*self.oc).pb.is_null() {
                    ff::avio_closep(&mut (*self.oc).pb);
                }
                ff::avformat_free_context(self.oc);
                self.oc = ptr::null_mut();
            }
        }
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        if !self.oc.is_null() {
            // Abandoned (error path): remove the partial file.
            self.close();
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// Writes a replay buffer snapshot to `dst` as a regular (faststart) MP4.
pub fn write_clip(dst: &Path, clip: &ClipData, comment: &str) -> Result<PathBuf> {
    let mut m = Muxer::create(dst, &clip.streams, false, comment)?;
    let mut order: Vec<(i64, usize, usize)> = Vec::new();
    for (si, pkts) in clip.packets.iter().enumerate() {
        let tb = clip.streams[si].time_base;
        for (pi, p) in pkts.iter().enumerate() {
            order.push((rescale(p.dts, tb, US), si, pi));
        }
    }
    order.sort_by_key(|&(t, s, i)| (t, s, i));
    for (_, si, pi) in order {
        m.write(si, &clip.packets[si][pi], clip.origin_us)?;
    }
    m.finish()
}

// ---------------------------------------------------------------------------
// Live recording

enum Msg {
    Packet(usize, PacketRef),
    Stop,
}

pub struct Recorder {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<Result<PathBuf>>>,
}

impl Recorder {
    /// Starts a fragmented MP4 (stays playable even if the app crashes).
    /// Writing begins at the first video keyframe received.
    pub fn start(path: PathBuf, streams: Vec<StreamDesc>, comment: String) -> Result<Self> {
        let muxer = Muxer::create(&path, &streams, true, &comment)?;
        let (tx, rx) = unbounded();
        let video = streams.iter().position(|s| s.kind == StreamKind::Video);
        let thread = std::thread::Builder::new()
            .name("gc-recorder".into())
            .spawn(move || record_loop(muxer, rx, video))?;
        Ok(Recorder { tx, thread: Some(thread) })
    }

    pub fn push(&self, stream: usize, pkt: PacketRef) {
        let _ = self.tx.send(Msg::Packet(stream, pkt));
    }

    pub fn stop(mut self) -> Result<PathBuf> {
        let _ = self.tx.send(Msg::Stop);
        match self.thread.take().unwrap().join() {
            Ok(r) => r,
            Err(_) => bail!("recorder thread panicked"),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = self.tx.send(Msg::Stop);
            let _ = t.join();
        }
    }
}

fn record_loop(mut muxer: Muxer, rx: Receiver<Msg>, video: Option<usize>) -> Result<PathBuf> {
    let mut origin: Option<i64> = None;
    let mut written = 0usize;
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Stop => break,
            Msg::Packet(si, p) => {
                if origin.is_none() {
                    if Some(si) == video && p.key || video.is_none() {
                        origin = Some(p.time_us);
                    } else {
                        continue;
                    }
                }
                let o = origin.unwrap();
                if p.time_us < o {
                    continue;
                }
                muxer.write(si, &p, o)?;
                written += 1;
            }
        }
    }
    if written == 0 {
        bail!("nothing was recorded");
    }
    muxer.finish()
}
