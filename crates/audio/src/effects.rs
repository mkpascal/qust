// Audio processing after the mix (the VST3 plugins): one chain at a time, run by the mixer in the
// output callback, after the streams are summed and before the tap, the volume and the balance.
//
// The slot outlives engines: whoever owns the chain (qust-vst) keeps it and hears, through
// `on_engine`, when an engine starts (with its mixing rate, which the chain must be set up for)
// and when it goes. The mixer only ever `try_lock`s the slot, so it never waits: while the owner
// swaps the chain (a pointer's worth of work), one block goes through unprocessed.

use crate::stream::lock;
use std::sync::{Arc, Mutex};

/// A chain of processing, run on the output thread. It must not block or allocate.
pub trait Effect: Send {
    /// Processes interleaved stereo frames in place, at the mixing rate the slot announced.
    fn process(&mut self, stereo: &mut [f32]);
}

type Listener = Box<dyn Fn(Option<u32>) + Send + Sync>;

/// Where an engine looks for its processing. Share one between the engines (`Config::effects`)
/// and the chain's owner.
#[derive(Default)]
pub struct EffectSlot {
    chain: Mutex<Option<Box<dyn Effect>>>,
    listener: Mutex<Option<Listener>>,
    rate: Mutex<Option<u32>>,
}

impl EffectSlot {
    pub fn new() -> Arc<EffectSlot> {
        Arc::new(EffectSlot::default())
    }

    /// Installs a chain (None: no processing) and returns the one it replaces, for the caller to
    /// drop on its own thread. Waits at most for the block being processed.
    pub fn set(&self, chain: Option<Box<dyn Effect>>) -> Option<Box<dyn Effect>> {
        std::mem::replace(&mut *lock(&self.chain), chain)
    }

    /// Called with the mixing rate when an engine starts, and with None when it goes (then the
    /// chain is no longer run). Called at once with the current engine's rate, if any.
    pub fn on_engine(&self, listener: impl Fn(Option<u32>) + Send + Sync + 'static) {
        let rate = *lock(&self.rate);
        listener(rate);
        *lock(&self.listener) = Some(Box::new(listener));
    }

    /// The rate of the engine running now.
    pub fn rate(&self) -> Option<u32> {
        *lock(&self.rate)
    }

    pub(crate) fn engine(&self, rate: Option<u32>) {
        *lock(&self.rate) = rate;
        if let Some(listener) = lock(&self.listener).as_ref() {
            listener(rate);
        }
    }

    // In the output callback: never waits.
    pub(crate) fn process(&self, stereo: &mut [f32]) {
        if let Ok(mut chain) = self.chain.try_lock() {
            if let Some(chain) = chain.as_mut() {
                chain.process(stereo);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Gain(f32);

    impl Effect for Gain {
        fn process(&mut self, stereo: &mut [f32]) {
            for sample in stereo {
                *sample *= self.0;
            }
        }
    }

    #[test]
    fn runs_the_chain_it_holds_and_hands_back_the_old_one() {
        let slot = EffectSlot::new();
        let mut buffer = [0.5f32; 8];
        slot.process(&mut buffer);
        assert_eq!(buffer, [0.5; 8]);
        assert!(slot.set(Some(Box::new(Gain(0.5)))).is_none());
        slot.process(&mut buffer);
        assert_eq!(buffer, [0.25; 8]);
        assert!(slot.set(None).is_some());
        slot.process(&mut buffer);
        assert_eq!(buffer, [0.25; 8]);
    }

    #[test]
    fn tells_the_owner_about_engines() {
        let slot = EffectSlot::new();
        slot.engine(Some(44100));
        let heard = Arc::new(AtomicU32::new(1));
        let seen = heard.clone();
        slot.on_engine(move |rate| seen.store(rate.unwrap_or(0), Ordering::Relaxed));
        assert_eq!(heard.load(Ordering::Relaxed), 44100);
        slot.engine(None);
        assert_eq!(heard.load(Ordering::Relaxed), 0);
        assert_eq!(slot.rate(), None);
    }
}
