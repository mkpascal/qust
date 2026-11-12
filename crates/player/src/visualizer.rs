// The analyzer's 60 Hz thread: the engine's tap → qust_spectrum → a 77-byte frame to the page,
// sent only when it differs from the last one. It runs only while there is something to animate:
// a page is attached and the player isn't stopped (and, while paused, until the bars have fallen).
// The player tells it the state and colors; the frame layout and maths are qust_spectrum's.

use qust_audio::Tap;
use qust_spectrum::{Spectrum, FRAME_LEN, LEAD_SECONDS};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 60);
// Further behind than this (a suspended PC), the schedule starts again from now.
const LATE: Duration = Duration::from_millis(100);
const BLANK: [u8; FRAME_LEN] = [0; FRAME_LEN];

pub type FrameSink = Arc<dyn Fn(&[u8]) + Send + Sync>;

#[derive(Clone)]
pub struct Visualizer {
    shared: Arc<Mutex<Shared>>,
}

struct Shared {
    spectrum: Spectrum,
    sink: Option<FrameSink>,
    tap: Option<Arc<Tap>>,
    last: Option<[u8; FRAME_LEN]>,
    // the player's state ("stopped", "paused", "playing", "connecting", "buffering")
    state: &'static str,
    running: bool,
}

impl Visualizer {
    pub fn new() -> Visualizer {
        let shared = Shared { spectrum: Spectrum::new(), sink: None, tap: None, last: None, state: "stopped", running: false };
        Visualizer { shared: Arc::new(Mutex::new(shared)) }
    }

    // A page attached (or reloaded): frames go to it, starting with the current one.
    pub fn attach(&self, sink: FrameSink, tap: Arc<Tap>) {
        let mut shared = self.shared.lock().unwrap();
        shared.sink = Some(sink);
        shared.tap = Some(tap);
        shared.last = None;
        self.wake(&mut shared);
    }

    // The thread sees it and ends.
    pub fn detach(&self) {
        let mut shared = self.shared.lock().unwrap();
        shared.sink = None;
        shared.last = None;
    }

    // On every state change, as setState() and stop() did with the JS Spectrum.
    pub fn set_state(&self, state: &'static str) {
        let mut shared = self.shared.lock().unwrap();
        shared.state = state;
        shared.spectrum.set_loading(state == "connecting" || state == "buffering");
        if state == "stopped" {
            shared.spectrum.set_loading(false);
            shared.spectrum.reset();
            // The page doesn't clear the canvas by itself.
            if let Some(sink) = shared.sink.clone() {
                if shared.last != Some(BLANK) {
                    sink(&BLANK);
                    shared.last = Some(BLANK);
                }
            }
        } else {
            self.wake(&mut shared);
        }
    }

    pub fn set_colors(&self, colors: &str) {
        let mut shared = self.shared.lock().unwrap();
        shared.spectrum.set_off(colors == "off");
        self.wake(&mut shared);
    }

    fn wake(&self, shared: &mut Shared) {
        if shared.running || shared.sink.is_none() || shared.state == "stopped" {
            return;
        }
        shared.running = true;
        let this = self.clone();
        let spawned = thread::Builder::new().name("qust-visualizer".into()).spawn(move || this.run());
        if spawned.is_err() {
            shared.running = false;
        }
    }

    fn run(&self) {
        // The first dt counts from the restart, never the gap before it.
        let mut last = Instant::now();
        let mut next = last;
        loop {
            next += FRAME;
            let now = Instant::now();
            if next > now {
                thread::sleep(next - now);
            } else if now - next > LATE {
                next = now;
            }
            let mut shared = self.shared.lock().unwrap();
            let Some(sink) = shared.sink.clone().filter(|_| shared.state != "stopped") else {
                shared.running = false;
                return;
            };
            let now = Instant::now();
            let dt = (now - last).as_secs_f32();
            last = now;
            let tap = shared.tap.clone();
            let frame = shared.spectrum.tick(dt, &mut |left, right| tap.as_ref()?.read(LEAD_SECONDS, left, right).map(|rate| rate as f32));
            // Sent under the lock, so a stop's blank frame can't be overtaken by a late one.
            if shared.last != Some(frame) {
                sink(&frame);
                shared.last = Some(frame);
            }
            if shared.state == "paused" && shared.spectrum.idle() {
                shared.running = false;
                return;
            }
        }
    }
}
