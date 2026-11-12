// What a loaded stream shares between the engine (control), its decode thread, the mixer and the
// event thread. Each field has one writer, noted below; everything the mixer touches is atomic.

use crate::ring::Ring;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};

// The mixer's view of a stream.
pub(crate) const IDLE: u8 = 0; // not started yet: waiting for the first cushion
pub(crate) const PLAYING: u8 = 1;
pub(crate) const WAITING: u8 = 2; // ran out after playing
pub(crate) const PRIMING: u8 = 3; // playing, but refilling after a seek

pub(crate) struct Segment {
    // the ring position where audio from `time` (seconds into the file) starts
    pub frame: u64,
    pub time: f64,
    // a seek asked for but not done yet by the decode thread
    pub pending: Option<f64>,
}

pub(crate) struct Stream {
    pub id: u64,
    pub key: String,
    pub ring: Ring,
    // the mixing rate, the ring's rate
    pub rate: u32,

    // Written by the engine.
    pub level: AtomicU32,
    pub paused: AtomicBool,
    pub closing: AtomicBool,
    pub stop: AtomicBool,
    pub seek: Mutex<Option<f64>>,

    // Written by the mixer.
    pub gain: AtomicU32,
    pub phase: AtomicU8,
    // frames spent priming since the last seek
    pub priming: AtomicU64,
    // every start and every stall: odd counts are "playing", even ones "waiting"
    pub transitions: AtomicU32,
    pub consumed: AtomicU64,
    // the cushion to wait for after the next stall (0: none yet), and frames played since the last
    pub resume: AtomicU64,
    pub run: AtomicU64,

    // Written by the decode thread.
    pub reprime: AtomicBool,
    pub finished: AtomicBool,
    pub error: Mutex<Option<String>>,
    pub segment: Mutex<Segment>,
    pub file: AtomicBool,
    pub duration: Mutex<Option<f64>>,
    pub duration_known: AtomicBool,
    // counts seeks: an end announced before a seek doesn't count after it
    pub epoch: AtomicU64,

    // Written by the event thread.
    pub duration_sent: AtomicBool,
    pub announced: AtomicU32,
    // epoch + 1 of the end (Ended or Failed) last announced, 0 for none
    pub done: AtomicU64,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Stream {
    pub fn new(id: u64, key: &str, frames: usize, rate: u32) -> Stream {
        Stream {
            id,
            key: key.to_string(),
            ring: Ring::new(frames),
            rate,
            level: AtomicU32::new(0f32.to_bits()),
            paused: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            seek: Mutex::new(None),
            gain: AtomicU32::new(0f32.to_bits()),
            phase: AtomicU8::new(IDLE),
            priming: AtomicU64::new(0),
            transitions: AtomicU32::new(0),
            consumed: AtomicU64::new(0),
            resume: AtomicU64::new(0),
            run: AtomicU64::new(0),
            reprime: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            segment: Mutex::new(Segment { frame: 0, time: 0.0, pending: None }),
            file: AtomicBool::new(false),
            duration: Mutex::new(None),
            duration_known: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            duration_sent: AtomicBool::new(false),
            announced: AtomicU32::new(0),
            done: AtomicU64::new(0),
        }
    }

    // The end was announced, and no seek came since.
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire) == self.epoch.load(Ordering::Acquire) + 1
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn set_duration(&self, duration: Option<f64>) {
        *lock(&self.duration) = duration;
        self.duration_sent.store(false, Ordering::Relaxed);
        self.duration_known.store(true, Ordering::Release);
    }

    // The play head, in seconds into the file.
    pub fn position(&self) -> f64 {
        let segment = lock(&self.segment);
        if let Some(target) = segment.pending {
            return target;
        }
        let read = self.ring.written() - self.ring.available();
        segment.time + read.saturating_sub(segment.frame) as f64 / self.rate as f64
    }
}
