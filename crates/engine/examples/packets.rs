//! Summarizes a clip's video packets (a tiny ffprobe): duration, frames per
//! second of video, gaps and keyframe spacing. `packets <clip.mp4>`
use ffmpeg_sys_next as ff;
use std::ffi::CString;
use std::ptr;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("clip path");
    let info = geniusclip_engine::media::probe(path.as_ref())?;
    println!("probe: duration {:.3} s, {}x{}, {:.2} fps, {} audio track(s)", info.duration, info.width, info.height, info.fps, info.audio_tracks);
    unsafe {
        let mut ic = ptr::null_mut();
        let c = CString::new(path)?;
        anyhow::ensure!(ff::avformat_open_input(&mut ic, c.as_ptr(), ptr::null(), ptr::null_mut()) >= 0, "open");
        anyhow::ensure!(ff::avformat_find_stream_info(ic, ptr::null_mut()) >= 0, "stream info");
        let v = ff::av_find_best_stream(ic, ff::AVMediaType::AVMEDIA_TYPE_VIDEO, -1, -1, ptr::null_mut(), 0);
        anyhow::ensure!(v >= 0, "no video");
        let st = *(*ic).streams.add(v as usize);
        let tb = (*st).time_base;
        let secs = |t: i64| t as f64 * tb.num as f64 / tb.den as f64;
        let pkt = ff::av_packet_alloc();
        let (mut times, mut keys, mut last_end) = (Vec::new(), Vec::new(), 0i64);
        while ff::av_read_frame(ic, pkt) >= 0 {
            if (*pkt).stream_index == v {
                let t = secs((*pkt).pts);
                times.push(t);
                if (*pkt).flags & ff::AV_PKT_FLAG_KEY != 0 {
                    keys.push(t);
                }
                last_end = (*pkt).pts + (*pkt).duration;
            }
            ff::av_packet_unref(pkt);
        }
        let mut pkt = pkt;
        ff::av_packet_free(&mut pkt);
        println!("stream: r_frame_rate {}/{}, avg_frame_rate {}/{}", (*st).r_frame_rate.num, (*st).r_frame_rate.den, (*st).avg_frame_rate.num, (*st).avg_frame_rate.den);
        ff::avformat_close_input(&mut ic);
        anyhow::ensure!(!times.is_empty(), "no video packets");
        let (first, last) = (times[0], *times.last().unwrap());
        println!("video: {} packets from {:.3} to {:.3} s, last ends at {:.3} s", times.len(), first, last, secs(last_end));
        let gaps: Vec<f64> = times.windows(2).map(|w| w[1] - w[0]).collect();
        let max_gap = gaps.iter().cloned().fold(0.0, f64::max);
        println!("largest gap between frames {:.0} ms", max_gap * 1000.0);
        // Frames per whole second of the video.
        let n = (secs(last_end)).ceil() as usize;
        let mut per = vec![0u32; n.max(1)];
        for t in &times {
            let i = (t.max(0.0) as usize).min(per.len() - 1);
            per[i] += 1;
        }
        println!("frames per second: {per:?}");
        let spacing: Vec<String> = keys.windows(2).map(|w| format!("{:.2}", w[1] - w[0])).collect();
        println!("{} keyframes at {:?}", keys.len(), keys.iter().map(|k| format!("{k:.2}")).collect::<Vec<_>>());
        println!("keyframe spacing (s): {}", spacing.join(" "));
    }
    Ok(())
}
