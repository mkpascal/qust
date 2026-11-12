// The drawer's behaviour (BEHAVIOUR.md 2.7, 2.9), driven through input dispatched into hidden
// windows (never shown, never focused, no OS input) against recording stand-ins for the player
// and the host. Asserts the commands sent and the selection after each step, including after
// list updates arriving from the player. No menu, picker or dialog is ever opened: the host
// stand-in keeps the menu and the test chooses an item.
//
//   cargo test -p qust-ui-playlist --features capture --test behaviour

use gpui::{point, px, App, AppContext, AsyncApp, Entity, FileDropEvent, Modifiers, MouseButton, Point};
use qust_ui::capture::{self, Job, Session};
use qust_ui::fixture::Fixture;
use qust_ui::{Entry, Playlist};
use qust_ui_playlist::harness::{self, external_paths, file_drop, Command, HostCall, Recorder};
use qust_ui_playlist::{Drawer, Item, Panel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const C: fn(usize) -> Item = Item::Cartridge;
const E: fn(u64) -> Item = Item::Entry;

struct Bench {
    session: Session,
    drawer: Entity<Drawer>,
    rec: Rc<Recorder>,
    fixture: Fixture,
}

fn ctrl() -> Modifiers {
    Modifiers { control: true, ..Default::default() }
}

fn shift() -> Modifiers {
    Modifiers { shift: true, ..Default::default() }
}

impl Bench {
    fn open(name: &str, cx: &mut App) -> Bench {
        let fixture = Fixture::load(name).expect("fixture");
        let job = Job { variant: name.into(), time: None, theme: "dark".into(), out: std::env::temp_dir().join("qust-playlist-behaviour.png"), fixture: fixture.clone() };
        let rec = Recorder::new();
        let (session, drawer) = harness::open(&job, &rec, cx).expect("window");
        // The pointer starts where the real cursor was: park it outside the drawer.
        session.mouse_move(point(0.0, 0.0), None, Modifiers::default(), cx);
        session.frames(2, cx);
        Bench { session, drawer, rec, fixture }
    }

    // The centre of an item's row, in window coordinates.
    fn at(&self, item: Item, cx: &App) -> Point<f32> {
        let r = self.drawer.read(cx).row_bounds(item).expect("row");
        point(r.left + 300.0, r.top + r.height / 2.0)
    }

    fn click_at(&self, p: Point<f32>, modifiers: Modifiers, count: usize, cx: &mut App) {
        self.session.frame(cx);
        self.session.mouse_move(p, None, modifiers, cx);
        self.session.mouse_down(p, MouseButton::Left, count, modifiers, cx);
        self.session.mouse_up(p, MouseButton::Left, count, modifiers, cx);
        self.session.frame(cx);
    }

    fn click(&self, item: Item, modifiers: Modifiers, cx: &mut App) {
        let p = self.at(item, cx);
        self.click_at(p, modifiers, 1, cx);
    }

    fn double_click(&self, item: Item, cx: &mut App) {
        let p = self.at(item, cx);
        self.click_at(p, Modifiers::default(), 1, cx);
        self.click_at(p, Modifiers::default(), 2, cx);
    }

    fn right_click_at(&self, p: Point<f32>, cx: &mut App) {
        self.session.frame(cx);
        self.session.mouse_move(p, None, Modifiers::default(), cx);
        self.session.mouse_down(p, MouseButton::Right, 1, Modifiers::default(), cx);
        self.session.mouse_up(p, MouseButton::Right, 1, Modifiers::default(), cx);
        self.session.frame(cx);
    }

    // Press, move in steps (drawing between), release unless `hold`.
    fn drag(&self, from: Point<f32>, to: Point<f32>, steps: usize, hold: bool, cx: &mut App) {
        self.session.frame(cx);
        self.session.mouse_move(from, None, Modifiers::default(), cx);
        self.session.mouse_down(from, MouseButton::Left, 1, Modifiers::default(), cx);
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            self.session.mouse_move(point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t), Some(MouseButton::Left), Modifiers::default(), cx);
            self.session.frame(cx);
        }
        if !hold {
            self.session.mouse_up(to, MouseButton::Left, 1, Modifiers::default(), cx);
            self.session.frame(cx);
        }
    }

    fn selection(&self, cx: &App) -> Vec<Item> {
        self.drawer.read(cx).selection()
    }

    fn entries(&self) -> Vec<Entry> {
        self.fixture.playlist.clone()
    }

    fn set_playlist(&self, entries: Vec<Entry>, cx: &mut App) {
        self.session.models.apply_playlist(Playlist { entries }, cx);
        self.session.frame(cx);
    }

    fn close(self, cx: &mut App) {
        file_drop(&self.session, FileDropEvent::Exited, cx);
        self.session.close(cx);
    }
}

