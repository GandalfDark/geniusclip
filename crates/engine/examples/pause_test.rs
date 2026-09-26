//! Checks that pausing capture (display off / locked) stops the pipeline and
//! resuming starts it again with a fresh buffer.
use geniusclip_engine::*;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    // Desktop Duplication needs a DPI-aware process (the app has it in its manifest).
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let engine = Engine::new(|_| {});
    engine.configure(EngineConfig { mic: false, ..Default::default() }, 30)?;
    engine.set_replay_enabled(true)?;
    for _ in 0..6 {
        std::thread::sleep(Duration::from_secs(1));
        let s = engine.status();
        println!("running: {} buffer {:.1}s {:.1} MB fps {:.0}", s.running, s.buffer_seconds, s.buffer_bytes as f64 / 1e6, s.fps);
    }
    engine.set_paused(true)?;
    std::thread::sleep(Duration::from_secs(2));
    println!("paused → running: {}", engine.status().running);
    engine.set_paused(false)?;
    std::thread::sleep(Duration::from_secs(5));
    let s = engine.status();
    println!("resumed → running: {} buffer {:.1}s encoder {}", s.running, s.buffer_seconds, s.encoder);
    engine.shutdown();
    Ok(())
}
