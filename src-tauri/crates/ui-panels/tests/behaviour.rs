// Behaviour of the Settings and Stream information windows (BEHAVIOUR.md 1, 3, 4), driven by
// events dispatched into hidden windows (never shown, never focused, no OS input), against the
// recording fake services: what each interaction asks the app to do, and what the view shows.
//
//   cargo test -p qust-ui-panels --features capture --test behaviour
//
// No native menu, tooltip or key capture is ever opened: the fake records menus (answered here),
// takes key captures (fed here), and the harness turns native tooltips off.

use gpui::{point, AppContext, AsyncApp, Modifiers, MouseButton, Point};
use qust_shell::menu::{copy_menu, Item, Menu, Position};
use qust_shell::recorder::Recorded;
use qust_ui::capture::Job;
use qust_ui::fixture::{self, Fixture};
use qust_ui::{Playback, Status, UpdateStatus};
use qust_ui_panels::harness::{self, Opened, Panel};
use qust_ui_panels::info::{InfoView, VALUES};
use qust_ui_panels::settings::{SettingsView, RECORDING_HINT};
use qust_ui_panels::ComboKind;
use qust_fluent::ShortcutDisplay;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Default)]
struct Checks {
    failed: Vec<String>,
    passed: usize,
}

type Shared = Rc<RefCell<Checks>>;

fn check(checks: &Shared, name: &str, ok: bool, detail: impl std::fmt::Debug) {
    println!("{} {name}: {detail:?}", if ok { "PASS" } else { "FAIL" });
    let mut c = checks.borrow_mut();
    if ok {
        c.passed += 1;
    } else {
        c.failed.push(name.to_string());
    }
}

fn job(name: &str) -> Job {
    let fixture = Fixture::load(name).expect("fixture");
    Job { variant: name.to_string(), time: None, theme: "dark".into(), out: std::env::temp_dir().join(format!("qust-panels-behaviour-{name}.png")), fixture }
}

async fn open(name: &str, cx: &mut AsyncApp) -> (Job, Opened) {
    let job = job(name);
    let opened = cx.update(|cx| {
        let opened = harness::open(&job, cx).expect("open");
        opened.session.frames(2, cx);
        opened
    });
    harness::wait_for_size(&opened, job.fixture.window_size(), cx).await;
    (job, opened)
}

async fn settle(opened: &Opened, cx: &mut AsyncApp) {
    harness::settle(opened, cx).await;
}

fn settings(opened: &Opened) -> gpui::Entity<SettingsView> {
    match &opened.panel {
        Panel::Settings(view) => view.clone(),
        Panel::Info(_) => panic!("not settings"),
    }
}

fn info(opened: &Opened) -> gpui::Entity<InfoView> {
    match &opened.panel {
        Panel::Info(view) => view.clone(),
        Panel::Settings(_) => panic!("not info"),
    }
}

fn at(job: &Job, name: &str) -> Point<f32> {
    harness::center(job, name).unwrap_or_else(|| panic!("no box for {name}"))
}

async fn click(opened: &Opened, p: Point<f32>, cx: &mut AsyncApp) {
    cx.update(|cx| opened.session.click(p, cx));
    settle(opened, cx).await;
}

async fn key(opened: &Opened, k: &'static str, cx: &mut AsyncApp) {
    cx.update(|cx| opened.session.key(k, cx));
    settle(opened, cx).await;
}

type MenuItems = Vec<(u32, String, bool)>;

fn items(menu: &Menu) -> MenuItems {
    menu.items
        .iter()
        .filter_map(|item| match item {
            Item::Action { id, text, checked, .. } => Some((*id, text.clone(), *checked)),
            Item::Separator => None,
        })
        .collect()
}

fn last_menu(opened: &Opened) -> Option<(MenuItems, Position)> {
    opened.fake.menus.borrow().last().map(|(menu, position)| (items(menu), *position))
}

fn calls_since(opened: &Opened, from: usize) -> Vec<String> {
    opened.fake.calls()[from..].to_vec()
}

fn mark(opened: &Opened) -> usize {
    opened.fake.calls().len()
}

