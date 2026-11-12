// The mixer runs in the output callback (or the Null output's timer thread), so it never blocks
// and never allocates: it takes a new stream list only when it can do so without waiting, reads
// the rings and atomics, and hands the list it replaced back to be dropped elsewhere.

use crate::stream::{lock, Stream, IDLE, PLAYING, PRIMING, WAITING};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// A stream starts once this much is buffered, so starting is quick.
const START_SECONDS: f64 = 0.15;
// After running out, it waits for more: 0.5 s after the first stall, twice that after each
// further one up to 2 s, so a stream arriving barely in real time (no burst on connect) settles
// instead of flapping. 30 s of playing without a stall starts over at 0.5 s.
const RESUME_SECONDS: f64 = 0.5;
const RESUME_MAX_SECONDS: f64 = 2.0;
const STEADY_SECONDS: f64 = 30.0;
// A seek that takes longer than this to refill counts as running out.
const PRIMING_SECONDS: f64 = 0.5;
// Level changes and pausing fade over this long.
const FADE_SECONDS: f32 = 0.015;
// Volume and balance changes are smoothed over this long, against zipper noise.
const SMOOTH_SECONDS: f32 = 0.005;
// What the tap keeps: well over the analyzer's 8192 frames plus the output latency.
const TAP_FRAMES: usize = 1 << 15;

pub(crate) struct MixShared {
    pub rate: u32,
    pending: Mutex<Option<Vec<Arc<Stream>>>>,
    retired: Mutex<Option<Vec<Arc<Stream>>>>,
    volume: AtomicU32,
    balance: AtomicU32,
    pub tap: Arc<Tap>,
}

impl MixShared {
    pub fn new(rate: u32) -> MixShared {
        MixShared {
            rate,
            pending: Mutex::new(None),
            retired: Mutex::new(None),
            volume: AtomicU32::new(1f32.to_bits()),
            balance: AtomicU32::new(0f32.to_bits()),
            tap: Arc::new(Tap::new()),
        }
    }

    // The streams the mixer should play from now on.
    pub fn publish(&self, streams: Vec<Arc<Stream>>) {
        let old = lock(&self.pending).replace(streams);
        drop(old);
    }

