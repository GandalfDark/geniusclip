//! Hardware video encoder fed with D3D11 NV12 surfaces (zero-copy on GPU).

use crate::config::{Codec, EngineConfig};
use crate::d3d::{VENDOR_AMD, VENDOR_INTEL, VENDOR_NVIDIA};
use crate::ffutil::*;
use anyhow::{anyhow, bail, Result};
use ffmpeg_sys_next as ff;
use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::Arc;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET};

#[repr(C)]
struct AVD3D11VADeviceContext {
    device: *mut c_void,
    device_context: *mut c_void,
    video_device: *mut c_void,
    video_context: *mut c_void,
    lock: Option<unsafe extern "C" fn(*mut c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    lock_ctx: *mut c_void,
    bind_flags: u32,
    misc_flags: u32,
}

#[repr(C)]
struct AVD3D11VAFramesContext {
    texture: *mut c_void,
    bind_flags: u32,
    misc_flags: u32,
    texture_infos: *mut c_void,
}

/// NVENC pixel rates (pixels per second) up to which a preset is used: p3
/// up to 1440p144, p2 up to 4K120, p1 beyond. Measured on 1440p60 game
/// footage at the app's bitrate, p3 costs the encoder 40% less than p5 with
/// a quarter-resolution first pass, for 0.2 less VMAF (97.1 against 97.3):
/// nothing anyone sees in a replay, while the encoder keeps less busy.
const NVENC_P3_MAX: u64 = 2560 * 1440 * 144;
const NVENC_P2_MAX: u64 = 3840 * 2160 * 120;
/// Variable frame rate: the encoder's own keyframe interval, in seconds of
/// full-rate frames (the capture loop forces one every second).
const VFR_GOP_SECONDS: i32 = 4;

pub struct VideoEncoder {
    ctx: CodecCtx,
    hw_device: *mut ff::AVBufferRef,
    hw_frames: *mut ff::AVBufferRef,
    pkt: AvPacket,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub time_base: ff::AVRational,
    pub params: Arc<CodecParams>,
    /// The codec asked for, when the GPU has no encoder for it and H.264
    /// is used instead.
    pub fallback: Option<Codec>,
}

unsafe impl Send for VideoEncoder {}

/// Candidate encoder names in preference order for the GPU vendor.
fn candidates(codec: Codec, vendor: u32) -> Vec<&'static str> {
    let (nv, amf, mf) = match codec {
        Codec::H264 => ("h264_nvenc", "h264_amf", "h264_mf"),
        Codec::Hevc => ("hevc_nvenc", "hevc_amf", "hevc_mf"),
        Codec::Av1 => ("av1_nvenc", "av1_amf", "av1_mf"),
    };
    match vendor {
        VENDOR_NVIDIA => vec![nv, mf],
        VENDOR_AMD => vec![amf, mf],
        VENDOR_INTEL => vec![mf],
        _ => vec![nv, amf, mf],
    }
}

/// Bitrate in bits/s for the configured quality at the given size/fps.
pub fn target_bitrate(cfg: &EngineConfig, w: u32, h: u32, fps: u32) -> i64 {
    if let Some(kbps) = cfg.bitrate_kbps.filter(|&k| k > 0) {
        return kbps as i64 * 1000;
    }
    // Reference: Mbit/s for 1080p60 H.264, scaled sub-linearly with pixel rate.
    let base = cfg.quality.reference_mbps();
    let ratio = (w as f64 * h as f64 * fps as f64) / (1920.0 * 1080.0 * 60.0);
    let codec_factor = match cfg.codec {
        Codec::H264 => 1.0,
        Codec::Hevc => 0.7,
        Codec::Av1 => 0.65,
    };
    (base * ratio.powf(0.75) * codec_factor * 1_000_000.0) as i64
}

impl VideoEncoder {
    /// An encoder for the configured codec; if the GPU has none for it (an
    /// older card and HEVC or AV1), H.264 instead: replay keeps working, and
    /// `fallback` tells what was asked for.
    pub fn new(device: &ID3D11Device, vendor: u32, cfg: &EngineConfig, width: u32, height: u32) -> Result<Self> {
        match Self::open_codec(device, vendor, cfg, width, height, true) {
            Err(e) if cfg.codec != Codec::H264 => {
                log::warn!("no {:?} encoder on this GPU ({e:#}); recording in H.264", cfg.codec);
                let h264 = EngineConfig { codec: Codec::H264, ..cfg.clone() };
                let mut enc = Self::open_codec(device, vendor, &h264, width, height, true)?;
                enc.fallback = Some(cfg.codec);
                Ok(enc)
            }
            r => r,
        }
    }

