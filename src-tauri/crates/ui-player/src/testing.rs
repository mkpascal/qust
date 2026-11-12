//! For captures and tests (feature `capture`): a recording stand-in for the player and the
//! outside world ([`Recorder`]), the view built in a harness window ([`build`]), and the reference
//! fixtures' actions replayed on it ([`run_actions`]) with real dispatched input where the page
//! got real input (presses, drags, right-clicks).
//!
//! Nothing here opens OS UI: menus are recorded (and can be "chosen" by a test), the picker is a
//! recorded call.

use crate::commands::{Answer, Commands};
use crate::host::{MenuDone, Panel, PlayerHost};
use crate::view::{ForcedStates, PlayerView, HEIGHT, WIDTH};
use futures::channel::oneshot;
use gpui::{div, point, prelude::*, px, AnyView, App, AppContext, Context, Entity, IntoElement, Keystroke, Modifiers, MouseButton, Point, Render, StyleRefinement, Window};
use qust_shell::menu::Menu;
use qust_ui::capture::{Session, Setup};
use qust_ui::fixture::{element_box, layout, Fixture};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A command the view sent.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Stop,
    SetInstant(bool),
    CycleColors,
    SetVolume(i64),
    SetBalance(i64),
    Seek(f64),
    SelectSlot(usize),
    Eject(usize),
    SaveCartridge(usize, String, String),
    MoveCartridge(usize, usize),
    EditEntry(u64, String, Option<String>),
    AddUrl(String, String),
}

/// A call to the outside world.
#[derive(Clone, Debug, PartialEq)]
pub enum HostCall {
    OpenPanel(Panel),
    SetDrawer(bool),
    AddFiles,
    Added(Vec<u64>),
    Menu(Menu),
}

#[derive(Default)]
struct Log {
    commands: Vec<Command>,
    calls: Vec<HostCall>,
    /// The page's command names (`cartridge_save`, `playlist_edit`, `playlist_add_url`) that
    /// answer with an error.
    errors: HashMap<String, String>,
    /// Answers wait until [`Recorder::release`].
    hold: bool,
    held: Vec<Box<dyn FnOnce()>>,
    menu_done: Option<MenuDone>,
    added: Vec<u64>,
}

/// The stand-in: records every command and outside call; answers at once (or when released).
#[derive(Clone, Default)]
pub struct Recorder(Rc<RefCell<Log>>);

impl Recorder {
    pub fn new() -> Recorder {
        let recorder = Recorder::default();
        recorder.0.borrow_mut().added = vec![100];
        recorder
    }

    pub fn commands(&self) -> Vec<Command> {
        self.0.borrow().commands.clone()
    }

    pub fn calls(&self) -> Vec<HostCall> {
        self.0.borrow().calls.clone()
    }

    pub fn clear(&self) {
        let mut log = self.0.borrow_mut();
        log.commands.clear();
        log.calls.clear();
    }

    /// `command` (`cartridge_save`, `playlist_edit`, `playlist_add_url`) answers `Err(message)`.
    pub fn fail(&self, command: &str, message: &str) {
        self.0.borrow_mut().errors.insert(command.to_string(), message.to_string());
    }

    pub fn succeed(&self, command: &str) {
        self.0.borrow_mut().errors.remove(command);
    }

    /// From now on answers wait until [`Recorder::release`].
    pub fn hold(&self, hold: bool) {
        self.0.borrow_mut().hold = hold;
    }

    /// Lets every held answer through.
    pub fn release(&self) {
        let held: Vec<_> = self.0.borrow_mut().held.drain(..).collect();
        for answer in held {
            answer();
        }
    }

    /// The last menu shown, if any.
    pub fn last_menu(&self) -> Option<Menu> {
        self.0.borrow().calls.iter().rev().find_map(|call| match call {
            HostCall::Menu(menu) => Some(menu.clone()),
            _ => None,
        })
    }

    /// Picks item `id` of the menu that is "open" (None: dismissed).
    pub fn choose(&self, id: Option<u32>, cx: &mut App) {
        let done = self.0.borrow_mut().menu_done.take();
        if let Some(done) = done {
            done(id, cx);
        }
    }

    fn record(&self, command: Command) {
        self.0.borrow_mut().commands.push(command);
    }

    fn answer<T: 'static>(&self, value: T) -> Answer<T> {
        let (tx, rx) = oneshot::channel();
        let mut log = self.0.borrow_mut();
        if log.hold {
            log.held.push(Box::new(move || {
                let _ = tx.send(value);
            }));
        } else {
            let _ = tx.send(value);
        }
        Box::pin(async move { rx.await.ok() })
    }

    fn result<T: 'static>(&self, command: &str, ok: T) -> Answer<Result<T, String>> {
        let error = self.0.borrow().errors.get(command).cloned();
        self.answer(match error {
            Some(message) => Err(message),
            None => Ok(ok),
        })
    }
}

