//! Captures and tests (feature `capture`): recording stand-ins for the player and the host, the
//! drawer mounted the way the app should mount it (a live layer of `WindowRoot` over a slot at
//! y = 267), and an interpreter for the reference fixtures' actions that drives the drawer
//! through real dispatched input wherever the page's harness used real input (presses, drags,
//! right-clicks, file drags), and through the drawer's own functions where the page's harness
//! called page functions (`select(…)`, `showDropIndex(…)`, `scrollTop`).
//!
//! Nothing here opens OS UI: the recording host keeps the menu it was asked to show and the
//! callback, and a test may "choose" an item with [`Recorder::choose`].

use crate::drawer::{Drawer, HEIGHT, WIDTH};
use crate::host::{DrawerHost, Panel, Pending, PlaylistCommands};
use crate::list::{Item, Rect};
use gpui::{
    div, point, prelude::*, px, AnyView, App, Context, Entity, ExternalPaths, FileDropEvent, Modifiers, MouseButton, PlatformInput, Point, Window,
};
use qust_shell::menu::Menu;
use qust_ui::capture::{Job, Session, Setup};
use qust_ui::fixture::Fixture;
use qust_ui::{portal_slot, WindowRoot};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// A command the drawer sent to the player.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    PlayCartridge(usize),
    PlayEntry(u64),
    Remove(Vec<u64>),
    Move(Vec<u64>, usize),
    AddPaths(Vec<PathBuf>, Option<usize>),
}

/// A call the drawer made on its host.
#[derive(Clone, Debug, PartialEq)]
pub enum HostCall {
    EditCartridge(usize),
    EditEntry(u64),
    AddUrl,
    AddFiles,
    OpenPanel(Panel),
    ShowMenu(Menu),
    OpenDrawer,
}

type Chosen = Box<dyn FnOnce(Option<u32>, &mut App)>;

/// Everything the drawer sent, and the answers it will get.
#[derive(Default)]
pub struct Recorder {
    pub commands: RefCell<Vec<Command>>,
    pub calls: RefCell<Vec<HostCall>>,
    pub dialog_open: Cell<bool>,
    /// What the next `add_paths` answers (the ids added), in order.
    pub add_answers: RefCell<VecDeque<Vec<u64>>>,
    chosen: RefCell<Option<Chosen>>,
    menu: RefCell<Option<Menu>>,
}

impl Recorder {
    pub fn new() -> Rc<Recorder> {
        Rc::new(Recorder::default())
    }

    pub fn take_commands(&self) -> Vec<Command> {
        std::mem::take(&mut self.commands.borrow_mut())
    }

    pub fn take_calls(&self) -> Vec<HostCall> {
        std::mem::take(&mut self.calls.borrow_mut())
    }

    /// The last menu shown, if any.
    pub fn last_menu(&self) -> Option<Menu> {
        self.menu.borrow().clone()
    }

    /// Chooses an item of the pending menu by its text (None: dismissed). False when no menu
    /// is pending or the text isn't in it.
    pub fn choose(&self, text: Option<&str>, cx: &mut App) -> bool {
        let Some(chosen) = self.chosen.borrow_mut().take() else { return false };
        let id = match text {
            None => None,
            Some(text) => {
                let Some(menu) = self.last_menu() else { return false };
                let found = menu.items.iter().find_map(|item| match item {
                    qust_shell::menu::Item::Action { id, text: t, .. } if t == text => Some(*id),
                    _ => None,
                });
                if found.is_none() {
                    return false;
                }
                found
            }
        };
        chosen(id, cx);
        true
    }
}

/// The player stand-in.
pub struct RecordingCommands(pub Rc<Recorder>);

impl PlaylistCommands for RecordingCommands {
    fn play_cartridge(&self, slot: usize) {
        self.0.commands.borrow_mut().push(Command::PlayCartridge(slot));
    }

    fn play_entry(&self, id: u64) {
        self.0.commands.borrow_mut().push(Command::PlayEntry(id));
    }

    fn remove_entries(&self, ids: Vec<u64>) {
        self.0.commands.borrow_mut().push(Command::Remove(ids));
    }

    fn move_entries(&self, ids: Vec<u64>, index: usize) {
        self.0.commands.borrow_mut().push(Command::Move(ids, index));
    }

    fn add_paths(&self, paths: Vec<PathBuf>, index: Option<usize>) -> Pending<Vec<u64>> {
        self.0.commands.borrow_mut().push(Command::AddPaths(paths, index));
        let answer = self.0.add_answers.borrow_mut().pop_front();
        Box::pin(async move { answer })
    }
}