fn entry(id: u64, title: &str) -> Entry {
    Entry { id, title: title.into(), path: Some(format!("C:\\Music\\{title}.mp3")), url: None, duration: Some(60.0), missing: false }
}

struct Checks {
    failures: Vec<String>,
    passed: usize,
}

impl Checks {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Debug) {
        println!("{} {name}: {detail:?}", if ok { "PASS" } else { "FAIL" });
        if ok {
            self.passed += 1;
        } else {
            self.failures.push(name.to_string());
        }
    }
}

fn selection_and_clicks(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-entries", cx);
    b.click(E(2), Modifiers::default(), cx);
    t.check("click selects the row alone", b.selection(cx) == vec![E(2)], b.selection(cx));
    t.check("the anchor and focus are the clicked row", b.drawer.read(cx).anchor() == Some(E(2)) && b.drawer.read(cx).focused_item() == Some(E(2)), b.drawer.read(cx).anchor());
    b.click(E(4), ctrl(), cx);
    t.check("Ctrl+click adds", b.selection(cx) == vec![E(2), E(4)], b.selection(cx));
    b.click(E(2), ctrl(), cx);
    t.check("Ctrl+click toggles off, anchor on it", b.selection(cx) == vec![E(4)] && b.drawer.read(cx).anchor() == Some(E(2)), (b.selection(cx), b.drawer.read(cx).anchor()));
    b.click(E(1), Modifiers::default(), cx);
    b.click(E(3), shift(), cx);
    t.check("Shift+click selects the range from the anchor", b.selection(cx) == vec![E(1), E(2), E(3)], b.selection(cx));
    b.click(C(1), shift(), cx);
    t.check("Shift ranges include the pinned cartridges", b.selection(cx) == vec![C(1), C(2), C(3), C(5), E(1)], b.selection(cx));
    t.check("status counts every selected row", b.drawer.read(cx).status() == "6 tracks · 1:15:59 · 5 selected", b.drawer.read(cx).status());
    // The divider gap: no row.
    let gap = point(300.0, b.drawer.read(cx).list_rect().top + 1.0 + 140.0);
    b.click_at(gap, ctrl(), 1, cx);
    t.check("Ctrl+click on empty space keeps the selection", b.selection(cx).len() == 5, b.selection(cx));
    b.click_at(gap, Modifiers::default(), 1, cx);
    t.check("click on empty space clears the selection", b.selection(cx).is_empty(), b.selection(cx));
    // Press on a row, release on another: the list is the click's target.
    let (from, to) = (b.at(E(1), cx), b.at(E(1), cx));
    b.session.mouse_move(from, None, Modifiers::default(), cx);
    b.session.mouse_down(from, MouseButton::Left, 1, Modifiers::default(), cx);
    b.session.mouse_up(point(to.x, to.y + 14.0), MouseButton::Left, 1, Modifiers::default(), cx);
    b.session.frame(cx);
    t.check("press and release on different rows (< 5 px): the list's click clears", b.selection(cx).is_empty(), b.selection(cx));
    b.double_click(E(3), cx);
    b.double_click(C(2), cx);
    t.check("double-click plays the track, and the pinned cartridge", b.rec.take_commands() == vec![Command::PlayEntry(3), Command::PlayCartridge(2)], ());
    t.check("double-click left the row selected", b.selection(cx) == vec![C(2)], b.selection(cx));
    b.close(cx);
}

