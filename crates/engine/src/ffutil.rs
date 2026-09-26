//! Thin helpers over the raw FFmpeg bindings.

use anyhow::{bail, Result};
use ffmpeg_sys_next as ff;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::Arc;

pub const EAGAIN: c_int = 11;
pub const US: ff::AVRational = ff::AVRational { num: 1, den: 1_000_000 };

pub fn err_str(code: c_int) -> String {
    let mut buf = [0 as c_char; 256];
    unsafe {
        ff::av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

pub fn check(code: c_int, what: &str) -> Result<c_int> {
    if code < 0 {
        bail!("{what}: {} ({code})", err_str(code));
    }
    Ok(code)
}

pub fn is_eagain(code: c_int) -> bool {
    code == ff::AVERROR(EAGAIN)
}

pub fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

pub const fn q(num: i32, den: i32) -> ff::AVRational {
    ff::AVRational { num, den }
}

pub fn rescale(v: i64, from: ff::AVRational, to: ff::AVRational) -> i64 {
    unsafe { ff::av_rescale_q(v, from, to) }
}

/// Sets a private (codec/muxer specific) option, logging instead of failing
/// when an option is not supported by the particular implementation.
pub unsafe fn set_opt(obj: *mut c_void, key: &str, val: &str) -> bool {
    let (k, v) = (cstr(key), cstr(val));
    let r = ff::av_opt_set(obj, k.as_ptr(), v.as_ptr(), ff::AV_OPT_SEARCH_CHILDREN as c_int);
    if r < 0 {
        log::debug!("option {key}={val} rejected: {}", err_str(r));
    }
    r >= 0
}

pub fn quiet_logs() {
    unsafe { ff::av_log_set_level(if cfg!(debug_assertions) { ff::AV_LOG_WARNING } else { ff::AV_LOG_ERROR } as c_int) };
}

/// Owned copy of stream codec parameters (extradata etc.), used to create
/// muxer streams long after the encoder that produced them is gone.
pub struct CodecParams(*mut ff::AVCodecParameters);
unsafe impl Send for CodecParams {}
unsafe impl Sync for CodecParams {}

impl CodecParams {
    pub unsafe fn from_context(ctx: *const ff::AVCodecContext) -> Result<Arc<Self>> {
        let p = ff::avcodec_parameters_alloc();
        if p.is_null() {
            bail!("avcodec_parameters_alloc failed");
        }
        let me = CodecParams(p);
        check(ff::avcodec_parameters_from_context(p, ctx), "avcodec_parameters_from_context")?;
        Ok(Arc::new(me))
    }
    pub unsafe fn from_params(src: *const ff::AVCodecParameters) -> Result<Arc<Self>> {
        let p = ff::avcodec_parameters_alloc();
        if p.is_null() {
            bail!("avcodec_parameters_alloc failed");
        }
        let me = CodecParams(p);
        check(ff::avcodec_parameters_copy(p, src), "avcodec_parameters_copy")?;
        Ok(Arc::new(me))
    }
    pub fn as_ptr(&self) -> *const ff::AVCodecParameters {
        self.0
    }
}

impl Drop for CodecParams {
    fn drop(&mut self) {
        unsafe { ff::avcodec_parameters_free(&mut self.0) };
    }
}

/// An encoded packet kept in the replay buffer.
/// Encoded bytes of a packet: in memory, or in a disk-buffer segment.
pub enum Payload {
    Mem(Box<[u8]>),
    Disk { seg: Arc<crate::disk::Segment>, offset: u64, len: u32 },
}

impl Payload {
    pub fn len(&self) -> usize {
        match self {
            Payload::Mem(b) => b.len(),
            Payload::Disk { len, .. } => *len as usize,
        }
    }

    /// Copies the bytes into `dst` (exactly `len()` long).
    pub fn read_into(&self, dst: &mut [u8]) -> std::io::Result<()> {
        match self {
            Payload::Mem(b) => {
                dst.copy_from_slice(b);
                Ok(())
            }
            Payload::Disk { seg, offset, .. } => seg.read_at(*offset, dst),
        }
    }
}

pub struct Packet {
    pub data: Payload,
    pub pts: i64,
    pub dts: i64,
    pub duration: i64,
    pub key: bool,
    /// Presentation time in microseconds on the engine (QPC) timeline.
    pub time_us: i64,
}

impl Packet {
    pub unsafe fn from_av(pkt: *const ff::AVPacket, tb: ff::AVRational) -> Self {
        let p = &*pkt;
        let data = if p.data.is_null() || p.size <= 0 {
            Box::<[u8]>::default()
        } else {
            std::slice::from_raw_parts(p.data, p.size as usize).into()
        };
        let pts = if p.pts == ff::AV_NOPTS_VALUE { p.dts } else { p.pts };
        Packet {
            data: Payload::Mem(data),
            pts,
            dts: if p.dts == ff::AV_NOPTS_VALUE { pts } else { p.dts },
            duration: p.duration,
            key: p.flags & ff::AV_PKT_FLAG_KEY as c_int != 0,
            time_us: rescale(pts, tb, US),
        }
    }
}

pub type PacketRef = Arc<Packet>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
}

/// Description of one elementary stream produced by the engine.
#[derive(Clone)]
pub struct StreamDesc {
    pub kind: StreamKind,
    pub params: Arc<CodecParams>,
    pub time_base: ff::AVRational,
    pub title: String,
}

unsafe impl Send for StreamDesc {}
unsafe impl Sync for StreamDesc {}

/// RAII wrapper for AVPacket.
pub struct AvPacket(pub *mut ff::AVPacket);
impl AvPacket {
    pub fn new() -> Self {
        AvPacket(unsafe { ff::av_packet_alloc() })
    }
}
impl Drop for AvPacket {
    fn drop(&mut self) {
        unsafe { ff::av_packet_free(&mut self.0) };
    }
}

/// RAII wrapper for AVFrame.
pub struct AvFrame(pub *mut ff::AVFrame);
impl AvFrame {
    pub fn new() -> Self {
        AvFrame(unsafe { ff::av_frame_alloc() })
    }
}
impl Drop for AvFrame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) };
    }
}

/// RAII wrapper for an encoder/decoder context.
pub struct CodecCtx(pub *mut ff::AVCodecContext);
unsafe impl Send for CodecCtx {}
impl Drop for CodecCtx {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.0) };
    }
}

/// Path → UTF-8 C string (FFmpeg converts UTF-8 to wide paths on Windows).
pub fn path_cstr(p: &std::path::Path) -> CString {
    cstr(&p.to_string_lossy())
}
