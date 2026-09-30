//! Writes a slowed-down copy of a clip.
//!
//!   cargo run -p geniusclip-engine --example slowmo -- <in.mp4> <out.mp4> [factor]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let factor = args.get(3).and_then(|f| f.parse().ok()).unwrap_or(2);
    let t = std::time::Instant::now();
    geniusclip_engine::remix::slow_down(args[1].as_ref(), args[2].as_ref(), factor)?;
    println!("slowed {factor}x in {:.1?}", t.elapsed());
    Ok(())
}