/// The host stand-in: records; menus are kept, never shown.
pub struct RecordingHost(pub Rc<Recorder>);

impl DrawerHost for RecordingHost {
    fn edit_cartridge(&self, slot: usize, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::EditCartridge(slot));
    }

    fn edit_entry(&self, id: u64, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::EditEntry(id));
    }

    fn add_url(&self, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::AddUrl);
    }

    fn add_files(&self, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::AddFiles);
    }

    fn open_panel(&self, panel: Panel, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::OpenPanel(panel));
    }

    fn show_menu(&self, menu: Menu, chosen: Box<dyn FnOnce(Option<u32>, &mut App)>, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::ShowMenu(menu.clone()));
        *self.0.menu.borrow_mut() = Some(menu);
        *self.0.chosen.borrow_mut() = Some(chosen);
    }

    fn open_drawer(&self, _: &mut Window, _: &mut App) {
        self.0.calls.borrow_mut().push(HostCall::OpenDrawer);
    }

    fn dialog_open(&self, _: &App) -> bool {
        self.0.dialog_open.get()
    }
}

/// The window's content as the app should compose it: the player's 267 px (empty here) with a
/// slot for the drawer under it; the drawer itself is a live layer of the root.
pub struct Stage {
    root: gpui::WeakEntity<WindowRoot>,
}

impl Stage {
    pub fn new(root: gpui::WeakEntity<WindowRoot>) -> Stage {
        Stage { root }
    }
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(div().absolute().left_0().top(px(qust_fluent::tokens::PLAYER_HEIGHT)).w(px(WIDTH)).h(px(HEIGHT)).child(portal_slot(self.root.clone(), "drawer")))
    }
}

/// Builds the stage and the drawer for a capture or test window. `open`: the fixture's drawer.
pub fn mount(setup: &Setup, recorder: &Rc<Recorder>, cx: &mut App) -> (AnyView, Entity<Drawer>, Option<Entity<crate::EqualizerLayer>>) {
    let open = setup.job.fixture.drawer;
    let commands: Rc<dyn PlaylistCommands> = Rc::new(RecordingCommands(recorder.clone()));
    let host: Rc<dyn DrawerHost> = Rc::new(RecordingHost(recorder.clone()));
    let drawer = cx.new(|cx| Drawer::new(setup.models, commands, host, open, cx));
    let layer = setup.root.upgrade().map(|root| crate::drawer::mount(&drawer, &root, cx));
    let stage = cx.new(|_| Stage { root: setup.root.clone() });
    (stage.into(), drawer, layer)
}

/// Opens a window for `job` with a mounted drawer (drawn twice so the slot is known).
pub fn open(job: &Job, recorder: &Rc<Recorder>, cx: &mut App) -> Result<(Session, Entity<Drawer>), String> {
    open_with_layer(job, recorder, cx).map(|(session, drawer, _)| (session, drawer))
}

/// [`open`], with the equalizer layer.
pub fn open_with_layer(job: &Job, recorder: &Rc<Recorder>, cx: &mut App) -> Result<(Session, Entity<Drawer>, Entity<crate::EqualizerLayer>), String> {
    type Mounted = (Entity<Drawer>, Option<Entity<crate::EqualizerLayer>>);
    let slot: Rc<RefCell<Option<Mounted>>> = Rc::new(RefCell::new(None));
    let out = slot.clone();
    let recorder = recorder.clone();
    let session = Session::open(job, cx, move |setup, _, cx| {
        let (view, drawer, layer) = mount(setup, &recorder, cx);
        *out.borrow_mut() = Some((drawer, layer));
        view
    })?;
    session.frames(3, cx);
    let (drawer, layer) = slot.borrow_mut().take().ok_or("no drawer")?;
    Ok((session, drawer, layer.ok_or("no layer")?))
}

/// Where the fixture's pointer is (the page harness's `mouse`).
#[derive(Default)]
pub struct Pointer {
    pub position: Point<f32>,
    pub pressed: bool,
}

/// What the interpreter couldn't do (actions that belong to other windows' parts).
#[derive(Debug, Default)]
pub struct Applied {
    pub skipped: Vec<String>,
}

fn center(rect: Rect, fx: f32, fy: f32) -> Point<f32> {
    point(rect.left + rect.width * fx, rect.top + rect.height * fy)
}

