//! cargo run -p geniusclip-engine --example remix_test -- <clip.mp4> <out_dir> [gains]
//! Extracts the audio tracks and waveform of a clip, then trims 1–6 s with
//! the given per-track gains (default: mic muted) into <out_dir>/mixed.mp4.
use geniusclip_engine::remix;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let clip = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out)?;
    let titles = remix::audio_tracks(&clip)?;
    println!("tracks: {titles:?} mix layout: {}", remix::is_mix_layout(&titles));
    let stem = clip.file_stem().unwrap().to_string_lossy().to_string();
    println!("extracted: {:?}", remix::extract_tracks(&clip, &out, &stem)?);
    let t = std::time::Instant::now();
    let skip: &[usize] = if remix::is_mix_layout(&titles) { &[0] } else { &[] };
    let peaks = remix::peaks(&clip, 1200, skip)?;
    println!("peaks in {:?}", t.elapsed());
    std::fs::write(out.join(format!("{stem}.peaks.json")), serde_json::to_vec(&peaks)?)?;
    let gains: Vec<f32> = args.get(3).map(|s| s.split(',').map(|g| g.parse().unwrap()).collect()).unwrap_or(vec![1.0, 1.0, 0.0]);
    remix::trim_with_gains(&clip, &out.join("mixed.mp4"), 1.0, 6.0, &gains)?;
    println!("mixed.mp4 written");
    Ok(())
}
