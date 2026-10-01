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
//!
//! The model sleeps through silence (see `Gate`) and while the microphone
//! is muted, which is most of the time for most people.

use crate::audio::RATE;
use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use df::tract::{DfParams, DfTract, RuntimeParams};
use df::Complex32;
use ndarray::Array2;
use std::collections::VecDeque;
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

/// What the model leaves of pure noise: the attenuation limit as a gain
/// (none at all at 100 dB, which the model treats as "no limit").
fn limit_gain(atten_db: f32) -> f32 {
    if atten_db >= 100.0 {
        0.0
    } else {
        10f32.powf(-atten_db / 20.0)
    }
}

/// Mean power of a hop in dB (full scale = 0 dB).
fn energy_db(samples: &[f32]) -> f32 {
    let power = samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32;
    10.0 * (power + 1e-10).log10()
}

/// The model's local SNR estimate (dB) below which it hears only noise: it
/// then applies a zero mask, so its output is the input at the attenuation
/// limit.
const NOISE_LSNR: f32 = -10.0;
/// Noise-only hops in a row before the model sleeps (≈300 ms).
const SLEEP_AFTER: u32 = 30;
/// After a wake by a short sound the model called noise (a key, a mouse
/// click): quiet noise-only hops before it sleeps again. Typing or a game's
/// keys would otherwise keep it awake (a click every <300 ms resets the
/// count above), though it only ever outputs them at the limit, as asleep.
const RESLEEP_AFTER: u32 = 3;
/// A hop this much louder (dB) than the noise floor wakes the model.
const WAKE_DB: f32 = 6.0;
/// Noise-only hops must also be within this (dB) of the floor to count
/// towards sleeping, so loud sounds the model calls noise keep it awake.
const QUIET_DB: f32 = 3.0;
/// Hops replayed into a waking model, output discarded, so its buffers and
/// recurrent state hold the audio just before the wake instead of the
/// moment it fell asleep.
const PREROLL: usize = 5;

/// Silence gate: the model costs the same for silence as for speech, and a
/// microphone is silent most of the time. While it sleeps, the output is
/// the input at the attenuation limit, delayed like the model's output,
/// which is what the model itself gives for noise.
#[derive(Default)]
struct Gate {
    asleep: bool,
    /// Noise-only hops in a row while awake.
    quiet: u32,
    /// Running noise floor (hop energy, dB): hops the model called noise
    /// and, while asleep, every hop that did not wake it.
    floor: Option<f32>,
    /// Woke and has heard only noise since: a key or a click woke it.
    only_noise: bool,
}

impl Gate {
    /// After the model ran on a hop of `energy` dB and estimated `lsnr`.
    /// True when it can sleep from the next hop on.
    fn heard(&mut self, lsnr: f32, energy: f32) -> bool {
        if lsnr < NOISE_LSNR {
            let floor = self.track(energy);
            self.quiet = if energy < floor + QUIET_DB { self.quiet + 1 } else { 0 };
        } else {
            self.quiet = 0;
            self.only_noise = false;
        }
        if self.quiet >= SLEEP_AFTER || (self.only_noise && self.quiet >= RESLEEP_AFTER) {
            self.asleep = true;
            self.quiet = 0;
        }
        self.asleep
    }

    /// While asleep: true when a hop of `energy` dB wakes the model.
    fn wakes(&mut self, energy: f32) -> bool {
        if self.floor.is_none_or(|f| energy > f + WAKE_DB) {
            self.wake();
            return true;
        }
        self.track(energy);
        false
    }

    fn wake(&mut self) {
        self.asleep = false;
        self.quiet = 0;
        self.only_noise = true;
    }

    /// Follows falling noise quickly and rising noise slowly, so a burst of
    /// keyboard noise hardly lifts the floor a wake is measured against.
    fn track(&mut self, energy: f32) -> f32 {
        let floor = match self.floor {
            None => energy,
            Some(f) => f + (energy - f) * if energy < f { 0.2 } else { 0.02 },
        };
        self.floor = Some(floor);
        floor
    }
}

