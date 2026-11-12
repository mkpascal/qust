// Qust on GPUI: starts the app, wires the player through `qust-ui`'s bridge, and opens the
// player window. For now the window shows a placeholder proving the pipeline (the live clock,
// the station and title from the player, the analyzer painting real frames); the window crates
// replace it:
//
// - `qust-ui-player` builds the player window's content (display, sliders, transport,
//   cartridges, dialog): it replaces `Placeholder` below, with the same live layers.
// - `qust-ui-playlist` adds the drawer under it (the window grows to 587 px).
// - `qust-ui-panels` opens Settings and Stream information (`open_panel`).
// - `qust-shell` adds the tray, global shortcuts, menus, single instance, start with Windows,
//   window ownership and the title bar theme (see `qust_ui::theme`), and decides what closing
//   the player window means (here it quits).
//
//   qust-app [--null-output] [--settings <file>] [--play <file> | --play-tone] [--measure <s>]
//
// `--null-output` plays silently (tests), `--settings` uses another settings file so test runs
// never touch the real one, `--play` plays a file once attached, `--play-tone` plays a generated
// tone, `--measure` prints CPU, memory and render counts after that many seconds and quits.

use gpui::{div, prelude::*, px, size, App, Bounds, Context, Entity, SharedString, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions};
use qust_fluent::{icon, surface, Frame, Icon, Marquee, TextStyles};
use qust_player::{OutputKind, Store};
use qust_ui::models::{Clock, NowPlaying};
use qust_ui::{analyzer_slot, portal_slot, AnalyzerView, Bridge, Playback, WindowRoot};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static CONTENT_RENDERS: AtomicU64 = AtomicU64::new(0);
static CLOCK_RENDERS: AtomicU64 = AtomicU64::new(0);

struct Args {
    null_output: bool,
    settings: Option<PathBuf>,
    play: Option<PathBuf>,
    play_tone: bool,
    measure: Option<f64>,
}

fn args() -> Args {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| all.iter().position(|a| a == flag).and_then(|i| all.get(i + 1)).cloned();
    Args {
        null_output: all.iter().any(|a| a == "--null-output"),
        settings: value("--settings").map(PathBuf::from),
        play: value("--play").map(PathBuf::from),
        play_tone: all.iter().any(|a| a == "--play-tone"),
        measure: value("--measure").and_then(|s| s.parse().ok()),
    }
}

/// The clock as a live layer: re-renders once a second, nothing else does.
struct ClockView {
    clock: Entity<Clock>,
}

impl Render for ClockView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        CLOCK_RENDERS.fetch_add(1, Ordering::Relaxed);
        let theme = qust_fluent::theme(cx);
        let clock = self.clock.read(cx);
        let idle = clock.state == Playback::Stopped;
        let dim = qust_fluent::Color::rgba(255, 255, 255, 89.0 / 255.0);
        let glyph = match clock.state {
            Playback::Stopped => Icon::Stop,
            Playback::Paused => Icon::Pause,
            _ => Icon::Play,
        };
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .font_display()
            .semibold()
            .type_size(22.0, 24.0)
            .tabular_nums()
            .text_color(if idle { dim } else { qust_fluent::Color::WHITE })
            .child(icon(glyph, 10.0).text_color(if idle { dim } else { theme.accent_light }))
            .child(qust_ui::format::clock(clock.seconds))
    }
}

/// The placeholder content: the display island, with the clock, analyzer and title left as
/// slots for live layers, and the station and state as plain content.
struct Placeholder {
    root: WeakEntity<WindowRoot>,
    now: Entity<NowPlaying>,
}

impl Render for Placeholder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        CONTENT_RENDERS.fetch_add(1, Ordering::Relaxed);
        let theme = *qust_fluent::theme(cx);
        let now = self.now.read(cx);
        div().size_full().pt(px(6.0)).px(px(22.0)).child(
            surface(Frame::island(&theme)).w(px(330.0)).h(px(91.0)).flex().items_center().gap(px(14.0)).p(px(10.0)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .pt(px(10.0))
                    .px(px(12.0))
                    .pb(px(12.0))
                    .rounded(px(6.0))
                    .bg(theme.display)
                    .child(div().w(px(75.0)).h(px(24.0)).child(portal_slot(self.root.clone(), "clock")))
                    .child(analyzer_slot(self.root.clone(), window)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.0))
                    .font_text()
                    .child(div().type_size(11.0, 14.0).text_color(theme.text_2).truncate().child(SharedString::from(now.station.clone())))
                    .child(div().w_full().h(px(20.0)).child(portal_slot(self.root.clone(), "title")))
                    .child(div().type_size(11.0, 16.0).text_color(theme.text_2).child(SharedString::from(now.state.name()))),
            ),
        )
    }
}

