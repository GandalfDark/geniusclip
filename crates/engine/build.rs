//! Copies the FFmpeg DLLs next to the built binaries so `cargo run`, examples
//! and `tauri dev` can load them without touching PATH.

use std::{env, fs, path::PathBuf};

const DLLS: &[&str] = &[
    "avcodec-63.dll",
    "avformat-63.dll",
    "avutil-61.dll",
    "swresample-7.dll",
    "swscale-10.dll",
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ffmpeg_bin = manifest.join("../../third_party/ffmpeg/bin");
    // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let profile_dir = out.ancestors().nth(3).unwrap().to_path_buf();

    for dir in [profile_dir.clone(), profile_dir.join("examples")] {
        let _ = fs::create_dir_all(&dir);
        for dll in DLLS {
            let src = ffmpeg_bin.join(dll);
            let dst = dir.join(dll);
            let stale = match (fs::metadata(&src), fs::metadata(&dst)) {
                (Ok(s), Ok(d)) => s.len() != d.len(),
                (Ok(_), Err(_)) => true,
                _ => false,
            };
            if stale {
                fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy {dll}: {e}"));
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../third_party/ffmpeg/bin");
}
