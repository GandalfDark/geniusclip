//! Microphone noise suppression with DeepFilterNet 3 (48 kHz, 10 ms hops).
//!
//! The model is not `Send`, so it lives on its own thread: the capture
//! thread hands over resampled chunks and the denoise thread writes the
//! result into the microphone ring. Loading the model (~0.3 s) therefore
//! never stalls capture. On/off and strength are read live per chunk, so
//! changing them does not restart the pipeline or clear the replay buffer.
//!
//! Output that arrives after the mixer has read that time is dropped, so a
//! thread that falls behind (a game using every core) passes the audio
//! through unprocessed until it has caught up, rather than losing the mic.

use crate::audio::RATE;
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL};

/// Chunks queued for the denoise thread (WASAPI packets, usually 10 ms).
/// When full, the capture thread writes the audio itself, unprocessed.
const QUEUE: usize = 64;
/// A chunk that waited longer is passed through: denoised it would reach
/// the ring too late (model latency 30 ms, mixer ≈ 125 ms, mic check 90 ms).
const MAX_WAIT: Duration = Duration::from_millis(40);

/// Live settings, shared with whoever feeds microphone audio.
#[derive(Default)]
pub struct DenoiseControl {
    enabled: AtomicBool,
    strength: AtomicU32,
    /// Set when the model could not be loaded (audio then passes through).
    failed: AtomicBool,
}

