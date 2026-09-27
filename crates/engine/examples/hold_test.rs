//! Checks `Engine::set_hold`: video only (no audio devices are opened),
//! 3 s live, 3 s held, 3 s live, then saves the replay: `hold_test <out.mp4>`.
use geniusclip_engine::*;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let out = std::env::args().nth(1).expect("output path");
    let engine = Engine::new(|_| {});
    engine.configure(EngineConfig { mic: false, system_audio: false, fps: 30, ..Default::default() }, 30)?;
    engine.set_replay_enabled(true)?;
    std::thread::sleep(Duration::from_secs(3));
    engine.set_hold(Some(HoldCard {
        title: "Открыто меню GeniusClip".into(),
        subtitle: "Видео продолжится, когда меню закроется".into(),
        accent: [139, 92, 246],
    }));
    println!("hold on");
    std::thread::sleep(Duration::from_secs(3));
    engine.set_hold(None);
    println!("hold off");
    std::thread::sleep(Duration::from_secs(3));
    let s = engine.status();
    println!("fps {:.0} dropped {}", s.fps, s.dropped_frames);
    engine.save_replay(out.into(), Some(9), String::new(), false)?;
    std::thread::sleep(Duration::from_secs(2));
    engine.shutdown();
    Ok(())
}