/// An element's box from a fixture selector (`#playlist > .row:nth-of-type(N)`,
/// `.playlist-empty`, `#playlist`).
pub fn element_box(selector: &str, drawer: &Entity<Drawer>, cx: &App) -> Option<Rect> {
    let drawer = drawer.read(cx);
    let selector = selector.trim();
    if selector == ".playlist-empty" || selector == "#playlist > .playlist-empty" {
        return drawer.empty_bounds();
    }
    if selector == "#playlist" {
        return Some(drawer.list_rect());
    }
    let n: usize = selector.strip_prefix("#playlist > .row:nth-of-type(")?.strip_suffix(')')?.parse().ok()?;
    // nth-of-type counts the list's divs: the pinned rows, the divider, then the entries.
    let pinned = drawer.rows().pinned().len();
    let row = if pinned > 0 && n > pinned + 1 {
        n - 2
    } else if pinned > 0 && n == pinned + 1 {
        return None;
    } else {
        n - 1
    };
    drawer.row_bounds(drawer.rows().item(row)?)
}

fn item_of_selector(selector: &str, drawer: &Entity<Drawer>, cx: &App) -> Option<Item> {
    let n: usize = selector.trim().strip_prefix("#playlist > .row:nth-of-type(")?.strip_suffix(')')?.parse().ok()?;
    let drawer = drawer.read(cx);
    let pinned = drawer.rows().pinned().len();
    let row = if pinned > 0 && n > pinned + 1 { n - 2 } else { n - 1 };
    drawer.rows().item(row)
}

// `select(3)`, `select(5, { extend: true })`, `select('slot1', { toggle: true })`.
fn parse_select(text: &str) -> Option<(Item, bool, bool)> {
    let inner = text.trim().strip_prefix("select(")?.strip_suffix(')')?;
    let (target, options) = match inner.split_once(',') {
        Some((target, options)) => (target.trim(), options),
        None => (inner.trim(), ""),
    };
    let item = match target.trim_matches('\'').trim_matches('"').strip_prefix("slot") {
        Some(slot) => Item::Cartridge(slot.parse().ok()?),
        None => Item::Entry(target.parse().ok()?),
    };
    Some((item, options.contains("toggle: true"), options.contains("extend: true")))
}

/// Dispatches a platform file-drop event into the window, as the OS would.
pub fn file_drop(session: &Session, event: FileDropEvent, cx: &mut App) {
    let _ = cx.update_window(session.any(), |_, window, cx| {
        window.dispatch_event(PlatformInput::FileDrop(event), cx);
    });
    session.frame(cx);
}

pub fn external_paths(paths: &[&str]) -> ExternalPaths {
    ExternalPaths(paths.iter().map(PathBuf::from).collect())
}