fn keyboard(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-entries", cx);
    b.click(E(1), Modifiers::default(), cx);
    b.session.key("down", cx);
    t.check("Down selects the next row", b.selection(cx) == vec![E(2)], b.selection(cx));
    b.session.key("shift-down", cx);
    b.session.key("shift-down", cx);
    t.check("Shift+Down extends from the anchor", b.selection(cx) == vec![E(2), E(3), E(4)], b.selection(cx));
    b.session.key("pageup", cx);
    t.check("PageUp: 10 rows up, clamped to the first", b.selection(cx) == vec![C(0)], b.selection(cx));
    b.session.key("pagedown", cx);
    t.check("PageDown: 10 rows down", b.selection(cx) == vec![E(6)], b.selection(cx));
    let max = b.drawer.read(cx).rows().max_scroll();
    t.check("the row is revealed (scrolled to the end)", b.drawer.read(cx).scroll_top() == max, (b.drawer.read(cx).scroll_top(), max));
    b.session.key("home", cx);
    t.check("Home: the first row, scrolled back", b.selection(cx) == vec![C(0)] && b.drawer.read(cx).scroll_top() == 0.0, (b.selection(cx), b.drawer.read(cx).scroll_top()));
    b.session.key("shift-end", cx);
    t.check("Shift+End: everything from the anchor", b.selection(cx).len() == 11, b.selection(cx).len());
    b.session.key("home", cx);
    b.session.key("enter", cx);
    b.session.key("end", cx);
    b.session.key("up", cx);
    b.session.key("enter", cx);
    t.check("Enter plays the focused cartridge, then the focused track", b.rec.take_commands() == vec![Command::PlayCartridge(0), Command::PlayEntry(5)], ());
    b.session.key("ctrl-a", cx);
    t.check("Ctrl+A selects every entry and no cartridge", b.selection(cx) == (1..=6).map(E).collect::<Vec<_>>(), b.selection(cx));
    // Delete: the selected entries; the selection moves to the entry after them.
    b.click(E(2), Modifiers::default(), cx);
    b.click(E(3), shift(), cx);
    b.click(C(1), ctrl(), cx);
    b.session.key("delete", cx);
    t.check("Delete removes the selected entries (not the cartridge)", b.rec.take_commands() == vec![Command::Remove(vec![2, 3])], ());
    t.check("the selection moved to the next entry", b.selection(cx) == vec![E(4)], b.selection(cx));
    // The player's answer: the list without 2 and 3.
    b.set_playlist(b.entries().into_iter().filter(|e| e.id != 2 && e.id != 3).collect(), cx);
    t.check("after the player's update the selection holds", b.selection(cx) == vec![E(4)] && b.drawer.read(cx).status().starts_with("4 tracks"), (b.selection(cx), b.drawer.read(cx).status()));
    b.session.key("backspace", cx);
    t.check("Backspace removes too", b.rec.take_commands() == vec![Command::Remove(vec![4])] && b.selection(cx) == vec![E(5)], b.selection(cx));
    b.click(C(0), Modifiers::default(), cx);
    b.session.key("delete", cx);
    t.check("Delete with only cartridges selected sends nothing", b.rec.take_commands().is_empty() && b.selection(cx) == vec![C(0)], b.selection(cx));
    b.close(cx);
}

fn focus_and_inert(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-entries", cx);
    let focused = |b: &Bench, cx: &mut App| cx.update_window(b.session.any(), |_, window, cx| b.drawer.read(cx).focus_handle().is_focused(window)).unwrap_or(false);
    b.session.key("tab", cx);
    let by_key = focused(&b, cx);
    let _ = cx.update_window(b.session.any(), |_, window, cx| window.focus_next(cx));
    t.check("Tab reaches the list while open", by_key || focused(&b, cx), (by_key, focused(&b, cx)));
    b.click(E(2), Modifiers::default(), cx);
    t.check("a click focuses it", focused(&b, cx), ());
    let _ = cx.update_window(b.session.any(), |_, window, cx| b.drawer.update(cx, |d, cx| d.set_open(false, window, cx)));
    b.session.frames(2, cx);
    t.check("closing blurs the list", !focused(&b, cx), ());
    b.click(E(4), Modifiers::default(), cx);
    let _ = cx.update_window(b.session.any(), |_, window, cx| window.focus_next(cx));
    t.check("closed: no clicks, no focus, no Tab stop", !focused(&b, cx) && b.selection(cx) == vec![E(2)], b.selection(cx));
    b.right_click_at(b.at(E(4), cx), cx);
    t.check("closed: no menu", b.rec.last_menu().is_none(), ());
    let _ = cx.update_window(b.session.any(), |_, window, cx| b.drawer.update(cx, |d, cx| d.set_open(true, window, cx)));
    b.session.frames(2, cx);
    b.click(E(4), Modifiers::default(), cx);
    t.check("open again: clicks work", b.selection(cx) == vec![E(4)] && focused(&b, cx), b.selection(cx));
    b.close(cx);
}

