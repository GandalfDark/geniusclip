//! Monotonic clock based on QueryPerformanceCounter.
//!
//! All engine timestamps are microseconds on the QPC timeline, so video
//! frames (paced by QPC) and WASAPI packets (stamped with QPC positions)
//! share one time base and stay in sync over long sessions.

use std::sync::OnceLock;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn freq() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        unsafe { QueryPerformanceFrequency(&mut f).ok() };
        f.max(1)
    })
}

/// Raw QPC ticks.
pub fn qpc() -> i64 {
    let mut t = 0i64;
    unsafe { QueryPerformanceCounter(&mut t).ok() };
    t
}

pub fn qpc_to_us(ticks: i64) -> i64 {
    (ticks as i128 * 1_000_000 / freq() as i128) as i64
}

pub fn us_to_qpc(us: i64) -> i64 {
    (us as i128 * freq() as i128 / 1_000_000) as i64
}

/// Current time in microseconds on the QPC timeline.
pub fn now_us() -> i64 {
    qpc_to_us(qpc())
}