    // Drops the list the mixer gave back, here rather than in the callback.
    pub fn collect(&self) {
        let old = lock(&self.retired).take();
        drop(old);
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_balance(&self, balance: f32) {
        self.balance.store(balance.clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

pub(crate) struct Mixer {
    shared: Arc<MixShared>,
    streams: Vec<Arc<Stream>>,
    volume: f32,
    balance: f32,
    pan: (f32, f32),
    fade_step: f32,
    smooth_step: f32,
    start: u64,
    resume: u64,
    resume_max: u64,
    steady: u64,
    priming_limit: u64,
}

impl Mixer {
    pub fn new(shared: Arc<MixShared>) -> Mixer {
        let rate = shared.rate as f32;
        let frames = |seconds: f64| (seconds * shared.rate as f64) as u64;
        let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
        let balance = f32::from_bits(shared.balance.load(Ordering::Relaxed));
        Mixer {
            streams: Vec::with_capacity(32),
            volume,
            balance,
            pan: pan_gains(balance),
            fade_step: 1.0 / (FADE_SECONDS * rate),
            smooth_step: 1.0 / (SMOOTH_SECONDS * rate),
            start: frames(START_SECONDS),
            resume: frames(RESUME_SECONDS),
            resume_max: frames(RESUME_MAX_SECONDS),
            steady: frames(STEADY_SECONDS),
            priming_limit: frames(PRIMING_SECONDS),
            shared,
        }
    }

    fn refresh(&mut self) {
        let Ok(mut retired) = self.shared.retired.try_lock() else { return };
        if retired.is_some() {
            return;
        }
        let Ok(mut pending) = self.shared.pending.try_lock() else { return };
        if let Some(streams) = pending.take() {
            *retired = Some(std::mem::replace(&mut self.streams, streams));
        }
    }

    // Fills `out` (interleaved stereo at the mixing rate) with the next frames.
    pub fn render(&mut self, out: &mut [f32]) {
        self.refresh();
        out.fill(0.0);
        let frames = out.len() / 2;
        for stream in &self.streams {
            self.mix(stream, out, frames);
        }
        self.shared.tap.write(out);

        let volume = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        let balance = f32::from_bits(self.shared.balance.load(Ordering::Relaxed));
        for frame in out.as_chunks_mut::<2>().0 {
            if self.volume != volume {
                self.volume = approach(self.volume, volume, self.smooth_step);
            }
            if self.balance != balance {
                self.balance = approach(self.balance, balance, self.smooth_step * 2.0);
                self.pan = pan_gains(self.balance);
            }
            let (left, right) = (frame[0], frame[1]);
            // Web Audio's StereoPannerNode on a stereo input.
            let (l, r) = if self.balance <= 0.0 {
                (left + right * self.pan.0, right * self.pan.1)
            } else {
                (left * self.pan.0, right + left * self.pan.1)
            };
            // Decoders don't clip (loud streams peak over full scale), and integer outputs would
            // wrap. The tap above keeps the overs, as the browser's analyser saw them.
            frame[0] = (l * self.volume).clamp(-1.0, 1.0);
            frame[1] = (r * self.volume).clamp(-1.0, 1.0);
        }
    }

    fn mix(&self, stream: &Stream, out: &mut [f32], frames: usize) {
        let mut phase = stream.phase.load(Ordering::Relaxed);
        stream.ring.skip_flushed();
        // A seek flushed the ring: refill quietly instead of reporting a stall.
        if stream.reprime.swap(false, Ordering::Acquire) && (phase == PLAYING || phase == PRIMING) {
            phase = PRIMING;
            stream.priming.store(0, Ordering::Relaxed);
        }
        let hold = stream.paused.load(Ordering::Relaxed) || stream.closing.load(Ordering::Relaxed);
        let target = if hold { 0.0 } else { f32::from_bits(stream.level.load(Ordering::Relaxed)) };
        let mut gain = f32::from_bits(stream.gain.load(Ordering::Relaxed));
        if hold && gain == 0.0 {
            stream.phase.store(phase, Ordering::Relaxed);
            return;
        }
        let available = stream.ring.available();
        let finished = stream.finished.load(Ordering::Acquire);
        let ready = |cushion: u64| available >= cushion || (finished && available > 0);
        match phase {
            IDLE if ready(self.start) => {
                phase = PLAYING;
                stream.transitions.fetch_add(1, Ordering::Release);
            }
            WAITING if ready(stream.resume.load(Ordering::Relaxed).max(self.resume)) => {
                phase = PLAYING;
                stream.transitions.fetch_add(1, Ordering::Release);
            }
            PRIMING if ready(self.start) => phase = PLAYING,
            PRIMING => {
                let primed = stream.priming.load(Ordering::Relaxed) + frames as u64;
                stream.priming.store(primed, Ordering::Relaxed);
                if primed >= self.priming_limit && !finished {
                    phase = WAITING;
                    self.stall(stream);
                }
            }
            _ => {}
        }
        if phase != PLAYING {
            stream.phase.store(phase, Ordering::Relaxed);
            return;
        }

        let take = (frames as u64).min(available);
        if gain == 0.0 && target == 0.0 {
            // Muted: consumed all the same, to stay at the live point.
        } else {
            for i in 0..take {
                if gain != target {
                    gain = approach(gain, target, self.fade_step);
                }
                let (left, right) = stream.ring.frame(i);
                let at = i as usize * 2;
                out[at] += left * gain;
                out[at + 1] += right * gain;
            }
            stream.gain.store(gain.to_bits(), Ordering::Relaxed);
        }
        stream.ring.advance(take);
        stream.consumed.fetch_add(take, Ordering::Relaxed);
        let run = stream.run.load(Ordering::Relaxed) + take;
        stream.run.store(run, Ordering::Relaxed);
        if run >= self.steady {
            stream.resume.store(0, Ordering::Relaxed);
        }
        if take < frames as u64 && !finished {
            phase = WAITING;
            self.stall(stream);
        }
        stream.phase.store(phase, Ordering::Relaxed);
    }

    // The stream ran out: it will wait for a bigger cushion than last time.
    fn stall(&self, stream: &Stream) {
        let resume = stream.resume.load(Ordering::Relaxed);
        let next = if resume == 0 { self.resume } else { (resume * 2).min(self.resume_max) };
        stream.resume.store(next, Ordering::Relaxed);
        stream.run.store(0, Ordering::Relaxed);
        stream.transitions.fetch_add(1, Ordering::Release);
    }
}

fn approach(value: f32, target: f32, step: f32) -> f32 {
    if (target - value).abs() <= step {
        target
    } else if target > value {
        value + step
    } else {
        value - step
    }
}

// The gains StereoPannerNode uses for a stereo input at this pan.
fn pan_gains(pan: f32) -> (f32, f32) {
    let x = if pan <= 0.0 { pan + 1.0 } else { pan };
    let angle = x * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// The pre-volume mix, for the analyzer. Written by the mixer, read by another thread; lock-free
/// on the mixer side.
pub struct Tap {
    samples: Box<[AtomicU32]>,
    written: AtomicU64,
    // frames between the end of what was mixed and what is heard now
    latency: AtomicU64,
    // the mixing rate while an output runs, else 0
    rate: AtomicU32,
}

impl Tap {
    fn new() -> Tap {
        Tap {
            samples: (0..TAP_FRAMES * 2).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicU64::new(0),
            latency: AtomicU64::new(0),
            rate: AtomicU32::new(0),
        }
    }

    fn write(&self, stereo: &[f32]) {
        let written = self.written.load(Ordering::Relaxed);
        for (i, frame) in stereo.as_chunks::<2>().0.iter().enumerate() {
            let slot = ((written + i as u64) % TAP_FRAMES as u64) as usize * 2;
            self.samples[slot].store(frame[0].to_bits(), Ordering::Relaxed);
            self.samples[slot + 1].store(frame[1].to_bits(), Ordering::Relaxed);
        }
        self.written.store(written + (stereo.len() / 2) as u64, Ordering::Release);
    }

    pub(crate) fn set_running(&self, rate: Option<u32>) {
        self.rate.store(rate.unwrap_or(0), Ordering::Release);
    }

    pub(crate) fn set_latency(&self, frames: u64) {
        self.latency.store(frames, Ordering::Relaxed);
    }

    /// Fills both slices (same length) with the most recent frames ending at the sample that will
    /// be heard `lead_seconds` from now (output latency accounted for; clamped to what has been
    /// mixed). Returns the output sample rate, or None while no output is running.
    pub fn read(&self, lead_seconds: f32, left: &mut [f32], right: &mut [f32]) -> Option<u32> {
        let rate = self.rate.load(Ordering::Acquire);
        if rate == 0 {
            return None;
        }
        let frames = left.len().min(right.len()).min(TAP_FRAMES / 2);
        let written = self.written.load(Ordering::Acquire);
        let lead = (lead_seconds.max(0.0) as f64 * rate as f64) as u64;
        let end = (written.saturating_sub(self.latency.load(Ordering::Relaxed)) + lead).min(written);
        let missing = (frames as u64).saturating_sub(end) as usize;
        left[..missing].fill(0.0);
        right[..missing].fill(0.0);
        let first = end - (frames - missing) as u64;
        for i in missing..frames {
            let slot = ((first + (i - missing) as u64) % TAP_FRAMES as u64) as usize * 2;
            left[i] = f32::from_bits(self.samples[slot].load(Ordering::Relaxed));
            right[i] = f32::from_bits(self.samples[slot + 1].load(Ordering::Relaxed));
        }
        left[frames..].fill(0.0);
        right[frames..].fill(0.0);
        Some(rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panner_law() {
        let (l, r) = pan_gains(0.0);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        // Full left: the right channel moves into the left one.
        let (l, r) = pan_gains(-1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
    }

    #[test]
    fn starts_fades_stalls_and_pauses() {
        let shared = Arc::new(MixShared::new(48000));
        let stream = Arc::new(Stream::new(1, "k", 48000, 48000));
        shared.publish(vec![stream.clone()]);
        let mut mixer = Mixer::new(shared.clone());
        let mut out = vec![0.0; 480 * 2];
        let transitions = || stream.transitions.load(Ordering::Relaxed);
        let consumed = || stream.consumed.load(Ordering::Relaxed);

        // Under the start cushion: waits.
        stream.ring.push(&vec![0.5; 1000 * 2]);
        mixer.render(&mut out);
        assert_eq!((transitions(), consumed()), (0, 0));

        // Over it: starts, at level 0 (consumed, silent).
        stream.ring.push(&vec![0.5; 7000 * 2]);
        mixer.render(&mut out);
        assert_eq!((transitions(), consumed()), (1, 480));
        assert!(out.iter().all(|s| *s == 0.0));

        // Level 1: fades in over 15 ms (720 frames), then full.
        stream.level.store(1f32.to_bits(), Ordering::Relaxed);
        mixer.render(&mut out);
        assert!(out[0] > 0.0 && out[0] < 0.01 && out[959] > 0.3 && out[959] < 0.5, "{} {}", out[0], out[959]);
        mixer.render(&mut out);
        assert!((out[959] - 0.5).abs() < 1e-6);

        // Paused: fades out, then stops consuming.
        stream.paused.store(true, Ordering::Relaxed);
        mixer.render(&mut out);
        mixer.render(&mut out);
        let held = consumed();
        mixer.render(&mut out);
        assert_eq!(consumed(), held);
        assert!(out.iter().all(|s| *s == 0.0));
        stream.paused.store(false, Ordering::Relaxed);

        // Running out: one stall.
        while stream.ring.available() > 0 {
            mixer.render(&mut out);
        }
        assert_eq!(transitions(), 2);
        mixer.render(&mut out);
        assert_eq!(transitions(), 2);

        // Volume and balance after the tap: the tap keeps the mix, the output is silent.
        stream.ring.push(&vec![0.5; 48000 * 2]);
        shared.set_volume(0.0);
        for _ in 0..10 {
            mixer.render(&mut out);
        }
        assert_eq!(transitions(), 3);
        assert!(out.iter().all(|s| *s == 0.0));
        let (mut left, mut right) = (vec![0.0; 256], vec![0.0; 256]);
        shared.tap.set_running(Some(48000));
        shared.tap.read(0.0, &mut left, &mut right);
        assert!((left[255] - 0.5).abs() < 1e-6 && (right[255] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_resume_cushion_grows_with_each_stall() {
        let shared = Arc::new(MixShared::new(48000));
        let stream = Arc::new(Stream::new(1, "k", 8 * 48000, 48000));
        shared.publish(vec![stream.clone()]);
        let mut mixer = Mixer::new(shared);
        let mut out = vec![0.0; 480 * 2];
        let transitions = || stream.transitions.load(Ordering::Relaxed);
        // Plays what it has, stalls, and returns the frames it waited for before playing again.
        let stall_and_resume = |mixer: &mut Mixer| {
            let mut out = vec![0.0; 480 * 2];
            while stream.ring.available() > 0 {
                mixer.render(&mut out);
            }
            mixer.render(&mut out);
            let stalled = transitions();
            let mut fed = 0;
            while transitions() == stalled {
                stream.ring.push(&[0.1; 480 * 2]);
                fed += 480;
                mixer.render(&mut out);
            }
            fed
        };
        stream.ring.push(&vec![0.1; 9600 * 2]);
        mixer.render(&mut out);
        assert_eq!(transitions(), 1);
        let waits: Vec<usize> = (0..4).map(|_| stall_and_resume(&mut mixer)).collect();
        assert_eq!(waits, [24000, 48000, 96000, 96000]);

        // 30 s without a stall: back to 0.5 s.
        stream.ring.push(&vec![0.1; 48000 * 2]);
        for _ in 0..3000 {
            if stream.ring.available() < 48000 {
                stream.ring.push(&vec![0.1; 48000 * 2]);
            }
            mixer.render(&mut out);
        }
        assert_eq!(stall_and_resume(&mut mixer), 24000);
    }

    #[test]
    fn tap_reads_the_latest_frames() {
        let tap = Tap::new();
        tap.set_running(Some(48000));
        let stereo: Vec<f32> = (0..2000).flat_map(|i| [i as f32, -(i as f32)]).collect();
        tap.write(&stereo);
        tap.set_latency(100);
        let (mut left, mut right) = (vec![9.0; 4], vec![9.0; 4]);
        assert_eq!(tap.read(0.0, &mut left, &mut right), Some(48000));
        assert_eq!(left, [1896.0, 1897.0, 1898.0, 1899.0]);
        assert_eq!(right, [-1896.0, -1897.0, -1898.0, -1899.0]);
        // A lead past what has been mixed is clamped.
        tap.read(1.0, &mut left, &mut right);
        assert_eq!(left, [1996.0, 1997.0, 1998.0, 1999.0]);
        // More than was ever mixed: zeros first.
        let (mut left, mut right) = (vec![9.0; 2004], vec![9.0; 2004]);
        tap.read(1.0, &mut left, &mut right);
        assert_eq!(&left[..5], &[0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(left[2003], 1999.0);
    }
}