fn list_updates(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-entries", cx);
    b.click(E(3), Modifiers::default(), cx);
    b.click(E(5), ctrl(), cx);
    b.click(C(1), ctrl(), cx);
    // Reordered, one new entry.
    let mut entries = b.entries();
    entries.reverse();
    entries.push(entry(7, "Seven"));
    b.set_playlist(entries.clone(), cx);
    t.check("selection kept by id across a rebuild (row order follows the list)", b.selection(cx) == vec![C(1), E(5), E(3)], b.selection(cx));
    b.set_playlist(entries.iter().filter(|e| e.id != 5).cloned().collect(), cx);
    t.check("removed entries leave the selection", b.selection(cx) == vec![C(1), E(3)], b.selection(cx));
    // The deck: cartridge 2 ejected.
    let mut state = b.fixture.state.clone().expect("state");
    state.cartridges[1] = None;
    b.session.models.apply_state(state, cx);
    b.session.frame(cx);
    t.check("an ejected cartridge leaves the pinned rows and the selection", b.selection(cx) == vec![E(3)] && b.drawer.read(cx).rows().pinned() == [0, 2, 3, 5], b.drawer.read(cx).rows().pinned().to_vec());
    // player-entry: a duration found updates the total.
    let renders = b.drawer.read(cx).stats.renders;
    let mut one = entries[0].clone();
    one.duration = Some(10_000.0);
    b.session.models.apply_entry(one, cx);
    b.session.frame(cx);
    t.check("player-entry updates the row and the total", b.drawer.read(cx).status().starts_with("6 tracks · 3:"), (b.drawer.read(cx).status(), b.drawer.read(cx).stats.renders - renders));
    b.close(cx);
}

fn reorder(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-entries", cx);
    let from = b.at(E(2), cx);
    // Under 5 px of vertical travel: no drag.
    b.drag(from, point(from.x + 40.0, from.y + 4.0), 2, true, cx);
    t.check("4 px: not a drag yet", !b.drawer.read(cx).is_reordering() && b.drawer.read(cx).drop_line().is_none(), ());
    b.session.mouse_up(point(from.x + 40.0, from.y + 4.0), MouseButton::Left, 1, Modifiers::default(), cx);
    b.rec.take_commands();
    // reorder-drag: press the second track, drag to 10 % into the fifth.
    let target = b.drawer.read(cx).row_bounds(E(5)).expect("row");
    b.drag(from, point(from.x, target.top + 2.6), 8, true, cx);
    t.check("dragging: the line before the fifth track (index 4)", b.drawer.read(cx).is_reordering() && b.drawer.read(cx).drop_line() == Some(4), b.drawer.read(cx).drop_line());
    t.check("the pressed track became the selection", b.selection(cx) == vec![E(2)], b.selection(cx));
    b.session.mouse_up(point(from.x, target.top + 2.6), MouseButton::Left, 1, Modifiers::default(), cx);
    b.session.frame(cx);
    t.check("release moves the selection before index 4", b.rec.take_commands() == vec![Command::Move(vec![2], 4)], ());
    t.check("the click ending the drag did nothing", b.selection(cx) == vec![E(2)] && b.drawer.read(cx).drop_line().is_none(), b.selection(cx));
    // A multi-selection is kept when pressing inside it; over the cartridges: index 0.
    b.click(E(3), Modifiers::default(), cx);
    b.click(E(5), ctrl(), cx);
    b.drag(b.at(E(3), cx), b.at(C(2), cx), 6, false, cx);
    t.check("dragging a multi-selection over the cartridges moves it to the start", b.rec.take_commands() == vec![Command::Move(vec![3, 5], 0)], b.selection(cx));
    // Outside the list: the end.
    b.drag(b.at(E(1), cx), point(400.0, 100.0), 6, false, cx);
    t.check("released outside the list: to the end", b.rec.take_commands() == vec![Command::Move(vec![1], 6)], ());
    // Esc cancels.
    b.drag(b.at(E(3), cx), b.at(E(5), cx), 4, true, cx);
    b.session.key("escape", cx);
    t.check("Esc cancels: no line, not dragging", !b.drawer.read(cx).is_reordering() && b.drawer.read(cx).drop_line().is_none(), ());
    let p = b.at(E(5), cx);
    b.session.mouse_up(p, MouseButton::Left, 1, Modifiers::default(), cx);
    b.session.frame(cx);
    t.check("…and the release after it moves nothing and selects nothing", b.rec.take_commands().is_empty() && b.selection(cx) == vec![E(3)], b.selection(cx));
    // Cartridge rows don't drag.
    b.drag(b.at(C(0), cx), b.at(E(4), cx), 6, false, cx);
    t.check("a pinned cartridge doesn't drag", b.rec.take_commands().is_empty(), ());
    // Modifiers don't drag.
    let p = b.at(E(1), cx);
    b.session.mouse_move(p, None, ctrl(), cx);
    b.session.mouse_down(p, MouseButton::Left, 1, ctrl(), cx);
    b.session.mouse_move(point(p.x, p.y + 60.0), Some(MouseButton::Left), ctrl(), cx);
    b.session.mouse_up(point(p.x, p.y + 60.0), MouseButton::Left, 1, ctrl(), cx);
    b.session.frame(cx);
    t.check("Ctrl+press doesn't drag", b.rec.take_commands().is_empty(), ());
    b.close(cx);

    // Autoscroll near the bottom edge (sixty tracks).
    let b = Bench::open("drawer-many", cx);
    let list = b.drawer.read(cx).list_rect();
    let from = b.at(E(101), cx);
    b.drag(from, point(from.x, list.bottom() - 10.0), 4, true, cx);
    let after = b.drawer.read(cx).scroll_top();
    for _ in 0..3 {
        b.session.mouse_move(point(from.x, list.bottom() - 10.0), Some(MouseButton::Left), Modifiers::default(), cx);
        b.session.frame(cx);
    }
    t.check("each move within 24 px of the bottom scrolls 12 px", b.drawer.read(cx).scroll_top() - after == 36.0, (after, b.drawer.read(cx).scroll_top()));
    b.session.mouse_up(point(from.x, list.bottom() - 10.0), MouseButton::Left, 1, Modifiers::default(), cx);
    b.session.frame(cx);
    let moved = b.rec.take_commands();
    t.check("the drop index follows the scroll", matches!(moved.as_slice(), [Command::Move(ids, index)] if ids == &vec![101] && *index == 7), moved);
    b.close(cx);
}

