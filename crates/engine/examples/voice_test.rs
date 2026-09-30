//! Records with Discord's voices on a track of their own (microphone off)
//! and prints the audio tracks of the result with their peak level. Opens
//! the default output device for loopback and captures the screen: run it
//! by hand, with something playing in Discord and elsewhere.
//!
//!   cargo run -p geniusclip-engine --example voice_test -- [seconds] [out.mp4]

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
    let out = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "target/voice-test/voice.mp4".into()));
    let dir = out.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::remove_file(&out);

    let (tx, rx) = crossbeam_channel::unbounded();
    let engine = Engine::new(move |e| match e {
        EngineEvent::Status(_) => {}
        other => {
            println!("event: {other:?}");
            let _ = tx.send(other);
        }
    });
    let cfg = EngineConfig { system_audio: true, mic: false, separate_tracks: true, voice_separate: true, ..Default::default() };
    engine.configure(cfg, 30)?;
    engine.start_recording(out.clone(), "voice_test".into())?;
    std::thread::sleep(Duration::from_secs(secs));
    engine.stop_recording()?;
    loop {
        match rx.recv_timeout(Duration::from_secs(60))? {
            EngineEvent::RecordingSaved { .. } => break,
            EngineEvent::RecordingFailed { error } => anyhow::bail!("recording failed: {error}"),
            _ => {}
        }
    }
    engine.shutdown();

    let titles = remix::audio_tracks(&out)?;
    println!("{}: mix layout {}", out.display(), remix::is_mix_layout(&titles));
    let (files, peaks) = remix::prepare_tracks(&out, &dir, "voice_track", 200, &[], &mut |_| {})?;
    for (k, title) in titles.iter().enumerate() {
        let peak = peaks[k].iter().copied().fold(0.0f32, f32::max);
        println!("  track {k} {title:?}: peak {peak:.3}, {}", files[k].display());
    }
    Ok(())
}