async fn settings_tests(c: &Shared, cx: &mut AsyncApp) {
    let (job, opened) = open("settings-default", cx).await;
    let view = settings(&opened);
    let models = opened.session.models.clone();

    // --- Combos (3.2) ---
    click(&opened, at(&job, "#themeCombo"), cx).await;
    let menu = last_menu(&opened);
    check(c, "theme combo: its menu, current choice checked", menu.as_ref().map(|m| m.0.clone()) == Some(vec![(1, "Use system setting".into(), true), (2, "Light".into(), false), (3, "Dark".into(), false)]), &menu);
    check(c, "theme combo: menu at the combo's bottom-left + 2 px", menu.as_ref().map(|m| m.1) == Some(Position::Client { x: 305.0, y: 88.0 }), menu.as_ref().map(|m| m.1));
    let from = mark(&opened);
    cx.update(|cx| opened.fake.answer_menu(Some(3), cx));
    settle(&opened, cx).await;
    let shown = cx.update(|cx| view.read(cx).choice(ComboKind::Theme));
    check(c, "theme: choosing Dark applies it (set_theme) and the combo shows it", calls_since(&opened, from) == ["set_theme dark"] && shown == "dark", (calls_since(&opened, from), shown));
    click(&opened, at(&job, "#themeCombo"), cx).await;
    check(c, "theme: the menu then checks Dark", last_menu(&opened).is_some_and(|m| m.0[2].2 && !m.0[0].2), last_menu(&opened));
    cx.update(|cx| opened.fake.answer_menu(None, cx));

    click(&opened, at(&job, "#colorsCombo"), cx).await;
    let menu = last_menu(&opened);
    check(c, "colors combo: Classic, White, Accent color, Off", menu.as_ref().map(|m| m.0.clone()) == Some(vec![(1, "Classic".into(), true), (2, "White".into(), false), (3, "Accent color".into(), false), (4, "Off".into(), false)]), &menu);
    check(c, "colors combo: menu position", menu.as_ref().map(|m| m.1) == Some(Position::Client { x: 305.0, y: 154.0 }), menu.as_ref().map(|m| m.1));
    let from = mark(&opened);
    cx.update(|cx| opened.fake.answer_menu(Some(4), cx));
    settle(&opened, cx).await;
    let colors = cx.update(|cx| models.settings.read(cx).colors.clone());
    check(c, "colors: saved as visualizer-colors and broadcast as colors", calls_since(&opened, from) == ["save_setting visualizer-colors \"off\"", "broadcast_setting colors \"off\""] && colors == "off", (calls_since(&opened, from), colors));

    click(&opened, at(&job, "#reconnectCombo"), cx).await;
    let menu = last_menu(&opened);
    let expected: Vec<(u32, String, bool)> =
        ["Until stopped", "1 minute", "5 minutes", "15 minutes", "30 minutes", "1 hour"].iter().enumerate().map(|(i, t)| (i as u32 + 1, t.to_string(), i == 2)).collect();
    check(c, "reconnect combo: the six choices in the page's order, 5 minutes checked", menu.as_ref().map(|m| m.0.clone()) == Some(expected), &menu);
    let from = mark(&opened);
    cx.update(|cx| opened.fake.answer_menu(None, cx));
    settle(&opened, cx).await;
    check(c, "a dismissed menu changes nothing", calls_since(&opened, from).is_empty(), calls_since(&opened, from));
    click(&opened, at(&job, "#reconnectCombo"), cx).await;
    let from = mark(&opened);
    cx.update(|cx| opened.fake.answer_menu(Some(1), cx));
    settle(&opened, cx).await;
    let shown = cx.update(|cx| view.read(cx).choice(ComboKind::Reconnect));
    check(c, "reconnect: Until stopped saved as 0 and broadcast as reconnectMinutes", calls_since(&opened, from) == ["save_setting reconnect-minutes 0", "broadcast_setting reconnectMinutes 0"] && shown == "0", (calls_since(&opened, from), shown));

    // The colors follow the player (the analyzer click broadcasts `colors`).
    cx.update(|cx| models.apply_setting("colors", &json!("white"), cx));
    settle(&opened, cx).await;
    let after_white = cx.update(|cx| view.read(cx).choice(ComboKind::Colors));
    cx.update(|cx| models.apply_setting("colors", &json!("pink"), cx));
    settle(&opened, cx).await;
    let after_pink = cx.update(|cx| view.read(cx).choice(ComboKind::Colors));
    check(c, "colors combo follows settings-changed from the player (valid values only)", after_white == "white" && after_pink == "white", (after_white, after_pink));

    // --- Play on launch (3.3) ---
    let track = at(&job, "body>.settings-list>.setting[2]>.toggle>.toggle-track");
    let from = mark(&opened);
    click(&opened, track, cx).await;
    let on = cx.update(|cx| view.read(cx).autoplay());
    check(c, "autoplay: a click on the track switches it on, saved and broadcast", on && calls_since(&opened, from) == ["save_setting autoplay true", "broadcast_setting autoplay true"], calls_since(&opened, from));
    click(&opened, at(&job, "#autoplayLabel"), cx).await;
    let on = cx.update(|cx| view.read(cx).autoplay());
    check(c, "autoplay: a click on the On / Off label switches it too", !on, on);

    // --- Keyboard: Tab order, Space, Enter (BEHAVIOUR 1) ---
    let order = cx.update(|cx| {
        let _ = cx.update_window(opened.session.any(), |_, window, cx| {
            let root = view.read(cx).root_focus();
            root.focus(window, cx);
        });
        let mut seen = Vec::new();
        for _ in 0..10 {
            opened.session.key("tab", cx);
            let focused = cx.update_window(opened.session.any(), |_, window, cx| view.read(cx).focused(window)).ok().flatten();
            seen.push(focused.unwrap_or("none"));
        }
        seen
    });
    check(c, "Tab order follows the page", order == ["theme", "colors", "autoplay", "reconnect", "background", "play", "stop", "toggle", "update", "auto"], &order);
    let back = cx.update(|cx| {
        opened.session.key("shift-tab", cx);
        cx.update_window(opened.session.any(), |_, window, cx| view.read(cx).focused(window)).ok().flatten()
    });
    check(c, "Shift+Tab goes back", back == Some("update"), back);
    // Back to Play on launch: Space toggles it, Enter doesn't.
    for _ in 0..6 {
        cx.update(|cx| opened.session.key("shift-tab", cx));
    }
    let focused = cx.update(|cx| cx.update_window(opened.session.any(), |_, window, cx| view.read(cx).focused(window)).ok().flatten());
    key(&opened, "space", cx).await;
    let after_space = cx.update(|cx| view.read(cx).autoplay());
    key(&opened, "enter", cx).await;
    let after_enter = cx.update(|cx| view.read(cx).autoplay());
    check(c, "a focused toggle: Space switches it, Enter doesn't", focused == Some("autoplay") && after_space && after_enter, (focused, after_space, after_enter));
    cx.update(|cx| opened.session.key("shift-tab", cx));
    let menus = opened.fake.menus.borrow().len();
    key(&opened, "enter", cx).await;
    check(c, "Enter on a focused combo opens its menu", opened.fake.menus.borrow().len() == menus + 1, opened.fake.menus.borrow().len());
    cx.update(|cx| opened.fake.answer_menu(None, cx));

    // --- Recording a shortcut (3.4), through the native capture ---
    let play_box = at(&job, "body>.settings-list>.setting[5]>.shortcut");
    let stop_box = at(&job, "body>.settings-list>.setting[6]>.shortcut");
    let toggle_box = at(&job, "body>.settings-list>.setting[7]>.shortcut");
    let from = mark(&opened);
    click(&opened, stop_box, cx).await;
    let (recording, row) = cx.update(|cx| (view.read(cx).recording(), view.read(cx).shortcut_row(1, cx)));
    check(
        c,
        "click: recording starts (shortcuts paused, keys captured, Press a shortcut…, the hint)",
        recording == Some("stop") && calls_since(&opened, from) == ["pause_shortcuts true", "capture_keys"] && row.display == ShortcutDisplay::Recording && row.status == RECORDING_HINT && opened.fake.capturing(),
        (recording, calls_since(&opened, from), &row),
    );
    opened.fake.press(Recorded::Ignore);
    settle(&opened, cx).await;
    check(c, "a key that doesn't count (modifier alone, plain letter) keeps recording", cx.update(|cx| view.read(cx).recording()) == Some("stop"), ());
    let from = mark(&opened);
    click(&opened, play_box, cx).await;
    check(c, "only one shortcut records at a time", cx.update(|cx| view.read(cx).recording()) == Some("stop") && calls_since(&opened, from).is_empty(), calls_since(&opened, from));
    opened.fake.press(Recorded::Set("Ctrl+Alt+KeyP".into()));
    settle(&opened, cx).await;
    let (recording, row) = cx.update(|cx| (view.read(cx).recording(), view.read(cx).shortcut_row(1, cx)));
    check(
        c,
        "a combination: set_shortcut, then the shortcuts resume, the capture ends, the new keys show",
        recording.is_none()
            && calls_since(&opened, from) == ["set_shortcut stop Ctrl+Alt+KeyP", "pause_shortcuts false"]
            && !opened.fake.capturing()
            && row.display == ShortcutDisplay::Keys { keys: vec!["Ctrl".into(), "Alt".into(), "P".into()], unavailable: false }
            && !row.error,
        (calls_since(&opened, from), &row),
    );
    let from = mark(&opened);
    click(&opened, play_box, cx).await;
    opened.fake.press(Recorded::Cancel);
    settle(&opened, cx).await;
    check(c, "Esc (or losing focus) cancels: nothing set, shortcuts resume, the window stays", calls_since(&opened, from) == ["pause_shortcuts true", "capture_keys", "pause_shortcuts false"] && opened.fake.closed.get() == 0, calls_since(&opened, from));
    let from = mark(&opened);
    click(&opened, play_box, cx).await;
    opened.fake.press(Recorded::Reset);
    settle(&opened, cx).await;
    let row = cx.update(|cx| view.read(cx).shortcut_row(0, cx));
    check(c, "Backspace / Delete reset to the default", calls_since(&opened, from).contains(&"set_shortcut play ".to_string()) && row.display == ShortcutDisplay::Keys { keys: vec!["Ctrl".into(), "Alt".into(), "Insert".into()], unavailable: false }, (calls_since(&opened, from), &row));
    for error in ["Already used for another action", "Another app is already using this shortcut", "That isn't a valid shortcut"] {
        opened.fake.errors.borrow_mut().insert("set_shortcut".into(), error.into());
        click(&opened, toggle_box, cx).await;
        opened.fake.press(Recorded::Set("Ctrl+Alt+KeyB".into()));
        settle(&opened, cx).await;
        let row = cx.update(|cx| view.read(cx).shortcut_row(2, cx));
        check(c, &format!("a refused shortcut shows \"{error}\" in red"), row.status == error && row.error && row.display == ShortcutDisplay::Unassigned, &row);
    }
    opened.fake.errors.borrow_mut().remove("set_shortcut");
    click(&opened, toggle_box, cx).await;
    let row = cx.update(|cx| view.read(cx).shortcut_row(2, cx));
    opened.fake.press(Recorded::Cancel);
    settle(&opened, cx).await;
    let after = cx.update(|cx| view.read(cx).shortcut_row(2, cx));
    check(c, "the error stays until the next change (recording again clears it)", row.status == RECORDING_HINT && !after.error, (&row, &after));
    let unavailable = cx.update(|cx| {
        let mut shortcuts = models.shortcuts.read(cx).shortcuts.clone();
        shortcuts.stop.registered = false;
        models.apply_shortcuts(shortcuts, cx);
        view.read(cx).shortcut_row(1, cx)
    });
    check(
        c,
        "a shortcut another app owns: struck-through keys, the red hint",
        unavailable.error && unavailable.status == "Another app is using this shortcut. Pick a different one." && matches!(unavailable.display, ShortcutDisplay::Keys { unavailable: true, .. }),
        &unavailable,
    );

    // The same through GPUI's own key events (no native capture: macOS, or tests).
    opened.fake.native_capture.set(false);
    for (target, keys, expected) in [
        (stop_box, vec!["ctrl-alt-q"], Some("set_shortcut stop Ctrl+Alt+KeyQ")),
        (toggle_box, vec!["shift-tab"], Some("set_shortcut toggle Shift+Tab")),
        (play_box, vec!["a", "f9"], Some("set_shortcut play F9")),
        (play_box, vec!["alt-backspace"], Some("set_shortcut play ")),
        (stop_box, vec!["escape"], None),
    ] {
        let from = mark(&opened);
        click(&opened, target, cx).await;
        for k in &keys {
            key(&opened, k, cx).await;
        }
        let calls = calls_since(&opened, from);
        let ok = match expected {
            Some(call) => calls.contains(&call.to_string()) && calls.last().map(String::as_str) == Some("pause_shortcuts false"),
            None => !calls.iter().any(|call| call.starts_with("set_shortcut")) && calls.last().map(String::as_str) == Some("pause_shortcuts false"),
        };
        check(c, &format!("GPUI keys {keys:?} while recording"), ok && cx.update(|cx| view.read(cx).recording()).is_none() && opened.fake.closed.get() == 0, calls);
    }
    opened.fake.native_capture.set(true);

    // Esc closes the window when nothing records.
    let from = mark(&opened);
    key(&opened, "escape", cx).await;
    check(c, "Esc closes Settings", calls_since(&opened, from) == ["close_window"], calls_since(&opened, from));

    // --- The background helper (3.3) ---
    let helper = at(&job, "body>.settings-list>.setting[4]>.toggle>.toggle-track");
    let from = mark(&opened);
    click(&opened, helper, cx).await;
    let row = cx.update(|cx| view.read(cx).background_row(cx));
    check(c, "helper: on, Rust's answer shown", calls_since(&opened, from) == ["set_background true"] && row.0 && !row.3, (calls_since(&opened, from), &row));
    opened.fake.errors.borrow_mut().insert("set_background".into(), "Couldn't change the Windows startup entry: Access is denied. (os error 5)".into());
    click(&opened, helper, cx).await;
    let row = cx.update(|cx| view.read(cx).background_row(cx));
    check(c, "helper: a failure turns it back and shows the error in red", row.0 && row.3 && row.2.starts_with("Couldn't change"), &row);
    opened.fake.errors.borrow_mut().remove("set_background");
    click(&opened, helper, cx).await;
    let row = cx.update(|cx| view.read(cx).background_row(cx));
    check(c, "helper: the next change clears the error", !row.0 && !row.3, &row);
    {
        let (job, unavailable) = open("settings-background-unavailable", cx).await;
        let row = cx.update(|cx| settings(&unavailable).read(cx).background_row(cx));
        click(&unavailable, at(&job, "body>.settings-list>.setting[4]>.toggle>.toggle-track"), cx).await;
        check(c, "helper unavailable: off, the installed-version text, inert", !row.0 && !row.1 && row.2 == "Works in the installed version of Qust." && unavailable.fake.calls().is_empty(), (&row, unavailable.fake.calls()));
        cx.update(|cx| unavailable.session.close(cx));
    }

    // --- Updates (3.5) ---
    let button = point(455.0, 725.0);
    let base = UpdateStatus { installed: true, current: "1.1.5".into(), status: "idle".into(), last_check: Some(1_794_392_500), auto: true, ..Default::default() };
    // (status, text, label, busy, accent, what a click asks for)
    #[allow(clippy::type_complexity)]
    let cases: Vec<(UpdateStatus, &str, &str, bool, bool, Option<&str>)> = vec![
        (UpdateStatus { installed: false, ..base.clone() }, "Updates work in the installed version of Qust.", "Check for updates", true, false, None),
        (base.clone(), "Last checked 2 h ago.", "Check for updates", false, false, Some("check_for_updates")),
        (UpdateStatus { last_check: None, ..base.clone() }, "Not checked yet.", "Check for updates", false, false, Some("check_for_updates")),
        (UpdateStatus { status: "checking".into(), ..base.clone() }, "Checking for updates…", "Check for updates", true, false, None),
        (UpdateStatus { status: "downloading".into(), version: Some("1.2.0".into()), progress: Some(42), ..base.clone() }, "Downloading version 1.2.0… 42%", "Check for updates", true, false, None),
        (UpdateStatus { status: "ready".into(), version: Some("1.2.0".into()), ..base.clone() }, "Version 1.2.0 is ready to install.", "Restart and update", false, true, Some("install_update")),
        (UpdateStatus { status: "installing".into(), ..base.clone() }, "Installing…", "Check for updates", true, false, None),
        (UpdateStatus { status: "error".into(), error: Some("timed out".into()), ..base.clone() }, "Couldn't update: timed out", "Try again", false, false, Some("check_for_updates")),
        (UpdateStatus { status: "current".into(), last_check: Some(1_794_399_990), ..base.clone() }, "You're up to date. Last checked just now.", "Check for updates", false, false, Some("check_for_updates")),
    ];
    for (status, text, label, busy, accent, call) in cases {
        cx.update(|cx| models.apply_update(status.clone(), cx));
        settle(&opened, cx).await;
        let (row, _) = cx.update(|cx| view.read(cx).update_state(cx));
        let from = mark(&opened);
        click(&opened, button, cx).await;
        let calls = calls_since(&opened, from);
        let ok = row.text == text && row.label == label && row.busy == busy && row.accent == accent && calls == call.into_iter().map(str::to_string).collect::<Vec<_>>();
        check(c, &format!("updates: {} {}", if status.installed { "installed" } else { "not installed" }, status.status), ok, (&row, calls));
    }
    let auto = at(&job, "body>.settings-list>.setting[9]>.toggle>.toggle-track");
    cx.update(|cx| models.apply_update(base.clone(), cx));
    settle(&opened, cx).await;
    let from = mark(&opened);
    click(&opened, auto, cx).await;
    let (_, on) = cx.update(|cx| view.read(cx).update_state(cx));
    check(c, "check automatically: off, saved (auto-update)", !on && calls_since(&opened, from) == ["save_setting auto-update false"], (on, calls_since(&opened, from)));
    cx.update(|cx| models.apply_update(UpdateStatus { installed: false, ..base.clone() }, cx));
    settle(&opened, cx).await;
    let from = mark(&opened);
    click(&opened, auto, cx).await;
    check(c, "check automatically is inert when not installed", calls_since(&opened, from).is_empty(), calls_since(&opened, from));

    // --- Scrolling (3.1) ---
    cx.update(|cx| opened.session.wheel(point(260.0, 400.0), -3.0, cx));
    let offset = cx.update(|cx| view.read(cx).scroll_handle().offset().y.as_f32());
    check(c, "the wheel scrolls the list", offset < 0.0, offset);

    cx.update(|cx| opened.session.close(cx));
    drop(view);

    // --- Closing mid-recording resumes the shortcuts (nothing here holds the view) ---
    let (job, opened) = open("settings-default", cx).await;
    click(&opened, at(&job, "body>.settings-list>.setting[7]>.shortcut"), cx).await;
    let recording = cx.update(|cx| settings(&opened).read(cx).recording());
    let fake = opened.fake.clone();
    let session = opened.session.clone();
    drop(opened);
    let from = fake.calls().len();
    cx.update(|cx| session.close(cx));
    cx.background_executor().timer(Duration::from_millis(50)).await;
    let calls = fake.calls()[from..].to_vec();
    check(c, "closing Settings while recording resumes the shortcuts", recording == Some("toggle") && calls == ["pause_shortcuts false"] && !fake.capturing(), calls);
}

