// The drawer with 10,000 entries: build time, memory, what scrolling renders, what a
// `player-entry` and a clock tick re-render, and the cost of the playing row's equalizer at
// 60 Hz. Drawn the way GPUI draws a visible window (`Window::draw`, no forced refresh), so cached
// views that aren't dirty are replayed, as in the app.
//
//   cargo test -p qust-ui-playlist --features capture --test scale [--release]

use gpui::{point, App, AppContext, Modifiers};
use qust_fluent::motion::Motion;
use qust_ui::capture::{self, Job, Session};
use qust_ui::fixture::Fixture;
use qust_ui::{Entry, Playback};
use qust_ui_playlist::harness::{self, Recorder};
use qust_ui_playlist::{Drawer, EqualizerLayer, Item};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

const N: u64 = 10_000;

fn entries() -> Vec<Entry> {
    (1..=N)
        .map(|id| Entry {
            id,
            title: format!("Artist {} - A track with a reasonably long title {id}", id % 97),
            path: Some(format!("C:\\Users\\Me\\Music\\Library\\Artist {}\\Album {}\\{id:05} A track.flac", id % 97, id % 13)),
            url: None,
            duration: Some(120.0 + (id % 300) as f64),
            missing: false,
        })
        .collect()
}

// Draws as the platform would: only dirty views re-render.
fn draw(session: &Session, cx: &mut App) {
    let _ = cx.update_window(session.any(), |_, window, cx| window.draw(cx).clear(cx));
}

fn stats(drawer: &gpui::Entity<Drawer>, cx: &App) -> (u64, u64) {
    let s = drawer.read(cx).stats;
    (s.renders, s.rows_built)
}

#[derive(Default, Debug)]
struct Report {
    lines: Vec<String>,
    failures: Vec<String>,
}

impl Report {
    fn note(&mut self, line: String) {
        println!("{line}");
        self.lines.push(line);
    }

