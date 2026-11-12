// The bridge end to end: a real `qust_player::Player` on the null output with a temporary
// store, playing a generated WAV, delivering into GPUI models; the content view must not
// re-render while the analyzer layer gets its frames.
//
//   cargo test -p qust-ui --features capture --test bridge
//
// Opens one hidden window (never shown, never focused) and draws it by hand at 60 Hz.

use gpui::{div, prelude::*, px, App, AppContext, Context, Entity, Window};
use qust_player::{OutputKind, Store};
use qust_ui::capture::window_options;
use qust_ui::models::NowPlaying;
use qust_ui::{analyzer_slot, AnalyzerView, Bridge, Playback, WindowRoot};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

struct Content {
    root: gpui::WeakEntity<WindowRoot>,
    now: Entity<NowPlaying>,
    renders: u64,
}

impl Render for Content {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        if std::env::var_os("QUST_TRACE").is_some() {
            eprintln!("content render {} at {:?}: {:?}", self.renders, std::time::SystemTime::now(), self.now.read(cx));
        }
        let station = self.now.read(cx).station.clone();
        div().size_full().p(px(40.0)).child(analyzer_slot(self.root.clone(), window)).child(station)
    }
}

#[derive(Default, Debug)]
struct Outcome {
    attached: bool,
    entries: usize,
    states: Vec<Playback>,
    clock_seconds: u64,
    frames_received: u64,
    frames_sent: u64,
    content_renders_while_playing: u64,
    now_changes_while_playing: u64,
    root_renders_while_playing: u64,
    draws: u64,
}

fn main() {
    let dir = std::env::temp_dir().join(format!("qust-ui-bridge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let wav = dir.join("tone.wav");
    qust_ui::diag::write_tone_wav(&wav, 20.0).expect("wav");
    let settings = dir.join("settings.json");
    std::fs::write(&settings, r#"{ "visualizer-colors": "classic", "volume": 0 }"#).expect("settings");
    let outcome = Rc::new(RefCell::new(Outcome::default()));
    let result = outcome.clone();

    qust_ui::application().run(move |cx: &mut App| {
        qust_ui::capture::init(cx);
        let bridge = Bridge::start(Store::open(settings.clone()), OutputKind::Null, cx);
        let models = bridge.models.clone();
        let window = cx
            .open_window(window_options(860.0, 267.0), |_, cx| {
                let placeholder: gpui::AnyView = cx.new(|_| gpui::EmptyView).into();
                let root = cx.new(|_| WindowRoot::new(placeholder));
                let weak = root.downgrade();
                let now = models.now.clone();
                let content = cx.new(|cx| {
                    cx.observe(&now, |_, _, cx| cx.notify()).detach();
                    Content { root: weak, now, renders: 0 }
                });
                let analyzer = cx.new(|cx| AnalyzerView::new(&models.analyzer, cx));
                root.update(cx, |root, cx| {
                    root.set_content(content, cx);
                    root.set_live("analyzer", analyzer, cx);
                });
                root
            })
            .expect("window");
        let _ = cx.update_window(window.into(), |_, window, _| window.resize(gpui::size(px(860.0), px(267.0))));
        // How often the now-playing model (station, title, specs) really changed.
        let now_changes = Rc::new(std::cell::Cell::new(0u64));
        let counter = now_changes.clone();
        cx.observe(&models.now, move |_, _| counter.set(counter.get() + 1)).detach();
        bridge.player.attach();
        let player = bridge.player.clone();
        let wav = wav.clone();
        cx.spawn(async move |cx| {
            let draw = |cx: &mut gpui::AsyncApp| {
                let _ = cx.update_window(window.into(), |_, window, cx| {
                    window.draw(cx).clear(cx);
                });
            };
            let ids = player.add_paths(vec![wav], None).get().await.unwrap_or_default();
            if let Some(id) = ids.first() {
                player.select_entry(*id);
            }
            // Wait for playback, drawing as a visible window would.
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                draw(cx);
                let playing = cx.update(|cx| models.clock.read(cx).state == Playback::Playing);
                if playing {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
            // Settle (the station and title arrive), then count over 3 s of playing.
            for _ in 0..30 {
                draw(cx);
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
            let content_renders = |cx: &mut gpui::AsyncApp| {
                cx.update(|cx| {
                    let root = window.read(cx).ok()?;
                    root.content().clone().downcast::<Content>().ok().map(|c| c.read(cx).renders)
                })
                .unwrap_or(0)
            };
            let root_renders = |cx: &mut gpui::AsyncApp| window.read_with(cx, |root, _| root.renders).unwrap_or(0);
            let (content0, root0) = (content_renders(cx), root_renders(cx));
            let changes0 = now_changes.get();
            let frames0 = cx.update(|cx| models.analyzer.read(cx).received);
            let mut draws = 0;
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_secs(3) {
                draw(cx);
                draws += 1;
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
            let content1 = content_renders(cx);
            let root1 = root_renders(cx);
            cx.update(|cx| {
                let mut out = result.borrow_mut();
                out.attached = models.snapshot.read(cx).attached;
                out.entries = models.playlist.read(cx).entries.len();
                out.states.push(models.clock.read(cx).state);
                out.clock_seconds = models.clock.read(cx).seconds;
                out.frames_received = models.analyzer.read(cx).received - frames0;
                out.frames_sent = player.frames_sent();
                out.content_renders_while_playing = content1 - content0;
                out.root_renders_while_playing = root1 - root0;
                out.draws = draws;
                out.now_changes_while_playing = now_changes.get() - changes0;
            });
            cx.update(|cx| {
                player.detach_and_wait(Duration::from_secs(3));
                cx.quit();
            });
        })
        .detach();
    });

    let outcome = outcome.borrow();
    println!("{outcome:#?}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(outcome.attached, "the attach snapshot arrived");
    assert_eq!(outcome.entries, 1, "the playlist model has the file");
    assert_eq!(outcome.states.last(), Some(&Playback::Playing), "playing");
    assert!(outcome.clock_seconds >= 3, "the clock model ticks: {}", outcome.clock_seconds);
    assert!(outcome.frames_received > 60, "analyzer frames reached the model: {}", outcome.frames_received);
    // The content re-renders only when what it shows changed (the specs arriving), never for the
    // analyzer's frames or the clock. GPUI also redraws everything (no cache reuse) when it
    // refreshes the window: activation, the pointer entering or leaving, a bounds or focus change;
    // one such refresh is allowed for.
    assert!(outcome.content_renders_while_playing <= outcome.now_changes_while_playing + 1, "the content view re-rendered for frames or the clock");
    assert!(outcome.frames_received > outcome.content_renders_while_playing * 30);
    println!("bridge: OK");
}
