//! Post-processing of saved files: probing, thumbnails, lossless trimming
//! and PNG screenshots.

use crate::ffutil::*;
use anyhow::{bail, Context, Result};
use ffmpeg_sys_next as ff;
use serde::Serialize;
use std::ffi::{c_int, CStr};
use std::path::Path;
use std::ptr;

pub(crate) struct Input(pub(crate) *mut ff::AVFormatContext);
impl Input {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        unsafe {
            let mut ic = ptr::null_mut();
            check(ff::avformat_open_input(&mut ic, path_cstr(path).as_ptr(), ptr::null(), ptr::null_mut()), "open")
                .with_context(|| path.display().to_string())?;
            let me = Input(ic);
            check(ff::avformat_find_stream_info(ic, ptr::null_mut()), "find_stream_info")?;
            Ok(me)
        }
    }
    pub(crate) fn streams(&self) -> &[*mut ff::AVStream] {
        unsafe { std::slice::from_raw_parts((*self.0).streams, (*self.0).nb_streams as usize) }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.0) };
    }
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub audio_tracks: u32,
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let input = Input::open(path)?;
    let mut info = MediaInfo::default();
    unsafe {
        let d = (*input.0).duration;
        info.duration = if d > 0 { d as f64 / ff::AV_TIME_BASE as f64 } else { 0.0 };
        for &st in input.streams() {
            let par = &*(*st).codecpar;
            match par.codec_type {
                ff::AVMediaType::AVMEDIA_TYPE_VIDEO if info.width == 0 => {
                    info.width = par.width as u32;
                    info.height = par.height as u32;
                    // The capture rate: with variable frame rate the average
                    // drops on a static screen, the timestamp grid does not.
                    let r = if (*st).r_frame_rate.num > 0 && (*st).r_frame_rate.den > 0 { (*st).r_frame_rate } else { (*st).avg_frame_rate };
                    info.fps = if r.den > 0 { r.num as f64 / r.den as f64 } else { 0.0 };
                    info.video_codec = CStr::from_ptr(ff::avcodec_get_name(par.codec_id)).to_string_lossy().into_owned();
                }
                ff::AVMediaType::AVMEDIA_TYPE_AUDIO => info.audio_tracks += 1,
                _ => {}
            }
        }
    }
    Ok(info)
}

