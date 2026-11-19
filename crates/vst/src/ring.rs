// Parameter changes between the plugin thread and the output callback: a fixed-size,
// single-producer single-consumer queue of (parameter id, normalized value). Neither side waits
// or allocates. When it is full, newer changes are dropped (a knob dragged faster than a block
// is processed loses nothing that matters: the next change carries the latest value).

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub struct ParamRing {
    ids: Box<[AtomicU32]>,
    values: Box<[AtomicU64]>,
    // Written by the producer / consumer only; both only grow (wrapping).
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl ParamRing {
    pub fn new(capacity: usize) -> ParamRing {
        let capacity = capacity.max(2).next_power_of_two();
        ParamRing {
            ids: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            values: (0..capacity).map(|_| AtomicU64::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn mask(&self) -> usize {
        self.ids.len() - 1
    }

    /// The producer's side. False when full.
    pub fn push(&self, id: u32, value: f64) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= self.ids.len() {
            return false;
        }
        let slot = head & self.mask();
        self.ids[slot].store(id, Ordering::Relaxed);
        self.values[slot].store(value.to_bits(), Ordering::Relaxed);
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// The consumer's side.
    pub fn pop(&self) -> Option<(u32, f64)> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let slot = tail & self.mask();
        let change = (self.ids[slot].load(Ordering::Relaxed), f64::from_bits(self.values[slot].load(Ordering::Relaxed)));
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_refuses_when_full() {
        let ring = ParamRing::new(4);
        for i in 0..4 {
            assert!(ring.push(i, i as f64 / 10.0));
        }
        assert!(!ring.push(9, 0.9));
        assert_eq!(ring.pop(), Some((0, 0.0)));
        assert!(ring.push(4, 0.4));
        let rest: Vec<_> = std::iter::from_fn(|| ring.pop()).collect();
        assert_eq!(rest, [(1, 0.1), (2, 0.2), (3, 0.3), (4, 0.4)]);
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn across_threads() {
        let ring = std::sync::Arc::new(ParamRing::new(64));
        let producer = ring.clone();
        let thread = std::thread::spawn(move || {
            for i in 0..10_000u32 {
                while !producer.push(i, i as f64) {
                    std::thread::yield_now();
                }
            }
        });
        let mut next = 0u32;
        while next < 10_000 {
            if let Some((id, value)) = ring.pop() {
                assert_eq!((id, value), (next, next as f64));
                next += 1;
            }
        }
        thread.join().unwrap();
    }
}