// Lets spawned tasks run (the player's answers resolve on the UI thread).
async fn settle(cx: &mut AsyncApp) {
    cx.background_executor().timer(Duration::from_millis(5)).await;
}

// Each step is its own update, as each OS event is in the app: observers and deferred calls
// run between steps.
async fn file_drops(t: &mut Checks, cx: &mut AsyncApp) {
    let b = cx.update(|cx| Bench::open("drawer-entries", cx));
    let pos = cx.update(|cx| {
        let over = b.drawer.read(cx).row_bounds(E(3)).expect("row");
        point(px(430.0), px(over.top + 2.0))
    });
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Entered { position: pos, paths: external_paths(&["C:\\a.mp3", "C:\\Music"]) }, cx));
    cx.update(|cx| t.check("files over the rows: the line at the nearest boundary (2)", b.drawer.read(cx).drop_line() == Some(2), b.drawer.read(cx).drop_line()));
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Pending { position: point(px(430.0), px(300.0)) }, cx));
    cx.update(|cx| t.check("over the cartridges: the start", b.drawer.read(cx).drop_line() == Some(0), b.drawer.read(cx).drop_line()));
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Pending { position: pos }, cx));
    b.rec.add_answers.borrow_mut().push_back(vec![7, 8]);
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Submit { position: pos }, cx));
    settle(cx).await;
    cx.update(|cx| {
        t.check("drop: the paths and the index go to the player", b.rec.take_commands() == vec![Command::AddPaths(vec!["C:\\a.mp3".into(), "C:\\Music".into()], Some(2))], ());
        t.check("the line is gone", b.drawer.read(cx).drop_line().is_none(), ());
    });
    // The answer arrived before its rows: selected once the playlist shows them.
    let mut entries = b.entries();
    entries.insert(2, entry(7, "Seven"));
    entries.insert(3, entry(8, "Eight"));
    cx.update(|cx| b.set_playlist(entries, cx));
    cx.update(|cx| t.check("the added entries are selected", b.selection(cx) == vec![E(7), E(8)] && b.drawer.read(cx).anchor() == Some(E(7)), b.selection(cx)));
    // Leave.
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Entered { position: pos, paths: external_paths(&["C:\\b.mp3"]) }, cx));
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Exited, cx));
    cx.update(|cx| t.check("leave hides the line", b.drawer.read(cx).drop_line().is_none(), ()));
    // The dialog open: ignored.
    b.rec.dialog_open.set(true);
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Entered { position: pos, paths: external_paths(&["C:\\b.mp3"]) }, cx));
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Submit { position: pos }, cx));
    settle(cx).await;
    cx.update(|cx| t.check("while the dialog is open drops are ignored", b.drawer.read(cx).drop_line().is_none() && b.rec.take_commands().is_empty(), ()));
    b.rec.dialog_open.set(false);
    // Closed: the drag opens the drawer; a drop over the player goes to the end.
    cx.update(|cx| {
        let _ = cx.update_window(b.session.any(), |_, window, cx| b.drawer.update(cx, |d, cx| d.set_open(false, window, cx)));
    });
    b.rec.take_calls();
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Entered { position: point(px(400.0), px(100.0)), paths: external_paths(&["C:\\c.mp3"]) }, cx));
    cx.update(|cx| {
        t.check("files over a closed drawer open it (the host is asked)", b.drawer.read(cx).is_open() && b.rec.take_calls() == vec![HostCall::OpenDrawer], ());
        t.check("…and show the line at the end", b.drawer.read(cx).drop_line() == Some(8), b.drawer.read(cx).drop_line());
    });
    b.rec.add_answers.borrow_mut().push_back(vec![9]);
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Submit { position: point(px(400.0), px(100.0)) }, cx));
    settle(cx).await;
    cx.update(|cx| {
        t.check("dropped over the player: at the end", b.rec.take_commands() == vec![Command::AddPaths(vec!["C:\\c.mp3".into()], Some(8))], ());
        b.close(cx);
    });
    // An empty playlist: the hint turns accent.
    let b = cx.update(|cx| Bench::open("drawer-empty", cx));
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Entered { position: point(px(430.0), px(420.0)), paths: external_paths(&["C:\\a.mp3"]) }, cx));
    cx.update(|cx| t.check("over an empty list: the hint turns accent, no line", b.drawer.read(cx).is_drop_empty() && b.drawer.read(cx).drop_line().is_none(), ()));
    cx.update(|cx| file_drop(&b.session, FileDropEvent::Exited, cx));
    cx.update(|cx| {
        t.check("leave: plain again", !b.drawer.read(cx).is_drop_empty(), ());
        b.close(cx);
    });
}