/// Draws a frame the way a visible window does: only what was notified re-renders (the harness's
/// `frame` refreshes the whole window).
fn draw(opened: &Opened, cx: &mut AsyncApp) {
    cx.update(|cx| {
        let _ = cx.update_window(opened.session.any(), |_, window, cx| {
            window.draw(cx).clear(cx);
        });
    });
}

/// The texts each info fixture shows, against the layout file's.
async fn info_texts(c: &Shared, cx: &mut AsyncApp) {
    for name in fixture::all().into_iter().filter(|n| n.starts_with("info-")) {
        let (job, opened) = open(&name, cx).await;
        settle(&opened, cx).await;
        let texts = cx.update(|cx| info(&opened).read(cx).texts().clone());
        let layout = fixture::layout("info", &name).expect("layout");
        let elements = layout["elements"].as_array().cloned().unwrap_or_default();
        let text_of = |id: &str| elements.iter().find(|e| e["name"] == format!("#{id}")).and_then(|e| e["text"].as_str().map(str::to_string));
        let mut wrong = Vec::new();
        for (id, ours) in [("station", &texts.station), ("title", &texts.title), ("state", &texts.state)] {
            if let Some(theirs) = text_of(id) {
                if &theirs != ours {
                    wrong.push(format!("{id}: {ours:?} vs {theirs:?}"));
                }
            }
        }
        let mut compared = 0;
        for (i, id) in VALUES.iter().enumerate() {
            if let Some(theirs) = text_of(id) {
                compared += 1;
                if theirs != texts.values[i] {
                    wrong.push(format!("{id}: {:?} vs {theirs:?}", texts.values[i]));
                }
            }
        }
        check(c, &format!("{name}: every text as info.js formatted it ({compared} values)"), wrong.is_empty() && compared > 0, &wrong);
        cx.update(|cx| opened.session.close(cx));
        let _ = job;
    }
}

