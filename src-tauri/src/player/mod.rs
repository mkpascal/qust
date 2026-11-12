// The player: cartridges, playlist, the active source, the state machine on top of the audio
// engine (reconnecting, the stall watchdog, instant switch), titles, volume, balance, settings and
// persistence. What the old page (script.js) did, minus the display.
//
// One thread owns everything (`Core`) and takes messages: calls from the commands (closures run
// on the core, answered once the events they cause are out), engine events, and its own timers.
// Nothing here needs Tauri: the outside is a `Host` (events to the windows), the `Store`, and the
// frame sink given at attach. commands.rs is the Tauri side.

mod commands;
mod machine;
mod model;
#[cfg(test)]
mod tests;

pub use commands::*;

use self::machine::Core;
use crate::store::Store;
use crate::visualizer::FrameSink;
use qust_audio::{Event, OutputKind};
use serde_json::Value;
use std::sync::mpsc;
use std::time::Instant;

// Events to the windows: `player-state`, `player-playlist`, `player-entry` (the player page),
// `player-status` and `settings-changed` (everyone).
pub trait Host: Send + 'static {
    fn emit(&self, event: &str, payload: Value);
}

type Reply = Box<dyn FnOnce() + Send>;
type Job = Box<dyn FnOnce(&mut Core) -> Reply + Send>;

enum Msg {
    Call(Job),
    Engine(Event),
    Quit(mpsc::Sender<()>),
}

#[derive(Clone)]
pub struct Player {
    tx: mpsc::Sender<Msg>,
}

impl Player {
    pub fn start(store: Store, host: impl Host, output: OutputKind) -> Player {
        let (tx, rx) = mpsc::channel();
        let player = Player { tx };
        let core_player = player.clone();
        std::thread::Builder::new()
            .name("qust-player".into())
            .spawn(move || run(Core::new(store, Box::new(host), output, core_player), rx))
            .expect("start the player thread");
        player
    }

    // Runs `f` on the player thread and waits for its answer, which comes after the events it
    // caused have been emitted. None only if the player is gone.
    pub(crate) fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut Core) -> T + Send + 'static) -> Option<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |core| {
            let value = f(core);
            Box::new(move || {
                let _ = tx.send(value);
            })
        });
        self.tx.send(Msg::Call(job)).ok()?;
        rx.recv().ok()
    }

    // The same without waiting (from the shortcut handler, other threads, the engine).
    pub(crate) fn cast(&self, f: impl FnOnce(&mut Core) + Send + 'static) {
        let job: Job = Box::new(move |core| {
            f(core);
            Box::new(|| {})
        });
        let _ = self.tx.send(Msg::Call(job));
    }

    fn event(&self, event: Event) {
        let _ = self.tx.send(Msg::Engine(event));
    }

    pub fn attach(&self, frames: FrameSink) -> Value {
        self.call(move |core| core.attach(frames)).unwrap_or(Value::Null)
    }

    // The player window is gone: stop and let go of every stream.
    pub fn detach(&self) {
        self.call(|core| core.detach());
    }

    // A shortcut, the tray or `--toggle`: "play", "stop" or "toggle".
    pub fn action(&self, action: &'static str) {
        self.cast(move |core| core.action(action));
    }

    // The helper opens the window: whether it should play once the page attaches.
    pub fn request_play(&self, play: bool) {
        self.cast(move |core| core.play_request = play);
    }

    pub fn setting_changed(&self, key: &str, value: Option<Value>) {
        let key = key.to_string();
        self.cast(move |core| core.setting_changed(&key, value));
    }

    pub fn status(&self) -> Value {
        self.call(|core| core.status_json()).unwrap_or(Value::Null)
    }

    // Stops the thread (tests; the app just exits).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn shutdown(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Msg::Quit(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

fn run(mut core: Core, rx: mpsc::Receiver<Msg>) {
    loop {
        let msg = match core.next_deadline() {
            Some(deadline) => match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(msg) => Some(msg),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => return,
            },
        };
        let mut reply = None;
        match msg {
            Some(Msg::Call(job)) => reply = Some(job(&mut core)),
            Some(Msg::Engine(event)) => core.engine_event(event),
            Some(Msg::Quit(done)) => {
                core.quit();
                let _ = done.send(());
                return;
            }
            None => {}
        }
        core.timers();
        core.refresh();
        if let Some(reply) = reply {
            reply();
        }
    }
}
