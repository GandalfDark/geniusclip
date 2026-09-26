//! Runs the microphone check for a few seconds at an inaudible volume and
//! prints the before/after peak levels (dBFS).
use geniusclip_engine::*;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let engine = Engine::new(|_| {});
    const VOL: f32 = 0.001;
    engine.configure(EngineConfig { mic_volume: VOL, noise_suppression: true, noise_strength: 100, ..Default::default() }, 30)?;
    let db = |x: f32| if x > 0.0 { 20.0 * (x / VOL).log10() } else { -120.0 };
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n2 = n.clone();
    log::info!("starting monitor");
    engine.start_mic_monitor(move |e| match e {
        MonitorEvent::Level { before, after } => {
            if n2.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 10 == 0 {
                println!("before {:6.1} dB   after {:6.1} dB", db(before), db(after));
            }
        }
        MonitorEvent::Stopped { error } => println!("stopped: {error:?}"),
    })?;
    std::thread::sleep(Duration::from_secs(4));
    engine.stop_mic_monitor();
    println!("level callbacks: {}", n.load(std::sync::atomic::Ordering::Relaxed));
    engine.shutdown();
    Ok(())
}