fn open_player(bridge: &Bridge, focus: bool, cx: &mut App) -> Option<gpui::WindowHandle<WindowRoot>> {
    let drawer = bridge.models.settings.read(cx).playlist_open;
    let height = if drawer { qust_fluent::tokens::PLAYER_HEIGHT + qust_fluent::tokens::DRAWER_HEIGHT } else { qust_fluent::tokens::PLAYER_HEIGHT };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(860.0), px(height)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("Qust".into()), appears_transparent: false, traffic_light_position: None }),
        is_resizable: false,
        is_minimizable: true,
        app_id: Some("qust".into()),
        focus,
        ..Default::default()
    };
    let models = bridge.models.clone();
    cx.open_window(options, move |_, cx| {
        let placeholder: gpui::AnyView = cx.new(|_| gpui::EmptyView).into();
        let root = cx.new(|_| WindowRoot::new(placeholder));
        let weak = root.downgrade();
        let now = models.now.clone();
        let content = cx.new(|cx| {
            cx.observe(&now, |_, _, cx| cx.notify()).detach();
            Placeholder { root: weak.clone(), now }
        });
        let analyzer = cx.new(|cx| AnalyzerView::new(&models.analyzer, cx));
        let clock_model = models.clock.clone();
        let clock = cx.new(|cx| {
            cx.observe(&clock_model, |_, _, cx| cx.notify()).detach();
            ClockView { clock: clock_model }
        });
        // The title marquee follows the now-playing model.
        let theme = *qust_fluent::theme(cx);
        let backdrop = theme.card.over(theme.bg);
        let marquee = cx.new(|_| Marquee::new("", true, (theme.text, theme.text_3), backdrop));
        let marquee_for_updates = marquee.clone();
        cx.observe(&models.now, move |now, cx| {
            let (title, dim) = { let now = now.read(cx); (now.title.clone(), now.title_dim) };
            marquee_for_updates.update(cx, |m, cx| m.set(title, dim, cx));
        })
        .detach();
        root.update(cx, |root, cx| {
            root.set_content(content, cx);
            root.set_live("analyzer", analyzer, cx);
            root.set_live("clock", clock, cx);
            root.set_live("title", marquee, cx);
        });
        root
    })
    .ok()
}

fn main() {
    let args = args();
    let store = match &args.settings {
        Some(path) => Store::open(path.clone()),
        None => match Store::load() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("qust: settings: {error}");
                std::process::exit(1);
            }
        },
    };
    let output = if args.null_output { OutputKind::Null } else { OutputKind::Device };
    qust_ui::application().run(move |cx: &mut App| {
        qust_ui::init(cx);
        let bridge = Bridge::start(store, output, cx);
        let choice = qust_ui::theme::ThemeChoice::parse(&bridge.models.settings.read(cx).theme);
        qust_ui::theme::apply(choice, cx);
        // A measuring run doesn't take focus from whoever is working.
        let Some(window) = open_player(&bridge, args.measure.is_none(), cx) else {
            eprintln!("qust: no window");
            cx.quit();
            return;
        };
        // Keep the theme current with the system (and re-read the accent on activation).
        let _ = window.update(cx, |_, window, cx| {
            for subscription in qust_ui::theme::follow(window, cx) {
                subscription.detach();
            }
        });
        bridge.player.attach();
        // Closing the player window: playback stops and pending saves are written (the shell
        // will decide whether the app stays in the tray).
        let player = bridge.player.clone();
        let _ = window.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |_, _| {
                player.detach_and_wait(Duration::from_secs(2));
                true
            });
        });
        let file = if args.play_tone {
            let path = std::env::temp_dir().join("qust-app-tone.wav");
            qust_ui::diag::write_tone_wav(&path, 120.0).ok().map(|_| path)
        } else {
            args.play.clone()
        };
        if let Some(file) = file {
            let player = bridge.player.clone();
            cx.spawn(async move |_| {
                if let Some(ids) = player.add_paths(vec![file], None).get().await {
                    if let Some(id) = ids.last() {
                        player.select_entry(*id);
                    }
                }
            })
            .detach();
        }
        if let Some(seconds) = args.measure {
            measure(window, bridge.clone(), seconds, cx);
        }
    });
}

// Prints CPU and memory over `seconds` (after a 3 s warm-up) and what re-rendered, then quits.
fn measure(window: gpui::WindowHandle<WindowRoot>, bridge: Bridge, seconds: f64, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let root_renders = |cx: &mut gpui::AsyncApp| window.read_with(cx, |root, _| root.renders).unwrap_or(0);
        let frames = |cx: &mut gpui::AsyncApp| cx.update(|cx| bridge.models.analyzer.read(cx).received);
        let start = qust_ui::diag::counters();
        let t0 = std::time::Instant::now();
        let (root0, content0, clock0, frames0) = (root_renders(cx), CONTENT_RENDERS.load(Ordering::Relaxed), CLOCK_RENDERS.load(Ordering::Relaxed), frames(cx));
        cx.background_executor().timer(Duration::from_secs_f64(seconds)).await;
        let end = qust_ui::diag::counters();
        let secs = t0.elapsed().as_secs_f64();
        let per = |a: u64, b: u64| (b - a) as f64 / secs;
        println!(
            "MEASURE {secs:.1} s: CPU {:.2} % of one core; per second: frames received {:.1}, root renders {:.1}, content renders {:.1}, clock renders {:.1}; private {:.1} MB, working set {:.1} MB",
            (end.cpu_ms - start.cpu_ms) / (secs * 10.0),
            per(frames0, frames(cx)),
            per(root0, root_renders(cx)),
            per(content0, CONTENT_RENDERS.load(Ordering::Relaxed)),
            per(clock0, CLOCK_RENDERS.load(Ordering::Relaxed)),
            end.private as f64 / 1e6,
            end.working_set as f64 / 1e6,
        );
        let player = bridge.player.clone();
        cx.update(|cx| {
            player.detach_and_wait(Duration::from_secs(2));
            cx.quit()
        });
    })
    .detach();
}
