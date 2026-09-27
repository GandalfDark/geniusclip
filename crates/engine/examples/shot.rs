//! One desktop screenshot through Desktop Duplication: `shot <out.png>`.
//! Used to check how capture-excluded windows come out.
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let out = std::env::args().nth(1).expect("output path");
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = geniusclip_engine::Engine::new(move |e| {
        let _ = tx.send(format!("{e:?}"));
    });
    engine.screenshot(out.into())?;
    println!("{}", rx.recv_timeout(Duration::from_secs(10))?);
    Ok(())
}
