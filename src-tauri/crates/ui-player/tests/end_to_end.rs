// The player window with the real player: `qust_player::Player` on the Null output (no sound),
// a temporary store, a live WAV stream served on 127.0.0.1 by this test. Through dispatched input
// only: load a cartridge with the dialog, play it, pause, play, change the volume, stop; the
// view follows the player. While it plays, the render counts show that only the live layers
// re-render (the analyzer, the clock, the playing cartridge's equalizer).
//
//   cargo test -p qust-ui-player --features capture --test end_to_end
//
// One hidden window, never shown or focused, drawn by hand at about 60 Hz.

use gpui::{point, App, AppContext, AsyncApp, Entity, Modifiers, MouseButton, Point};
use qust_player::{OutputKind, Store};
use qust_ui::capture::{window_options, Session};
use qust_ui::fixture::{element_box, layout};
use qust_ui::{Bridge, Playback, WindowRoot};
use qust_ui_player::testing::Recorder;
use qust_ui_player::{Commands, PlayerHost, PlayerView};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::{Duration, Instant};

const RATE: u32 = 48000;

fn wav_header(frames: u32) -> Vec<u8> {
    let bytes = frames.saturating_mul(4);
    let mut data = Vec::new();
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

/// A live stream: a WAV header for an hour (the engine takes it for a stream), one second at
/// once, then real time.
fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let port = listener.local_addr().expect("port").port();
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(match socket.try_clone() {
                    Ok(s) => s,
                    Err(_) => return,
                });
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) <= 2 {
                        break;
                    }
                }
                let mut socket = socket;
                let head = "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nicy-name: Test Radio\r\nConnection: close\r\n\r\n";
                if socket.write_all(head.as_bytes()).is_err() || socket.write_all(&wav_header(RATE * 3600)).is_err() {
                    return;
                }
                let start = Instant::now();
                let mut sent = 0u64;
                loop {
                    let due = ((1.0 + start.elapsed().as_secs_f64()) * RATE as f64) as u64;
                    if due > sent {
                        if socket.write_all(&pcm(sent, (due - sent) as usize)).is_err() {
                            return;
                        }
                        sent = due;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
        }
    });
    port
}

#[derive(Debug, Default)]
struct Outcome {
    steps: Vec<String>,
    failures: Vec<String>,
    renders: Vec<(String, u64)>,
    draws: u64,
    frames_received: u64,
}