/// Runs the fixture's actions on the mounted drawer.
pub fn apply_actions(fixture: &Fixture, session: &Session, drawer: &Entity<Drawer>, cx: &mut App) -> Applied {
    let mut applied = Applied::default();
    let mut pointer = Pointer::default();
    let mut drop_y: Option<f32> = None;
    // The page harness's pointer starts at (0, 0); GPUI's starts wherever the real cursor was
    // when the window opened (rows under it would show hover).
    session.mouse_move(pointer.position, None, Modifiers::default(), cx);
    for action in &fixture.actions {
        session.frame(cx);
        if let Some(text) = action.get("eval").and_then(Value::as_str) {
            if let Some((item, toggle, extend)) = parse_select(text) {
                drawer.update(cx, |d, cx| d.select(item, toggle, extend, cx));
            } else if let Some(arg) = text.strip_prefix("showDropIndex(").and_then(|t| t.strip_suffix(')')) {
                let index = if arg.trim() == "entries.length" { drawer.read(cx).rows().entries() } else { arg.trim().parse().unwrap_or(0) };
                drawer.update(cx, |d, cx| d.show_drop_index(Some(index), cx));
            } else if text.contains("window.__dropY") && text.contains("rowOf.get(3)") {
                drop_y = drawer.read(cx).row_bounds(Item::Entry(3)).map(|r| r.top + 2.0);
            } else if text.contains("__REF__.dragDrop") {
                let y = drop_y.unwrap_or(0.0);
                file_drop(session, FileDropEvent::Entered { position: point(px(430.0), px(y)), paths: external_paths(&["C:\\a.mp3"]) }, cx);
            } else {
                applied.skipped.push(text.to_string());
            }
        } else if let Some(spec) = action.get("dragDrop") {
            let position = point(px(spec["position"]["x"].as_f64().unwrap_or(0.0) as f32), px(spec["position"]["y"].as_f64().unwrap_or(0.0) as f32));
            let paths: Vec<&str> = spec["paths"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let event = match spec["type"].as_str() {
                Some("over") | Some("enter") => FileDropEvent::Entered { position, paths: external_paths(&paths) },
                Some("drop") => FileDropEvent::Submit { position },
                _ => FileDropEvent::Exited,
            };
            file_drop(session, event, cx);
        } else if let Some(force) = action.get("force").and_then(Value::as_object) {
            let mut items = Vec::new();
            for (selector, states) in force {
                let hover = states.as_array().is_some_and(|s| s.iter().any(|v| v.as_str() == Some("hover")));
                match item_of_selector(selector, drawer, cx) {
                    Some(item) if hover => items.push(item),
                    _ => applied.skipped.push(format!("force {selector}")),
                }
            }
            drawer.update(cx, |d, cx| d.force_hover(items, cx));
        } else if let Some(top) = action.get("top").and_then(Value::as_f64) {
            drawer.update(cx, |d, cx| d.set_scroll_top(top as f32, cx));
        } else if let Some(target) = action.get("press").or_else(|| action.get("dragTo")).or_else(|| action.get("contextmenu")) {
            let (selector, fx, fy) = match target {
                Value::String(s) => (s.clone(), 0.5, 0.5),
                spec => (spec["selector"].as_str().unwrap_or("").to_string(), spec["fx"].as_f64().unwrap_or(0.5) as f32, spec["fy"].as_f64().unwrap_or(0.5) as f32),
            };
            let Some(rect) = element_box(&selector, drawer, cx) else {
                applied.skipped.push(format!("no box for {selector}"));
                continue;
            };
            let to = center(rect, fx, fy);
            let steps = action["steps"].as_u64().unwrap_or(1).max(1);
            let from = pointer.position;
            let button = pointer.pressed.then_some(MouseButton::Left);
            for step in 1..=steps {
                let t = step as f32 / steps as f32;
                pointer.position = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                session.mouse_move(pointer.position, button, Modifiers::default(), cx);
                session.frame(cx);
            }
            if action.get("press").is_some() {
                pointer.pressed = true;
                session.mouse_down(pointer.position, MouseButton::Left, 1, Modifiers::default(), cx);
            }
            if action.get("contextmenu").is_some() {
                session.mouse_down(pointer.position, MouseButton::Right, 1, Modifiers::default(), cx);
                session.mouse_up(pointer.position, MouseButton::Right, 1, Modifiers::default(), cx);
            }
        } else {
            applied.skipped.push(action.to_string());
        }
    }
    session.frames(2, cx);
    applied
}

/// Runs the actions on an opened window and captures it to `job.out`, then closes it.
pub fn finish_capture(job: &Job, session: &Session, drawer: &Entity<Drawer>, cx: &mut App) -> Result<(PathBuf, f32, Applied), String> {
    let applied = apply_actions(&job.fixture, session, drawer, cx);
    session.frames(2, cx);
    let scale = session.scale_factor(cx);
    let saved = session.capture(&job.out, cx).map(|_| (job.out.clone(), scale, applied));
    // A file drag left hovering is app-wide in GPUI (it would turn hover styles off in the
    // next window): end it as the OS would.
    file_drop(session, FileDropEvent::Exited, cx);
    session.close(cx);
    saved
}

/// Captures every job (one app run). Each window is opened, given a moment (the client size
/// applies, the SVG icons load on first use), then driven and captured.
pub fn capture_all(jobs: Vec<Job>) -> Vec<Result<(PathBuf, f32, Applied), String>> {
    let results = Rc::new(RefCell::new(Vec::new()));
    let out = results.clone();
    qust_ui::capture::run(move |cx| {
        cx.spawn(async move |cx| {
            for job in jobs {
                let recorder = Recorder::new();
                let opened = cx.update(|cx| open(&job, &recorder, cx));
                let result = match opened {
                    Ok((session, drawer)) => {
                        cx.background_executor().timer(Duration::from_millis(60)).await;
                        cx.update(|cx| {
                            session.frames(2, cx);
                            finish_capture(&job, &session, &drawer, cx)
                        })
                    }
                    Err(error) => Err(error),
                };
                out.borrow_mut().push(result);
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let results = results.borrow_mut().drain(..).collect();
    results
}
