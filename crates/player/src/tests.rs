// The player end to end, without a window: a real engine on the Null output, a store in a
// temporary file, a host that records what would go to the windows, WAV files written here and
// a small HTTP server in the test that streams WAV in real time (with ICY titles), drops or
// refuses connections, or serves a file. No network beyond 127.0.0.1, no sound.

use super::machine::Core;
use super::{Host, Player};
use crate::store::Store;
use qust_audio::OutputKind;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RATE: u32 = 48000;

/* Files */

static UNIQUE: AtomicUsize = AtomicUsize::new(0);

fn temp(name: &str, extension: &str) -> PathBuf {
    let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("qust-player-test-{}-{n}-{name}.{extension}", std::process::id()))
}

fn wav_header(frames: u32) -> Vec<u8> {
    let bytes = frames.saturating_mul(4);
    let mut data = Vec::with_capacity(44);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&36u32.saturating_add(bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&RATE.to_le_bytes());
    data.extend_from_slice(&(RATE * 4).to_le_bytes());
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    data
}

// Frames of a 440 Hz tone, from frame `from`.
fn pcm(from: u64, frames: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(frames * 4);
    for i in 0..frames as u64 {
        let t = (from + i) as f64 / RATE as f64;
        let sample = ((t * 440.0 * std::f64::consts::TAU).sin() * 0.3 * 32767.0) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }
    data
}

fn wav_bytes(seconds: f64) -> Vec<u8> {
    let frames = (seconds * RATE as f64) as u32;
    let mut data = wav_header(frames);
    data.extend(pcm(0, frames as usize));
    data
}

fn wav_file(name: &str, seconds: f64) -> String {
    let path = temp(name, "wav");
    std::fs::write(&path, wav_bytes(seconds)).unwrap();
    path.to_string_lossy().to_string()
}

/* The HTTP server */