fn main() {
    let dir = std::env::temp_dir().join(format!("qust-ui-player-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let settings = dir.join("settings.json");
    std::fs::write(&settings, r#"{ "visualizer-colors": "classic", "volume": 70 }"#).expect("settings");
    let port = serve();
    let url = format!("http://127.0.0.1:{port}/live");
    let outcome = Rc::new(RefCell::new(Outcome::default()));
    let result = outcome.clone();

    qust_ui::application().run(move |cx: &mut App| {
        qust_ui::capture::init(cx);
        let bridge = Bridge::start(Store::open(settings.clone()), OutputKind::Null, cx);
        let models = bridge.models.clone();
        let recorder = Recorder::new();
        let view_slot: Rc<RefCell<Option<Entity<PlayerView>>>> = Rc::default();
        let slot = view_slot.clone();
        let player = bridge.player.clone();
        let rec = recorder.clone();
        let window_models = models.clone();
        let handle = cx
            .open_window(window_options(860.0, 267.0), move |window, cx| {
                let placeholder: gpui::AnyView = cx.new(|_| gpui::EmptyView).into();
                let root = cx.new(|_| WindowRoot::new(placeholder));
                let commands: Rc<dyn Commands> = Rc::new(player);
                let host: Rc<dyn PlayerHost> = Rc::new(rec);
                let weak = root.downgrade();
                let view = cx.new(|cx| PlayerView::new(&window_models, commands, host, weak, window, cx));
                *slot.borrow_mut() = Some(view.clone());
                root.update(cx, |root, cx| root.set_content(view, cx));
                root
            })
            .expect("window");
        let _ = cx.update_window(handle.into(), |_, window, _| window.resize(gpui::size(gpui::px(860.0), gpui::px(267.0))));
        let session = Session { handle, models: models.clone() };
        bridge.player.attach();
        let view = view_slot.borrow().clone().expect("view");
        let player = bridge.player.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let reference = layout("player", "stopped").unwrap_or_default();
            let at = |name: &str, fx: f32| -> Point<f32> {
                let b = element_box(&reference, name, 1.0).unwrap_or([0.0; 4]);
                point(b[0] + fx * b[2], b[1] + b[3] / 2.0)
            };
            let mut out = Outcome::default();
            // A frame as a visible window draws it: only what is dirty re-renders (Session::frame
            // refreshes the whole window first, which re-renders everything).
            let draw = |cx: &mut AsyncApp| {
                // A hidden window gets no vsync: deliver the next-frame callbacks (animations
                // asking for frames) by hand, as the platform would before drawing.
                let _ = cx.update(|cx| {
                    cx.update_window(session.handle.into(), |_, window, cx| {
                        window.simulate_next_frame(cx);
                        window.draw(cx).clear(cx);
                    })
                });
            };
            // Draws until `done` holds (or `seconds` pass).
            async fn until(cx: &mut AsyncApp, session: &Session, seconds: f64, done: impl Fn(&App) -> bool) -> bool {
                let start = Instant::now();
                while start.elapsed().as_secs_f64() < seconds {
                    if cx.update(|cx| {
                        session.frame(cx);
                        done(cx)
                    }) {
                        return true;
                    }
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                }
                false
            }
            let fail = |out: &mut Outcome, what: &str| out.failures.push(what.to_string());
            let attached = until(cx, &session, 5.0, |cx| models.snapshot.read(cx).attached).await;
            if !attached {
                fail(&mut out, "the player never attached");
            }

            // Load cartridge 1 through the dialog.
            cx.update(|cx| session.click(at("#cartridges>.cartridge[0]>.cartridge-body", 0.5), cx));
            let opened = cx.update(|cx| view.read(cx).dialog_open(cx));
            out.steps.push(format!("click on the empty first slot: dialog open {opened}"));
            cx.update(|cx| session.type_text("Test Radio", cx));
            cx.update(|cx| session.key("tab", cx));
            cx.update(|cx| session.type_text(&url, cx));
            cx.update(|cx| session.key("enter", cx));
            let loaded = until(cx, &session, 5.0, |cx| models.deck.read(cx).cartridges.first().cloned().flatten().is_some_and(|c| c.title == "Test Radio")).await;
            let closed = cx.update(|cx| !view.read(cx).dialog_open(cx));
            out.steps.push(format!("saved: cartridge loaded {loaded}, dialog closed {closed}"));
            if !(loaded && closed) {
                fail(&mut out, "loading the cartridge");
            }

            // Play it.
            cx.update(|cx| session.click(at("#cartridges>.cartridge[0]>.cartridge-body", 0.5), cx));
            let playing = until(cx, &session, 10.0, |cx| models.clock.read(cx).state == Playback::Playing).await;
            out.steps.push(format!("click on the cartridge: playing {playing}"));
            if !playing {
                fail(&mut out, "playing");
            }
            // The pointer leaves the controls (as a listener's would): hover changes and tooltips
            // redraw the window, which isn't what is measured here.
            cx.update(|cx| session.mouse_move(point(5.0, 262.0), None, Modifiers::default(), cx));
            // Settle (title, specs), then count renders over 3 s of playing.
            until(cx, &session, 4.0, |_| false).await;
            let counts = |cx: &mut AsyncApp| {
                cx.update(|cx| {
                    let v = view.read(cx);
                    let live = v.live().cloned();
                    let mut counts = vec![
                        ("player (content)".to_string(), v.renders),
                        ("display".to_string(), v.display().read(cx).renders),
                        ("sliders".to_string(), v.sliders().read(cx).renders),
                        ("transport".to_string(), v.transport().read(cx).renders),
                        ("cartridges".to_string(), v.cartridges().read(cx).renders),
                        ("deck bar".to_string(), v.deck().read(cx).renders),
                    ];
                    if let Some(live) = live {
                        counts.push(("live: analyzer".to_string(), live.analyzer.read(cx).renders));
                        counts.push(("live: clock".to_string(), live.clock.read(cx).renders));
                        counts.push(("live: seek row".to_string(), live.seek.read(cx).renders));
                        counts.push(("live: badge of the playing cartridge".to_string(), live.badges[0].read(cx).renders));
                        counts.push(("live: other badges".to_string(), live.badges[1..].iter().map(|b| b.read(cx).renders).sum()));
                    }
                    counts.push(("window root".to_string(), session.handle.read(cx).map(|r| r.renders).unwrap_or(0)));
                    counts
                })
            };
            let before = counts(cx);
            let frames0 = cx.update(|cx| models.analyzer.read(cx).received);
            let start = Instant::now();
            let mut draws = 0;
            while start.elapsed() < Duration::from_secs(3) {
                draw(cx);
                draws += 1;
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
            let after = counts(cx);
            out.draws = draws;
            out.frames_received = cx.update(|cx| models.analyzer.read(cx).received) - frames0;
            out.renders = before.iter().zip(after.iter()).map(|((name, a), (_, b))| (name.clone(), b - a)).collect();
            for (name, n) in &out.renders {
                if !name.starts_with("live") && name != "window root" && *n > 1 {
                    out.failures.push(format!("{name} re-rendered {n} times while playing"));
                }
            }

            // Pause, play, then the volume, then stop: the view follows.
            cx.update(|cx| session.click(at("#pauseButton", 0.5), cx));
            let paused = until(cx, &session, 5.0, |cx| models.clock.read(cx).state == Playback::Paused).await;
            out.steps.push(format!("Pause: paused {paused}"));
            cx.update(|cx| session.click(at("#playButton", 0.5), cx));
            let resumed = until(cx, &session, 10.0, |cx| models.clock.read(cx).state == Playback::Playing).await;
            out.steps.push(format!("Play: playing {resumed}"));
            cx.update(|cx| {
                session.frame(cx);
                let from = at("#volumeSlider>.slider-rail", 0.7);
                let to = at("#volumeSlider>.slider-rail", 0.3);
                session.mouse_move(from, None, Modifiers::default(), cx);
                session.mouse_down(from, MouseButton::Left, 1, Modifiers::default(), cx);
                session.mouse_move(to, Some(MouseButton::Left), Modifiers::default(), cx);
                session.frame(cx);
                session.mouse_up(to, MouseButton::Left, 1, Modifiers::default(), cx);
            });
            let volume = until(cx, &session, 5.0, |cx| models.levels.read(cx).volume == 30).await;
            let shown = cx.update(|cx| view.read(cx).sliders().read(cx).volume.shown);
            out.steps.push(format!("volume dragged to 30 %: player volume 30 {volume}, label {shown}%"));
            cx.update(|cx| session.click(at("#stopButton", 0.5), cx));
            let stopped = until(cx, &session, 5.0, |cx| models.clock.read(cx).state == Playback::Stopped).await;
            out.steps.push(format!("Stop: stopped {stopped}"));
            for (ok, what) in [(paused, "pause"), (resumed, "resume"), (volume && shown == 30, "volume"), (stopped, "stop")] {
                if !ok {
                    fail(&mut out, what);
                }
            }
            cx.update(|cx| {
                player.detach_and_wait(Duration::from_secs(3));
                // Detaching writes the pending saves.
                let stored = player.setting("volume");
                out.steps.push(format!("after detach: stored volume {stored:?}"));
                if stored != Some(serde_json::json!(30)) {
                    out.failures.push(format!("stored volume {stored:?}"));
                }
                session.close(cx);
                cx.quit();
            });
            *result.borrow_mut() = out;
        })
        .detach();
    });

    let outcome = outcome.borrow();
    for step in &outcome.steps {
        println!("{step}");
    }
    println!("over 3 s of playing ({} frames drawn, {} analyzer frames received), renders:", outcome.draws, outcome.frames_received);
    for (name, n) in &outcome.renders {
        println!("  {name:<40} {n}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !outcome.failures.is_empty() {
        println!("FAILED: {:?}", outcome.failures);
        std::process::exit(1);
    }
    println!("end_to_end: OK");
}
