//! Manual end-to-end test: buffer a few seconds, save a clip, a screenshot
//! and a short live recording, then print what was produced.
//!
//! cargo run -p geniusclip-engine --example replay -- [seconds] [out_dir]

use geniusclip_engine::*;
use std::path::PathBuf;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8);
    let out = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "target/replay-test".into()));
    std::fs::create_dir_all(&out)?;

    println!("FFmpeg {}", ffmpeg_version());
    for m in list_monitors()? {
        println!("monitor: {:?}", m);
    }
    for d in list_audio_devices(false)? {
        println!("output: {} {}", d.name, if d.is_default { "(default)" } else { "" });
    }
    for d in list_audio_devices(true)? {
        println!("input:  {} {}", d.name, if d.is_default { "(default)" } else { "" });
    }
    println!("foreground: {:?}", game::foreground_app());

    let engine = Engine::new(|e| match e {
        EngineEvent::Status(_) => {}
        other => println!("event: {other:?}"),
    });
    engine.configure(EngineConfig::default(), 30)?;
    engine.set_replay_enabled(true)?;

    std::thread::sleep(Duration::from_secs(2));
    engine.start_recording(out.join("recording.mp4"), "test".into())?;
    for _ in 0..secs {
        std::thread::sleep(Duration::from_secs(1));
        let s = engine.status();
        println!(
            "buffer {:.1}s {:.1} MB | {} {}x{} {:.1} fps dropped {} | rec {:.1}s",
            s.buffer_seconds,
            s.buffer_bytes as f64 / 1e6,
            s.encoder,
            s.width,
            s.height,
            s.fps,
            s.dropped_frames,
            s.recording_seconds
        );
    }
    engine.stop_recording()?;
    println!("save 1: {:?}", engine.save_replay(out.join("clip.mp4"), None, "test".into(), true)?);
    std::thread::sleep(Duration::from_secs(3));
    println!("save 2: {:?}", engine.save_replay(out.join("clip2.mp4"), None, "test".into(), true)?);
    std::thread::sleep(Duration::from_millis(300));
    println!("save 3: {:?}", engine.save_replay(out.join("clip3.mp4"), None, "test".into(), true)?);
    engine.screenshot(out.join("shot.png"))?;
    std::thread::sleep(Duration::from_secs(3));

    let clip = out.join("clip.mp4");
    if clip.exists() {
        println!("clip info: {:?}", media::probe(&clip)?);
        media::thumbnail(&clip, &out.join("thumb.jpg"), 480, 1.0)?;
        media::trim(&clip, &out.join("trimmed.mp4"), 2.5, 5.0)?;
        println!("trimmed info: {:?}", media::probe(&out.join("trimmed.mp4"))?);
    }
    engine.shutdown();
    Ok(())
}