async fn info_tests(c: &Shared, cx: &mut AsyncApp) {
    let (job, opened) = open("info-playing-stream", cx).await;
    let view = info(&opened);
    let models = opened.session.models.clone();
    settle(&opened, cx).await;
    let requests = |opened: &Opened| opened.fake.calls().iter().filter(|call| call.starts_with("stream_info")).count();
    check(c, "opening: the details are asked for at once, by the stream's URL", opened.fake.calls().first().map(String::as_str) == Some("stream_info http://stream.c9.fr/c9-128.mp3"), opened.fake.calls());
    let before = requests(&opened);
    cx.background_executor().timer(Duration::from_millis(2100)).await;
    settle(&opened, cx).await;
    let polled = requests(&opened) - before;
    check(c, "while playing: once a second", (2..=3).contains(&polled) && cx.update(|cx| view.read(cx).polling()), polled);

    let stopped = Status { state: Playback::Stopped, ..Default::default() };
    cx.update(|cx| models.apply_status(stopped, cx));
    settle(&opened, cx).await;
    let before = requests(&opened);
    let texts = cx.update(|cx| view.read(cx).texts().clone());
    cx.background_executor().timer(Duration::from_millis(1300)).await;
    settle(&opened, cx).await;
    check(
        c,
        "stopped: the poll stops, every value is a dash, the hint shows",
        requests(&opened) == before && !cx.update(|cx| view.read(cx).polling()) && texts.values.iter().all(|v| v == "—") && texts.title == "Start a cartridge to see its stream details" && texts.state == "Stopped",
        (requests(&opened) - before, &texts.title),
    );
    let playing = Status { state: Playback::Playing, station: Some("C9 Radio".into()), url: Some("http://stream.c9.fr/c9-128.mp3".into()), buffer: Some(2.43), stalls: 2, status_title: None };
    cx.update(|cx| models.apply_status(playing.clone(), cx));
    settle(&opened, cx).await;
    check(c, "playing again: asked for at once, the poll restarts", requests(&opened) == before + 1 && cx.update(|cx| view.read(cx).polling()), requests(&opened) - before);

    // Re-rendered only when a text changes (2.43 → 2.44 s still shows "2.4 s").
    draw(&opened, cx);
    let renders = cx.update(|cx| view.read(cx).renders);
    cx.update(|cx| models.apply_status(Status { buffer: Some(2.44), ..playing.clone() }, cx));
    draw(&opened, cx);
    let same = cx.update(|cx| view.read(cx).renders) == renders;
    cx.update(|cx| models.apply_status(Status { buffer: Some(3.0), ..playing.clone() }, cx));
    draw(&opened, cx);
    let changed = cx.update(|cx| view.read(cx).renders) > renders;
    check(c, "re-renders only when a shown text changes", same && changed, (same, changed));

    // Right-click on a value: selected whole, then the native Copy menu at the cursor.
    let format = at(&job, "#format");
    cx.update(|cx| {
        opened.session.frame(cx);
        opened.session.mouse_move(format, None, Modifiers::default(), cx);
        opened.session.mouse_down(format, MouseButton::Right, 1, Modifiers::default(), cx);
        opened.session.mouse_up(format, MouseButton::Right, 1, Modifiers::default(), cx);
    });
    settle(&opened, cx).await;
    let menu = opened.fake.menus.borrow().last().cloned();
    let selected = cx.update(|cx| view.read(cx).selected());
    check(c, "right-click a value: it is selected and Copy is offered at the cursor", menu == Some((copy_menu(), Position::Cursor)) && selected == Some("format"), (&menu, selected));
    cx.update(|cx| opened.fake.answer_menu(Some(0xE002), cx));
    check(c, "Copy puts the value on the clipboard", opened.fake.clipboard.borrow().last().map(String::as_str) == Some("MP3"), opened.fake.clipboard.borrow().clone());
    click(&opened, point(300.0, 580.0), cx).await;
    check(c, "a click elsewhere clears the selection", cx.update(|cx| view.read(cx).selected()).is_none(), ());
    let bitrate = at(&job, "#bitrate");
    cx.update(|cx| {
        opened.session.mouse_down(bitrate, MouseButton::Right, 1, Modifiers::default(), cx);
        opened.session.mouse_up(bitrate, MouseButton::Right, 1, Modifiers::default(), cx);
    });
    settle(&opened, cx).await;
    cx.update(|cx| opened.fake.answer_menu(None, cx));
    key(&opened, "ctrl-c", cx).await;
    check(c, "Ctrl+C copies the selected value", opened.fake.clipboard.borrow().last().map(String::as_str) == Some("128 kbps"), opened.fake.clipboard.borrow().clone());
    let label = at(&job, "body>.info>.info-cards>.info-card[0]>dl>dt[0]");
    let menus = opened.fake.menus.borrow().len();
    cx.update(|cx| {
        opened.session.mouse_down(label, MouseButton::Right, 1, Modifiers::default(), cx);
        opened.session.mouse_up(label, MouseButton::Right, 1, Modifiers::default(), cx);
    });
    settle(&opened, cx).await;
    check(c, "right-click outside a value shows nothing", opened.fake.menus.borrow().len() == menus, ());

    // Tooltips: the full text of a value (truncated or not).
    let title = at(&job, "#title");
    cx.update(|cx| opened.session.mouse_move(title, None, Modifiers::default(), cx));
    for _ in 0..8 {
        cx.background_executor().timer(Duration::from_millis(100)).await;
        cx.update(|cx| opened.session.frame(cx));
    }
    let tooltip = cx.update(|cx| qust_fluent::tooltip::last_shown(cx));
    check(c, "the title's tooltip is its full text", tooltip.as_deref() == Some("Superman Lovers - Starlight"), &tooltip);

    let from = mark(&opened);
    key(&opened, "escape", cx).await;
    check(c, "Esc closes Stream information", calls_since(&opened, from) == ["close_window"], calls_since(&opened, from));
    cx.update(|cx| opened.session.close(cx));
}

fn main() {
    let checks: Shared = Rc::new(RefCell::new(Checks::default()));
    let shared = checks.clone();
    qust_ui::capture::run(move |cx| {
        cx.spawn(async move |cx| {
            settings_tests(&shared, cx).await;
            info_tests(&shared, cx).await;
            info_texts(&shared, cx).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let checks = checks.borrow();
    println!("\n{} passed, {} failed", checks.passed, checks.failed.len());
    assert!(checks.failed.is_empty(), "failed: {:#?}", checks.failed);
}
