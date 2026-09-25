//! GeniusClip capture engine: DXGI desktop duplication → GPU colour
//! conversion → hardware encoding (NVENC/AMF/MF) → in-memory replay buffer,
//! plus WASAPI audio, live recording, screenshots and clip utilities.

mod audio;
mod buffer;
pub mod clock;
mod config;
mod convert;
mod cursor;
mod d3d;
mod dup;
mod engine;
mod ffutil;
pub mod game;
pub mod media;
mod mux;
mod venc;

pub use audio::{list_devices as list_audio_devices, AudioDevice};
pub use config::{Codec, EngineConfig, Quality, Resolution};
pub use d3d::{list_monitors, MonitorInfo};
pub use engine::{Engine, EngineEvent, EngineStatus};
pub use venc::{output_size, target_bitrate};

pub fn ffmpeg_version() -> String {
    unsafe { std::ffi::CStr::from_ptr(ffmpeg_sys_next::av_version_info()).to_string_lossy().into_owned() }
}
