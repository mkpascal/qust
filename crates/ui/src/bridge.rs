//! The bridge between `qust-player` and GPUI.
//!
//! - **Events in.** [`UiHost`] is the player's `Host`: on the player thread it parses each
//!   event into a typed [`Event`] and sends it down an unbounded channel (never blocks). A task on
//!   the UI thread applies them, in order, to the [`Models`].
//! - **Frames in.** The `FrameSink` given to `attach` writes the newest frame into a slot and
//!   wakes the UI task only if it isn't already woken: frames coalesce (the newest wins, the
//!   last one — the blank frame on stop — is never lost), and the 60 Hz thread never waits for
//!   the UI. The slot is a mutex held for a 77-byte copy on both sides.
//! - **Commands out.** [`Player`] (cheap to clone) queues each command on one command thread,
//!   in order, so the UI thread never waits for the player (`qust_player::Player`'s calls block
//!   until the player has handled them). A command with an answer returns a [`Reply`]; the
//!   answer travels back through the same channel as the events, so when a `Reply` resolves,
//!   every event the command caused has already been applied (the page relied on that: "the
//!   cartridge drop animation starts when `cartridge_move` resolves").
//! - **Attach ordering.** `player_attach` returns the snapshot after the events it caused were
//!   emitted (autoplay, stream sync). The UI task holds back events until the snapshot arrives,
//!   applies the snapshot, then the held events, as the page's `early` queue did.

use crate::model::{Entry, PlayerState, Playlist, SettingChange, Status, StreamInfo};
use crate::models::{Models, SettingsModel};
use gpui::{App, AsyncApp, Global};
use qust_fluent::analyzer::{Frame, FRAME_LEN};
use qust_player::{FrameSink, Host, OutputKind, Store};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// An event from the player, typed.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    State(PlayerState),
    Playlist(Playlist),
    Entry(Entry),
    Status(Status),
    Setting(SettingChange),
    /// Anything else (with its raw payload).
    Other(String, Value),
}

impl Event {
    /// `event` / `payload` as `Host::emit` gets them.
    pub fn parse(event: &str, payload: Value) -> Event {
        let parsed = match event {
            "player-state" => serde_json::from_value(payload.clone()).map(Event::State).ok(),
            "player-playlist" => serde_json::from_value(payload.clone()).map(Event::Playlist).ok(),
            "player-entry" => serde_json::from_value(payload.clone()).map(Event::Entry).ok(),
            "player-status" => serde_json::from_value(payload.clone()).map(Event::Status).ok(),
            "settings-changed" => serde_json::from_value(payload.clone()).map(Event::Setting).ok(),
            _ => None,
        };
        parsed.unwrap_or_else(|| Event::Other(event.to_string(), payload))
    }
}

type ReplyFn = Box<dyn FnOnce() + Send>;

enum Msg {
    // An attach is about to run: hold back state events until its snapshot.
    Attaching,
    Event(Event),
    Frame,
    Snapshot(PlayerState, Playlist),
    Reply(ReplyFn),
}

/// The player's `Host`: typed events to the UI thread.
pub struct UiHost {
    tx: smol::channel::Sender<Msg>,
}

impl Host for UiHost {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.tx.try_send(Msg::Event(Event::parse(event, payload)));
    }
}

/// The newest analyzer frame, waiting for the UI thread.
struct FrameSlot {
    frame: Mutex<Frame>,
    pending: AtomicBool,
    sent: AtomicU64,
}

/// An answer that will come back on the UI thread, after the events its command caused.
pub struct Reply<T>(smol::channel::Receiver<T>);

impl<T> Reply<T> {
    /// The answer; None if the player went away.
    pub async fn get(self) -> Option<T> {
        self.0.recv().await.ok()
    }
}

type Command = Box<dyn FnOnce(&qust_player::Player) + Send>;

/// The player, for views: every call returns at once.
#[derive(Clone)]
pub struct Player {
    commands: mpsc::Sender<Command>,
    ui: smol::channel::Sender<Msg>,
    slot: Arc<FrameSlot>,
    store: Store,
}

impl Player {
    fn queue(&self, command: impl FnOnce(&qust_player::Player) + Send + 'static) {
        let _ = self.commands.send(Box::new(command));
    }