/// Streaming denoiser on the shared 48 kHz timeline.
pub struct Denoiser {
    model: DfTract,
    hop: usize,
    /// Model latency in samples; output timestamps are shifted back by it.
    delay: i64,
    atten: f32,
    /// `limit_gain(atten)`: what is left of the input while the model sleeps.
    lim: f32,
    /// Timeline index of input sample 0 (re-based after large gaps).
    base: Option<i64>,
    fed: i64,
    produced: i64,
    pending: Vec<f32>,
    /// The last input samples (mono, the last one is number `fed - 1`),
    /// `delay + hop` at least: what the model has not output yet when it is
    /// bypassed, the input side of a fade, and the pre-roll of a wake.
    raw: VecDeque<f32>,
    raw_cap: usize,
    /// Output samples still to drop: the model's latency holds audio from
    /// before it started or was bypassed (that time is already written).
    discard: usize,
    /// Output samples left in the fade from the input (times `fade_from`)
    /// to the model's output: 1 after a start or resume (unprocessed), the
    /// limit after a wake (what sleeping gave), 0 after muted silence.
    fade: usize,
    fade_from: f32,
    gate: Gate,
    /// Off: the model runs on every hop (tests compare against that).
    gated: bool,
    /// Hops the model ran on (pre-roll included).
    calls: u64,
    inp: Array2<f32>,
    outp: Array2<f32>,
    /// Scratch for `track_noise`: spectrum and features of a hop.
    spec: Vec<Complex32>,
    erb_feat: Vec<f32>,
    cplx_feat: Vec<Complex32>,
}

