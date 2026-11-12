// The player's state, on its own thread (lib.rs). A port of the old page's player (script.js):
// the same functions under the same names where they still exist, the same constants, the same
// store keys. An <audio> element there is a stream in the engine here, under the same key.

use super::model::{self, Cartridge, Entry, COLOR_CYCLE, SLOTS, URL_ERROR};
use super::{Host, Player};
use crate::store::Store;
use crate::visualizer::{FrameSink, Visualizer};
use qust_audio::{Config, Engine, Event, OutputKind, Source, Status, StreamId};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const LIVE_RESUME: Duration = Duration::from_secs(10);
const INFO_INTERVAL: Duration = Duration::from_secs(2);
const STATUS_INTERVAL: Duration = Duration::from_secs(15);
// Reconnecting after a drop: right away, then a try every second for 15 tries, then every 15 s
// until the time set in Settings (`reconnect-minutes`, 0 = never give up) runs out.
const RETRY_FAST_TRIES: u32 = 15;
const RETRY_FAST_DELAY: Duration = Duration::from_secs(1);
const RETRY_SLOW_DELAY: Duration = Duration::from_secs(15);
// A try that hasn't started playing by then failed (the engine gives up connecting after 10 s).
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(12);
// Playing, but the clock hasn't moved for this long: the stream is stuck. Also how long a live
// stream may wait for audio after running out before it reconnects: the engine resumes once it
// has a cushion of up to 2 s again, which a stream sent in real time takes about as long to fill.
const STALL_TIMEOUT: Duration = Duration::from_secs(3);
// Playing this long again ends a reconnect sequence; a drop before that continues it.
const RECOVERED_TIMEOUT: Duration = Duration::from_secs(10);
// At most this many streams at once: the six cartridges (a playing URL entry takes one's place).
const MAX_STREAMS: usize = 6;
const CLOCK_INTERVAL: Duration = Duration::from_millis(250);
const SECOND: Duration = Duration::from_secs(1);
const SAVE_PLAYLIST_DELAY: Duration = Duration::from_millis(400);
const SAVE_LEVEL_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Stopped,
    Connecting,
    Playing,
    Buffering,
    Paused,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Stopped => "stopped",
            State::Connecting => "connecting",
            State::Playing => "playing",
            State::Buffering => "buffering",
            State::Paused => "paused",
        }
    }

    fn idle(self) -> bool {
        matches!(self, State::Stopped | State::Paused)
    }
}

// What plays: a cartridge or a playlist entry, as active() gave it.
#[derive(Clone, Debug)]
struct Src {
    key: String,
    title: String,
    path: Option<String>,
    url: Option<String>,
    slot: Option<usize>,
    entry: Option<u64>,
}

impl Src {
    fn source(&self) -> Source {
        match &self.path {
            Some(path) => Source::File(PathBuf::from(path)),
            None => Source::Url(self.url.clone().unwrap_or_default()),
        }
    }

    fn same(&self, other: &Src) -> bool {
        self.key == other.key && self.slot == other.slot && self.entry == other.entry
    }
}

#[derive(Clone, Copy)]
enum Step {
    Reconnect,
    Lost,
}

// The reconnect sequence in progress.
struct Retry {
    since: Instant,
    tries: u32,
    started_at: Instant,
    timer: Option<(Instant, Step)>,
    recovered_at: Option<Instant>,
}

pub(crate) struct Core {
    host: Box<dyn Host>,
    store: Store,
    player: Player,
    output: OutputKind,
    pub(crate) engine: Option<Engine>,
    vis: Visualizer,
    attached: bool,
    pub(crate) play_request: bool,

    cartridges: [Option<Cartridge>; SLOTS],
    playlist: Vec<Entry>,
    next_id: u64,
    active_slot: Option<usize>,
    active_entry: Option<u64>,
    state: State,
    instant: bool,
    volume: i64,
    balance: i64,
    colors: &'static str,
    autoplay: bool,
    reconnect_minutes: f64,

    paused_at: Instant,
    paused_key: Option<String>,
    stream_info: Value,
    status_title: Option<String>,
    last_status_fetch: Option<Instant>,
    // times playback stalled since the stream started
    stalls: u32,
    retry: Option<Retry>,
    // why playback stopped on its own ("Connection lost"), shown in place of "Stopped"
    notice: Option<String>,
    // the active stream's last seen progress, for the stall watchdog
    progress: Option<(StreamId, f64, Instant)>,
    // The clock shows clock_base (time carried over a reconnect) plus the active stream's time
    // since clock_start (its time when it became active). A file's clock is its position.
    clock_start: f64,
    clock_base: f64,
    // playlist entries skipped in a row because they wouldn't play
    skips: usize,
    probing: bool,

    // timers
    waiting_until: Option<Instant>,
    info_at: Option<Instant>,
    background_retry: Option<Instant>,
    clock_at: Instant,
    second_at: Instant,
    save_playlist_at: Option<Instant>,
    save_volume_at: Option<Instant>,
    save_balance_at: Option<Instant>,
    force_state: bool,
    force_status: bool,
    // what the page was last sent, without the clock fields
    last_state: Option<Value>,
    last_status: Option<Value>,
}