fn shape(menu: &qust_shell::menu::Menu) -> Vec<String> {
    menu.items
        .iter()
        .map(|item| match item {
            qust_shell::menu::Item::Action { text, enabled, .. } => format!("{text}{}", if *enabled { "" } else { " (off)" }),
            qust_shell::menu::Item::Separator => "-".into(),
        })
        .collect()
}

fn menus(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-entries", cx);
    b.right_click_at(b.at(E(3), cx), cx);
    let menu = b.rec.last_menu().map(|m| shape(&m)).unwrap_or_default();
    t.check("right-click on an unselected track selects it", b.selection(cx) == vec![E(3)], b.selection(cx));
    t.check("the track menu", menu == ["Play", "Edit…", "Remove", "-", "Add file…", "Add URL…", "Clear playlist", "-", "Stream information", "Settings"], &menu);
    b.rec.choose(Some("Remove"), cx);
    b.session.frame(cx);
    t.check("Remove", b.rec.take_commands() == vec![Command::Remove(vec![3])] && b.selection(cx) == vec![E(4)], b.selection(cx));
    b.click(E(1), Modifiers::default(), cx);
    b.click(E(3), shift(), cx);
    b.right_click_at(b.at(E(2), cx), cx);
    let menu = b.rec.last_menu().map(|m| shape(&m)).unwrap_or_default();
    t.check("inside a selection: kept, \"Remove 3 tracks\"", menu.get(2).map(String::as_str) == Some("Remove 3 tracks") && b.selection(cx).len() == 3, &menu);
    b.rec.choose(Some("Edit…"), cx);
    b.session.frame(cx);
    t.check("Edit… opens the Edit track dialog for the clicked track", b.rec.take_calls().ends_with(&[HostCall::EditEntry(2)]), ());
    b.right_click_at(b.at(C(3), cx), cx);
    let menu = b.rec.last_menu().map(|m| shape(&m)).unwrap_or_default();
    t.check("the cartridge row menu", menu == ["Play", "Edit cartridge…", "-", "Add file…", "Add URL…", "Clear playlist", "-", "Stream information", "Settings"] && b.selection(cx) == vec![C(3)], &menu);
    b.rec.choose(Some("Edit cartridge…"), cx);
    b.session.frame(cx);
    t.check("Edit cartridge… opens its dialog", b.rec.take_calls().ends_with(&[HostCall::EditCartridge(3)]), ());
    b.right_click_at(b.at(C(3), cx), cx);
    b.rec.choose(Some("Play"), cx);
    t.check("Play on the cartridge row", b.rec.take_commands() == vec![Command::PlayCartridge(3)], ());
    // Elsewhere: the status line.
    let status = point(100.0, 565.0);
    b.right_click_at(status, cx);
    let menu = b.rec.last_menu().map(|m| shape(&m)).unwrap_or_default();
    t.check("elsewhere in the drawer", menu == ["Add file…", "Add URL…", "Clear playlist", "-", "Stream information", "Settings"], &menu);
    for (text, call) in [("Add file…", HostCall::AddFiles), ("Add URL…", HostCall::AddUrl), ("Stream information", HostCall::OpenPanel(Panel::StreamInformation)), ("Settings", HostCall::OpenPanel(Panel::Settings))] {
        b.right_click_at(status, cx);
        b.rec.take_calls();
        b.rec.choose(Some(text), cx);
        b.session.frame(cx);
        t.check(&format!("{text} goes to the host"), b.rec.take_calls() == vec![call.clone()], ());
    }
    b.right_click_at(status, cx);
    b.rec.choose(None, cx);
    t.check("a dismissed menu does nothing", b.rec.take_commands().is_empty(), ());
    b.right_click_at(status, cx);
    b.rec.choose(Some("Clear playlist"), cx);
    // (The stand-in player doesn't apply removals: the list still has the six entries.)
    t.check("Clear playlist removes every entry", b.rec.take_commands() == vec![Command::Remove(vec![1, 2, 3, 4, 5, 6])], ());
    b.rec.dialog_open.set(true);
    b.rec.take_calls();
    b.right_click_at(b.at(E(1), cx), cx);
    t.check("no menu while the dialog is open", b.rec.take_calls().is_empty(), ());
    b.close(cx);

    let b = Bench::open("drawer-empty", cx);
    b.right_click_at(point(430.0, 450.0), cx);
    let menu = b.rec.last_menu().map(|m| shape(&m)).unwrap_or_default();
    t.check("empty playlist: Clear playlist disabled", menu.get(2).map(String::as_str) == Some("Clear playlist (off)"), &menu);
    b.close(cx);
}