    fn check(&mut self, name: &str, ok: bool, detail: String) {
        self.note(format!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" }));
        if !ok {
            self.failures.push(name.to_string());
        }
    }
}

// Draws at about 60 Hz for 3 s with the deck stopped or playing; (frames, renders/s, rows/s,
// CPU % of a core).
async fn measure(cx: &mut gpui::AsyncApp, session: &Session, drawer: &gpui::Entity<Drawer>, layer: &gpui::Entity<EqualizerLayer>, playing: bool) -> (u64, f64, f64, f64, f64) {
    let mut state = cx.update(|cx| session.models.snapshot.read(cx).state.clone());
    state.state = if playing { Playback::Playing } else { Playback::Stopped };
    cx.update(|cx| session.models.apply_state(state, cx));
    // A hidden window gets no frame callbacks, so `request_animation_frame` never fires here: the
    // notify it would send to the equalizer layer is sent by hand while playing.
    let frame = |cx: &mut App| {
        if playing {
            layer.update(cx, |_, cx| cx.notify());
        }
        draw(session, cx);
    };
    for _ in 0..30 {
        cx.update(|cx| frame(cx));
        cx.background_executor().timer(Duration::from_millis(16)).await;
    }
    let c0 = qust_ui::diag::counters();
    let (r0, b0) = cx.update(|cx| stats(drawer, cx));
    let l0 = cx.update(|cx| layer.read(cx).renders);
    let t0 = Instant::now();
    let mut frames = 0;
    while t0.elapsed() < Duration::from_secs(3) {
        cx.update(|cx| frame(cx));
        frames += 1;
        cx.background_executor().timer(Duration::from_millis(16)).await;
    }
    let c1 = qust_ui::diag::counters();
    let (r1, b1) = cx.update(|cx| stats(drawer, cx));
    let l1 = cx.update(|cx| layer.read(cx).renders);
    let secs = t0.elapsed().as_secs_f64();
    (frames, (r1 - r0) as f64 / secs, (b1 - b0) as f64 / secs, (c1.cpu_ms - c0.cpu_ms) / (secs * 10.0), (l1 - l0) as f64 / secs)
}

fn main() {
    let mut raw = Fixture::load("drawer-entries").expect("fixture").raw;
    raw["playlist"] = serde_json::to_value(entries()).expect("json");
    let fixture = Fixture::from_value("scale".into(), raw);
    let job = Job { variant: "scale".into(), time: None, theme: "dark".into(), out: std::env::temp_dir().join("qust-playlist-scale.png"), fixture };
    let report = Rc::new(RefCell::new(Report::default()));
    let out = report.clone();
    let first = report.clone();
    capture::run(move |cx: &mut App| {
        let mut r = Report::default();
        let before = qust_ui::diag::counters();
        let started = Instant::now();
        let recorder = Recorder::new();
        let (session, drawer, layer) = harness::open_with_layer(&job, &recorder, cx).expect("window");
        let opened = started.elapsed();
        let after = qust_ui::diag::counters();
        r.note(format!("window with 10,000 entries: models + drawer + 3 frames in {:.1} ms; private memory +{:.1} MB", opened.as_secs_f64() * 1000.0, (after.private as f64 - before.private as f64) / 1e6));
        // The drawer alone: a second one over the same models.
        let t0 = Instant::now();
        let models = session.models.clone();
        let host = Rc::new(harness::RecordingHost(recorder.clone()));
        let commands = Rc::new(harness::RecordingCommands(recorder.clone()));
        let second = cx.new(|cx| Drawer::new(&models, commands, host, true, cx));
        r.note(format!("Drawer::new over 10,000 entries (index, totals): {:.2} ms", t0.elapsed().as_secs_f64() * 1000.0));
        drop(second);
        // A full playlist rebuild (player-playlist).
        let t0 = Instant::now();
        let mut list = entries();
        list.reverse();
        models.apply_playlist(qust_ui::Playlist { entries: list }, cx);
        let rebuilt = t0.elapsed();
        let (r0, b0) = stats(&drawer, cx);
        let t0 = Instant::now();
        draw(&session, cx);
        let render = t0.elapsed();
        let (r1, b1) = stats(&drawer, cx);
        r.note(format!("player-playlist of 10,000 (model + drawer sync): {:.2} ms; the frame after it: {:.2} ms, {} render(s), {} rows built", rebuilt.as_secs_f64() * 1000.0, render.as_secs_f64() * 1000.0, r1 - r0, b1 - b0));
        r.check("a rebuild renders only the visible rows", b1 - b0 <= 16, format!("{} rows", b1 - b0));

        // Idle frames: nothing re-renders.
        let (r0, _) = stats(&drawer, cx);
        for _ in 0..10 {
            draw(&session, cx);
        }
        let (r1, _) = stats(&drawer, cx);
        r.check("idle frames re-render nothing", r1 == r0, format!("{} renders over 10 frames", r1 - r0));

        // Clock ticks: player-state with only the time moving.
        let mut state = session.models.snapshot.read(cx).state.clone();
        state.state = Playback::Playing;
        state.active_id = Some(5_000);
        state.active_slot = None;
        session.models.apply_state(state.clone(), cx);
        draw(&session, cx);
        let (r0, _) = stats(&drawer, cx);
        for second in 1..=20 {
            state.elapsed = second as f64 * 0.25;
            state.position = Some(second as f64 * 0.25);
            session.models.apply_state(state.clone(), cx);
            draw(&session, cx);
        }
        let (r1, _) = stats(&drawer, cx);
        r.check("20 clock ticks (5 s of player-state) re-render nothing", r1 == r0, format!("{} renders", r1 - r0));
        // Volume, title: not the drawer's business either.
        state.volume = 10;
        state.title = "Another title".into();
        session.models.apply_state(state.clone(), cx);
        draw(&session, cx);
        let (r2, _) = stats(&drawer, cx);
        r.check("volume and title changes re-render nothing", r2 == r1, format!("{} renders", r2 - r1));

        // Scrolling: only the visible rows are built.
        let rows = drawer.read(cx).rows().clone();
        let p = drawer.read(cx).row_bounds(rows.item(0).unwrap_or(Item::Entry(1))).map(|b| point(b.left + 200.0, b.top + 10.0)).unwrap_or(point(300.0, 300.0));
        session.mouse_move(p, None, Modifiers::default(), cx);
        draw(&session, cx);
        let (r0, b0) = stats(&drawer, cx);
        let t0 = Instant::now();
        let mut max_rows = 0;
        for _ in 0..200 {
            let (_, before) = stats(&drawer, cx);
            let _ = cx.update_window(session.any(), |_, window, cx| {
                window.dispatch_event(
                    gpui::PlatformInput::Scroll(gpui::ScrollEvent { position: gpui::point(gpui::px(p.x), gpui::px(p.y)), delta: gpui::ScrollDelta::Lines(gpui::point(0.0, -3.0)), ..Default::default() }),
                    cx,
                );
            });
            draw(&session, cx);
            let (_, after) = stats(&drawer, cx);
            max_rows = max_rows.max(after - before);
        }
        let elapsed = t0.elapsed();
        let (r1, b1) = stats(&drawer, cx);
        let scroll = drawer.read(cx).scroll_top();
        r.note(format!("200 wheel notches (to {scroll:.0} px of {:.0}): {} renders, {} rows built, at most {max_rows} per frame, {:.3} ms per frame", rows.max_scroll(), r1 - r0, b1 - b0, elapsed.as_secs_f64() * 1000.0 / 200.0));
        r.check("scrolling builds only the visible rows", max_rows <= 13 && r1 - r0 <= 200, format!("at most {max_rows} rows per frame"));

        // player-entry: a shown row, then one off screen.
        let visible = drawer.read(cx).rows().visible(drawer.read(cx).scroll_top(), qust_ui_playlist::list::CLIENT_HEIGHT);
        let shown_id = match drawer.read(cx).rows().item(visible.start + 3) {
            Some(Item::Entry(id)) => id,
            _ => 1,
        };
        let mut entry = session.models.playlist.read(cx).entry(shown_id).cloned().expect("entry");
        entry.title = "Renamed".into();
        let (r0, b0) = stats(&drawer, cx);
        let t0 = Instant::now();
        session.models.apply_entry(entry, cx);
        let applied = t0.elapsed();
        draw(&session, cx);
        let (r1, b1) = stats(&drawer, cx);
        r.check("player-entry on a shown row: one render of the visible rows", r1 - r0 == 1, format!("{} render(s), {} rows built; apply_entry {:.3} ms (the model's linear search)", r1 - r0, b1 - b0, applied.as_secs_f64() * 1000.0));
        let mut far = session.models.playlist.read(cx).entry(9_999).cloned().expect("entry");
        far.missing = true;
        let (r0, _) = stats(&drawer, cx);
        session.models.apply_entry(far.clone(), cx);
        draw(&session, cx);
        let (r1, _) = stats(&drawer, cx);
        r.check("player-entry off screen (no total change): no render", r1 == r0, format!("{} renders", r1 - r0));
        far.duration = Some(1.0);
        session.models.apply_entry(far, cx);
        draw(&session, cx);
        let (r2, b2) = stats(&drawer, cx);
        r.check("player-entry off screen changing the total: one render (the status line)", r2 - r1 == 1, format!("{} render(s)", r2 - r1));
        let _ = b2;

        // Ctrl+A over 10,000 and the status line.
        let t0 = Instant::now();
        drawer.update(cx, |d, cx| d.select(rows.item(rows.pinned().len()).unwrap_or(Item::Entry(1)), false, false, cx));
        let _ = cx.update_window(session.any(), |_, window, cx| window.focus_next(cx));
        session.key("ctrl-a", cx);
        r.note(format!("Ctrl+A over 10,000: {:.2} ms; status \"{}\"", t0.elapsed().as_secs_f64() * 1000.0, drawer.read(cx).status()));

        // The playing row's equalizer at 60 Hz (live motion), against an idle drawer.
        session.models.apply_state(state.clone(), cx);
        let row = drawer.read(cx).rows().row_of(Item::Entry(5_000)).unwrap_or(0);
        let top = drawer.read(cx).rows().row_top(row);
        drawer.update(cx, |d, cx| d.set_scroll_top(top - 100.0, cx));
        cx.set_global(Motion { frozen: None, reduced: false });
        draw(&session, cx);
        let session_for_task = session.clone();
        let drawer_for_task = drawer.clone();
        cx.spawn(async move |cx| {
            let idle = measure(cx, &session_for_task, &drawer_for_task, &layer, false).await;
            let playing = measure(cx, &session_for_task, &drawer_for_task, &layer, true).await;
            cx.update(|cx| {
                let mut r = std::mem::take(&mut *out.borrow_mut());
                r.note(format!("idle drawer, {} frames drawn in 3 s: {:.1} renders/s, CPU {:.2} % of a core (the whole process, frames drawn by hand)", idle.0, idle.1, idle.3));
                r.note(format!("an entry playing on screen: equalizer layer {:.1} renders/s, drawer {:.1} renders/s ({:.0} rows built/s), CPU {:.2} % of a core", playing.4, playing.1, playing.2, playing.3));
                r.check("the equalizer animates in its own layer; the drawer doesn't re-render for it", playing.4 > 30.0 && playing.1 < 1.0, format!("layer {:.1}/s, drawer {:.1}/s", playing.4, playing.1));
                *out.borrow_mut() = r;
                session_for_task.close(cx);
                cx.quit();
            });
        })
        .detach();
        *first.borrow_mut() = r;
    });
    let report = report.borrow();
    println!("scale: {} failures", report.failures.len());
    assert!(report.failures.is_empty(), "failed: {:?}", report.failures);
}
