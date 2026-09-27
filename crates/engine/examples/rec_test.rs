//! Records the screen (no audio devices are opened) and checks that the
//! recording comes out as a regular MP4.
//!
//!   cargo run -p geniusclip-engine --example rec_test -- [seconds] [out.mp4]

use geniusclip_engine::*;
use std::path::PathBuf;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Desktop duplication needs per-monitor DPI awareness on mixed-scale setups.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let out = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "target/rec-test/rec.mp4".into()));
    std::fs::create_dir_all(out.parent().unwrap())?;
    let _ = std::fs::remove_file(&out);

    let (tx, rx) = crossbeam_channel::unbounded();
    let engine = Engine::new(move |e| match e {
        EngineEvent::Status(_) => {}
        other => {
            println!("event: {other:?}");
            let _ = tx.send(other);
        }
    });
    engine.configure(EngineConfig { system_audio: false, mic: false, ..Default::default() }, 30)?;
    engine.start_recording(out.clone(), "rec_test".into())?;
    std::thread::sleep(Duration::from_secs(secs));
    engine.stop_recording()?;
    loop {
        match rx.recv_timeout(Duration::from_secs(60))? {
            EngineEvent::RecordingSaved { .. } => break,
            EngineEvent::RecordingFailed { error } => anyhow::bail!("recording failed: {error}"),
            _ => {}
        }
    }
    let mut f = std::fs::File::open(&out)?;
    println!("needs finish: {}", finalize::needs_finish(&mut f)?);
    engine.shutdown();
    Ok(())
}
