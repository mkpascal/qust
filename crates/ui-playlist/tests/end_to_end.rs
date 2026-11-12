// End to end with the real player (`qust_player::Player` on the null output, a temporary store):
// generated WAV files dropped on the window through GPUI's file-drop path, expanded by the
// player, shown with their probed durations; one played by double-click; reordered by drag;
// removed with Delete. Commands go through `qust_ui::Player`, events come back through the
// bridge into the models the drawer observes. One hidden window, never shown or focused; no
// sound (null output); never the real settings file.
//
//   cargo test -p qust-ui-playlist --features capture --test end_to_end

use gpui::{point, px, App, AppContext, AsyncApp, Entity, FileDropEvent, Modifiers, MouseButton, Point};
use qust_player::{OutputKind, Store};
use qust_ui::capture::{window_options, Session};
use qust_ui::{Bridge, Playback, WindowRoot};
use qust_ui_playlist::harness::{external_paths, file_drop, Recorder, RecordingHost, Stage};
use qust_ui_playlist::{Drawer, Item};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

fn draw(session: &Session, cx: &mut App) {
    let _ = cx.update_window(session.any(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

// Draws (as a visible window would) until `done` or the timeout.
async fn wait(cx: &mut AsyncApp, session: &Session, seconds: f64, done: impl Fn(&App) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        if cx.update(|cx| {
            draw(session, cx);
            done(cx)
        }) {
            return true;
        }
        cx.background_executor().timer(Duration::from_millis(20)).await;
    }
    false
}

fn ids(session: &Session, cx: &App) -> Vec<u64> {
    session.models.playlist.read(cx).entries.iter().map(|e| e.id).collect()
}

fn center(drawer: &Entity<Drawer>, item: Item, cx: &App) -> Point<f32> {
    let r = drawer.read(cx).row_bounds(item).expect("row");
    point(r.left + 300.0, r.top + r.height / 2.0)
}

fn main() {
    let dir = std::env::temp_dir().join(format!("qust-playlist-e2e-{}", std::process::id()));
    let music = dir.join("Music");
    std::fs::create_dir_all(&music).expect("temp dir");
    // A folder (expanded in natural order) and a loose file.
    for (name, seconds) in [("02 Second.wav", 3.0), ("10 Tenth.wav", 4.0), ("01 First.wav", 2.0)] {
        qust_ui::diag::write_tone_wav(&music.join(name), seconds).expect("wav");
    }
    let loose = dir.join("Loose.wav");
    qust_ui::diag::write_tone_wav(&loose, 5.0).expect("wav");
    let settings = dir.join("settings.json");
    std::fs::write(&settings, r#"{ "volume": 0 }"#).expect("settings");

    let failures = Rc::new(RefCell::new(Vec::<String>::new()));
    let out = failures.clone();
    let paths: Vec<PathBuf> = vec![music.clone(), loose.clone()];
    qust_ui::application().run(move |cx: &mut App| {
        qust_ui::capture::init(cx);
        let bridge = Bridge::start(Store::open(settings.clone()), OutputKind::Null, cx);
        let models = bridge.models.clone();
        let recorder = Recorder::new();
        let player = Rc::new(bridge.player.clone());
        let drawer_slot: Rc<RefCell<Option<Entity<Drawer>>>> = Rc::new(RefCell::new(None));
        let slot = drawer_slot.clone();
        let host = recorder.clone();
        let handle = cx
            .open_window(window_options(860.0, 587.0), move |_, cx| {
                let placeholder: gpui::AnyView = cx.new(|_| gpui::EmptyView).into();
                let root = cx.new(|_| WindowRoot::new(placeholder));
                let drawer = cx.new(|cx| Drawer::new(&models, player, Rc::new(RecordingHost(host)), true, cx));
                qust_ui_playlist::mount(&drawer, &root, cx);
                let stage = cx.new(|_| Stage::new(root.downgrade()));
                root.update(cx, |root, cx| root.set_content(stage, cx));
                *slot.borrow_mut() = Some(drawer);
                root
            })
            .expect("window");
        let _ = cx.update_window(handle.into(), |_, window, _| window.resize(gpui::size(px(860.0), px(587.0))));
        let session = Session { handle, models: bridge.models.clone() };
        let drawer = drawer_slot.borrow_mut().take().expect("drawer");
        bridge.player.attach();
        let player = bridge.player.clone();
        cx.spawn(async move |cx| {
            let fail = |name: &str, ok: bool, detail: String| {
                println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
                if !ok {
                    out.borrow_mut().push(name.to_string());
                }
            };
            wait(cx, &session, 5.0, |cx| session.models.snapshot.read(cx).attached).await;
            cx.background_executor().timer(Duration::from_millis(100)).await;
            cx.update(|cx| session.mouse_move(point(0.0, 0.0), None, Modifiers::default(), cx));
            // Drop the folder and the file over the (empty) list.
            let at = point(px(430.0), px(400.0));
            let paths_text: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            let refs: Vec<&str> = paths_text.iter().map(String::as_str).collect();
            cx.update(|cx| file_drop(&session, FileDropEvent::Entered { position: at, paths: external_paths(&refs) }, cx));
            cx.update(|cx| file_drop(&session, FileDropEvent::Submit { position: at }, cx));
            let added = wait(cx, &session, 10.0, |cx| ids(&session, cx).len() == 4).await;
            let titles: Vec<String> = cx.update(|cx| session.models.playlist.read(cx).entries.iter().map(|e| e.title.clone()).collect());
            fail("the dropped folder and file became four entries, the folder in natural order", added && titles == ["01 First", "02 Second", "10 Tenth", "Loose"], format!("{titles:?}"));
            let all = cx.update(|cx| ids(&session, cx));
            wait(cx, &session, 2.0, |cx| drawer.read(cx).selection().len() == 4).await;
            fail("the added entries are selected", cx.update(|cx| drawer.read(cx).selection()) == all.iter().map(|id| Item::Entry(*id)).collect::<Vec<_>>(), format!("{:?}", cx.update(|cx| drawer.read(cx).selection())));
            let probed = wait(cx, &session, 15.0, |cx| session.models.playlist.read(cx).entries.iter().all(|e| e.duration.is_some_and(|d| d > 0.0))).await;
            let durations: Vec<Option<f64>> = cx.update(|cx| session.models.playlist.read(cx).entries.iter().map(|e| e.duration).collect());
            fail("durations probed (player-entry) and shown", probed && cx.update(|cx| drawer.read(cx).status()).starts_with("4 tracks · 0:14"), format!("{durations:?} / {}", cx.update(|cx| drawer.read(cx).status())));

            // Double-click the second entry: it plays.
            let second = all[1];
            let p = cx.update(|cx| center(&drawer, Item::Entry(second), cx));
            cx.update(|cx| {
                for count in [1, 2] {
                    session.mouse_move(p, None, Modifiers::default(), cx);
                    session.mouse_down(p, MouseButton::Left, count, Modifiers::default(), cx);
                    session.mouse_up(p, MouseButton::Left, count, Modifiers::default(), cx);
                }
            });
            let playing = wait(cx, &session, 10.0, |cx| {
                let deck = session.models.deck.read(cx);
                deck.state == Playback::Playing && deck.active_id == Some(second)
            })
            .await;
            fail("double-click plays the entry", playing, format!("{:?}", cx.update(|cx| session.models.deck.read(cx).clone())));
            let placed = cx.update(|cx| drawer.read(cx).equalizer_placement());
            fail("its row shows the equalizer (the layer has its place)", placed.is_some(), format!("{placed:?}"));

            // Drag the first entry below the last.
            let first = all[0];
            let (from, to) = cx.update(|cx| {
                let last = drawer.read(cx).row_bounds(Item::Entry(all[3])).expect("row");
                (center(&drawer, Item::Entry(first), cx), point(300.0, last.top + last.height - 2.0))
            });
            cx.update(|cx| {
                session.mouse_move(from, None, Modifiers::default(), cx);
                session.mouse_down(from, MouseButton::Left, 1, Modifiers::default(), cx);
                for step in 1..=6 {
                    let t = step as f32 / 6.0;
                    session.mouse_move(point(from.x, from.y + (to.y - from.y) * t), Some(MouseButton::Left), Modifiers::default(), cx);
                    draw(&session, cx);
                }
                session.mouse_up(to, MouseButton::Left, 1, Modifiers::default(), cx);
            });
            let expected = vec![all[1], all[2], all[3], all[0]];
            let moved = wait(cx, &session, 5.0, |cx| ids(&session, cx) == expected).await;
            fail("dragging the first entry to the end reorders the playlist", moved, format!("{:?}", cx.update(|cx| ids(&session, cx))));
            fail("…and it stays selected", cx.update(|cx| drawer.read(cx).selection()) == vec![Item::Entry(first)], format!("{:?}", cx.update(|cx| drawer.read(cx).selection())));

            // Delete it: removed, the selection moves on (here, to the new last entry).
            cx.update(|cx| session.key("delete", cx));
            let removed = wait(cx, &session, 5.0, |cx| ids(&session, cx) == expected[..3]).await;
            fail("Delete removes it", removed, format!("{:?}", cx.update(|cx| ids(&session, cx))));
            fail("the selection moved to the new last entry", cx.update(|cx| drawer.read(cx).selection()) == vec![Item::Entry(all[3])], format!("{:?}", cx.update(|cx| drawer.read(cx).selection())));
            fail("no host call was needed", recorder.take_calls().is_empty(), String::new());

            cx.update(|cx| {
                file_drop(&session, FileDropEvent::Exited, cx);
                player.detach_and_wait(Duration::from_secs(3));
                session.close(cx);
                cx.quit();
            });
        })
        .detach();
    });
    let _ = std::fs::remove_dir_all(&dir);
    let failures = failures.borrow();
    println!("end to end: {} failures", failures.len());
    assert!(failures.is_empty(), "failed: {failures:?}");
}