#[derive(Clone)]
enum Plan {
    // A live stream: `burst` seconds at once, then real time. It stops sending for `gap`
    // (start, length) and closes after `drop_after` seconds.
    Live { burst: f64, title: Option<&'static str>, gap: Option<(f64, f64)>, drop_after: Option<f64> },
    // Closes the connection without answering.
    Refuse,
    NotFound,
    // A file with a length, ranges accepted.
    File(Arc<Vec<u8>>),
    Json(String),
}

fn live() -> Plan {
    Plan::Live { burst: 1.0, title: None, gap: None, drop_after: None }
}

type Plans = Arc<dyn Fn(&str, usize) -> Plan + Send + Sync>;

struct Server {
    port: u16,
    connections: Arc<Mutex<HashMap<String, usize>>>,
}

impl Server {
    // `plans(path, n)` decides what the n-th connection (from 1) to `path` gets.
    fn start(plans: impl Fn(&str, usize) -> Plan + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let plans: Plans = Arc::new(plans);
        let counts = connections.clone();
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let (plans, counts) = (plans.clone(), counts.clone());
                thread::spawn(move || serve(socket, plans, counts));
            }
        });
        Server { port, connections }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn count(&self, path: &str) -> usize {
        self.connections.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

fn serve(socket: TcpStream, plans: Plans, counts: Arc<Mutex<HashMap<String, usize>>>) {
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut range = None;
    let mut meta = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) <= 2 {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("range: bytes=") {
            range = value.trim().trim_end_matches('-').parse::<usize>().ok();
        }
        meta |= lower.starts_with("icy-metadata: 1");
    }
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
    let n = {
        let mut counts = counts.lock().unwrap();
        let count = counts.entry(path.clone()).or_default();
        *count += 1;
        *count
    };
    let mut socket = socket;
    let _ = socket.set_nodelay(true);
    match plans(&path, n) {
        Plan::Refuse => {}
        Plan::NotFound => {
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
        Plan::Json(body) => {
            let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
        Plan::File(bytes) => {
            let from = range.unwrap_or(0).min(bytes.len());
            let head = match range {
                Some(_) => format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\n", bytes.len() - 1, bytes.len()),
                None => "HTTP/1.1 200 OK\r\n".to_string(),
            };
            let _ = write!(socket, "{head}Content-Type: audio/wav\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len() - from);
            let _ = socket.write_all(&bytes[from..]);
        }
        Plan::Live { burst, title, gap, drop_after } => {
            let _ = stream_live(&mut socket, burst, title.filter(|_| meta), gap, drop_after);
        }
    }
}

const METAINT: usize = 16000;

fn stream_live(socket: &mut TcpStream, burst: f64, title: Option<&str>, gap: Option<(f64, f64)>, drop_after: Option<f64>) -> std::io::Result<()> {
    let mut head = "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nicy-name: Test Radio\r\nServer: Qust test\r\nConnection: close\r\n".to_string();
    if title.is_some() {
        head += &format!("icy-metaint: {METAINT}\r\n");
    }
    socket.write_all(format!("{head}\r\n").as_bytes())?;
    let mut until_meta = METAINT;
    let mut write = |socket: &mut TcpStream, mut data: &[u8]| -> std::io::Result<()> {
        while !data.is_empty() {
            let Some(title) = title else { return socket.write_all(data) };
            let count = data.len().min(until_meta);
            socket.write_all(&data[..count])?;
            data = &data[count..];
            until_meta -= count;
            if until_meta == 0 {
                let mut text = format!("StreamTitle='{title}';").into_bytes();
                text.resize(text.len().div_ceil(16) * 16, 0);
                socket.write_all(&[(text.len() / 16) as u8])?;
                socket.write_all(&text)?;
                until_meta = METAINT;
            }
        }
        Ok(())
    };
    // A WAV header for an hour: the engine takes it for a stream (no length, ICY or not).
    write(socket, &wav_header(RATE * 3600))?;
    let start = Instant::now();
    let mut sent = 0u64;
    loop {
        let t = start.elapsed().as_secs_f64();
        if drop_after.is_some_and(|at| t >= at) {
            return Ok(());
        }
        let paused = gap.map_or(0.0, |(at, length)| (t - at).clamp(0.0, length));
        let due = ((burst + t - paused) * RATE as f64) as u64;
        if due > sent {
            write(socket, &pcm(sent, (due - sent) as usize))?;
            sent = due;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/* The player */

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<(String, Value)>>>);

impl Host for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

struct Rig {
    player: Player,
    events: Recorder,
    frames: Arc<Mutex<Vec<Vec<u8>>>>,
    // the store, which the tests also write as the Settings window does (set_data)
    settings: Store,
    store: PathBuf,
    // a second player opens it next
    keep: bool,
}

impl Rig {
    // A player on a new store holding `settings`.
    fn new(name: &str, settings: Value) -> Rig {
        let store = temp(name, "json");
        std::fs::write(&store, serde_json::to_string(&settings).unwrap()).unwrap();
        Rig::open(store)
    }

    fn open(store: PathBuf) -> Rig {
        let events = Recorder::default();
        let settings = Store::open(store.clone());
        let player = Player::start(settings.clone(), events.clone(), OutputKind::Null);
        // As main.rs wires it.
        let watcher = player.clone();
        settings.watch(move |key, value| watcher.setting_changed(key, value.cloned()));
        Rig { player, events, frames: Arc::default(), settings, store, keep: false }
    }

    fn attach(&self) -> Value {
        let frames = self.frames.clone();
        self.player.attach(Arc::new(move |frame: &[u8]| frames.lock().unwrap().push(frame.to_vec())))
    }

    fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut Core) -> T + Send + 'static) -> T {
        self.player.call(f).expect("the player")
    }

    fn state(&self) -> Value {
        self.call(|core| core.state_json())
    }

    // Polls the state until `ok`, or fails with what it was.
    fn wait(&self, seconds: f64, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            let state = self.state();
            if ok(&state) {
                return state;
            }
            assert!(start.elapsed().as_secs_f64() < seconds, "waited {seconds} s for {what}; state: {state:#}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_state(&self, seconds: f64, name: &str) -> Value {
        self.wait(seconds, name, |state| state["state"] == name)
    }

    fn events(&self, name: &str) -> Vec<Value> {
        self.events.0.lock().unwrap().iter().filter(|(event, _)| event == name).map(|(_, payload)| payload.clone()).collect()
    }

    // The titles the page was shown, in order, without repeats.
    fn titles(&self) -> Vec<String> {
        let mut titles: Vec<String> = Vec::new();
        for state in self.events("player-state") {
            let title = state["title"].as_str().unwrap_or("").to_string();
            if titles.last() != Some(&title) {
                titles.push(title);
            }
        }
        titles
    }

    fn keys(&self) -> Vec<String> {
        self.call(|core| core.engine.as_ref().map(|engine| engine.keys()).unwrap_or_default())
    }

    fn saved(&self, key: &str) -> Option<Value> {
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&self.store).unwrap()).unwrap();
        data.get(key).cloned()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.player.shutdown();
        if !self.keep {
            let _ = std::fs::remove_file(&self.store);
        }
    }
}

fn sleep(seconds: f64) {
    thread::sleep(Duration::from_secs_f64(seconds));
}

fn num(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}

/* Tests */

#[test]
fn attach_gives_the_snapshot_then_events_and_frames() {
    let server = Server::start(|_, _| live());
    let rig = Rig::new(
        "attach",
        json!({
            "player1-title": "One", "player1-url": server.url("/one"),
            "last-selected-player": "player1", "autoplay": true,
            "playlist": [{ "title": "A file", "path": "C:/nowhere/a.mp3", "duration": 12.5 }],
        }),
    );
    // Nothing reaches the page, and nothing connects, before it attaches.
    sleep(0.3);
    assert!(rig.events.0.lock().unwrap().is_empty());
    assert_eq!(server.count("/one"), 0);

    let snapshot = rig.attach();
    let state = &snapshot["state"];
    assert_eq!(state["state"], "stopped");
    assert_eq!(state["cartridges"][0], json!({ "title": "One", "url": server.url("/one"), "preload": null }));
    assert_eq!(state["activeSlot"], 0);
    assert_eq!(state["station"], "One");
    assert_eq!(state["title"], "Stopped");
    assert_eq!(state["volume"], 50);
    let entries = snapshot["playlist"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["title"], "A file");
    assert_eq!(entries[0]["duration"], 12.5);
    assert!(entries[0]["id"].is_u64());

    // Autoplay: the events after the snapshot start from it.
    let first = rig.events("player-state").first().cloned().expect("a state after attach");
    assert_eq!(first["state"], "connecting");
    assert_eq!(first["title"], "Connecting…");
    assert_eq!(first["titleDim"], true);
    rig.wait_state(5.0, "playing");

    // The analyzer: frames that change, lit bars with the tone playing at level 1.
    sleep(1.0);
    {
        let frames = rig.frames.lock().unwrap();
        assert!(frames.iter().all(|frame| frame.len() == qust_spectrum::FRAME_LEN));
        assert!(frames.windows(2).all(|pair| pair[0] != pair[1]), "a frame was sent twice in a row");
        assert!(frames.iter().any(|frame| frame[1..].chunks(4).any(|bar| bar[0] > 0)), "no bars lit");
    }
    rig.call(|core| core.stop());
    let frames = rig.frames.lock().unwrap();
    assert!(frames.last().unwrap().iter().all(|b| *b == 0), "the last frame isn't blank");
    let count = frames.len();
    drop(frames);
    sleep(0.3);
    assert_eq!(rig.frames.lock().unwrap().len(), count, "frames after stopping");
    // Statuses for the info window went out too.
    assert!(rig.events("player-status").iter().any(|status| status["state"] == "playing"));
}

#[test]
fn cartridge_play_pause_resume_stop() {
    let server = Server::start(|_, _| Plan::Live { burst: 1.0, title: Some("Artist - Song"), gap: None, drop_after: None });
    let rig = Rig::new("cartridge", json!({ "player2-title": "Radio", "player2-url": server.url("/live") }));
    let state = rig.attach()["state"].clone();
    assert_eq!((state["station"].as_str(), state["title"].as_str()), (Some("No cartridge loaded"), Some("Load a cartridge to start")));

    rig.call(|core| core.select_slot(1));
    let state = rig.state();
    assert_eq!(state["state"], "connecting");
    assert_eq!(state["station"], "Radio");
    let state = rig.wait(6.0, "the ICY title", |state| state["title"] == "Artist - Song");
    assert_eq!(state["state"], "playing");
    assert_eq!(state["titleDim"], false);
    assert_eq!(state["samplerate"], 48000);
    assert_eq!(state["channels"], 2);
    assert!(state["duration"].is_null() && state["seekable"] == false);
    assert_eq!(rig.saved("last-selected-player"), Some(json!("player2")));
    sleep(1.0);

    rig.call(|core| core.pause());
    let paused = rig.state();
    assert_eq!(paused["title"], "Artist - Song (paused)");
    sleep(1.0);
    let still = rig.state();
    assert!((num(&still["elapsed"]) - num(&paused["elapsed"])).abs() < 0.1, "the clock moved while paused");

    // Within 10 s, the same connection carries on.
    rig.call(|core| core.play());
    let resumed = rig.wait_state(2.0, "playing");
    assert_eq!(server.count("/live"), 1);
    assert!(num(&resumed["elapsed"]) >= num(&paused["elapsed"]) - 0.05);

    rig.call(|core| core.stop());
    let state = rig.state();
    assert_eq!((state["state"].as_str(), state["title"].as_str(), state["elapsed"].as_f64()), (Some("stopped"), Some("Stopped"), Some(0.0)));
    assert!(state["bitrate"].is_null() && state["channels"].is_null());
    // With instant switch off nothing stays loaded, so the output can close.
    assert!(rig.keys().is_empty());
    let titles = rig.titles();
    for title in ["Connecting…", "Artist - Song", "Artist - Song (paused)", "Stopped"] {
        assert!(titles.iter().any(|shown| shown == title), "never showed {title}: {titles:?}");
    }
}

#[test]
fn a_stream_that_never_starts_is_not_retried() {
    let server = Server::start(|_, _| Plan::NotFound);
    let rig = Rig::new("never", json!({ "player1-title": "Gone", "player1-url": server.url("/gone") }));
    rig.attach();
    rig.call(|core| core.select_slot(0));
    let state = rig.wait(5.0, "the failure", |state| state["state"] == "stopped");
    assert_eq!(state["title"], "Couldn't play this stream");
    assert_eq!(state["titleDim"], true);
    sleep(2.5);
    assert_eq!(server.count("/gone"), 1);
    assert_eq!(rig.state()["title"], "Couldn't play this stream");
}

#[test]
fn a_drop_reconnects_and_keeps_the_clock() {
    let server = Server::start(|_, n| Plan::Live { burst: 1.0, title: Some("Song"), gap: None, drop_after: (n == 1).then_some(3.0) });
    let rig = Rig::new("drop", json!({ "player1-title": "Radio", "player1-url": server.url("/drop") }));
    rig.attach();
    rig.call(|core| core.select_slot(0));
    rig.wait(6.0, "the title", |state| state["title"] == "Song");
    let before = num(&rig.wait(4.0, "the drop", |state| state["state"] != "playing")["elapsed"]);
    let state = rig.wait(5.0, "playing again", |state| state["state"] == "playing" && server.count("/drop") == 2);
    assert!(num(&state["elapsed"]) >= before - 0.05, "clock went back from {before} to {}", state["elapsed"]);
    assert!(rig.titles().iter().any(|title| title == "Reconnecting…"), "{:?}", rig.titles());
    // The title carries over until the new connection sends one.
    assert_eq!(state["title"], "Song");
    let details = qust_audio::info(&server.url("/drop"));
    assert_eq!(details["reconnects"], 1);
    sleep(1.5);
    assert!(num(&rig.state()["elapsed"]) > before + 1.0);
}

#[test]
fn gives_up_after_the_reconnect_limit() {
    let server = Server::start(|_, n| if n == 1 { Plan::Live { burst: 0.5, title: None, gap: None, drop_after: Some(1.5) } } else { Plan::Refuse });
    // 0.05 minutes = 3 s, set the way the Settings window does.
    let rig = Rig::new("giveup", json!({ "player1-title": "Radio", "player1-url": server.url("/flaky") }));
    rig.attach();
    rig.settings.write("reconnect-minutes", Some(json!(0.05)));
    rig.call(|core| core.select_slot(0));
    rig.wait_state(5.0, "playing");
    rig.wait(5.0, "reconnecting", |state| state["title"] == "Reconnecting…");
    let start = Instant::now();
    let state = rig.wait(8.0, "giving up", |state| state["state"] == "stopped");
    assert_eq!(state["title"], "Connection lost");
    let waited = start.elapsed().as_secs_f64();
    assert!((1.5..5.0).contains(&waited), "gave up after {waited} s");
    // Right away, then about once a second.
    let tries = server.count("/flaky") - 1;
    assert!((3..=5).contains(&tries), "{tries} tries");
}

#[test]
fn a_short_wait_for_audio_does_not_reconnect() {
    let server = Server::start(|_, _| Plan::Live { burst: 0.3, title: None, gap: Some((2.0, 1.0)), drop_after: None });
    let rig = Rig::new("grace", json!({ "player1-title": "Radio", "player1-url": server.url("/gap") }));
    rig.attach();
    rig.call(|core| core.select_slot(0));
    rig.wait_state(5.0, "playing");
    rig.wait_state(4.0, "buffering");
    let state = rig.wait_state(3.0, "playing");
    assert_eq!(server.count("/gap"), 1);
    assert_eq!(state["title"], "Test Radio");
    assert!(!rig.titles().iter().any(|title| title == "Reconnecting…"));
    assert_eq!(rig.player.status()["stalls"], 1);
}

#[test]
fn a_long_wait_for_audio_reconnects() {
    let server = Server::start(|_, n| Plan::Live { burst: 0.3, title: None, gap: (n == 1).then_some((2.0, 20.0)), drop_after: None });
    let rig = Rig::new("stall", json!({ "player1-title": "Radio", "player1-url": server.url("/stall") }));
    rig.attach();
    rig.call(|core| core.select_slot(0));
    rig.wait_state(5.0, "playing");
    rig.wait_state(4.0, "buffering");
    let waiting = Instant::now();
    rig.wait(6.0, "reconnecting", |state| state["title"] == "Reconnecting…");
    let grace = waiting.elapsed().as_secs_f64();
    assert!((2.7..3.6).contains(&grace), "reconnected {grace} s after waiting");
    rig.wait(5.0, "playing again", |state| state["state"] == "playing");
    assert_eq!(server.count("/stall"), 2);
    assert_eq!(rig.player.status()["stalls"], 1);
}

#[test]
fn instant_switch() {
    let server = Server::start(|_, _| live());
    let mut settings = json!({});
    for n in 1..=6 {
        settings[format!("player{n}-title")] = json!(format!("Station {n}"));
        settings[format!("player{n}-url")] = json!(server.url(&format!("/s{n}")));
    }
    let rig = Rig::new("instant", settings);
    rig.attach();
    assert!(rig.keys().is_empty());
    rig.call(|core| core.set_instant(true));
    assert_eq!(rig.saved("instant-switch"), Some(json!(true)));
    // Every cartridge streams in the background while stopped (a local one can be live already).
    let state = rig.state();
    assert!(state["cartridges"].as_array().unwrap().iter().all(|cartridge| cartridge["preload"] == "loading" || cartridge["preload"] == "live"));
    let state = rig.wait(5.0, "all live", |state| state["cartridges"].as_array().unwrap().iter().all(|cartridge| cartridge["preload"] == "live"));
    assert_eq!(state["instant"], true);
    assert_eq!(rig.keys().len(), 6);

    // Starting and switching are immediate: no connecting, no new connection.
    sleep(1.0);
    rig.call(|core| core.select_slot(0));
    let state = rig.state();
    assert_eq!(state["state"], "playing");
    assert!(num(&state["elapsed"]) < 0.3, "the clock starts with the station: {}", state["elapsed"]);
    assert!(state["cartridges"][0]["preload"].is_null());
    assert_eq!(state["cartridges"][1]["preload"], "live");
    rig.call(|core| core.select_slot(1));
    let state = rig.state();
    assert_eq!(state["state"], "playing");
    assert_eq!(state["activeSlot"], 1);
    assert_eq!(state["cartridges"][0]["preload"], "live");
    assert!((1..=6).all(|n| server.count(&format!("/s{n}")) == 1));
    assert!(!rig.events("player-state").iter().any(|state| state["state"] == "connecting"));

    // A playlist URL playing takes one of the six.
    let ids = rig.call({
        let url = server.url("/list");
        move |core| core.add_url(url, String::new())
    });
    let id = ids.unwrap()[0];
    rig.call(move |core| core.select_entry(id));
    rig.wait_state(5.0, "playing");
    let keys = rig.keys();
    assert_eq!(keys.len(), 6, "{keys:?}");
    assert!(keys.contains(&server.url("/list")));

    // Stopped, the cartridges stay; with instant switch off, nothing does.
    rig.call(|core| core.stop());
    assert_eq!(rig.keys().len(), 6);
    let state = rig.state();
    assert!(state["cartridges"].as_array().unwrap().iter().all(|cartridge| !cartridge["preload"].is_null()));
    rig.call(|core| core.set_instant(false));
    assert!(rig.keys().is_empty());
    assert!(rig.state()["cartridges"].as_array().unwrap().iter().all(|cartridge| cartridge["preload"].is_null()));
}

#[test]
fn playlist_plays_through_skips_missing_and_stops_after_the_last() {
    let a = wav_file("a", 1.0);
    let c = wav_file("c", 1.2);
    let missing = temp("missing", "wav").to_string_lossy().to_string();
    let rig = Rig::new("through", json!({}));
    rig.attach();
    let items = vec![json!({ "path": a }), json!({ "path": missing, "title": "Missing" }), json!({ "path": c })];
    let ids = rig.call(move |core| core.add_entries(&items, None));
    assert_eq!(ids.len(), 3);
    let playlist = rig.events("player-playlist");
    let entries = playlist.last().unwrap()["entries"].as_array().unwrap().clone();
    assert_eq!(entries[0]["title"], format!("{}", PathBuf::from(&a).file_stem().unwrap().to_string_lossy()));
    assert_eq!(entries[1]["title"], "Missing");
    // Nothing was active: the first entry is what Play plays.
    let state = rig.state();
    assert_eq!(state["activeId"], ids[0]);
    assert_eq!(state["station"], "Playlist · 1 of 3");

    // Durations, one at a time; the file that isn't there is marked.
    let start = Instant::now();
    loop {
        let entries = rig.call(|core| core.playlist_json())["entries"].clone();
        if entries[0]["duration"].is_number() && entries[2]["duration"].is_number() && entries[1]["missing"] == true {
            assert!((num(&entries[0]["duration"]) - 1.0).abs() < 0.01);
            assert!((num(&entries[2]["duration"]) - 1.2).abs() < 0.01);
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "{entries:#}");
        sleep(0.02);
    }
    assert!(rig.events("player-entry").iter().any(|entry| entry["id"] == ids[1] && entry["missing"] == true));

    let first = ids[0];
    rig.call(move |core| core.select_entry(first));
    rig.wait_state(3.0, "playing");
    let state = rig.state();
    assert!((num(&state["duration"]) - 1.0).abs() < 0.01 && state["seekable"] == true);
    let last = ids[2];
    rig.wait(5.0, "the last entry", |state| state["activeId"] == last && state["state"] == "playing");
    assert_eq!(rig.state()["station"], "Playlist · 3 of 3");
    let state = rig.wait_state(5.0, "stopped");
    assert_eq!(state["activeId"], last);
    assert_eq!(state["title"], "Stopped");
    // Stopped, the seek layout stays for the file, from its saved duration.
    assert!((num(&state["duration"]) - 1.2).abs() < 0.01);
    assert!(state["seekable"] == false && state["position"].is_null());
    let entries = rig.call(|core| core.playlist_json())["entries"].clone();
    assert_eq!(entries[1]["missing"], true);
    sleep(0.5);
    let saved = rig.saved("playlist").unwrap();
    assert_eq!(saved[0], json!({ "title": entries[0]["title"], "path": a, "duration": entries[0]["duration"] }));
    assert_eq!(rig.saved("playlist-index"), Some(json!(2)));
    assert_eq!(rig.saved("last-selected-player"), Some(json!("playlist")));
    for path in [a, c] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn a_playlist_of_only_missing_files_says_so() {
    let rig = Rig::new("allmissing", json!({}));
    rig.attach();
    let items = vec![json!({ "path": temp("x", "mp3").to_string_lossy() }), json!({ "path": temp("y", "mp3").to_string_lossy() })];
    let ids = rig.call(move |core| core.add_entries(&items, None));
    let first = ids[0];
    rig.call(move |core| core.select_entry(first));
    let state = rig.wait(5.0, "the failure", |state| state["state"] == "stopped" && state["title"] != "Stopped");
    assert_eq!(state["title"], "Couldn't play this file");
}

#[test]
fn seek_pause_and_resume_a_file() {
    let file = wav_file("seek", 8.0);
    let rig = Rig::new("seek", json!({}));
    rig.attach();
    let items = vec![json!({ "path": file })];
    let id = rig.call(move |core| core.add_entries(&items, None))[0];
    rig.call(move |core| core.select_entry(id));
    let state = rig.wait_state(3.0, "playing");
    assert!((num(&state["duration"]) - 8.0).abs() < 0.01);
    assert_eq!(state["seekable"], true);

    rig.call(|core| core.seek(5.0));
    let state = rig.state();
    assert!((num(&state["position"]) - 5.0).abs() < 0.05, "{}", state["position"]);
    sleep(0.5);
    let state = rig.state();
    assert!((5.2..5.8).contains(&num(&state["position"])), "{}", state["position"]);
    assert!((num(&state["elapsed"]) - num(&state["position"])).abs() < 0.05, "a file's clock is its position");

    rig.call(|core| core.pause());
    let paused = num(&rig.state()["position"]);
    let id_before = rig.call(|core| core.engine.as_ref().unwrap().keys());
    sleep(1.2);
    assert!((num(&rig.state()["position"]) - paused).abs() < 0.05);
    // A file resumes where it paused, on the same stream.
    rig.call(|core| core.play());
    assert_eq!(rig.state()["state"], "playing");
    sleep(0.3);
    let position = num(&rig.state()["position"]);
    assert!((paused + 0.15..paused + 0.6).contains(&position), "paused at {paused}, now {position}");
    assert_eq!(rig.keys(), id_before);
    let _ = std::fs::remove_file(file);
}

#[test]
fn online_file_with_a_length_seeks_and_ends() {
    let bytes = Arc::new(wav_bytes(3.0));
    let server = Server::start(move |_, _| Plan::File(bytes.clone()));
    let rig = Rig::new("online", json!({}));
    rig.attach();
    let url = server.url("/music/Some%20Tune.wav");
    let ids = rig.call(move |core| core.add_url(url, String::new())).unwrap();
    let entry = rig.call(|core| core.playlist_json())["entries"][0].clone();
    assert_eq!(entry["title"], "Some Tune");
    let id = ids[0];
    rig.call(move |core| core.select_entry(id));
    let state = rig.wait(4.0, "playing with a length", |state| state["state"] == "playing" && state["duration"].is_number());
    assert!((num(&state["duration"]) - 3.0).abs() < 0.05);
    // The length found while playing is kept on the entry.
    assert!(rig.events("player-entry").iter().any(|entry| entry["id"] == id && entry["duration"].is_number()));
    rig.call(|core| core.seek(2.0));
    let state = rig.wait_state(3.0, "stopped");
    assert_eq!(state["title"], "Stopped");
}

#[test]
fn playlist_edit_move_and_remove() {
    let server = Server::start(|_, _| live());
    let one = wav_file("one", 5.0);
    let two = wav_file("two", 5.0);
    let rig = Rig::new("edit", json!({}));
    rig.attach();
    let items = vec![json!({ "url": server.url("/first") }), json!({ "path": one, "title": "One" }), json!({ "path": two })];
    let ids = rig.call(move |core| core.add_entries(&items, None));
    let (stream, file_one, file_two) = (ids[0], ids[1], ids[2]);

    // A bad address is refused with the old message.
    let error = rig.call(move |core| core.edit_entry(stream, String::new(), Some("ftp://x".into())));
    assert_eq!(error, Err("Enter an address starting with http:// or https://".into()));

    rig.call(move |core| core.select_entry(stream));
    rig.wait_state(5.0, "playing");
    // Changing the address of what plays reconnects to the new one; the name follows it.
    let url = server.url("/second");
    rig.call(move |core| core.edit_entry(stream, String::new(), Some(url))).unwrap();
    let entry = rig.events("player-entry").last().unwrap().clone();
    assert_eq!(entry["url"], server.url("/second"));
    assert_eq!(entry["title"], format!("127.0.0.1:{}", server.port));
    rig.wait(5.0, "playing the new address", |state| state["state"] == "playing" && server.count("/second") == 1);
    // A rename alone doesn't reconnect.
    let url = server.url("/second");
    rig.call(move |core| core.edit_entry(stream, "Renamed".into(), Some(url))).unwrap();
    assert_eq!(rig.state()["station"], "Playlist · 1 of 3");
    // An entry shows its own name, not the stream's.
    assert_eq!(rig.state()["title"], "Renamed");
    sleep(0.3);
    assert_eq!(server.count("/second"), 1);
    // A file's path can't change; its name can.
    rig.call(move |core| core.edit_entry(file_two, String::new(), None)).unwrap();
    assert_eq!(rig.events("player-entry").last().unwrap()["title"], PathBuf::from(&two).file_stem().unwrap().to_string_lossy().to_string());

    // Moving puts the entries before the index as the list is.
    rig.call(move |core| core.move_entries(&[stream], 3));
    let order: Vec<Value> = rig.events("player-playlist").last().unwrap()["entries"].as_array().unwrap().iter().map(|entry| entry["id"].clone()).collect();
    assert_eq!(order, vec![json!(file_one), json!(file_two), json!(stream)]);
    assert_eq!(rig.state()["station"], "Playlist · 3 of 3");
    rig.call(move |core| core.move_entries(&[file_two, stream], 0));
    let order: Vec<Value> = rig.call(|core| core.playlist_json())["entries"].as_array().unwrap().iter().map(|entry| entry["id"].clone()).collect();
    assert_eq!(order, vec![json!(file_two), json!(stream), json!(file_one)]);

    // Removing what plays stops it, and the next entry becomes what Play plays.
    rig.call(move |core| core.remove_entries(&[stream]));
    let state = rig.state();
    assert_eq!(state["state"], "stopped");
    assert_eq!(state["activeId"], file_one);
    assert_eq!(state["station"], "Playlist · 2 of 2");
    assert!(rig.keys().is_empty());
    rig.call(move |core| core.remove_entries(&[file_one, file_two]));
    let state = rig.state();
    assert!(state["activeId"].is_null());
    assert_eq!((state["station"].as_str(), state["title"].as_str()), (Some("No cartridge loaded"), Some("Load a cartridge to start")));
    for path in [one, two] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn cartridges_settings_and_persistence() {
    let server = Server::start(|_, _| live());
    let mut rig = Rig::new("persist", json!({}));
    rig.keep = true;
    rig.attach();
    let bad = rig.call(|core| core.save_cartridge(0, "X".into(), "nope".into()));
    assert_eq!(bad, Err("Enter an address starting with http:// or https://".into()));
    let (one, three) = (server.url("/one"), server.url("/three"));
    let url = one.clone();
    rig.call(move |core| core.save_cartridge(0, String::new(), url)).unwrap();
    let url = three.clone();
    rig.call(move |core| core.save_cartridge(2, "Three".into(), url)).unwrap();
    let state = rig.state();
    assert_eq!(state["cartridges"][0]["title"], format!("127.0.0.1:{}", server.port));
    assert_eq!(rig.saved("player3-title"), Some(json!("Three")));
    assert_eq!(rig.saved("player3-url"), Some(json!(three)));

    rig.call(|core| core.select_slot(2));
    rig.wait_state(5.0, "playing");
    // The active slot follows a move; streams don't notice.
    rig.call(|core| core.move_cartridge(2, 4));
    let state = rig.state();
    assert_eq!(state["activeSlot"], 4);
    assert_eq!(state["state"], "playing");
    assert!(state["cartridges"][2].is_null());
    assert_eq!(rig.saved("player3-url"), None);
    assert_eq!(rig.saved("player5-url"), Some(json!(three)));
    assert_eq!(rig.saved("last-selected-player"), Some(json!("player5")));
    assert_eq!(server.count("/three"), 1);
    // A swap.
    rig.call(|core| core.move_cartridge(0, 4));
    assert_eq!(rig.state()["activeSlot"], 0);
    assert_eq!(rig.saved("player1-url"), Some(json!(three)));
    assert_eq!(rig.saved("player5-url"), Some(json!(one)));
    // Saving a new address for what plays reconnects.
    let url = server.url("/three-b");
    rig.call(move |core| core.save_cartridge(0, "Three".into(), url)).unwrap();
    rig.wait(5.0, "the new address", |state| state["state"] == "playing" && server.count("/three-b") == 1);
    assert_eq!(rig.saved("last-selected-url"), Some(json!(server.url("/three-b"))));

    // Volume, balance and colors.
    rig.call(|core| core.set_volume(30));
    rig.call(|core| core.set_balance(-120));
    let state = rig.state();
    assert_eq!((state["volume"].as_i64(), state["balance"].as_i64()), (Some(30), Some(-100)));
    assert_eq!(rig.saved("volume"), None, "saved before the debounce");
    rig.call(|core| core.cycle_colors());
    assert_eq!(rig.state()["colors"], "white");
    assert_eq!(rig.events("settings-changed").last(), Some(&json!({ "key": "colors", "value": "white" })));
    assert_eq!(rig.saved("visualizer-colors"), Some(json!("white")));
    // Settings written by the Settings window.
    rig.settings.write("visualizer-colors", Some(json!("accent")));
    let state = rig.wait(1.0, "the colors", |state| state["colors"] == "accent");
    assert_eq!(rig.events("player-state").last().unwrap()["colors"], "accent");
    assert_eq!(state["colors"], "accent");
    rig.settings.write("autoplay", Some(json!(true)));

    // A playlist, with the entry last played.
    let items = vec![json!({ "url": server.url("/entry"), "title": "Entry", "duration": null })];
    let id = rig.call(move |core| core.add_entries(&items, None))[0];
    rig.call(move |core| core.select_entry(id));
    rig.wait_state(5.0, "playing");
    rig.call(|core| core.set_instant(true));
    rig.call(|core| core.eject(4));
    sleep(0.5);
    assert_eq!(rig.saved("volume"), Some(json!(30)));
    assert_eq!(rig.saved("balance"), Some(json!(-100)));
    assert_eq!(rig.saved("player5-url"), None);

    // Ejecting what plays stops it and forgets it.
    let store = rig.store.clone();
    rig.call(|core| core.select_slot(0));
    rig.wait_state(5.0, "playing");
    rig.call(|core| core.eject(0));
    let state = rig.state();
    assert_eq!(state["state"], "stopped");
    assert!(state["activeSlot"].is_null() && state["activeId"].is_null());
    assert_eq!(rig.saved("last-selected-player"), None);
    let url = server.url("/kept");
    rig.call(move |core| core.save_cartridge(1, "Kept".into(), url)).unwrap();
    rig.call(move |core| core.select_entry(id));
    rig.call(|core| core.stop());
    drop(rig);

    // A second player on the same file restores everything.
    let rig = Rig::open(store);
    let snapshot = rig.attach();
    let state = &snapshot["state"];
    assert_eq!(state["cartridges"][1]["title"], "Kept");
    assert!(state["cartridges"][0].is_null() && state["cartridges"][4].is_null());
    assert_eq!((state["volume"].as_i64(), state["balance"].as_i64()), (Some(30), Some(-100)));
    assert_eq!(state["colors"], "accent");
    assert_eq!(state["instant"], true);
    assert_eq!(state["activeId"], snapshot["playlist"]["entries"][0]["id"]);
    assert_eq!(state["station"], "Playlist · 1 of 1");
    assert_eq!(snapshot["playlist"]["entries"][0]["title"], "Entry");
    // Instant switch preloads the cartridge again once attached; autoplay is only for a cartridge.
    rig.wait(5.0, "the preload", |state| state["cartridges"][1]["preload"] == "live");
    assert_eq!(rig.state()["state"], "stopped");
}

#[test]
fn autoplay_and_play_requests_wait_for_the_window() {
    let server = Server::start(|_, _| live());
    let rig = Rig::new("request", json!({ "player1-title": "Radio", "player1-url": server.url("/r"), "last-selected-player": "player1" }));
    // A shortcut with no window: nothing plays, nothing connects, until the window attaches.
    rig.player.action("toggle");
    sleep(0.3);
    assert_eq!(server.count("/r"), 0);
    assert!(rig.call(|core| core.engine.is_none()), "the engine started without a window");
    rig.attach();
    rig.wait_state(5.0, "playing");
    rig.player.action("toggle");
    assert_eq!(rig.state()["state"], "stopped");
    rig.player.action("play");
    rig.wait_state(5.0, "playing");
    rig.player.action("play");
    rig.player.action("stop");
    assert_eq!(rig.state()["state"], "stopped");

    // The window closing stops everything; a new window plays only when asked.
    rig.call(|core| core.set_instant(true));
    rig.call(|core| core.select_slot(0));
    rig.wait_state(5.0, "playing");
    rig.player.detach();
    assert!(rig.keys().is_empty());
    assert_eq!(rig.state()["state"], "stopped");
    let before = rig.events.0.lock().unwrap().len();
    sleep(0.5);
    assert_eq!(rig.events.0.lock().unwrap().len(), before, "events with no window");
    rig.player.request_play(false);
    rig.attach();
    rig.wait(5.0, "the preload", |state| state["cartridges"][0]["preload"] == "live");
    assert_eq!(rig.state()["state"], "stopped");
}

#[test]
fn status_for_the_info_window_and_the_icecast_title() {
    let server = Server::start(|path, _| match path {
        "/status-json.xsl" => Plan::Json(r#"{"icestats":{"source":[{"listenurl":"http://elsewhere/other","title":"No"},{"listenurl":"http://x/mount","title":"From the status page"}]}}"#.into()),
        _ => live(),
    });
    let rig = Rig::new("status", json!({ "player1-title": "Radio", "player1-url": server.url("/mount") }));
    rig.attach();
    let status = rig.player.status();
    assert_eq!(status, json!({ "state": "stopped", "station": null, "url": null, "buffer": null, "stalls": 0, "statusTitle": null }));
    rig.call(|core| core.select_slot(0));
    let state = rig.wait(5.0, "the status page's title", |state| state["title"] == "From the status page");
    assert_eq!(state["state"], "playing");
    let status = rig.player.status();
    assert_eq!(status["state"], "playing");
    assert_eq!(status["station"], "Radio");
    assert_eq!(status["url"], server.url("/mount"));
    assert_eq!(status["statusTitle"], "From the status page");
    assert!(num(&status["buffer"]) > 0.3, "{status}");
    assert_eq!(server.count("/status-json.xsl"), 1);
    // Every second, and on changes.
    sleep(1.2);
    assert!(rig.events("player-status").len() >= 2);
    // What the info window reads, by the bare URL.
    let details = crate::stream_info(server.url("/mount"));
    assert_eq!(details["name"], "Test Radio");
    assert_eq!(details["server"], "Qust test");
}