/// Decodes one frame near `at_seconds` and writes it as a JPEG no wider than `max_w`.
pub fn thumbnail(path: &Path, out: &Path, max_w: u32, at_seconds: f64) -> Result<()> {
    let input = Input::open(path)?;
    unsafe {
        let vidx = ff::av_find_best_stream(input.0, ff::AVMediaType::AVMEDIA_TYPE_VIDEO, -1, -1, ptr::null_mut(), 0);
        check(vidx, "no video stream")?;
        let st = input.streams()[vidx as usize];
        let par = (*st).codecpar;
        let dec = ff::avcodec_find_decoder((*par).codec_id);
        if dec.is_null() {
            bail!("no decoder");
        }
        let dctx = CodecCtx(ff::avcodec_alloc_context3(dec));
        check(ff::avcodec_parameters_to_context(dctx.0, par), "parameters_to_context")?;
        // One frame is wanted: a frame-threaded decoder (thread_count 0)
        // decodes about as many frames as there are cores before the first
        // comes out, and thumbnails are made several at a time.
        (*dctx.0).thread_count = 1;
        (*dctx.0).thread_type = ff::FF_THREAD_SLICE as c_int;
        check(ff::avcodec_open2(dctx.0, dec, ptr::null_mut()), "open decoder")?;

        if at_seconds > 0.0 {
            let ts = rescale((at_seconds * 1_000_000.0) as i64, US, (*st).time_base);
            ff::av_seek_frame(input.0, vidx, ts, ff::AVSEEK_FLAG_BACKWARD as c_int);
        }

        let pkt = AvPacket::new();
        let frame = AvFrame::new();
        let mut got = false;
        while !got && ff::av_read_frame(input.0, pkt.0) >= 0 {
            if (*pkt.0).stream_index == vidx {
                if ff::avcodec_send_packet(dctx.0, pkt.0) >= 0 && ff::avcodec_receive_frame(dctx.0, frame.0) >= 0 {
                    got = true;
                }
            }
            ff::av_packet_unref(pkt.0);
        }
        if !got {
            ff::avcodec_send_packet(dctx.0, ptr::null());
            got = ff::avcodec_receive_frame(dctx.0, frame.0) >= 0;
        }
        if !got {
            bail!("could not decode a frame");
        }
        let f = &*frame.0;
        let (sw, sh) = (f.width, f.height);
        let dw = (max_w as i32).min(sw) & !1;
        let dh = ((sh as i64 * dw as i64 / sw as i64) as i32).max(2) & !1;
        encode_image(
            out,
            ff::AVCodecID::AV_CODEC_ID_MJPEG,
            ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P,
            dw,
            dh,
            |dst| {
                let sws = ff::sws_getContext(
                    sw,
                    sh,
                    std::mem::transmute::<c_int, ff::AVPixelFormat>(f.format),
                    dw,
                    dh,
                    ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P,
                    ff::SwsFlags::SWS_BILINEAR as c_int,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                if sws.is_null() {
                    bail!("sws_getContext");
                }
                ff::sws_scale(sws, f.data.as_ptr() as _, f.linesize.as_ptr(), 0, sh, (*dst).data.as_ptr(), (*dst).linesize.as_ptr());
                ff::sws_freeContext(sws);
                Ok(())
            },
        )
    }
}

/// Encodes a single still image with an FFmpeg image encoder.
unsafe fn encode_image(
    out: &Path,
    codec_id: ff::AVCodecID,
    fmt: ff::AVPixelFormat,
    w: i32,
    h: i32,
    fill: impl FnOnce(*mut ff::AVFrame) -> Result<()>,
) -> Result<()> {
    let codec = ff::avcodec_find_encoder(codec_id);
    if codec.is_null() {
        bail!("image encoder missing");
    }
    let ctx = CodecCtx(ff::avcodec_alloc_context3(codec));
    let c = &mut *ctx.0;
    c.width = w;
    c.height = h;
    c.pix_fmt = fmt;
    c.time_base = q(1, 1);
    if codec_id == ff::AVCodecID::AV_CODEC_ID_MJPEG {
        c.flags |= ff::AV_CODEC_FLAG_QSCALE as c_int;
        c.global_quality = 3 * ff::FF_QP2LAMBDA as c_int;
        c.color_range = ff::AVColorRange::AVCOL_RANGE_JPEG;
        c.strict_std_compliance = -1;
    } else {
        c.compression_level = 3;
    }
    check(ff::avcodec_open2(ctx.0, codec, ptr::null_mut()), "open image encoder")?;
    let frame = AvFrame::new();
    (*frame.0).width = w;
    (*frame.0).height = h;
    (*frame.0).format = fmt as c_int;
    check(ff::av_frame_get_buffer(frame.0, 0), "av_frame_get_buffer")?;
    if codec_id == ff::AVCodecID::AV_CODEC_ID_MJPEG {
        (*frame.0).quality = c.global_quality;
    }
    fill(frame.0)?;
    check(ff::avcodec_send_frame(ctx.0, frame.0), "send image")?;
    ff::avcodec_send_frame(ctx.0, ptr::null());
    let pkt = AvPacket::new();
    check(ff::avcodec_receive_packet(ctx.0, pkt.0), "receive image")?;
    let data = std::slice::from_raw_parts((*pkt.0).data, (*pkt.0).size as usize);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, data).context("write image")?;
    Ok(())
}

/// Saves a BGRA image (rows of `pitch` bytes) as PNG.
pub fn save_png_bgra(out: &Path, w: u32, h: u32, pitch: usize, bgra: &[u8]) -> Result<()> {
    unsafe {
        encode_image(out, ff::AVCodecID::AV_CODEC_ID_PNG, ff::AVPixelFormat::AV_PIX_FMT_RGB24, w as i32, h as i32, |f| {
            let ls = (*f).linesize[0] as usize;
            let dst = std::slice::from_raw_parts_mut((*f).data[0], ls * h as usize);
            for y in 0..h as usize {
                let src = &bgra[y * pitch..y * pitch + w as usize * 4];
                let row = &mut dst[y * ls..y * ls + w as usize * 3];
                for (s, d) in src.chunks_exact(4).zip(row.chunks_exact_mut(3)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                }
            }
            Ok(())
        })
    }
}