fn scrolling(t: &mut Checks, cx: &mut App) {
    let b = Bench::open("drawer-many", cx);
    let p = b.at(E(102), cx);
    b.session.wheel(p, -3.0, cx);
    t.check("a wheel notch (3 lines) scrolls 100 px", (b.drawer.read(cx).scroll_top() - 100.0).abs() < 0.01, b.drawer.read(cx).scroll_top());
    b.session.wheel(p, 30.0, cx);
    t.check("clamped at the top", b.drawer.read(cx).scroll_top() == 0.0, b.drawer.read(cx).scroll_top());
    // The thumb: 47 px at the top of the track (x 830–834).
    let list = b.drawer.read(cx).list_rect();
    let thumb = point(list.right() - 6.0, list.top + 20.0);
    b.drag(thumb, point(thumb.x, thumb.y + 47.0), 4, false, cx);
    let expected = 47.0 * 1425.0 / (282.0 - 47.0);
    t.check("dragging the thumb scrolls in proportion", (b.drawer.read(cx).scroll_top() - expected).abs() < 0.5, (b.drawer.read(cx).scroll_top(), expected));
    t.check("…without touching the selection", b.selection(cx).is_empty() && b.rec.take_commands().is_empty(), ());
    let before = b.drawer.read(cx).scroll_top();
    b.click_at(point(thumb.x, list.bottom() - 10.0), Modifiers::default(), 1, cx);
    t.check("a press on the track pages (246 px)", (b.drawer.read(cx).scroll_top() - before - 246.0).abs() < 0.5, (before, b.drawer.read(cx).scroll_top()));
    b.close(cx);
}