impl DenoiseControl {
    pub fn set(&self, enabled: bool, strength: u32) {
        self.strength.store(strength.min(100), Ordering::Relaxed);
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn get(&self) -> (bool, u32) {
        (self.enabled.load(Ordering::Relaxed), self.strength.load(Ordering::Relaxed))
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// Peak levels since the last `take`, for meters.
#[derive(Default)]
pub struct Levels {
    before: AtomicU32,
    after: AtomicU32,
}

impl Levels {
    fn raise(slot: &AtomicU32, samples: &[f32]) {
        let peak = samples.iter().fold(0f32, |m, x| m.max(x.abs()));
        let _ = slot.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |b| (peak > f32::from_bits(b)).then_some(peak.to_bits()));
    }

    /// (before, after) peaks since the previous call.
    pub fn take(&self) -> (f32, f32) {
        (f32::from_bits(self.before.swap(0, Ordering::Relaxed)), f32::from_bits(self.after.swap(0, Ordering::Relaxed)))
    }
}

/// Strength 0..=100 → attenuation limit in dB. Low values keep a little
/// background (more natural voice); 100 removes all noise.
pub fn atten_db(strength: u32) -> f32 {
    if strength >= 100 {
        100.0
    } else {
        6.0 + strength as f32 * 0.44
    }
}

/// Streaming denoiser on the shared 48 kHz timeline.
pub struct Denoiser {
    model: DfTract,
    hop: usize,
    /// Model latency in samples; output timestamps are shifted back by it.
    delay: i64,
    atten: f32,
    /// Timeline index of input sample 0 (re-based after large gaps).
    base: Option<i64>,
    fed: i64,
    produced: i64,
    pending: Vec<f32>,
    inp: Array2<f32>,
    outp: Array2<f32>,
}

impl Denoiser {
    pub fn new(strength: u32) -> Result<Denoiser> {
        let atten = atten_db(strength);
        let model = DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(1).with_atten_lim(atten))?;
        anyhow::ensure!(model.sr as i64 == RATE, "model sample rate {}", model.sr);
        let hop = model.hop_size;
        Ok(Denoiser {
            // Measured: output lags input by (lookahead + 1) hops = 30 ms.
            delay: (hop * (model.lookahead + 1)) as i64,
            hop,
            model,
            atten,
            base: None,
            fed: 0,
            produced: 0,
            pending: Vec::with_capacity(hop),
            inp: Array2::zeros((1, hop)),
            outp: Array2::zeros((1, hop)),
        })
    }

    /// Continues the stream at `idx` after chunks bypassed the model, rather
    /// than feeding the skipped time to it as silence.
    pub fn resume_at(&mut self, idx: i64) {
        if self.base.is_some() {
            self.base = Some(idx - self.fed);
        }
    }

    pub fn set_strength(&mut self, strength: u32) {
        let a = atten_db(strength);
        if a != self.atten {
            self.model.set_atten_lim(a);
            self.atten = a;
        }
    }

    /// Feeds interleaved stereo samples whose first sample is at timeline
    /// `idx`. Replaces `out` with denoised stereo and returns the timeline
    /// index of its first sample (None when a full hop is not ready yet).
    pub fn process(&mut self, idx: i64, stereo: &[f32], out: &mut Vec<f32>) -> Option<i64> {
        out.clear();
        let base = *self.base.get_or_insert(idx);
        let gap = idx - (base + self.fed);
        if gap.abs() > RATE / 2 {
            // Long pause or clock jump: continue the stream from here.
            self.base = Some(idx - self.fed);
        }
        let start = self.base.unwrap() + self.produced - self.delay;
        if gap > RATE / 40 && gap <= RATE / 2 {
            // Short hole (dropped packets): keep the timeline with silence.
            for _ in 0..gap {
                self.push(0.0, out);
            }
        }
        for f in stereo.chunks_exact(2) {
            self.push((f[0] + f[1]) * 0.5, out);
        }
        (!out.is_empty()).then_some(start)
    }

    fn push(&mut self, x: f32, out: &mut Vec<f32>) {
        self.pending.push(x);
        self.fed += 1;
        if self.pending.len() < self.hop {
            return;
        }
        for (d, s) in self.inp.iter_mut().zip(&self.pending) {
            *d = *s;
        }
        if let Err(e) = self.model.process(self.inp.view(), self.outp.view_mut()) {
            log::warn!("denoise: {e:#}");
            self.outp.assign(&self.inp);
        }
        for &y in self.outp.iter() {
            out.push(y);
            out.push(y);
        }
        self.produced += self.hop as i64;
        self.pending.clear();
    }
}

/// A chunk of microphone audio: timeline index, interleaved stereo, and
/// when it was queued.
pub type Chunk = (i64, Vec<f32>, Instant);

/// Runs a denoise thread. Every chunk sent to the returned channel (up to
/// `QUEUE`) comes back through `write` — denoised while enabled and keeping
/// up, untouched otherwise. The thread ends when the sender is dropped.
pub fn spawn(control: Arc<DenoiseControl>, levels: Option<Arc<Levels>>, write: impl Fn(i64, &[f32]) + Send + 'static) -> std::io::Result<Sender<Chunk>> {
    let (tx, rx): (Sender<Chunk>, Receiver<Chunk>) = crossbeam_channel::bounded(QUEUE);
    std::thread::Builder::new().name("gc-denoise".into()).spawn(move || {
        // Like the encoder: above the game's threads, as late output is lost.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }
        let mut model: Option<Denoiser> = None;
        let mut failed = false;
        let mut out = Vec::new();
        // Behind: chunks pass through until the queue has drained. Skipped:
        // the model missed chunks and continues from the next one it gets.
        let (mut behind, mut skipped) = (false, false);
        // What queued up while the model loaded is expected: no warning.
        let mut load_backlog = false;
        let mut warned: Option<Instant> = None;
        for (idx, samples, queued) in rx.iter() {
            let was_behind = behind;
            behind = queued.elapsed() > MAX_WAIT || (behind && !rx.is_empty());
            load_backlog &= behind;
            let (on, strength) = control.get();
            if on && model.is_none() && !failed {
                match Denoiser::new(strength) {
                    Ok(d) => {
                        model = Some(d);
                        load_backlog = true;
                    }
                    Err(e) => {
                        log::error!("noise suppression unavailable: {e:#}");
                        control.failed.store(true, Ordering::Relaxed);
                        failed = true;
                    }
                }
            }
            let quiet = load_backlog || warned.is_some_and(|t| t.elapsed() < Duration::from_secs(60));
            if on && behind && !was_behind && model.is_some() && !quiet {
                log::warn!("noise suppression fell behind, passing the microphone through until it catches up");
                warned = Some(Instant::now());
            }
            let processed = match model.as_mut().filter(|_| on && !behind) {
                Some(d) => {
                    if std::mem::take(&mut skipped) {
                        d.resume_at(idx);
                    }
                    d.set_strength(strength);
                    let at = d.process(idx, &samples, &mut out);
                    if let Some(at) = at {
                        write(at, &out);
                    }
                    &out[..]
                }
                None => {
                    skipped = model.is_some();
                    write(idx, &samples);
                    &samples[..]
                }
            };
            if let Some(l) = &levels {
                Levels::raise(&l.before, &samples);
                Levels::raise(&l.after, processed);
            }
        }
    })?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads the embedded model (also catches tract rejecting it in debug
    /// builds) and checks that pure noise is removed while a tone survives.
    #[test]
    fn model_loads_and_removes_noise() {
        let mut d = Denoiser::new(100).expect("model loads");
        let mut out = Vec::new();
        let mut seed = 7u32;
        let (mut noise_in, mut noise_out) = (0f64, 0f64);
        for block in 0..300 {
            let stereo: Vec<f32> = (0..480)
                .flat_map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let x = (seed as f32 / u32::MAX as f32 - 0.5) * 0.05;
                    [x, x]
                })
                .collect();
            d.process(block * 480, &stereo, &mut out);
            if block > 50 {
                noise_in += stereo.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
                noise_out += out.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
            }
        }
        let reduction_db = 10.0 * (noise_in / noise_out.max(1e-12)).log10();
        assert!(reduction_db > 20.0, "noise only reduced by {reduction_db:.1} dB");
    }
}