/// Losslessly cuts [start, end] seconds out of `input` into `output`.
/// The video starts at the preceding keyframe internally; an MP4 edit list
/// makes players begin exactly at `start`.
pub fn trim(input_path: &Path, output: &Path, start: f64, end: f64) -> Result<()> {
    if end <= start {
        bail!("empty selection");
    }
    let input = Input::open(input_path)?;
    let start_us = (start * 1_000_000.0) as i64;
    let end_us = (end * 1_000_000.0) as i64;
    unsafe {
        let mut descs = Vec::new();
        let mut map = vec![None; input.streams().len()];
        for (i, &st) in input.streams().iter().enumerate() {
            let par = (*st).codecpar;
            let kind = match (*par).codec_type {
                ff::AVMediaType::AVMEDIA_TYPE_VIDEO => StreamKind::Video,
                ff::AVMediaType::AVMEDIA_TYPE_AUDIO => StreamKind::Audio,
                _ => continue,
            };
            let title = dict_get((*st).metadata, "handler_name").unwrap_or_default();
            map[i] = Some(descs.len());
            descs.push(StreamDesc { kind, params: CodecParams::from_params(par)?, time_base: (*st).time_base, title });
        }
        let comment = dict_get((*input.0).metadata, "comment").unwrap_or_default();
        let mut mux = crate::mux::Muxer::create(output, &descs, false, &comment)?;

        ff::av_seek_frame(input.0, -1, start_us * ff::AV_TIME_BASE as i64 / 1_000_000, ff::AVSEEK_FLAG_BACKWARD as c_int);
        let pkt = AvPacket::new();
        let mut done = vec![false; descs.len()];
        let mut seen_key = vec![false; descs.len()];
        // Each video stream's newest packet waits for the next one: when that
        // is past the end, it is the last frame and lasts until the end (a
        // variable frame rate video can hold a frame for a while).
        let mut held: Vec<Option<Packet>> = descs.iter().map(|_| None).collect();
        while ff::av_read_frame(input.0, pkt.0) >= 0 {
            let si = (*pkt.0).stream_index as usize;
            let Some(oi) = map.get(si).copied().flatten() else {
                ff::av_packet_unref(pkt.0);
                continue;
            };
            let d = &descs[oi];
            let p = Packet::from_av(pkt.0, d.time_base);
            ff::av_packet_unref(pkt.0);
            if p.time_us > end_us {
                if let Some(last) = held[oi].take() {
                    mux.write_last(oi, &last, start_us, end_us)?;
                }
                done[oi] = true;
                if done.iter().all(|&x| x) {
                    break;
                }
                continue;
            }
            match d.kind {
                StreamKind::Video => {
                    if !seen_key[oi] {
                        if !p.key {
                            continue;
                        }
                        seen_key[oi] = true;
                    }
                    if let Some(prev) = held[oi].replace(p) {
                        mux.write(oi, &prev, start_us)?;
                    }
                }
                StreamKind::Audio => {
                    if p.time_us < start_us {
                        continue;
                    }
                    mux.write(oi, &p, start_us)?;
                }
            }
        }
        // The file ended first: its last frames keep their own durations.
        for (oi, last) in held.into_iter().enumerate() {
            if let Some(last) = last {
                mux.write(oi, &last, start_us)?;
            }
        }
        mux.finish()?;
    }
    Ok(())
}

pub(crate) unsafe fn dict_get(d: *const ff::AVDictionary, key: &str) -> Option<String> {
    let e = ff::av_dict_get(d, cstr(key).as_ptr(), ptr::null(), 0);
    if e.is_null() {
        None
    } else {
        Some(CStr::from_ptr((*e).value).to_string_lossy().into_owned())
    }
}