impl Commands for Recorder {
    fn play(&self) {
        self.record(Command::Play)
    }
    fn pause(&self) {
        self.record(Command::Pause)
    }
    fn stop(&self) {
        self.record(Command::Stop)
    }
    fn set_instant(&self, on: bool) {
        self.record(Command::SetInstant(on))
    }
    fn cycle_colors(&self) {
        self.record(Command::CycleColors)
    }
    fn set_volume(&self, value: i64) {
        self.record(Command::SetVolume(value))
    }
    fn set_balance(&self, value: i64) {
        self.record(Command::SetBalance(value))
    }
    fn seek(&self, seconds: f64) {
        self.record(Command::Seek(seconds))
    }
    fn select_slot(&self, slot: usize) {
        self.record(Command::SelectSlot(slot))
    }
    fn eject(&self, slot: usize) {
        self.record(Command::Eject(slot))
    }
    fn save_cartridge(&self, slot: usize, title: String, url: String) -> Answer<Result<(), String>> {
        self.record(Command::SaveCartridge(slot, title, url));
        self.result("cartridge_save", ())
    }
    fn move_cartridge(&self, from: usize, to: usize) -> Answer<()> {
        self.record(Command::MoveCartridge(from, to));
        self.answer(())
    }
    fn edit_entry(&self, id: u64, title: String, url: Option<String>) -> Answer<Result<(), String>> {
        self.record(Command::EditEntry(id, title, url));
        self.result("playlist_edit", ())
    }
    fn add_url(&self, url: String, title: String) -> Answer<Result<Vec<u64>, String>> {
        self.record(Command::AddUrl(url, title));
        let added = self.0.borrow().added.clone();
        self.result("playlist_add_url", added)
    }
}

impl PlayerHost for Recorder {
    fn open_panel(&self, panel: Panel, _: &mut Window, _: &mut App) {
        self.0.borrow_mut().calls.push(HostCall::OpenPanel(panel));
    }
    fn set_drawer(&self, open: bool, _: &mut Window, _: &mut App) {
        self.0.borrow_mut().calls.push(HostCall::SetDrawer(open));
    }
    fn add_files(&self, _: &mut Window, _: &mut App) {
        self.0.borrow_mut().calls.push(HostCall::AddFiles);
    }
    fn added(&self, ids: Vec<u64>, _: &mut Window, _: &mut App) {
        self.0.borrow_mut().calls.push(HostCall::Added(ids));
    }
    fn show_menu(&self, menu: Menu, _: &mut Window, _: &mut App, done: MenuDone) {
        let mut log = self.0.borrow_mut();
        log.calls.push(HostCall::Menu(menu));
        log.menu_done = Some(done);
    }
}

/// The harness window's content: the player view at the top, the rest of the window (the
/// drawer's place, when the fixture has it open) left to the root's background.
pub struct Harness {
    pub player: Entity<PlayerView>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.player.clone().cached(StyleRefinement::default().w(px(WIDTH)).h(px(HEIGHT + crate::view::DRAWER_HEIGHT))))
    }
}

/// The view in a harness window, with `recorder` for the player and the outside world.
pub fn build(setup: &Setup, recorder: &Recorder, window: &mut Window, cx: &mut App) -> (AnyView, Entity<PlayerView>) {
    let commands: Rc<dyn Commands> = Rc::new(recorder.clone());
    let host: Rc<dyn PlayerHost> = Rc::new(recorder.clone());
    let models = setup.models.clone();
    let root = setup.root.clone();
    let player = cx.new(|cx| PlayerView::new(&models, commands, host, root, window, cx));
    let harness = cx.new(|_| Harness { player: player.clone() });
    (harness.into(), player)
}

/// Waits until the window is laid out at `size` (CSS px): `Session::open`'s resize lands a moment
/// later, and until then the first frames lay out at the system's default size, so early input
/// would go to the wrong place.
pub async fn wait_for_size(session: &Session, size: (f32, f32), cx: &mut gpui::AsyncApp) {
    let mut settled = 0;
    for _ in 0..200 {
        let now = cx.update(|cx| {
            session.frames(1, cx);
            cx.update_window(session.any(), |_, window, _| window.viewport_size()).ok()
        });
        let at_size = now.is_some_and(|s| (s.width.as_f32() - size.0).abs() < 0.5 && (s.height.as_f32() - size.1).abs() < 0.5);
        // Three times in a row: a late resize can still come after the first.
        settled = if at_size { settled + 1 } else { 0 };
        if settled >= 3 {
            break;
        }
        cx.background_executor().timer(std::time::Duration::from_millis(10)).await;
    }
    cx.update(|cx| {
        // The pointer starts where the user's cursor is: park it on bare background (the window's
        // bottom-left padding) so nothing shows a stray hover.
        session.mouse_move(point(2.0, 264.0), None, Modifiers::default(), cx);
        session.frames(2, cx);
    });
}

