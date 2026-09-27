//! Dumps frames of a clip as PNGs: `frames <clip.mp4> <out dir> <count>`.
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (clip, out, n) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), a[3].parse::<u32>()?);
    std::fs::create_dir_all(&out)?;
    let info = geniusclip_engine::media::probe(&clip)?;
    println!("duration {:.1}s {}x{}", info.duration, info.width, info.height);
    for i in 0..n {
        let t = info.duration * (i as f64 + 0.5) / n as f64;
        geniusclip_engine::media::thumbnail(&clip, &out.join(format!("{i:02}-{t:05.1}s.png")), 480, t)?;
    }
    Ok(())
}
