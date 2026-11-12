// The player: cartridges, playlist, the active source, the state machine on top of the audio
// engine (reconnecting, the stall watchdog, instant switch), titles, volume, balance, settings and
// persistence. What the old page (script.js) did, minus the display.
//
// One thread owns everything (`Core`) and takes messages: calls from the commands (closures run
// on the core, answered once the events they cause are out), engine events, and its own timers.
// Nothing here needs Tauri: the outside is a `Host` (events to the windows), the `Store`, and the
// frame sink given at attach. A UI shell drives it through `Player`'s methods (the Tauri app's
// player/commands.rs is one).

mod accent;
mod machine;
mod model;
pub mod playlist;
mod store;
#[cfg(test)]
mod tests;
mod visualizer;

pub use accent::accent;
pub use qust_audio::OutputKind;
pub use store::{electron_config_path, settings_path, Store};
pub use visualizer::FrameSink;

use self::machine::Core;
use qust_audio::Event;
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

    // What the page asks for (one Tauri command each, player/commands.rs in the app). Each returns
    // once the player has handled it and emitted what it changed.

    pub fn play(&self) {
        self.call(|core| core.play());
    }

    pub fn pause(&self) {
        self.call(|core| core.pause());
    }

    pub fn stop(&self) {
        self.call(|core| core.stop());
    }

    pub fn seek(&self, seconds: f64) {
        self.call(move |core| core.seek(seconds));
    }

    // 0–100
    pub fn set_volume(&self, value: i64) {
        self.call(move |core| core.set_volume(value));
    }

    // −100–100
    pub fn set_balance(&self, value: i64) {
        self.call(move |core| core.set_balance(value));
    }

    pub fn set_instant(&self, on: bool) {
        self.call(move |core| core.set_instant(on));
    }

    pub fn cycle_colors(&self) {
        self.call(|core| core.cycle_colors());
    }

    // Plays cartridge `slot` (0–5).
    pub fn select_slot(&self, slot: usize) {
        self.call(move |core| core.select_slot(slot));
    }

    pub fn save_cartridge(&self, slot: usize, title: String, url: String) -> Result<(), String> {
        self.call(move |core| core.save_cartridge(slot, title, url)).unwrap_or(Ok(()))
    }

    pub fn eject(&self, slot: usize) {
        self.call(move |core| core.eject(slot));
    }

    pub fn move_cartridge(&self, from: usize, to: usize) {
        self.call(move |core| core.move_cartridge(from, to));
    }

    // Plays playlist entry `id`.
    pub fn select_entry(&self, id: u64) {
        self.call(move |core| core.select_entry(id));
    }

    // The ids added.
    pub fn add_url(&self, url: String, title: String) -> Result<Vec<u64>, String> {
        self.call(move |core| core.add_url(url, title)).unwrap_or(Ok(Vec::new()))
    }

    // Entries from `playlist::expand` (dropped or picked), before `index` or at the end. The ids
    // added.
    pub fn add_entries(&self, items: Vec<Value>, index: Option<usize>) -> Vec<u64> {
        if items.is_empty() {
            return Vec::new();
        }
        self.call(move |core| core.add_entries(&items, index)).unwrap_or_default()
    }

    pub fn remove_entries(&self, ids: Vec<u64>) {
        self.call(move |core| core.remove_entries(&ids));
    }

    // Before `index` in the list as it is.
    pub fn move_entries(&self, ids: Vec<u64>, index: usize) {
        self.call(move |core| core.move_entries(&ids, index));
    }

    // `url: None` for a local file.
    pub fn edit_entry(&self, id: u64, title: String, url: Option<String>) -> Result<(), String> {
        self.call(move |core| core.edit_entry(id, title, url)).unwrap_or(Ok(()))
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

// The details of a stream or file (the Stream information window), by its bare URL or path.
pub fn stream_info(url: String) -> Value {
    let lower = url.to_ascii_lowercase();
    let key = if lower.starts_with("http://") || lower.starts_with("https://") { url } else { format!("file:{url}") };
    qust_audio::info(&key)
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