    /// Runs `f` on the command thread; its answer resolves on the UI thread after the events it
    /// caused.
    pub fn ask<T: Send + 'static>(&self, f: impl FnOnce(&qust_player::Player) -> T + Send + 'static) -> Reply<T> {
        let (tx, rx) = smol::channel::bounded(1);
        let ui = self.ui.clone();
        self.queue(move |player| {
            let value = f(player);
            let _ = ui.try_send(Msg::Reply(Box::new(move || {
                let _ = tx.try_send(value);
            })));
        });
        Reply(rx)
    }

    /// The player window opened: attach it (frames start, the engine starts, autoplay).
    pub fn attach(&self) {
        let ui = self.ui.clone();
        let slot = self.slot.clone();
        let wake = self.ui.clone();
        let _ = self.ui.try_send(Msg::Attaching);
        self.queue(move |player| {
            let sink: FrameSink = Arc::new(move |bytes: &[u8]| {
                if bytes.len() != FRAME_LEN {
                    return;
                }
                if let Ok(mut frame) = slot.frame.lock() {
                    frame.copy_from_slice(bytes);
                }
                slot.sent.fetch_add(1, Ordering::Relaxed);
                if !slot.pending.swap(true, Ordering::AcqRel) {
                    let _ = wake.try_send(Msg::Frame);
                }
            });
            let snapshot = player.attach(sink);
            let state = serde_json::from_value(snapshot["state"].clone()).unwrap_or_default();
            let playlist = serde_json::from_value(snapshot["playlist"].clone()).unwrap_or_default();
            let _ = ui.try_send(Msg::Snapshot(state, playlist));
        });
    }

    /// The player window is gone (pending saves written, playback stops, the engine goes).
    pub fn detach(&self) {
        self.queue(|player| player.detach());
    }

    /// The same, waiting up to `timeout` for it to finish (when the app is about to exit, so
    /// the pending saves are written first). Blocks the calling thread.
    pub fn detach_and_wait(&self, timeout: std::time::Duration) -> bool {
        let (tx, rx) = mpsc::channel();
        self.queue(move |player| {
            player.detach();
            let _ = tx.send(());
        });
        rx.recv_timeout(timeout).is_ok()
    }

    pub fn play(&self) {
        self.queue(|p| p.play());
    }
    pub fn pause(&self) {
        self.queue(|p| p.pause());
    }
    pub fn stop(&self) {
        self.queue(|p| p.stop());
    }
    pub fn seek(&self, seconds: f64) {
        self.queue(move |p| p.seek(seconds));
    }
    /// 0–100.
    pub fn set_volume(&self, value: i64) {
        self.queue(move |p| p.set_volume(value));
    }
    /// −100–100.
    pub fn set_balance(&self, value: i64) {
        self.queue(move |p| p.set_balance(value));
    }
    pub fn set_instant(&self, on: bool) {
        self.queue(move |p| p.set_instant(on));
    }
    pub fn cycle_colors(&self) {
        self.queue(|p| p.cycle_colors());
    }
    /// A shortcut, the tray or `--toggle`: "play", "stop" or "toggle".
    pub fn action(&self, action: &'static str) {
        self.queue(move |p| p.action(action));
    }
    pub fn request_play(&self, play: bool) {
        self.queue(move |p| p.request_play(play));
    }
    /// Plays cartridge `slot` (0–5).
    pub fn select_slot(&self, slot: usize) {
        self.queue(move |p| p.select_slot(slot));
    }
    pub fn save_cartridge(&self, slot: usize, title: String, url: String) -> Reply<Result<(), String>> {
        self.ask(move |p| p.save_cartridge(slot, title, url))
    }
    pub fn eject(&self, slot: usize) {
        self.queue(move |p| p.eject(slot));
    }
    /// Swap, or move to an empty slot. Resolves once the deck shows the new order.
    pub fn move_cartridge(&self, from: usize, to: usize) -> Reply<()> {
        self.ask(move |p| p.move_cartridge(from, to))
    }
    /// Plays playlist entry `id`.
    pub fn select_entry(&self, id: u64) {
        self.queue(move |p| p.select_entry(id));
    }
    /// The ids added.
    pub fn add_url(&self, url: String, title: String) -> Reply<Result<Vec<u64>, String>> {
        self.ask(move |p| p.add_url(url, title))
    }
    /// Dropped or picked paths: expanded (folders, `.m3u` / `.pls`) on the command thread, added
    /// before `index` (or at the end). The ids added.
    pub fn add_paths(&self, paths: Vec<PathBuf>, index: Option<usize>) -> Reply<Vec<u64>> {
        self.ask(move |p| p.add_entries(qust_player::playlist::expand(&paths), index))
    }
    pub fn remove_entries(&self, ids: Vec<u64>) {
        self.queue(move |p| p.remove_entries(ids));
    }
    /// Before `index` in the list as it is.
    pub fn move_entries(&self, ids: Vec<u64>, index: usize) -> Reply<()> {
        self.ask(move |p| p.move_entries(ids, index))
    }
    /// `url: None` for a local file.
    pub fn edit_entry(&self, id: u64, title: String, url: Option<String>) -> Reply<Result<(), String>> {
        self.ask(move |p| p.edit_entry(id, title, url))
    }
    /// `player_status` (the Stream information window as it opens).
    pub fn status(&self) -> Reply<Status> {
        self.ask(|p| serde_json::from_value(p.status()).unwrap_or_default())
    }
    /// `stream_info(url)`: the details of a stream or file, by its bare URL or path.
    pub fn stream_info(&self, url: String) -> Reply<StreamInfo> {
        self.ask(move |_| serde_json::from_value(qust_player::stream_info(url)).unwrap_or_default())
    }

    /// A setting written by a window (`saveData`): saved on the command thread; the player hears
    /// about the keys it follows (`Store::write`).
    pub fn write_setting(&self, key: &str, value: Option<Value>) {
        let store = self.store.clone();
        let key = key.to_string();
        self.queue(move |_| store.write(&key, value));
    }

    /// A setting as the store has it now (reads memory, never the disk).
    pub fn setting(&self, key: &str) -> Option<Value> {
        self.store.get(key)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Frames the player has sent since it started (measurements).
    pub fn frames_sent(&self) -> u64 {
        self.slot.sent.load(Ordering::Relaxed)
    }
}