async fn adds(t: &mut Checks, cx: &mut AsyncApp) {
    let b = cx.update(|cx| Bench::open("drawer-many", cx));
    // Add URL's answer for an entry already in the list (the event came first).
    let mut entries = b.entries();
    entries.push(entry(500, "Added"));
    cx.update(|cx| b.set_playlist(entries, cx));
    cx.update(|cx| {
        let _ = cx.update_window(b.session.any(), |_, window, cx| b.drawer.update(cx, |d, cx| d.select_added(vec![500], window, cx)));
    });
    cx.update(|cx| {
        let max = b.drawer.read(cx).rows().max_scroll();
        t.check("Add URL: the new entry selected and revealed", b.selection(cx) == vec![E(500)] && b.drawer.read(cx).scroll_top() == max, (b.selection(cx), b.drawer.read(cx).scroll_top()));
    });
    // Add file while closed: the drawer opens, the entries are selected and shown.
    cx.update(|cx| {
        let _ = cx.update_window(b.session.any(), |_, window, cx| b.drawer.update(cx, |d, cx| d.set_open(false, window, cx)));
    });
    b.rec.take_calls();
    b.rec.add_answers.borrow_mut().push_back(vec![501, 502]);
    cx.update(|cx| {
        let _ = cx.update_window(b.session.any(), |_, window, cx| b.drawer.update(cx, |d, cx| d.add_paths(vec!["C:\\x.flac".into()], None, window, cx)));
    });
    settle(cx).await;
    let mut entries = b.entries();
    entries.insert(0, entry(501, "First"));
    entries.insert(1, entry(502, "Second"));
    entries.push(entry(500, "Added"));
    cx.update(|cx| b.set_playlist(entries, cx));
    cx.update(|cx| {
        t.check("Add file: sent", b.rec.take_commands() == vec![Command::AddPaths(vec!["C:\\x.flac".into()], None)], ());
        t.check("…the drawer opened itself (the host asked)", b.drawer.read(cx).is_open() && b.rec.take_calls() == vec![HostCall::OpenDrawer], ());
        t.check("…the new entries selected, the first shown", b.selection(cx) == vec![E(501), E(502)] && b.drawer.read(cx).scroll_top() <= b.drawer.read(cx).rows().row_top(5), (b.selection(cx), b.drawer.read(cx).scroll_top()));
        b.close(cx);
    });
}

// Tooltips need GPUI's hover delay (500 ms): drawn in real time. The row's is its path; the
// pinned cartridge's radio icon has its own ("Cartridge N"), as its `title` did.
async fn tooltips(t: &mut Checks, cx: &mut AsyncApp) {
    let b = cx.update(|cx| Bench::open("drawer-entries", cx));
    let (row, icon, cartridge_row) = cx.update(|cx| {
        let icon = b.drawer.read(cx).row_bounds(C(0)).map(|r| point(r.left + 31.0, r.top + 9.0)).expect("row");
        (b.at(E(1), cx), icon, b.at(C(2), cx))
    });
    let cases = [
        ("a track's tooltip is its path", row, r"C:\Users\Me\Music\Daft Punk\Discovery\01 One More Time.flac"),
        ("the radio icon's is \"Cartridge 1\"", icon, "Cartridge 1"),
        ("a pinned cartridge row's is its URL", cartridge_row, "http://stream.radioparadise.com/aac-320"),
    ];
    for (name, at, expected) in cases {
        cx.update(|cx| {
            b.session.mouse_move(at, None, Modifiers::default(), cx);
            b.session.frame(cx);
        });
        for _ in 0..40 {
            cx.update(|cx| b.session.frame(cx));
            cx.background_executor().timer(Duration::from_millis(20)).await;
        }
        let shown = cx.update(|cx| qust_fluent::tooltip::last_shown(cx)).map(|s| s.to_string()).unwrap_or_default();
        t.check(name, shown == expected, shown);
    }
    cx.update(|cx| b.close(cx));
}

fn main() {
    let results = Rc::new(RefCell::new(Checks { failures: Vec::new(), passed: 0 }));
    let out = results.clone();
    capture::run(move |cx: &mut App| {
        // Synchronous scenarios run here, each event its own update.
        let mut t = Checks { failures: Vec::new(), passed: 0 };
        selection_and_clicks(&mut t, cx);
        keyboard(&mut t, cx);
        focus_and_inert(&mut t, cx);
        list_updates(&mut t, cx);
        reorder(&mut t, cx);
        menus(&mut t, cx);
        scrolling(&mut t, cx);
        // Those that wait for the player's answers or for time.
        cx.spawn(async move |cx| {
            file_drops(&mut t, cx).await;
            adds(&mut t, cx).await;
            tooltips(&mut t, cx).await;
            *out.borrow_mut() = t;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let results = results.borrow();
    println!("behaviour: {} passed, {} failed", results.passed, results.failures.len());
    assert!(results.failures.is_empty(), "failed: {:?}", results.failures);
}