/// The text caret hidden, as the reference renders have it (its blink can't be frozen). Call it
/// after the theme is installed (a theme install gives the caret its color back).
pub fn hide_caret(cx: &mut App) {
    if cx.try_global::<gpui_component::Theme>().is_some() {
        // Alpha 0 means "unset" to GPUI Kit (it then uses the text color): almost 0 instead.
        gpui_component::Theme::global_mut(cx).colors.caret = gpui::hsla(0.0, 0.0, 0.0, 0.0001);
    }
}

/// The fixture's forced pseudo-states, installed for the views to read.
pub fn install_forced(fixture: &Fixture, cx: &mut App) {
    let mut states = ForcedStates::default();
    for (selector, forced) in fixture.forced_all() {
        let entry = states.by_selector.entry(selector).or_default();
        *entry = entry.union(forced);
    }
    cx.set_global(states);
}

/// The fixture's `errors` given to the recorder.
pub fn install_errors(fixture: &Fixture, recorder: &Recorder) {
    for (command, message) in &fixture.errors {
        if let Some(message) = message.as_str() {
            recorder.fail(command, message);
        }
    }
}

/// A fixture selector (`#cartridges > .cartridge:nth-child(3) > .cartridge-body`,
/// `#balanceSlider .slider-rail`) as the layout files name the element
/// (`#cartridges>.cartridge[2]>.cartridge-body`).
pub fn layout_name(selector: &str) -> String {
    let mut name = String::new();
    for (i, part) in selector.split(|c: char| c == '>' || c.is_whitespace()).filter(|p| !p.is_empty()).enumerate() {
        if i > 0 {
            name.push('>');
        }
        match part.split_once(":nth-child(").or_else(|| part.split_once(":nth-of-type(")) {
            Some((base, n)) => {
                let n: usize = n.trim_end_matches(')').parse().unwrap_or(1);
                name.push_str(&format!("{base}[{}]", n.saturating_sub(1)));
            }
            None => name.push_str(part),
        }
    }
    name
}

/// Where an action's target is: the element's box, `fx` / `fy` across it (default the middle),
/// plus `dx` / `dy`.
fn target_point(layout: &Value, target: &Value) -> Option<Point<f32>> {
    let (selector, fx, fy, dx, dy) = match target {
        Value::String(selector) => (selector.clone(), 0.5, 0.5, 0.0, 0.0),
        Value::Object(spec) => (
            spec.get("selector")?.as_str()?.to_string(),
            spec.get("fx").and_then(Value::as_f64).unwrap_or(0.5) as f32,
            spec.get("fy").and_then(Value::as_f64).unwrap_or(0.5) as f32,
            spec.get("dx").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            spec.get("dy").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        ),
        _ => return None,
    };
    let name = layout_name(&selector);
    // A bare class (`.transport`) is named by its path in the layout (`body>.app>.player>.transport`).
    let b = element_box(layout, &name, 1.0).or_else(|| {
        let suffix = format!(">{name}");
        let found = layout["elements"].as_array()?.iter().find(|e| e["name"].as_str().is_some_and(|n| n.ends_with(&suffix)))?;
        element_box(layout, found["name"].as_str()?, 1.0)
    })?;
    Some(point(b[0] + fx * b[2] + dx, b[1] + fy * b[3] + dy))
}

fn number_args(text: &str) -> Vec<f64> {
    let inner = text.split_once('(').and_then(|(_, rest)| rest.rsplit_once(')')).map_or("", |(args, _)| args);
    inner.split(',').filter_map(|a| a.trim().parse().ok()).collect()
}

/// What the actions left for the test or capture to know.
#[derive(Default, Debug)]
pub struct Replay {
    /// Actions this crate doesn't interpret (the drawer's).
    pub skipped: Vec<Value>,
    /// The menus shown by `contextmenu` actions.
    pub menus: Vec<Menu>,
}

