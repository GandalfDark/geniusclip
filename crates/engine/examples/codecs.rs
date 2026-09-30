//! Lists the codecs the GPU can encode (opens small encoders, no capture).
//!
//!   cargo run -p geniusclip-engine --example codecs -- [monitor]
fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let monitor = std::env::args().nth(1);
    let t = std::time::Instant::now();
    let codecs = geniusclip_engine::supported_codecs(monitor.as_deref())?;
    println!("{codecs:?} in {:.0?}", t.elapsed());
    Ok(())
}