impl Denoiser {
    pub fn new(strength: u32) -> Result<Denoiser> {
        let atten = atten_db(strength);
        let model = DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(1).with_atten_lim(atten))?;
        anyhow::ensure!(model.sr as i64 == RATE, "model sample rate {}", model.sr);
        let hop = model.hop_size;
        // Measured: output lags input by (lookahead + 1) hops = 30 ms.
        let delay = hop * (model.lookahead + 1);
        let raw_cap = (delay + hop).max((PREROLL + 1) * hop);
        let (n_freqs, nb_erb, nb_df) = (model.n_freqs, model.nb_erb, model.nb_df);
        Ok(Denoiser {
            spec: vec![Complex32::default(); n_freqs],
            erb_feat: vec![0.0; nb_erb],
            cplx_feat: vec![Complex32::default(); nb_df],
            delay: delay as i64,
            hop,
            model,
            atten,
            lim: limit_gain(atten),
            base: None,
            fed: 0,
            produced: 0,
            pending: Vec::with_capacity(hop),
            raw: VecDeque::with_capacity(raw_cap),
            raw_cap,
            // Output before the first input is the model's warm-up.
            discard: delay,
            fade: hop,
            fade_from: 1.0,
            gate: Gate::default(),
            gated: true,
            calls: 0,
            inp: Array2::zeros((1, hop)),
            outp: Array2::zeros((1, hop)),
        })
    }

    /// The model is bypassed from the next chunk on (behind, or turned off).
    /// Replaces `out` with what it took in but has not output yet — its
    /// latency plus a partial hop — unprocessed, and returns its timeline
    /// index, so the switch leaves no silent gap.
    pub fn bypass(&mut self, out: &mut Vec<f32>) -> Option<i64> {
        out.clear();
        let base = self.base?;
        // First input sample whose output has not been written.
        let from = (self.produced + self.discard as i64 - self.delay).max(self.fed - self.raw.len() as i64).max(0);
        for k in from..self.fed {
            let x = self.raw[self.raw.len() - (self.fed - k) as usize];
            out.push(x);
            out.push(x);
        }
        (!out.is_empty()).then_some(base + from)
    }

    /// Continues the stream at `idx` after chunks bypassed the model, rather
    /// than feeding the skipped time to it as silence. `from`: the level
    /// (gain on the input) of what was written meanwhile, faded out.
    pub fn resume_at(&mut self, idx: i64, from: f32) {
        // Input from before the bypass went out unprocessed (see `bypass`):
        // processing it now, as if it ran on into the new input, would click.
        self.fed -= self.pending.len() as i64;
        self.pending.clear();
        self.raw.clear();
        if self.base.is_some() {
            self.base = Some(idx - self.fed);
        }
        // The model still holds `delay` samples from before; then fade in.
        self.discard = self.delay as usize;
        self.fade = self.hop;
        self.fade_from = from;
        // Its state is stale anyway: let it judge the new audio itself.
        self.gate.wake();
    }

    pub fn set_strength(&mut self, strength: u32) {
        let a = atten_db(strength);
        if a != self.atten {
            self.model.set_atten_lim(a);
            self.atten = a;
            self.lim = limit_gain(a);
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
        let mut start = None;
        if gap > RATE / 40 && gap <= RATE / 2 {
            // Short hole (dropped packets): keep the timeline with silence.
            for _ in 0..gap {
                self.push(0.0, out, &mut start);
            }
        }
        for f in stereo.chunks_exact(2) {
            self.push((f[0] + f[1]) * 0.5, out, &mut start);
        }
        start
    }

    /// Feeds one sample; a full hop is processed and appended to `out`
    /// (`start` gets the timeline index of the first sample appended).
    fn push(&mut self, x: f32, out: &mut Vec<f32>, start: &mut Option<i64>) {
        self.pending.push(x);
        if self.raw.len() == self.raw_cap {
            self.raw.pop_front();
        }
        self.raw.push_back(x);
        self.fed += 1;
        if self.pending.len() < self.hop {
            return;
        }
        let energy = energy_db(&self.pending);
        let mut run = true;
        if self.gate.asleep {
            run = self.gate.wakes(energy);
            if run {
                self.preroll();
                // From what sleeping gave (the input at the limit) to the model.
                self.fade = self.hop;
                self.fade_from = self.lim;
            } else {
                self.track_noise();
            }
        }
        if run {
            for (d, s) in self.inp.iter_mut().zip(&self.pending) {
                *d = *s;
            }
            let lsnr = match self.model.process(self.inp.view(), self.outp.view_mut()) {
                Ok(lsnr) => lsnr,
                Err(e) => {
                    log::warn!("denoise: {e:#}");
                    self.outp.assign(&self.inp);
                    0.0
                }
            };
            self.calls += 1;
            if self.gated {
                self.gate.heard(lsnr, energy);
            }
        }
        for i in 0..self.hop {
            if self.discard > 0 {
                self.discard -= 1;
                continue;
            }
            // Output number n is input sample n - delay.
            let k = self.produced + i as i64 - self.delay;
            start.get_or_insert(self.base.unwrap() + k);
            let back = (self.fed - k) as usize;
            let x = (1..=self.raw.len()).contains(&back).then(|| self.raw[self.raw.len() - back]);
            // Asleep: what the model gives for noise, through the same latency.
            let mut y = if run { self.outp[[0, i]] } else { x.unwrap_or(0.0) * self.lim };
            if self.fade > 0 {
                // From the input (at `fade_from`) to the model's output, so the
                // model's start, restart or wake does not click.
                if let Some(x) = x {
                    let x = x * self.fade_from;
                    let w = 1.0 - self.fade as f32 / self.hop as f32;
                    y = x + (y - x) * w;
                }
                self.fade -= 1;
            }
            out.push(y);
            out.push(y);
        }
        self.produced += self.hop as i64;
        self.pending.clear();
    }

    /// Asleep: feeds the hop to the model's feature normalization only (a
    /// small FFT, no network). Its running means (≈1 s time constant) would
    /// otherwise wake up as they were when the model fell asleep rather
    /// than as an always-running model would have them.
    fn track_noise(&mut self) {
        let alpha = self.model.alpha;
        let nb_df = self.model.nb_df;
        let st = &mut self.model.df_states[0];
        st.analysis(&self.pending, &mut self.spec);
        st.feat_erb(&self.spec, alpha, &mut self.erb_feat);
        st.feat_cplx(&self.spec[..nb_df], alpha, &mut self.cplx_feat);
    }

    /// Replays the hops before the current one into the model (output
    /// discarded), so it wakes with the recent past in its buffers.
    fn preroll(&mut self) {
        let n = (self.raw.len() / self.hop).saturating_sub(1).min(PREROLL);
        let first = self.raw.len() - (n + 1) * self.hop;
        for h in 0..n {
            for (j, d) in self.inp.iter_mut().enumerate() {
                *d = self.raw[first + h * self.hop + j];
            }
            if let Err(e) = self.model.process(self.inp.view(), self.outp.view_mut()) {
                log::warn!("denoise: {e:#}");
            }
            self.calls += 1;
        }
    }
}

