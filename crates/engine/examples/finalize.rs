//! Makes a copy of a fragmented recording a regular MP4.
//!
//!   cargo run -p geniusclip-engine --example finalize -- <in.mp4> <out.mp4> [live]
//!
//! `live` copies the file in small pieces and indexes it as it grows, the
//! way a recording is indexed while it is written.

use geniusclip_engine::finalize::{finalize_file, Index};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (src, dst) = (&args[1], &args[2]);
    let live = args.get(3).is_some_and(|a| a == "live");
    let started = Instant::now();
    if live {
        let mut input = File::open(src)?;
        let mut out = File::create(dst)?;
        let mut reader = File::open(dst)?;
        let mut index = Index::new();
        // Odd-sized pieces, so boxes are cut at every possible place.
        let mut buf = vec![0u8; 777_777];
        let mut steps = 0;
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            out.flush()?;
            index.advance(&mut reader)?;
            steps += 1;
        }
        drop(out);
        drop(reader);
        let mut f = OpenOptions::new().read(true).write(true).open(dst)?;
        index.advance(&mut f)?;
        let t = Instant::now();
        index.finish(&mut f)?;
        println!("live: {steps} steps, finish took {:.1?}", t.elapsed());
    } else {
        std::fs::copy(src, dst)?;
        let copied = started.elapsed();
        let mut f = OpenOptions::new().read(true).write(true).open(dst)?;
        let t = Instant::now();
        let outcome = finalize_file(&mut f)?;
        println!("copy {copied:.1?}, {outcome:?} in {:.1?}", t.elapsed());
    }
    Ok(())
}
