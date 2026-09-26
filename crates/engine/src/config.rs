use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

impl Quality {
    /// Target Mbit/s for 1080p60 H.264; other sizes/codecs are scaled from this.
    pub fn reference_mbps(self) -> f64 {
        match self {
            Quality::Low => 8.0,
            Quality::Medium => 14.0,
            Quality::High => 20.0,
            Quality::Ultra => 32.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Native,
    P720,
    P1080,
    P1440,
    P2160,
}

impl Resolution {
    pub fn height(self) -> Option<u32> {
        match self {
            Resolution::Native => None,
            Resolution::P720 => Some(720),
            Resolution::P1080 => Some(1080),
            Resolution::P1440 => Some(1440),
            Resolution::P2160 => Some(2160),
        }
    }
}

/// Everything that affects the capture/encode pipeline. Changing any field
/// restarts the pipeline (and clears the replay buffer).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineConfig {
    /// GDI device name of the monitor (`\\.\DISPLAY1`); None = primary.
    pub monitor: Option<String>,
    pub fps: u32,
    pub resolution: Resolution,
    pub codec: Codec,
    pub quality: Quality,
    /// Overrides `quality` when set.
    pub bitrate_kbps: Option<u32>,
    pub capture_cursor: bool,
    pub system_audio: bool,
    /// Endpoint id; None = follow the Windows default device.
    pub system_device: Option<String>,
    pub system_volume: f32,
    pub mic: bool,
    pub mic_device: Option<String>,
    pub mic_volume: f32,
    /// Also store game and microphone as separate audio tracks (for editing).
    pub separate_tracks: bool,
    /// Keep the replay buffer in temporary files instead of RAM.
    pub disk_buffer: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            monitor: None,
            fps: 60,
            resolution: Resolution::Native,
            codec: Codec::H264,
            quality: Quality::High,
            bitrate_kbps: None,
            capture_cursor: true,
            system_audio: true,
            system_device: None,
            system_volume: 1.0,
            mic: true,
            mic_device: None,
            mic_volume: 1.0,
            separate_tracks: true,
            disk_buffer: false,
        }
    }
}