/// A chunk of microphone audio: timeline index, interleaved stereo, and
/// when it was queued.
pub type Chunk = (i64, Vec<f32>, Instant);

/// Runs a denoise thread. Every chunk sent to the returned channel (up to
/// `QUEUE`) comes back through `write` — denoised while enabled and keeping
/// up, untouched otherwise. While `muted()` (the microphone is muted or at
/// volume 0) and suppression is on, the model rests and silence is written:
/// the mixer applies the volume only when it reads, ≈125 ms later, so
/// unprocessed audio would be heard just after unmuting. The thread ends
/// when the sender is dropped.
/// How long the model stays loaded with suppression off.
const MODEL_IDLE: Duration = Duration::from_secs(30);

pub fn spawn(
    control: Arc<DenoiseControl>,
    levels: Option<Arc<Levels>>,
    muted: impl Fn() -> bool + Send + 'static,
    write: impl Fn(i64, &[f32]) + Send + 'static,
) -> std::io::Result<Sender<Chunk>> {
    let (tx, rx): (Sender<Chunk>, Receiver<Chunk>) = crossbeam_channel::bounded(QUEUE);
    std::thread::Builder::new().name("gc-denoise".into()).spawn(move || {
        // Like the encoder: above the game's threads, as late output is lost.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }
        let mut model: Option<Denoiser> = None;
        let mut failed = false;
        let (mut out, mut tail, mut zeros) = (Vec::new(), Vec::new(), Vec::new());
        // Behind: chunks pass through until the queue has drained. Skipped:
        // the model missed chunks and continues from the next one it gets.
        let (mut behind, mut skipped) = (false, false);
        // The last chunk written was silence for a muted microphone.
        let mut silenced = false;
        // What queued up while the model loaded is expected: no warning.
        let mut load_backlog = false;
        let mut warned: Option<Instant> = None;
        // Suppression is off (chunks arrive only for the mic check then, see
        // `capture_thread`): the model is let go after a while.
        let mut off_since: Option<Instant> = None;
        loop {
            let (idx, samples, queued) = match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(chunk) => chunk,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if control.get().0 {
                        off_since = None;
                    } else if model.is_some() && off_since.get_or_insert_with(Instant::now).elapsed() > MODEL_IDLE {
                        log::info!("noise suppression model unloaded (off)");
                        model = None;
                        skipped = false;
                        off_since = None;
                    }
                    continue;
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            let was_behind = behind;
            behind = queued.elapsed() > MAX_WAIT || (behind && !rx.is_empty());
            load_backlog &= behind;
            let (on, strength) = control.get();
            let mute = on && muted();
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
            if on && !mute && behind && !was_behind && model.is_some() && !quiet {
                log::warn!("noise suppression fell behind, passing the microphone through until it catches up");
                warned = Some(Instant::now());
            }
            let processed = match model.as_mut().filter(|_| on && !mute && !behind) {
                Some(d) => {
                    if std::mem::take(&mut skipped) {
                        // Fade in from what was written meanwhile.
                        d.resume_at(idx, if silenced { 0.0 } else { 1.0 });
                    }
                    silenced = false;
                    d.set_strength(strength);
                    let at = d.process(idx, &samples, &mut out);
                    if let Some(at) = at {
                        write(at, &out);
                    }
                    &out[..]
                }
                None => {
                    if let Some(d) = model.as_mut() {
                        // First chunk past the model: what it still holds goes
                        // out unprocessed instead of leaving a 30-40 ms hole.
                        if !std::mem::replace(&mut skipped, true) {
                            if let Some(at) = d.bypass(&mut tail) {
                                write(at, &tail);
                            }
                        }
                    }
                    silenced = mute;
                    if mute {
                        zeros.clear();
                        zeros.resize(samples.len(), 0.0);
                        write(idx, &zeros);
                        &zeros[..]
                    } else {
                        write(idx, &samples);
                        &samples[..]
                    }
                }
            };
            if let Some(l) = &levels {
                Levels::raise(&l.before, &samples);
                Levels::raise(&l.after, processed);
            }
        }
        if let Some(d) = model.filter(|d| d.produced > 0) {
            let hops = d.produced / d.hop as i64;
            log::info!("noise suppression: the model ran on {:.0}% of {hops} hops", d.calls as f64 * 100.0 / hops as f64);
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

    /// Output, the unprocessed tail at a bypass and the output after a
    /// resume follow each other on the timeline without holes or overlaps.
    #[test]
    fn bypass_and_resume_keep_the_timeline() {
        let mut d = Denoiser::new(50).expect("model loads");
        let mut out = Vec::new();
        // Not a multiple of the hop: a partial hop is pending at the bypass.
        let n = 441i64;
        let chunk = vec![0.01f32; n as usize * 2];
        let (mut idx, mut end) = (1000i64, None);
        let feed = |d: &mut Denoiser, idx: i64, end: &mut Option<i64>, out: &mut Vec<f32>| {
            if let Some(at) = d.process(idx, &chunk, out) {
                assert_eq!(at, end.unwrap_or(at), "output continues where it ended");
                *end = Some(at + out.len() as i64 / 2);
            }
        };
        for _ in 0..20 {
            feed(&mut d, idx, &mut end, &mut out);
            if end.is_none() {
                assert!(out.is_empty());
            }
            idx += n;
        }
        assert!(end.is_some());
        let at = d.bypass(&mut out).expect("tail");
        assert_eq!(Some(at), end);
        assert_eq!(at + out.len() as i64 / 2, idx, "tail reaches the end of the input");
        // Passed through meanwhile, then back to the model.
        idx += 10 * n;
        d.resume_at(idx, 1.0);
        end = Some(idx);
        for _ in 0..20 {
            feed(&mut d, idx, &mut end, &mut out);
            idx += n;
        }
        // Behind the input by the model's latency and at most a hop.
        assert!(end.unwrap() >= idx - d.delay - d.hop as i64, "output resumed");
    }

    /// The gate's decisions from (lsnr, energy) per hop.
    #[test]
    fn gate_state_machine() {
        let mut g = Gate::default();
        for _ in 1..SLEEP_AFTER {
            assert!(!g.heard(-15.0, -50.0));
        }
        assert!(g.heard(-15.0, -50.0), "sleeps after ~300 ms of noise");
        assert!(!g.wakes(-47.0), "noise does not wake it");
        assert!(g.wakes(-42.0), "a hop 6+ dB above the floor wakes it");
        // Woken by a short sound the model calls noise (a key): asleep again
        // as soon as it has died away.
        for _ in 1..RESLEEP_AFTER {
            assert!(!g.heard(-15.0, -50.0));
        }
        assert!(g.heard(-15.0, -50.0), "asleep again after a click");
        assert!(g.wakes(-42.0));
        // Speech: the full count after it.
        g.heard(5.0, -30.0);
        for _ in 1..SLEEP_AFTER {
            assert!(!g.heard(-15.0, -50.0));
        }
        assert!(g.heard(-15.0, -50.0));
        g.wakes(-20.0);
        // Loud sounds the model calls noise (a rustle, a fan up close) keep
        // it awake while they last, and hardly lift the floor.
        for _ in 0..2 * SLEEP_AFTER {
            assert!(!g.heard(-15.0, -30.0));
        }
        assert!(g.floor.unwrap() < -35.0, "floor {:?}", g.floor);
    }

    /// Room noise with key clicks every 120..500 ms (typing, a game's keys).
    fn typing(hops: usize, hop: usize, seed: &mut u32) -> (Vec<f32>, Vec<bool>) {
        let mut rnd = || {
            *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *seed as f32 / u32::MAX as f32
        };
        let len = hops * hop;
        let mut x: Vec<f32> = (0..len).map(|_| (rnd() - 0.5) * 0.006).collect();
        let mut click_hop = vec![false; hops];
        let mut at = (0.2 * RATE as f32) as usize;
        while at < len {
            // A 4 ms knock decaying over ~3 ms.
            for i in 0..(0.012 * RATE as f32) as usize {
                if let Some(s) = x.get_mut(at + i) {
                    *s += (rnd() - 0.5) * 0.4 * (-(i as f32) / (0.003 * RATE as f32)).exp();
                }
            }
            click_hop[at / hop] = true;
            at += ((0.12 + 0.38 * rnd()) * RATE as f32) as usize;
        }
        (x, click_hop)
    }

    /// Typing (or a game's keys) between words: the model sleeps through
    /// most of it, as each click is noise to it.
    #[test]
    fn typing_lets_the_model_sleep() {
        let mut d = Denoiser::new(80).expect("model loads");
        let hop = d.hop;
        let hops = 2000;
        let (mut input, clicks) = typing(hops, hop, &mut 7);
        // A word in the middle of it.
        let voice_at = 1000;
        for (i, v) in vowel(100 * hop).into_iter().enumerate() {
            input[voice_at * hop + i] += v;
        }
        let mut out = Vec::new();
        let mut asleep = 0;
        for (c, chunk) in input.chunks(hop).enumerate() {
            let stereo: Vec<f32> = chunk.iter().flat_map(|&x| [x, x]).collect();
            d.process((c * hop) as i64, &stereo, &mut out);
            asleep += d.gate.asleep as usize;
            if (voice_at..voice_at + 100).contains(&c) {
                assert!(!d.gate.asleep, "asleep in the word, hop {}", c - voice_at);
            }
        }
        let share = d.calls as f64 / hops as f64;
        println!("typing: model ran on {:.0}% of hops, asleep {:.0}%", share * 100.0, asleep as f64 * 100.0 / hops as f64);
        assert!(share < 0.5, "the model ran on {:.0}% of the hops", share * 100.0);
        // What the model says about click hops and the noise between them.
        let mut m = Denoiser::new(80).expect("model loads");
        let (mut on_click, mut after, mut between) = (Vec::new(), Vec::new(), Vec::new());
        for (c, chunk) in input.chunks(hop).enumerate() {
            for (dst, s) in m.inp.iter_mut().zip(chunk) {
                *dst = *s;
            }
            let lsnr = m.model.process(m.inp.view(), m.outp.view_mut()).unwrap();
            if clicks[c] {
                on_click.push(lsnr);
            } else if c > 0 && clicks[c - 1] {
                after.push(lsnr);
            } else {
                between.push(lsnr);
            }
        }
        let stats = |v: &[f32]| {
            let mut v = v.to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let below = v.iter().filter(|&&x| x < NOISE_LSNR).count() as f64 * 100.0 / v.len() as f64;
            format!("n {} median {:.1} p90 {:.1} below -10: {below:.0}%", v.len(), v[v.len() / 2], v[v.len() * 9 / 10])
        };
        println!("lsnr on a click hop: {}", stats(&on_click));
        println!("lsnr on the hop after: {}", stats(&after));
        println!("lsnr between clicks: {}", stats(&between));
    }

    /// A sung vowel ("ah"): a glottal pulse train with a gliding pitch
    /// through three formant resonators, in two syllables, ≈ -20 dBFS.
    fn vowel(len: usize) -> Vec<f32> {
        let sr = RATE as f32;
        let mut ys = [[0f32; 2]; 3];
        let (mut phase, mut out) = (0f32, Vec::with_capacity(len));
        for n in 0..len {
            let t = n as f32 / sr;
            phase += (130.0 + 25.0 * (std::f32::consts::TAU * 2.0 * t).sin()) / sr;
            let mut x = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            for (k, (f, bw)) in [(730.0f32, 90.0f32), (1090.0, 110.0), (2440.0, 170.0)].into_iter().enumerate() {
                let r = (-std::f32::consts::PI * bw / sr).exp();
                let c = 2.0 * r * (std::f32::consts::TAU * f / sr).cos();
                let y = x * (1.0 - r) + c * ys[k][0] - r * r * ys[k][1];
                ys[k] = [y, ys[k][0]];
                x = y;
            }
            // Two syllables with a short dip between them.
            let env = (std::f32::consts::PI * (t / 0.5).fract()).sin().max(0.15);
            out.push(x * env);
        }
        let rms = (out.iter().map(|x| x * x).sum::<f32>() / len as f32).sqrt();
        out.iter().map(|x| x * 0.1 / rms.max(1e-9)).collect()
    }

    /// Feeds noise, a voice-like sound and noise again through a gated and
    /// an ungated denoiser: the gated one sleeps through the noise, wakes at
    /// once for the voice, keeps the timeline, and sounds like the other.
    #[test]
    fn silence_gate_sleeps_and_wakes_cleanly() {
        let mut gated = Denoiser::new(80).expect("model loads");
        let mut plain = Denoiser::new(80).expect("model loads");
        plain.gated = false;
        let hop = gated.hop;
        let (noise_hops, voice_hops) = (200usize, 100usize);
        let total = 2 * noise_hops + voice_hops;
        let mut seed = 11u32;
        let mut input = Vec::with_capacity(total * hop);
        let voice = vowel(voice_hops * hop);
        for n in 0..total * hop {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let mut x = (seed as f32 / u32::MAX as f32 - 0.5) * 0.01;
            if let Some(v) = n.checked_sub(noise_hops * hop).and_then(|i| voice.get(i)) {
                x += v;
            }
            input.push(x);
        }
        // Timeline-indexed outputs; chunks of 10 ms like WASAPI's.
        let (mut ya, mut yb) = (vec![f32::NAN; input.len()], vec![f32::NAN; input.len()]);
        let (mut end_a, mut end_b) = (None::<i64>, None::<i64>);
        let mut asleep = Vec::new();
        let mut out = Vec::new();
        for (c, chunk) in input.chunks(hop).enumerate() {
            let stereo: Vec<f32> = chunk.iter().flat_map(|&x| [x, x]).collect();
            let at = (c * hop) as i64;
            for (d, y, end) in [(&mut gated, &mut ya, &mut end_a), (&mut plain, &mut yb, &mut end_b)] {
                if let Some(s) = d.process(at, &stereo, &mut out) {
                    assert_eq!(s, end.unwrap_or(s), "output continues where it ended");
                    for (i, f) in out.chunks_exact(2).enumerate() {
                        y[s as usize + i] = f[0];
                    }
                    *end = Some(s + out.len() as i64 / 2);
                }
            }
            asleep.push(gated.gate.asleep);
        }
        assert_eq!(end_a, end_b);
        // Asleep within ~0.5 s of noise, awake for the whole voice (woken
        // by its first hop), asleep again once the model has called the
        // noise after it noise for ~300 ms (its estimate settles slowly).
        assert!(asleep[50..noise_hops].iter().all(|&a| a), "sleeps through the first noise");
        assert!(asleep[noise_hops..noise_hops + voice_hops].iter().all(|&a| !a), "awake for the voice");
        assert!(asleep[noise_hops + voice_hops + 100..].iter().all(|&a| a), "sleeps again after it");
        let share = gated.calls as f64 / plain.calls as f64;
        assert!(share < 0.5, "the model ran on {:.0}% of the hops", share * 100.0);

        let energy = |f: &dyn Fn(usize) -> f32, r: std::ops::Range<usize>| r.map(|i| (f(i) as f64).powi(2)).sum::<f64>();
        let db = |a: f64, b: f64| 10.0 * (a / b.max(1e-30)).log10();
        // Asleep: the model's own result for noise, to rounding.
        let quiet = 60 * hop..(noise_hops - 3) * hop;
        let quiet_err = db(energy(&|i| ya[i] - yb[i], quiet.clone()), energy(&|i| input[i], quiet.clone()));
        // What comes out at the wake is still noise, faded from the sleeping
        // output to the pre-rolled model's: the same again. (Without the
        // pre-roll the model would start from where it fell asleep.)
        let wake = (noise_hops - 3) * hop..(noise_hops - 2) * hop;
        let wake_err = db(energy(&|i| ya[i] - yb[i], wake.clone()), energy(&|i| yb[i], wake.clone()));
        // The start of the voice is not held back more than by a model that
        // never slept. (Past the wake the two models' recurrent states
        // differ, and DeepFilterNet reacts to them strongly on a synthetic
        // vowel, so later samples are not compared.)
        let onset = noise_hops * hop..(noise_hops + 10) * hop;
        let onset_db = db(energy(&|i| ya[i], onset.clone()), energy(&|i| yb[i], onset.clone()));
        println!(
            "off the ungated model: asleep {quiet_err:.1} dB, wake {wake_err:.1} dB; voice onset {onset_db:+.1} dB vs never sleeping; model ran on {:.0}% of hops",
            share * 100.0
        );
        assert!(quiet_err < -80.0, "asleep output differs by {quiet_err:.1} dB");
        assert!(wake_err < -40.0, "output at the wake differs by {wake_err:.1} dB");
        assert!(onset_db > -3.0, "voice onset {onset_db:.1} dB lower");
    }
}