fn user_agent() -> String {
    format!("Qust/{}", env!("CARGO_PKG_VERSION"))
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(on) => *on,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

impl Core {
    // init() in the old page: everything from the store.
    pub fn new(store: Store, host: Box<dyn Host>, output: OutputKind, player: Player) -> Core {
        let now = Instant::now();
        let mut cartridges: [Option<Cartridge>; SLOTS] = Default::default();
        for (slot, cartridge) in cartridges.iter_mut().enumerate() {
            let n = slot + 1;
            let url = store.get(&format!("player{n}-url")).as_ref().and_then(model::text);
            if let Some(url) = url {
                let title = store.get(&format!("player{n}-title")).as_ref().and_then(model::text).unwrap_or_else(|| model::host(&url));
                *cartridge = Some(Cartridge { title, url });
            }
        }
        let last = store.get("last-selected-player");
        let last_text = match &last {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let active_slot = last_text
            .replacen("player", "", 1)
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|digits| digits.parse::<usize>().ok())
            .and_then(|n| n.checked_sub(1))
            .filter(|slot| *slot < SLOTS && cartridges[*slot].is_some());

        let mut next_id = 1;
        let mut playlist = Vec::new();
        if let Some(Value::Array(items)) = store.get("playlist") {
            for item in &items {
                if let Some(entry) = model::to_entry(item, next_id) {
                    playlist.push(entry);
                    next_id += 1;
                }
            }
        }
        let index = store.get("playlist-index").and_then(|value| value.as_u64());
        let active_entry = match (last_text == "playlist", index) {
            (true, Some(index)) => playlist.get(index as usize).map(|entry: &Entry| entry.id),
            _ => None,
        };
        let number = |key: &str| store.get(key).and_then(|value| value.as_f64());
        let volume = number("volume").map_or(50, |v| v.round().clamp(0.0, 100.0) as i64);
        let balance = number("balance").map_or(0, |v| v.round().clamp(-100.0, 100.0) as i64);
        let colors = store
            .get("visualizer-colors")
            .and_then(|value| COLOR_CYCLE.iter().find(|name| value.as_str() == Some(name)).copied())
            .unwrap_or("classic");
        let autoplay = store.get("autoplay") == Some(Value::Bool(true));
        let reconnect_minutes = number("reconnect-minutes").filter(|m| *m >= 0.0).unwrap_or(5.0);
        let instant = store.get("instant-switch") == Some(Value::Bool(true));

        let vis = Visualizer::new();
        vis.set_colors(colors);
        Core {
            host,
            store,
            player,
            output,
            engine: None,
            vis,
            attached: false,
            play_request: false,
            cartridges,
            playlist,
            next_id,
            active_slot: if active_entry.is_some() { None } else { active_slot },
            active_entry,
            state: State::Stopped,
            instant,
            volume,
            balance,
            colors,
            autoplay,
            reconnect_minutes,
            paused_at: now,
            paused_key: None,
            stream_info: json!({}),
            status_title: None,
            last_status_fetch: None,
            stalls: 0,
            retry: None,
            notice: None,
            progress: None,
            clock_start: 0.0,
            clock_base: 0.0,
            skips: 0,
            probing: false,
            waiting_until: None,
            info_at: None,
            background_retry: None,
            clock_at: now,
            second_at: now,
            save_playlist_at: None,
            save_volume_at: None,
            save_balance_at: None,
            force_state: false,
            force_status: false,
            last_state: None,
            last_status: None,
        }
    }

    /* The page */

    // The player page loaded: the engine starts with its window. The snapshot is taken before
    // anything is emitted to it, and events follow it in order (the page queues the early ones).
    pub fn attach(&mut self, frames: FrameSink) -> Value {
        if self.engine.is_none() {
            let player = self.player.clone();
            let config = Config { codecs: qust_codecs::registry(), output: self.output, user_agent: user_agent() };
            let engine = Engine::new(config, move |event| player.event(event));
            engine.set_volume(self.volume as f32 / 100.0);
            engine.set_balance(self.balance as f32 / 100.0);
            self.engine = Some(engine);
        }
        let tap = self.engine.as_ref().map(|engine| engine.tap()).expect("the engine");
        self.attached = true;
        self.vis.attach(frames, tap);
        self.vis.set_state(self.state.name());
        let state = self.state_json();
        self.last_state = Some(comparable(&state, &["elapsed", "position"]));
        let snapshot = json!({ "state": state, "playlist": self.playlist_json() });
        self.probe_durations();
        self.sync_streams();
        // Opened by the background helper to play, or "Play on startup".
        if (self.play_request || self.autoplay) && self.active_slot.is_some() && self.state == State::Stopped {
            self.play();
        }
        self.play_request = false;
        snapshot
    }

    // The player window is gone. Playback doesn't outlive it, and the engine goes too, so Qust
    // left in the tray runs none of its threads; the next window starts a new one.
    pub fn detach(&mut self) {
        self.flush_saves();
        self.attached = false;
        self.stop();
        self.engine = None;
        self.vis.detach();
        self.last_state = None;
        self.last_status = None;
    }

    pub fn quit(&mut self) {
        self.detach();
    }

    pub fn action(&mut self, action: &str) {
        if !self.attached {
            self.play_request = action != "stop";
            return;
        }
        match action {
            "play" if self.state.idle() => self.play(),
            "stop" => self.stop(),
            "toggle" if self.state.idle() => self.play(),
            "toggle" => self.stop(),
            _ => {}
        }
    }

    fn emit(&self, event: &str, payload: Value) {
        if self.attached {
            self.host.emit(event, payload);
        }
    }

    fn emit_playlist(&self) {
        self.emit("player-playlist", self.playlist_json());
    }

    fn emit_entry(&self, id: u64) {
        if let Some(entry) = self.entry(id) {
            self.emit("player-entry", entry.json());
        }
    }

    pub fn playlist_json(&self) -> Value {
        json!({ "entries": self.playlist.iter().map(Entry::json).collect::<Vec<_>>() })
    }

    // After every message: what changed goes out (the clock fields alone only with the timers).
    pub fn refresh(&mut self) {
        if !self.attached {
            return;
        }
        let state = self.state_json();
        let key = comparable(&state, &["elapsed", "position"]);
        if self.force_state || self.last_state.as_ref() != Some(&key) {
            self.host.emit("player-state", state);
            self.last_state = Some(key);
        }
        let status = self.status_json();
        let key = comparable(&status, &["buffer"]);
        if self.force_status || self.last_status.as_ref() != Some(&key) {
            self.host.emit("player-status", status);
            self.last_status = Some(key);
        }
        self.force_state = false;
        self.force_status = false;
    }

    /* Streams */

    fn active(&self) -> Option<Src> {
        if let Some(cartridge) = self.active_slot.and_then(|slot| self.cartridges[slot].as_ref()) {
            let url = cartridge.url.clone();
            return Some(Src { key: url.clone(), title: cartridge.title.clone(), path: None, url: Some(url), slot: self.active_slot, entry: None });
        }
        let entry = self.entry(self.active_entry?)?;
        Some(Src { key: entry.key(), title: entry.title.clone(), path: entry.path.clone(), url: entry.url.clone(), slot: None, entry: Some(entry.id) })
    }

    fn entry(&self, id: u64) -> Option<&Entry> {
        self.playlist.iter().find(|entry| entry.id == id)
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.playlist.iter_mut().find(|entry| entry.id == id)
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.playlist.iter().position(|entry| entry.id == id)
    }

    fn status(&self, key: &str) -> Option<Status> {
        self.engine.as_ref()?.status(key)
    }

    // The active stream, if it is loaded: current() in the old page.
    fn current(&self) -> Option<(Src, Status)> {
        let source = self.active()?;
        let status = self.status(&source.key)?;
        Some((source, status))
    }

    // An element's currentTime: a file's position, a stream's time played.
    fn time(status: &Status) -> f64 {
        if status.duration.is_some() {
            status.position
        } else {
            status.played
        }
    }

    fn load(&self, key: &str, source: Source, start_at: Option<f64>) {
        if let Some(engine) = &self.engine {
            engine.load(key, source, start_at);
        }
    }

    // Keeps the right streams connected: the active one while playing, plus, in instant mode, every
    // loaded cartridge (muted), at most six streams in all, even while stopped. Only the 6
    // cartridges preload, never playlist entries. Everything else is unloaded, so with instant
    // switch off nothing stays loaded once stopped.
    fn sync_streams(&mut self) {
        let Some(engine) = self.engine.clone() else { return };
        if !self.attached {
            engine.unload_all();
            return;
        }
        let source = self.active();
        let active_key = source.as_ref().filter(|_| self.state != State::Stopped).map(|source| source.key.clone());
        let mut wanted: Vec<String> = active_key.iter().cloned().collect();
        if self.instant {
            let mut room = MAX_STREAMS - usize::from(active_key.is_some() && source.as_ref().is_some_and(|source| source.path.is_none()));
            for cartridge in self.cartridges.iter().flatten() {
                if wanted.contains(&cartridge.url) || room == 0 {
                    continue;
                }
                wanted.push(cartridge.url.clone());
                room -= 1;
            }
        }
        for key in engine.keys() {
            if !wanted.contains(&key) {
                engine.unload(&key);
            }
        }
        for key in &wanted {
            if Some(key) == active_key.as_ref() {
                continue;
            }
            match engine.status(key) {
                None => {
                    engine.load(key, Source::Url(key.clone()), None);
                }
                Some(status) if status.failed || status.ended => {
                    engine.load(key, Source::Url(key.clone()), None);
                }
                Some(status) if status.paused => engine.set_paused(key, false),
                Some(_) => {}
            }
        }
        for key in engine.keys() {
            engine.set_level(&key, if Some(&key) == active_key.as_ref() { 1.0 } else { 0.0 });
        }
    }

    // A background stream (instant switch) dropped: reconnect it a little later.
    fn retry_background(&mut self) {
        if self.background_retry.is_none() {
            self.background_retry = Some(Instant::now() + RETRY_SLOW_DELAY);
        }
    }

    // The old element events, from the engine.
    pub fn engine_event(&mut self, event: Event) {
        let (key, id) = match &event {
            Event::Playing { key, id } | Event::Waiting { key, id } | Event::Ended { key, id } | Event::Failed { key, id, .. } | Event::Duration { key, id, .. } => (key.clone(), *id),
        };
        let Some(status) = self.status(&key).filter(|status| status.id == id) else { return };
        let source = self.active();
        let current = source.as_ref().is_some_and(|source| source.key == key);
        match event {
            Event::Playing { .. } => {
                if !current || self.state.idle() {
                    return;
                }
                if let Some(retry) = &mut self.retry {
                    retry.timer = None;
                    retry.recovered_at = Some(Instant::now());
                }
                self.waiting_until = None;
                self.skips = 0;
                self.set_state(State::Playing);
            }
            // The buffer ran out. A file downloading slower than it plays just waits; a live stream
            // gets STALL_TIMEOUT to come back before it reconnects.
            Event::Waiting { .. } => {
                if !current || self.state != State::Playing {
                    return;
                }
                if status.duration.is_none() {
                    self.stalls += 1;
                    self.waiting_until = Some(Instant::now() + STALL_TIMEOUT);
                }
                self.set_state(State::Buffering);
            }
            Event::Duration { seconds, .. } => {
                let Some(id) = self.active_entry.filter(|_| current) else { return };
                let Some(entry) = self.entry_mut(id).filter(|entry| entry.key() == key) else { return };
                if entry.duration == seconds {
                    return;
                }
                entry.duration = seconds;
                self.emit_entry(id);
                self.save_playlist_soon();
            }
            // The connection closed or failed, or a file ended. A source that never played isn't
            // retried (wrong address, missing file): the playlist skips it.
            Event::Ended { .. } | Event::Failed { .. } => {
                let ended = matches!(event, Event::Ended { .. });
                if !current || self.state == State::Stopped {
                    self.retry_background();
                } else if self.state == State::Paused {
                } else if ended && status.duration.is_some() {
                    self.play_next();
                } else if self.state == State::Connecting && self.retry.is_none() {
                    self.could_not_play();
                } else {
                    self.lost();
                }
            }
        }
    }

    fn set_state(&mut self, next: State) {
        self.state = next;
        self.vis.set_state(next.name());
    }

    /* Playback */

    pub fn play(&mut self) {
        let Some(source) = self.active() else { return };
        if !self.attached {
            self.play_request = true;
            return;
        }
        self.end_retry();
        self.notice = None;
        self.waiting_until = None;
        let key = source.key.clone();
        let status = self.status(&key);
        let finite = status.as_ref().is_some_and(|status| status.duration.is_some());
        // A file resumes where it was paused; a stream only shortly after, or it rejoins the live
        // point. One that failed or ended meanwhile connects again.
        let resume = self.state == State::Paused
            && self.paused_key.as_deref() == Some(key.as_str())
            && status.as_ref().is_some_and(|status| !status.failed && !status.ended)
            && (finite || self.paused_at.elapsed() < LIVE_RESUME);
        let live = !resume && !finite && status.as_ref().is_some_and(|status| status.live);
        if !resume {
            self.stalls = 0;
            self.stream_info = json!({});
            self.status_title = None;
            self.last_status_fetch = None;
            self.clock_base = 0.0;
            self.clock_start = match (&status, live) {
                (Some(status), true) => status.played,
                _ => 0.0,
            };
            if !live {
                self.load(&key, source.source(), None);
            }
        }
        self.set_state(if live {
            State::Playing
        } else if resume {
            State::Buffering
        } else {
            State::Connecting
        });
        if let Some(engine) = &self.engine {
            engine.set_paused(&key, false);
        }
        // The engine has no event for unpausing: a stream that was playing plays on at once.
        if resume && self.status(&key).is_some_and(|status| status.live) {
            self.set_state(State::Playing);
        }
        self.start_info();
        self.sync_streams();
    }

    pub fn pause(&mut self) {
        if self.state.idle() {
            return;
        }
        self.end_retry();
        self.waiting_until = None;
        let key = self.active().map(|source| source.key);
        if let (Some(engine), Some(key)) = (&self.engine, &key) {
            engine.set_paused(key, true);
        }
        self.paused_key = key;
        self.paused_at = Instant::now();
        self.set_state(State::Paused);
    }

    pub fn stop(&mut self) {
        self.end_retry();
        self.notice = None;
        self.waiting_until = None;
        self.set_state(State::Stopped);
        self.sync_streams();
        self.info_at = None;
        self.stream_info = json!({});
        self.status_title = None;
    }

    // Stops and says why in place of the title.
    fn fail(&mut self, text: &str) {
        self.stop();
        self.notice = Some(text.to_string());
    }

    // The active source never started. In the playlist, that entry is marked and the next one
    // plays, until every entry has been tried.
    fn could_not_play(&mut self) {
        let Some(id) = self.active_entry else {
            return self.fail("Couldn't play this stream");
        };
        let file = self.entry(id).is_some_and(|entry| entry.path.is_some());
        if let Some(entry) = self.entry_mut(id) {
            entry.missing = true;
        }
        self.emit_entry(id);
        self.skips += 1;
        if self.skips < self.playlist.len() && self.play_next() {
            return;
        }
        self.skips = 0;
        self.fail(if file { "Couldn't play this file" } else { "Couldn't play this stream" });
    }

    // The playlist entry after the active one (none after the last: the playlist plays once).
    fn play_next(&mut self) -> bool {
        let next = self.active_entry.and_then(|id| self.index_of(id)).and_then(|index| self.playlist.get(index + 1)).map(|entry| entry.id);
        match next {
            Some(id) => {
                self.select_entry(id);
                true
            }
            None => {
                self.stop();
                false
            }
        }
    }

    /* Reconnecting */

    fn end_retry(&mut self) {
        self.retry = None;
    }

    // The active stream dropped or got stuck: reconnect right away the first time, then on the
    // schedule above. Each try that fails (error, closed, or not playing in time) calls this again.
    fn lost(&mut self) {
        if self.state.idle() || self.active().is_none() {
            return;
        }
        if self.state == State::Playing {
            self.stalls += 1;
        }
        self.waiting_until = None;
        let now = Instant::now();
        let retry = self.retry.get_or_insert(Retry { since: now, tries: 0, started_at: now, timer: None, recovered_at: None });
        retry.timer = None;
        retry.recovered_at = None;
        // Counted from the start of the last try, which may itself have taken a while to fail.
        let interval = if retry.tries < RETRY_FAST_TRIES { RETRY_FAST_DELAY } else { RETRY_SLOW_DELAY };
        let delay = if retry.tries == 0 { Duration::ZERO } else { interval.saturating_sub(now - retry.started_at) };
        let limit = Duration::from_secs_f64(self.reconnect_minutes * 60.0);
        if !limit.is_zero() && (now + delay) - retry.since > limit {
            return self.fail("Connection lost");
        }
        retry.timer = Some((now + delay, Step::Reconnect));
        if self.state != State::Buffering {
            self.set_state(State::Buffering);
        }
    }

    // A fresh connection under the same key: the old one may hold a dead connection or a stuck
    // decoder. A file picks up where it was; a stream's clock carries on from where it was.
    fn reconnect(&mut self) {
        let Some(source) = self.active() else { return };
        if self.retry.is_none() || self.state.idle() {
            return;
        }
        let now = Instant::now();
        let position = self.status(&source.key).filter(|status| status.duration.is_some()).map(|status| status.position);
        self.clock_base = if position.is_none() { self.elapsed() } else { 0.0 };
        self.clock_start = 0.0;
        self.load(&source.key, source.source(), position);
        self.sync_streams();
        if let Some(retry) = &mut self.retry {
            retry.tries += 1;
            retry.started_at = now;
            retry.timer = Some((now + ATTEMPT_TIMEOUT, Step::Lost));
        }
    }

    // Every second: catch a stream whose clock stopped without an event (the player would otherwise
    // sit on "playing" with no sound). The engine recovers its output device by itself; while it
    // has none, nothing moves and that isn't the stream's fault.
    fn watch(&mut self) {
        if self.state.idle() {
            self.progress = None;
            return;
        }
        if self.retry.as_ref().and_then(|retry| retry.recovered_at).is_some_and(|at| at.elapsed() > RECOVERED_TIMEOUT) {
            self.end_retry();
        }
        let current = self.current().filter(|_| self.state == State::Playing);
        let Some((_, status)) = current else {
            self.progress = None;
            return;
        };
        let now = Instant::now();
        let time = Self::time(&status);
        match self.progress {
            Some((id, seen, _)) if id == status.id && seen == time => {}
            _ => {
                self.progress = Some((status.id, time, now));
                return;
            }
        }
        if self.progress.is_some_and(|(_, _, at)| now - at < STALL_TIMEOUT) {
            return;
        }
        self.progress = None;
        let output = self.engine.as_ref().and_then(|engine| engine.tap().read(0.0, &mut [], &mut []));
        if output.is_some() {
            self.lost();
        }
    }

    /* Stream details: the engine's info, and the Icecast status page as a title fallback */

    fn start_info(&mut self) {
        self.poll_info();
        self.info_at = Some(Instant::now() + INFO_INTERVAL);
    }

    fn poll_info(&mut self) {
        let Some(source) = self.active() else { return };
        let mut info = qust_audio::info(&source.key);
        if !info.is_object() {
            info = json!({});
        }
        // Keep the last title across a reconnect, until the new connection sends one.
        if self.retry.is_some() && !truthy(&info["title"]) && truthy(&self.stream_info["title"]) {
            info["title"] = self.stream_info["title"].clone();
        }
        // A file whose header doesn't give its bitrate (Ogg, M4A, FLAC): its average.
        let duration = self.status(&source.key).and_then(|status| status.duration);
        if let (false, Some(size), Some(duration)) = (truthy(&info["bitrate"]), info["size"].as_f64().filter(|size| *size > 0.0), duration) {
            info["bitrate"] = json!((size * 8.0 / duration / 1000.0).round() as u64);
        }
        self.stream_info = info;
        // Only streams have an Icecast status page.
        let stream = source.path.is_none() && duration.is_none();
        let due = self.last_status_fetch.is_none_or(|at| at.elapsed() > STATUS_INTERVAL);
        if stream && !truthy(&self.stream_info["title"]) && due {
            self.last_status_fetch = Some(Instant::now());
            let player = self.player.clone();
            let url = source.url.clone().unwrap_or_default();
            // Blocking for up to 10 s, so not on this thread. The answer is dropped if the
            // station changed meanwhile.
            let _ = thread::Builder::new().name("qust-status-title".into()).spawn(move || {
                let title = qust_audio::icecast_title(&url, &user_agent());
                player.cast(move |core| {
                    if core.active().is_some_and(|now| now.same(&source)) {
                        core.status_title = title;
                    }
                });
            });
        }
    }

    /* Display */

    // The first and second lines and whether the second is dim, as renderTitle().
    fn titles(&self) -> (String, String, bool) {
        let source = self.active();
        let station = match self.active_entry {
            Some(id) => match self.index_of(id) {
                Some(index) => format!("Playlist · {} of {}", index + 1, self.playlist.len()),
                None => "Playlist".into(),
            },
            None => source.as_ref().map_or_else(|| "No cartridge loaded".into(), |source| source.title.clone()),
        };
        let Some(source) = source else {
            return (station, "Load a cartridge to start".into(), true);
        };
        let (title, dim) = match self.state {
            State::Stopped => (self.notice.clone().unwrap_or_else(|| "Stopped".into()), true),
            State::Connecting => ("Connecting…".into(), true),
            State::Buffering if self.retry.is_some() => ("Reconnecting…".into(), true),
            _ => {
                let info = |field: &str| model::text(&self.stream_info[field]);
                let mut title = info("title").or_else(|| self.status_title.clone().filter(|title| !title.is_empty()));
                if source.entry.is_none() {
                    title = title.or_else(|| info("name"));
                }
                let title = title.unwrap_or_else(|| source.title.clone());
                (if self.state == State::Paused { format!("{title} (paused)") } else { title }, false)
            }
        };
        (station, title, dim)
    }

    fn elapsed(&self) -> f64 {
        self.clock_base + self.current().map_or(0.0, |(_, status)| (Self::time(&status) - self.clock_start).max(0.0))
    }

    pub fn state_json(&self) -> Value {
        let (station, title, dim) = self.titles();
        let cartridges: Vec<Value> = self
            .cartridges
            .iter()
            .enumerate()
            .map(|(slot, cartridge)| {
                let Some(cartridge) = cartridge else { return Value::Null };
                // Instant switch: a bolt on the other cartridges, faint while connecting, solid once live.
                let preloaded = self.instant && (Some(slot) != self.active_slot || self.state == State::Stopped);
                let preload = match self.status(&cartridge.url).filter(|_| preloaded) {
                    Some(status) if status.live => json!("live"),
                    Some(_) => json!("loading"),
                    None => Value::Null,
                };
                json!({ "title": cartridge.title, "url": cartridge.url, "preload": preload })
            })
            .collect();
        let stopped = self.state == State::Stopped;
        let info = |field: &str| if stopped { Value::Null } else { Some(self.stream_info[field].clone()).filter(truthy).unwrap_or(Value::Null) };
        // The seek bar: for a file with a known length (seekDuration and renderSeek in the old page).
        let current = self.current();
        let finite = current.as_ref().and_then(|(_, status)| status.duration);
        let entry_duration = self.active_entry.and_then(|id| self.entry(id)).and_then(|entry| entry.duration);
        let seekable = !stopped && finite.is_some();
        json!({
            "state": self.state.name(),
            "cartridges": cartridges,
            "activeSlot": self.active_slot,
            "activeId": self.active_entry,
            "instant": self.instant,
            "station": station,
            "title": title,
            "titleDim": dim,
            "bitrate": info("bitrate"),
            "samplerate": info("samplerate"),
            "channels": info("channels"),
            "volume": self.volume,
            "balance": self.balance,
            "colors": self.colors,
            "elapsed": if stopped { 0.0 } else { self.elapsed() },
            "duration": finite.or(entry_duration),
            "position": current.filter(|_| seekable).map(|(_, status)| status.position),
            "seekable": seekable,
        })
    }

    // For the Stream information window (`player-status`, `player_status`).
    pub fn status_json(&self) -> Value {
        let source = self.active();
        let buffer = match self.state {
            State::Stopped => Value::Null,
            _ => json!(self.current().map_or(0.0, |(_, status)| status.buffered.max(0.0))),
        };
        json!({
            "state": self.state.name(),
            "station": source.as_ref().map(|source| source.title.clone()),
            "url": source.as_ref().and_then(|source| source.path.clone().or_else(|| source.url.clone())),
            "buffer": buffer,
            "stalls": self.stalls,
            "statusTitle": self.status_title,
        })
    }

    /* Commands */

    pub fn seek(&mut self, seconds: f64) {
        if self.state == State::Stopped {
            return;
        }
        if let (Some((source, status)), Some(engine)) = (self.current(), &self.engine) {
            if status.duration.is_some() {
                engine.seek(&source.key, seconds);
            }
        }
    }

    pub fn set_volume(&mut self, value: i64) {
        self.volume = value.clamp(0, 100);
        if let Some(engine) = &self.engine {
            engine.set_volume(self.volume as f32 / 100.0);
        }
        self.save_volume_at = Some(Instant::now() + SAVE_LEVEL_DELAY);
    }

    pub fn set_balance(&mut self, value: i64) {
        self.balance = value.clamp(-100, 100);
        if let Some(engine) = &self.engine {
            engine.set_balance(self.balance as f32 / 100.0);
        }
        self.save_balance_at = Some(Instant::now() + SAVE_LEVEL_DELAY);
    }

    pub fn set_instant(&mut self, on: bool) {
        self.instant = on;
        self.store.set("instant-switch", json!(on));
        self.sync_streams();
    }

    // Clicking the analyzer cycles its colors, "off" included (also in Settings).
    pub fn cycle_colors(&mut self) {
        let index = COLOR_CYCLE.iter().position(|name| *name == self.colors).unwrap_or(0);
        let value = COLOR_CYCLE[(index + 1) % COLOR_CYCLE.len()];
        self.set_colors(value);
        self.store.set("visualizer-colors", json!(value));
        self.emit("settings-changed", json!({ "key": "colors", "value": value }));
    }

    fn set_colors(&mut self, colors: &'static str) {
        self.colors = colors;
        self.vis.set_colors(colors);
    }

    // Written to the store by the Settings window.
    pub fn setting_changed(&mut self, key: &str, value: Option<Value>) {
        let value = value.unwrap_or(Value::Null);
        match key {
            "visualizer-colors" => {
                if let Some(colors) = COLOR_CYCLE.iter().find(|name| value.as_str() == Some(name)) {
                    self.set_colors(colors);
                }
            }
            "autoplay" => self.autoplay = value == Value::Bool(true),
            "reconnect-minutes" => {
                if let Some(minutes) = value.as_f64().filter(|minutes| *minutes >= 0.0) {
                    self.reconnect_minutes = minutes;
                }
            }
            _ => {}
        }
    }

    /* Cartridges */

    fn save_slot(&self, slot: usize) {
        let n = slot + 1;
        match &self.cartridges[slot] {
            Some(cartridge) => {
                self.store.set(&format!("player{n}-title"), json!(cartridge.title));
                self.store.set(&format!("player{n}-url"), json!(cartridge.url));
            }
            None => {
                self.store.remove(&format!("player{n}-title"));
                self.store.remove(&format!("player{n}-url"));
            }
        }
    }

    pub fn select_slot(&mut self, slot: usize) {
        let Some(cartridge) = self.cartridges.get(slot).cloned().flatten() else { return };
        if !self.instant && self.state != State::Stopped {
            self.stop();
        }
        self.active_slot = Some(slot);
        self.active_entry = None;
        self.store.set("last-selected-player", json!(format!("player{}", slot + 1)));
        self.store.set("last-selected-title", json!(cartridge.title));
        self.store.set("last-selected-url", json!(cartridge.url));
        self.play();
    }

    pub fn save_cartridge(&mut self, slot: usize, title: String, url: String) -> Result<(), String> {
        let url = url.trim().to_string();
        if !model::valid_url(&url) {
            return Err(URL_ERROR.into());
        }
        if slot >= SLOTS {
            return Ok(());
        }
        let previous = self.cartridges[slot].take();
        let title = match title.trim() {
            "" => model::host(&url),
            title => title.to_string(),
        };
        self.cartridges[slot] = Some(Cartridge { title: title.clone(), url: url.clone() });
        self.save_slot(slot);
        if Some(slot) == self.active_slot {
            self.store.set("last-selected-title", json!(title));
            self.store.set("last-selected-url", json!(url));
            if previous.is_some_and(|previous| previous.url != url) && self.state != State::Stopped {
                self.stop();
                self.play();
            }
        }
        self.sync_streams();
        Ok(())
    }

    pub fn eject(&mut self, slot: usize) {
        if slot >= SLOTS {
            return;
        }
        self.cartridges[slot] = None;
        self.save_slot(slot);
        if Some(slot) == self.active_slot {
            self.stop();
            self.active_slot = None;
            self.store.remove("last-selected-player");
            self.store.remove("last-selected-title");
            self.store.remove("last-selected-url");
        }
        self.sync_streams();
    }

    // Swaps two slots, or moves a cartridge to an empty one. Streams are keyed by URL, so they
    // aren't disturbed.
    pub fn move_cartridge(&mut self, from: usize, to: usize) {
        if from >= SLOTS || to >= SLOTS || from == to {
            return;
        }
        self.cartridges.swap(from, to);
        self.save_slot(from);
        self.save_slot(to);
        if self.active_slot == Some(from) {
            self.active_slot = Some(to);
        } else if self.active_slot == Some(to) {
            self.active_slot = Some(from);
        }
        if let Some(slot) = self.active_slot {
            self.store.set("last-selected-player", json!(format!("player{}", slot + 1)));
        }
    }

    /* Playlist */

    fn save_playlist_soon(&mut self) {
        self.save_playlist_at = Some(Instant::now() + SAVE_PLAYLIST_DELAY);
    }

    fn save_playlist(&self) {
        self.store.set("playlist", Value::Array(self.playlist.iter().map(Entry::saved).collect()));
        if let Some(index) = self.active_entry.and_then(|id| self.index_of(id)) {
            self.store.set("playlist-index", json!(index));
        }
    }

    pub fn select_entry(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else { return };
        self.active_slot = None;
        self.active_entry = Some(id);
        if std::mem::take(&mut self.playlist[index].missing) {
            self.emit_entry(id);
        }
        self.store.set("last-selected-player", json!("playlist"));
        self.store.set("playlist-index", json!(index));
        self.play();
    }

    // Adds files or URLs ({ path | url, title?, duration? }) at `index` (the end without one).
    // With nothing to play yet, the first one becomes what Play plays.
    pub fn add_entries(&mut self, items: &[Value], index: Option<usize>) -> Vec<u64> {
        let mut entries = Vec::new();
        for item in items {
            if let Some(entry) = model::to_entry(item, self.next_id) {
                self.next_id += 1;
                entries.push(entry);
            }
        }
        let ids: Vec<u64> = entries.iter().map(|entry| entry.id).collect();
        if ids.is_empty() {
            return ids;
        }
        let at = index.unwrap_or(self.playlist.len()).min(self.playlist.len());
        self.playlist.splice(at..at, entries);
        if self.active().is_none() {
            self.active_entry = Some(ids[0]);
        }
        self.save_playlist_soon();
        self.emit_playlist();
        self.probe_durations();
        ids
    }

    pub fn add_url(&mut self, url: String, title: String) -> Result<Vec<u64>, String> {
        let url = url.trim().to_string();
        if !model::valid_url(&url) {
            return Err(URL_ERROR.into());
        }
        let title = title.trim();
        Ok(self.add_entries(&[json!({ "url": url, "title": if title.is_empty() { Value::Null } else { json!(title) } })], None))
    }

    // Removing the active entry stops it; the next one becomes what Play plays.
    pub fn remove_entries(&mut self, ids: &[u64]) {
        let removing: HashSet<u64> = ids.iter().copied().collect();
        if !self.playlist.iter().any(|entry| removing.contains(&entry.id)) {
            return;
        }
        if let Some(active) = self.active_entry.filter(|id| removing.contains(id)) {
            if self.state != State::Stopped {
                self.stop();
            }
            let index = self.index_of(active).unwrap_or(0);
            let after = self.playlist[index + 1..].iter().find(|entry| !removing.contains(&entry.id));
            let last = self.playlist.iter().rev().find(|entry| !removing.contains(&entry.id));
            self.active_entry = after.or(last).map(|entry| entry.id);
        }
        self.playlist.retain(|entry| !removing.contains(&entry.id));
        self.save_playlist_soon();
        self.emit_playlist();
    }

    // Moves the entries before `index` (an index in the playlist as it is).
    pub fn move_entries(&mut self, ids: &[u64], index: usize) {
        let moving: HashSet<u64> = ids.iter().copied().collect();
        if !self.playlist.iter().any(|entry| moving.contains(&entry.id)) {
            return;
        }
        let index = index.min(self.playlist.len());
        let before = self.playlist[index..].iter().find(|entry| !moving.contains(&entry.id)).map(|entry| entry.id);
        let (moved, mut rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.playlist).into_iter().partition(|entry| moving.contains(&entry.id));
        let at = before.and_then(|id| rest.iter().position(|entry| entry.id == id)).unwrap_or(rest.len());
        rest.splice(at..at, moved);
        self.playlist = rest;
        self.save_playlist_soon();
        self.emit_playlist();
    }

    // Renames an entry, and for a URL entry changes its address. The active entry reconnects to a
    // new address if it was playing.
    pub fn edit_entry(&mut self, id: u64, title: String, url: Option<String>) -> Result<(), String> {
        let Some(entry) = self.entry(id) else { return Ok(()) };
        let url = match entry.path {
            Some(_) => None,
            None => {
                let url = url.unwrap_or_default().trim().to_string();
                if !model::valid_url(&url) {
                    return Err(URL_ERROR.into());
                }
                Some(url)
            }
        };
        if let Some(url) = url.filter(|url| Some(url) != entry.url.as_ref()) {
            let was_playing = self.active_entry == Some(id) && self.state != State::Stopped;
            if was_playing {
                self.stop();
            }
            if let Some(entry) = self.entry_mut(id) {
                entry.url = Some(url);
                entry.duration = None;
                entry.missing = false;
            }
            if was_playing {
                self.play();
            }
        }
        if let Some(entry) = self.entry_mut(id) {
            entry.title = match title.trim() {
                "" => entry.default_title(),
                title => title.to_string(),
            };
        }
        self.save_playlist_soon();
        self.emit_entry(id);
        Ok(())
    }

    // Durations of local files, read one at a time on another thread.
    fn probe_durations(&mut self) {
        if self.probing || !self.attached {
            return;
        }
        let Some(entry) = self.playlist.iter_mut().find(|entry| entry.path.is_some() && entry.duration.is_none() && !entry.probed) else { return };
        entry.probed = true;
        let (id, path) = (entry.id, PathBuf::from(entry.path.clone().unwrap_or_default()));
        self.probing = true;
        let player = self.player.clone();
        let spawned = thread::Builder::new().name("qust-probe".into()).spawn(move || {
            let result = qust_audio::probe_duration(&path, qust_codecs::registry());
            player.cast(move |core| core.probed(id, result));
        });
        if spawned.is_err() {
            self.probing = false;
        }
    }

    fn probed(&mut self, id: u64, result: Result<Option<f64>, String>) {
        self.probing = false;
        if let Some(entry) = self.entry_mut(id) {
            let changed = match result {
                Err(_) => !std::mem::replace(&mut entry.missing, true),
                Ok(Some(duration)) => entry.duration.replace(duration) != Some(duration),
                Ok(None) => false,
            };
            if changed {
                self.emit_entry(id);
                self.save_playlist_soon();
            }
        }
        self.probe_durations();
    }

    /* Timers */

    pub fn next_deadline(&self) -> Option<Instant> {
        let mut deadlines = vec![self.waiting_until, self.info_at, self.background_retry, self.save_playlist_at, self.save_volume_at, self.save_balance_at];
        deadlines.push(self.retry.as_ref().and_then(|retry| retry.timer).map(|(at, _)| at));
        if self.attached {
            deadlines.push(Some(self.second_at));
            if self.state != State::Stopped {
                deadlines.push(Some(self.clock_at));
            }
        }
        deadlines.into_iter().flatten().min()
    }

    pub fn timers(&mut self) {
        let now = Instant::now();
        let due = |at: Option<Instant>| at.is_some_and(|at| at <= now);
        if let Some((at, step)) = self.retry.as_ref().and_then(|retry| retry.timer) {
            if at <= now {
                if let Some(retry) = &mut self.retry {
                    retry.timer = None;
                }
                match step {
                    Step::Reconnect => self.reconnect(),
                    Step::Lost => self.lost(),
                }
            }
        }
        if due(self.waiting_until) {
            self.waiting_until = None;
            if self.state == State::Buffering && self.retry.is_none() {
                self.lost();
            }
        }
        if due(self.background_retry) {
            self.background_retry = None;
            self.sync_streams();
        }
        if due(self.info_at) {
            self.poll_info();
            self.info_at = Some(now + INFO_INTERVAL);
        }
        if self.attached && self.second_at <= now {
            self.second_at = now + SECOND;
            self.watch();
            self.force_status = true;
        }
        if self.attached && self.state != State::Stopped && self.clock_at <= now {
            self.clock_at = now + CLOCK_INTERVAL;
            self.force_state = true;
        }
        if due(self.save_playlist_at) {
            self.save_playlist_at = None;
            self.save_playlist();
        }
        if due(self.save_volume_at) {
            self.save_volume_at = None;
            self.store.set("volume", json!(self.volume));
        }
        if due(self.save_balance_at) {
            self.save_balance_at = None;
            self.store.set("balance", json!(self.balance));
        }
    }

    fn flush_saves(&mut self) {
        if self.save_playlist_at.take().is_some() {
            self.save_playlist();
        }
        if self.save_volume_at.take().is_some() {
            self.store.set("volume", json!(self.volume));
        }
        if self.save_balance_at.take().is_some() {
            self.store.set("balance", json!(self.balance));
        }
    }
}

// A copy without the fields that change continuously, to tell whether anything else did.
fn comparable(value: &Value, skip: &[&str]) -> Value {
    let mut value = value.clone();
    if let Some(map) = value.as_object_mut() {
        for field in skip {
            map.remove(*field);
        }
    }
    value
}