/// The bridge, a GPUI global: the models and the player handle.
#[derive(Clone)]
pub struct Bridge {
    pub models: Models,
    pub player: Player,
}

impl Global for Bridge {}

impl Bridge {
    /// Starts the player (on `output`, with `store`) and the UI task. Nothing is attached yet:
    /// call `player.attach()` when the player window opens.
    pub fn start(store: Store, output: OutputKind, cx: &mut App) -> Bridge {
        let models = Models::new(cx);
        models.apply_settings(SettingsModel::from_values(|key| store.get(key)), cx);
        let (tx, rx) = smol::channel::unbounded::<Msg>();
        let core = qust_player::Player::start(store.clone(), UiHost { tx: tx.clone() }, output);
        let (commands, queue) = mpsc::channel::<Command>();
        let worker = core.clone();
        let spawned = std::thread::Builder::new().name("qust-ui-commands".into()).spawn(move || {
            while let Ok(command) = queue.recv() {
                command(&worker);
            }
        });
        if let Err(error) = spawned {
            eprintln!("qust-ui: no command thread: {error}");
        }
        let slot = Arc::new(FrameSlot { frame: Mutex::new([0; FRAME_LEN]), pending: AtomicBool::new(false), sent: AtomicU64::new(0) });
        let player = Player { commands, ui: tx, slot: slot.clone(), store };
        let task_models = models.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            // Events that arrive before the attach snapshot wait for it.
            let mut early: Option<Vec<Event>> = None;
            while let Ok(msg) = rx.recv().await {
                cx.update(|cx| match msg {
                    Msg::Attaching => early = Some(Vec::new()),
                    Msg::Event(event) => match &mut early {
                        Some(queue) if matches!(event, Event::State(_) | Event::Playlist(_) | Event::Entry(_)) => queue.push(event),
                        _ => apply(&task_models, event, cx),
                    },
                    Msg::Snapshot(state, playlist) => {
                        task_models.apply_state(state, cx);
                        task_models.apply_playlist(playlist, cx);
                        for event in early.take().unwrap_or_default() {
                            apply(&task_models, event, cx);
                        }
                    }
                    Msg::Frame => {
                        slot.pending.store(false, Ordering::Release);
                        let frame = slot.frame.lock().map(|frame| *frame).unwrap_or([0; FRAME_LEN]);
                        task_models.apply_frame(frame, cx);
                    }
                    Msg::Reply(reply) => reply(),
                });
            }
        })
        .detach();
        let bridge = Bridge { models, player };
        cx.set_global(bridge.clone());
        bridge
    }

    /// The installed bridge.
    pub fn global(cx: &App) -> &Bridge {
        cx.global::<Bridge>()
    }
}

fn apply(models: &Models, event: Event, cx: &mut App) {
    match event {
        Event::State(state) => models.apply_state(state, cx),
        Event::Playlist(playlist) => models.apply_playlist(playlist, cx),
        Event::Entry(entry) => models.apply_entry(entry, cx),
        Event::Status(status) => models.apply_status(status, cx),
        Event::Setting(change) => models.apply_setting(&change.key, &change.value, cx),
        Event::Other(_, _) => {}
    }
}
