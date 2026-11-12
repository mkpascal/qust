// A single-producer, single-consumer ring of stereo frames: the decode thread writes, the mixer
// reads. Samples are stored as the bits of f32s in relaxed atomics, which compile to plain loads
// and stores, so neither side ever locks and no unsafe code is needed. Positions only grow, so
// "how much is buffered" is a subtraction.
//
// Flushing (a seek) is done by the producer: it marks everything written so far as stale, and the
// consumer skips to that mark the next time it looks. The producer never touches the read side.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub struct Ring {
    samples: Box<[AtomicU32]>,
    // in frames
    capacity: u64,
    write: AtomicU64,
    read: AtomicU64,
    flush_to: AtomicU64,
}

impl Ring {
    pub fn new(frames: usize) -> Ring {
        Ring {
            samples: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(),
            capacity: frames as u64,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            flush_to: AtomicU64::new(0),
        }
    }

    // Producer: writes as many of these interleaved stereo frames as there is room for, and says
    // how many frames that was.
    pub fn push(&self, stereo: &[f32]) -> usize {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        let room = self.capacity - (write - read);
        let frames = (stereo.len() as u64 / 2).min(room);
        for i in 0..frames {
            let slot = ((write + i) % self.capacity) as usize * 2;
            let at = i as usize * 2;
            self.samples[slot].store(stereo[at].to_bits(), Ordering::Relaxed);
            self.samples[slot + 1].store(stereo[at + 1].to_bits(), Ordering::Relaxed);
        }
        self.write.store(write + frames, Ordering::Release);
        frames as usize
    }

    // Producer: drops everything buffered. Returns the write position, where new audio starts.
    pub fn flush(&self) -> u64 {
        let write = self.write.load(Ordering::Relaxed);
        self.flush_to.store(write, Ordering::Release);
        write
    }

    pub fn written(&self) -> u64 {
        self.write.load(Ordering::Acquire)
    }

    // Frames buffered and not stale. Either side may ask.
    pub fn available(&self) -> u64 {
        let write = self.write.load(Ordering::Acquire);
        let start = self.read.load(Ordering::Acquire).max(self.flush_to.load(Ordering::Acquire));
        write.saturating_sub(start)
    }

    // Consumer: applies a pending flush. Returns true when it skipped something.
    pub fn skip_flushed(&self) -> bool {
        let flush_to = self.flush_to.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);
        if flush_to > read {
            self.read.store(flush_to, Ordering::Release);
            return true;
        }
        false
    }

    // Consumer: the frame `offset` frames past the read position (which must be available).
    #[inline]
    pub fn frame(&self, offset: u64) -> (f32, f32) {
        let slot = ((self.read.load(Ordering::Relaxed) + offset) % self.capacity) as usize * 2;
        (
            f32::from_bits(self.samples[slot].load(Ordering::Relaxed)),
            f32::from_bits(self.samples[slot + 1].load(Ordering::Relaxed)),
        )
    }

    // Consumer: marks frames as read, freeing their room.
    pub fn advance(&self, frames: u64) {
        let read = self.read.load(Ordering::Relaxed);
        self.read.store(read + frames, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::Ring;

    #[test]
    fn wraps_and_flushes() {
        let ring = Ring::new(4);
        assert_eq!(ring.push(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0]), 3);
        assert_eq!(ring.push(&[4.0, -4.0, 5.0, -5.0]), 1);
        assert_eq!(ring.available(), 4);
        assert_eq!(ring.frame(0), (1.0, -1.0));
        ring.advance(2);
        assert_eq!(ring.push(&[5.0, -5.0, 6.0, -6.0]), 2);
        assert_eq!(ring.frame(3), (6.0, -6.0));
        let start = ring.flush();
        assert_eq!(ring.available(), 0);
        assert_eq!(start, 6);
        assert!(ring.skip_flushed());
        assert_eq!(ring.push(&[7.0, -7.0]), 1);
        assert_eq!(ring.available(), 1);
        assert_eq!(ring.frame(0), (7.0, -7.0));
    }
}