/// Replays the fixture's actions on the view in `session`.
pub fn run_actions(fixture: &Fixture, variant: &str, session: &Session, player: &Entity<PlayerView>, recorder: &Recorder, cx: &mut App) -> Replay {
    let mut replay = Replay::default();
    let layout = layout(&fixture.window, variant).unwrap_or(Value::Null);
    let window = session.any();
    let mut held: Option<Point<f32>> = None;
    for action in &fixture.actions {
        session.frame(cx);
        if action.get("force").is_some() || action.get("wait").is_some() {
            continue;
        }
        if let Some(code) = action.get("eval").and_then(Value::as_str) {
            let code = code.trim();
            let args = number_args(code);
            let player = player.clone();
            let done = cx
                .update_window(window, |_, window, cx| {
                    player.update(cx, |view, cx| {
                        if code.starts_with("openDialog(") {
                            view.open_cartridge_dialog(args.first().copied().unwrap_or(0.0) as usize, window, cx);
                        } else if code.starts_with("openUrlDialog(") {
                            view.open_url_dialog(window, cx);
                        } else if code.starts_with("openEntryDialog(") {
                            view.open_entry_dialog(args.first().copied().unwrap_or(0.0) as u64, window, cx);
                        } else if let Some(value) = code.strip_prefix("ui.streamURL.value = ") {
                            let text = value.trim().trim_matches(|c| c == '\'' || c == '"').to_string();
                            if let Some(dialog) = view.dialog() {
                                dialog.update(cx, |dialog, cx| dialog.set_url_text(&text, window, cx));
                            }
                        } else if code.starts_with("ui.dialogForm.requestSubmit(") {
                            if let Some(dialog) = view.dialog() {
                                dialog.update(cx, |dialog, cx| dialog.submit(window, cx));
                            }
                        } else if code.starts_with("moveCartridge(") && args.len() == 2 {
                            let (from, to) = (args[0] as usize, args[1] as usize);
                            view.cartridges().update(cx, |cartridges, cx| cartridges.move_cartridge(from, to, cx));
                        } else {
                            return false;
                        }
                        true
                    })
                })
                .unwrap_or(false);
            if !done {
                replay.skipped.push(action.clone());
            }
            continue;
        }
        if let (Some("player-state"), Some(payload)) = (action.get("emit").and_then(Value::as_str), action.get("payload")) {
            if let Ok(state) = serde_json::from_value(payload.clone()) {
                session.models.apply_state(state, cx);
            }
            continue;
        }
        if let Some(target) = action.get("press") {
            if let Some(p) = target_point(&layout, target) {
                session.mouse_move(p, None, Modifiers::default(), cx);
                session.mouse_down(p, MouseButton::Left, 1, Modifiers::default(), cx);
                held = Some(p);
            }
            continue;
        }
        if let Some(target) = action.get("dragTo") {
            let steps = action.get("steps").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
            if let (Some(to), Some(from)) = (target_point(&layout, target), held) {
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let p = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                    session.mouse_move(p, Some(MouseButton::Left), Modifiers::default(), cx);
                    session.frame(cx);
                }
                held = Some(to);
            }
            continue;
        }
        if action.get("release").is_some() {
            if let Some(p) = held.take() {
                session.mouse_up(p, MouseButton::Left, 1, Modifiers::default(), cx);
            }
            continue;
        }
        if let Some(target) = action.get("contextmenu") {
            if let Some(p) = target_point(&layout, target) {
                let before = recorder.calls().len();
                session.mouse_move(p, None, Modifiers::default(), cx);
                session.frame(cx);
                session.mouse_down(p, MouseButton::Right, 1, Modifiers::default(), cx);
                session.frame(cx);
                session.mouse_up(p, MouseButton::Right, 1, Modifiers::default(), cx);
                for call in recorder.calls().into_iter().skip(before) {
                    if let HostCall::Menu(menu) = call {
                        replay.menus.push(menu);
                    }
                }
                // The menu is dismissed.
                recorder.choose(None, cx);
            } else {
                replay.skipped.push(action.clone());
            }
            continue;
        }
        if let Some(key) = action.get("key") {
            let name = key.get("key").and_then(Value::as_str).unwrap_or("");
            let gpui_key = match name {
                "Escape" => "escape",
                "Enter" => "enter",
                "Tab" => "tab",
                _ => "",
            };
            if !gpui_key.is_empty() {
                if let Ok(stroke) = Keystroke::parse(gpui_key) {
                    let _ = cx.update_window(window, |_, window, cx| {
                        window.dispatch_keystroke(stroke, cx);
                    });
                }
            }
            continue;
        }
        replay.skipped.push(action.clone());
    }
    session.frame(cx);
    replay
}

/// The recorded menus of a layout file (`menus`), as `(text, enabled, has accelerator)` rows
/// (None for a separator).
pub fn reference_menus(fixture: &Fixture, variant: &str) -> Vec<Vec<Option<(String, bool, bool)>>> {
    let Some(layout) = layout(&fixture.window, variant) else { return Vec::new() };
    layout["menus"]
        .as_array()
        .map(|menus| {
            menus
                .iter()
                .map(|menu| {
                    menu.as_array()
                        .map(|items| {
                            items
                                .iter()
                                .map(|item| {
                                    if item.get("item").and_then(Value::as_str) == Some("Separator") {
                                        None
                                    } else {
                                        Some((
                                            item["text"].as_str().unwrap_or("").to_string(),
                                            item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                                            item.get("accelerator").and_then(Value::as_str).is_some(),
                                        ))
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}