    fn open_codec(device: &ID3D11Device, vendor: u32, cfg: &EngineConfig, width: u32, height: u32, log_it: bool) -> Result<Self> {
        let mut last_err = anyhow!("no encoder candidates");
        for name in candidates(cfg.codec, vendor) {
            match unsafe { Self::open(device, name, cfg, width, height, log_it) } {
                Ok(e) => {
                    if log_it {
                        log::info!("video encoder: {name} {width}x{height}@{} {} kbps", cfg.fps, e.bitrate() / 1000);
                    }
                    return Ok(e);
                }
                Err(e) => {
                    if log_it {
                        log::warn!("encoder {name} failed: {e:#}");
                    }
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    fn bitrate(&self) -> i64 {
        unsafe { (*self.ctx.0).bit_rate }
    }

    unsafe fn open(device: &ID3D11Device, name: &str, cfg: &EngineConfig, width: u32, height: u32, log_it: bool) -> Result<Self> {
        let codec = ff::avcodec_find_encoder_by_name(cstr(name).as_ptr());
        if codec.is_null() {
            bail!("{name} not built into FFmpeg");
        }

        // Hardware device wrapping our own D3D11 device.
        let hw_device = BufRef(ff::av_hwdevice_ctx_alloc(ff::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA));
        if hw_device.0.is_null() {
            bail!("av_hwdevice_ctx_alloc");
        }
        let devctx = (*hw_device.0).data as *mut ff::AVHWDeviceContext;
        let d3dctx = (*devctx).hwctx as *mut AVD3D11VADeviceContext;
        // FFmpeg releases this reference when the device context is freed.
        (*d3dctx).device = device.clone().into_raw();
        check(ff::av_hwdevice_ctx_init(hw_device.0), "av_hwdevice_ctx_init")?;

        let hw_frames = BufRef(ff::av_hwframe_ctx_alloc(hw_device.0));
        if hw_frames.0.is_null() {
            bail!("av_hwframe_ctx_alloc");
        }
        let fctx = (*hw_frames.0).data as *mut ff::AVHWFramesContext;
        (*fctx).format = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
        (*fctx).sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
        (*fctx).width = width as c_int;
        (*fctx).height = height as c_int;
        // 0 = dynamic pool of individual textures: drivers reject NV12
        // texture *arrays* that are also render targets (video processor output).
        (*fctx).initial_pool_size = 0;
        let d3dframes = (*fctx).hwctx as *mut AVD3D11VAFramesContext;
        (*d3dframes).bind_flags = D3D11_BIND_RENDER_TARGET.0 as u32;
        check(ff::av_hwframe_ctx_init(hw_frames.0), "av_hwframe_ctx_init")?;

        let ctx = CodecCtx(ff::avcodec_alloc_context3(codec));
        if ctx.0.is_null() {
            bail!("avcodec_alloc_context3");
        }
        let c = &mut *ctx.0;
        let fps = cfg.fps.clamp(10, 240) as i32;
        c.width = width as c_int;
        c.height = height as c_int;
        c.time_base = q(1, fps);
        c.framerate = q(fps, 1);
        c.pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_D3D11;
        c.sw_pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_NV12;
        c.hw_frames_ctx = ff::av_buffer_ref(hw_frames.0);
        // 1 s keyframes: precise clip starts and fast seeking. With variable
        // frame rate a GOP counted in frames would stretch on a static
        // screen, so the capture loop forces a keyframe every second of wall
        // time and the encoder's own interval is only a fallback beyond it.
        c.gop_size = if cfg.constant_fps { fps } else { fps * VFR_GOP_SECONDS };
        c.keyint_min = fps;
        c.max_b_frames = 0;
        c.flags |= ff::AV_CODEC_FLAG_GLOBAL_HEADER as c_int;
        c.color_range = ff::AVColorRange::AVCOL_RANGE_MPEG;
        c.colorspace = ff::AVColorSpace::AVCOL_SPC_BT709;
        c.color_primaries = ff::AVColorPrimaries::AVCOL_PRI_BT709;
        c.color_trc = ff::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        let bitrate = target_bitrate(cfg, width, height, fps as u32);
        c.bit_rate = bitrate;
        c.rc_max_rate = bitrate * 3 / 2;
        c.rc_buffer_size = (bitrate * 2) as c_int;

        let p = c.priv_data;
        if name.ends_with("_nvenc") {
            let rate = width as u64 * height as u64 * fps as u64;
            let preset = match rate {
                r if r <= NVENC_P3_MAX => "p3",
                r if r <= NVENC_P2_MAX => "p2",
                _ => "p1",
            };
            if log_it {
                log::info!("{name}: preset {preset} ({:.0} Mpx/s)", rate as f64 / 1e6);
            }
            set_opt(p, "preset", preset);
            set_opt(p, "tune", "hq");
            set_opt(p, "rc", "vbr");
            set_opt(p, "multipass", "disabled");
            set_opt(p, "spatial-aq", "1");
            set_opt(p, "forced-idr", "1");
            if name.starts_with("h264") {
                set_opt(p, "profile", "high");
            }
        } else if name.ends_with("_amf") {
            // The same tiers as NVENC's: the balanced preset up to 1440p144,
            // the fastest beyond (the quality one costs the encoder much
            // more for a difference nobody sees in a replay).
            let rate = width as u64 * height as u64 * fps as u64;
            let quality = if rate <= NVENC_P3_MAX { "balanced" } else { "speed" };
            if log_it {
                log::info!("{name}: quality preset {quality} ({:.0} Mpx/s)", rate as f64 / 1e6);
            }
            set_opt(p, "usage", "transcoding");
            set_opt(p, "quality", quality);
            set_opt(p, "rc", "vbr_peak");
            set_opt(p, "forced_idr", "1");
        } else if name.ends_with("_mf") {
            set_opt(p, "hw_encoding", "1");
            set_opt(p, "rate_control", "u_vbr");
            set_opt(p, "scenario", "display_remoting");
        }

        check(ff::avcodec_open2(ctx.0, codec, ptr::null_mut()), &format!("avcodec_open2({name})"))?;
        let params = CodecParams::from_context(ctx.0)?;

        Ok(VideoEncoder {
            time_base: (*ctx.0).time_base,
            ctx,
            hw_device: hw_device.into_raw(),
            hw_frames: hw_frames.into_raw(),
            pkt: AvPacket::new(),
            name: name.to_string(),
            width,
            height,
            params,
            fallback: None,
        })
    }

    /// Surface allocator that can live on the capture thread while the
    /// encoder itself runs on its own thread.
    pub fn pool(&self) -> SurfacePool {
        SurfacePool(unsafe { ff::av_buffer_ref(self.hw_frames) })
    }

    /// Encodes a surface obtained from the pool and drains produced packets.
    /// Returns false when the encoder would not take the frame (dropped).
    pub fn submit(&mut self, frame: HwFrame, pts: i64, force_key: bool, out: &mut dyn FnMut(Packet)) -> Result<bool> {
        unsafe {
            let f = &mut *(frame.0).0;
            f.pts = pts;
            f.pict_type = if force_key { ff::AVPictureType::AV_PICTURE_TYPE_I } else { ff::AVPictureType::AV_PICTURE_TYPE_NONE };
            let mut r = ff::avcodec_send_frame(self.ctx.0, (frame.0).0);
            if is_eagain(r) {
                // Output is full: take the finished packets, then retry once.
                self.drain(out)?;
                r = ff::avcodec_send_frame(self.ctx.0, (frame.0).0);
            }
            drop(frame);
            if r < 0 && !is_eagain(r) {
                check(r, "avcodec_send_frame")?;
            }
            self.drain(out)?;
            Ok(r >= 0)
        }
    }

    fn drain(&mut self, out: &mut dyn FnMut(Packet)) -> Result<()> {
        unsafe {
            loop {
                let r = ff::avcodec_receive_packet(self.ctx.0, self.pkt.0);
                if is_eagain(r) || r == ff::AVERROR_EOF {
                    return Ok(());
                }
                check(r, "avcodec_receive_packet")?;
                self.emit(out);
            }
        }
    }

    /// Hands the packet just received to `out`.
    unsafe fn emit(&mut self, out: &mut dyn FnMut(Packet)) {
        if (*self.pkt.0).duration <= 0 {
            (*self.pkt.0).duration = 1;
        }
        out(Packet::from_av(self.pkt.0, self.time_base));
        ff::av_packet_unref(self.pkt.0);
    }

    /// Ends the stream and hands out every packet still inside the encoder.
    pub fn flush(&mut self, out: &mut dyn FnMut(Packet)) {
        // Iteration caps (≈ 1 ms apart when waiting): a stuck encoder must
        // not hold up stopping the pipeline for long.
        const TRIES: u32 = 1000;
        const MAX_PACKETS: u32 = 10_000;
        unsafe {
            // A backed-up encoder refuses end-of-stream (EAGAIN) like a frame:
            // take its finished packets, then ask again.
            let mut accepted = false;
            for _ in 0..TRIES {
                let r = ff::avcodec_send_frame(self.ctx.0, ptr::null());
                if !is_eagain(r) {
                    // EOF: already draining.
                    accepted = r >= 0 || r == ff::AVERROR_EOF;
                    if !accepted {
                        log::warn!("encoder flush: {}", err_str(r));
                    }
                    break;
                }
                if self.drain(out).is_err() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if !accepted {
                log::warn!("encoder did not take end of stream; its last frames are lost");
                return;
            }
            let (mut waits, mut packets) = (0, 0);
            while waits < TRIES && packets < MAX_PACKETS {
                let r = ff::avcodec_receive_packet(self.ctx.0, self.pkt.0);
                if r == ff::AVERROR_EOF {
                    return;
                }
                if is_eagain(r) {
                    // Hardware encoders can still be finishing a frame.
                    waits += 1;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                if r < 0 {
                    log::warn!("encoder flush: {}", err_str(r));
                    return;
                }
                self.emit(out);
                packets += 1;
            }
            log::warn!("encoder flush did not reach the end of stream");
        }
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        unsafe {
            // Context must go before the frames/device it references.
            ff::avcodec_free_context(&mut self.ctx.0);
            ff::av_buffer_unref(&mut self.hw_frames);
            ff::av_buffer_unref(&mut self.hw_device);
        }
    }
}

/// A D3D11 NV12 encoder surface (refcounted AVFrame from the hw pool).
pub struct HwFrame(AvFrame);
unsafe impl Send for HwFrame {}

pub struct SurfacePool(*mut ff::AVBufferRef);
unsafe impl Send for SurfacePool {}

impl SurfacePool {
    /// Takes a free surface; returns the frame plus its texture and array slice.
    pub fn acquire(&self) -> Result<(HwFrame, ID3D11Texture2D, u32)> {
        unsafe {
            let frame = AvFrame::new();
            check(ff::av_hwframe_get_buffer(self.0, frame.0, 0), "av_hwframe_get_buffer")?;
            let f = &*frame.0;
            let raw = f.data[0] as *mut c_void;
            let tex = ID3D11Texture2D::from_raw_borrowed(&raw).ok_or_else(|| anyhow!("null surface"))?.clone();
            let slice = f.data[1] as usize as u32;
            Ok((HwFrame(frame), tex, slice))
        }
    }
}

impl Drop for SurfacePool {
    fn drop(&mut self) {
        unsafe { ff::av_buffer_unref(&mut self.0) };
    }
}

/// Owned AVBufferRef that is unref'd on drop unless released.
struct BufRef(*mut ff::AVBufferRef);
impl BufRef {
    fn into_raw(mut self) -> *mut ff::AVBufferRef {
        std::mem::replace(&mut self.0, ptr::null_mut())
    }
}
impl Drop for BufRef {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ff::av_buffer_unref(&mut self.0) };
        }
    }
}

/// Even dimensions no larger than the codec allows, preserving aspect.
pub fn output_size(cfg: &EngineConfig, src_w: u32, src_h: u32) -> (u32, u32) {
    let (mut w, mut h) = match cfg.resolution.height() {
        Some(th) if th < src_h => ((src_w as f64 * th as f64 / src_h as f64).round() as u32, th),
        _ => (src_w, src_h),
    };
    let max_w = match cfg.codec {
        Codec::H264 => 4096,
        _ => 8192,
    };
    if w > max_w {
        h = (h as f64 * max_w as f64 / w as f64).round() as u32;
        w = max_w;
    }
    (w & !1, h & !1)
}

/// The codecs the GPU of the monitor (None: the primary) can encode, found by
/// opening a small encoder of each kind (no capture involved).
pub fn supported_codecs(monitor: Option<&str>) -> Result<Vec<Codec>> {
    let out = crate::d3d::find_output(monitor)?;
    let (device, _ctx) = crate::d3d::create_device(&out.adapter)?;
    let mut ok = Vec::new();
    for codec in [Codec::H264, Codec::Hevc, Codec::Av1] {
        let cfg = EngineConfig { codec, fps: 30, ..EngineConfig::default() };
        if VideoEncoder::open_codec(&device, out.info.vendor_id, &cfg, 640, 360, false).is_ok() {
            ok.push(codec);
        }
    }
    log::info!("encodable codecs: {ok:?}");
    Ok(ok)
}
