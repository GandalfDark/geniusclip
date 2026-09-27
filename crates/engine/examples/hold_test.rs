//! Checks `Engine::set_hold`: video only (no audio devices are opened),
//! 3 s live, 3 s held, 3 s live, then saves the replay:
//! `hold_test <out.mp4> [cfr] [\\.\DISPLAYn]` (`cfr`: constant frame rate).
use geniusclip_engine::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    env_logger::init();
    let out = std::env::args().nth(1).expect("output path");
    let rest: Vec<String> = std::env::args().skip(2).collect();
    let constant_fps = rest.iter().any(|a| a == "cfr");
    let monitor = rest.iter().find(|a| a.starts_with(r"\\.\")).cloned();
    // When the save was asked for, to time it.
    let asked: Arc<Mutex<Option<Instant>>> = Arc::default();
    let asked2 = asked.clone();
    let engine = Engine::new(move |e| {
        if !matches!(e, EngineEvent::Status(_)) {
            let after = asked2.lock().unwrap().map(|t| format!(" ({} ms after the request)", t.elapsed().as_millis()));
            println!("{e:?}{}", after.unwrap_or_default());
        }
    });
    engine.configure(EngineConfig { mic: false, system_audio: false, fps: 30, constant_fps, monitor, ..Default::default() }, 30)?;
    engine.set_replay_enabled(true)?;
    std::thread::sleep(Duration::from_secs(3));
    println!("live: status fps {:.0}", engine.status().fps);
    engine.set_hold(Some(HoldCard {
        title: "Открыто меню GeniusClip".into(),
        subtitle: "Видео продолжится, когда меню закроется".into(),
        accent: [139, 92, 246],
    }));
    println!("hold on");
    std::thread::sleep(Duration::from_secs(3));
    println!("held: status fps {:.0}", engine.status().fps);
    engine.set_hold(None);
    println!("hold off");
    std::thread::sleep(Duration::from_secs(3));
    let s = engine.status();
    println!("fps {:.0} dropped {}", s.fps, s.dropped_frames);
    *asked.lock().unwrap() = Some(Instant::now());
    engine.save_replay(out.into(), Some(9), String::new(), false)?;
    std::thread::sleep(Duration::from_secs(2));
    engine.shutdown();
    Ok(())
}
