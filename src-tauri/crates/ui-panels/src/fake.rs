//! A recording [`PanelServices`] for captures and tests: it logs every call, answers from a
//! reference fixture (its `errors`, `info`, `now`, `version`), keeps the models current the way
//! the app would (shortcuts after a change or a resume, the helper after a change), and never
//! touches the OS: menus are recorded and answered by the test ([`FakeServices::answer_menu`]),
//! key captures are fed by the test ([`FakeServices::press`]), the clipboard is a list.

use crate::services::{InfoRequest, MenuDone, PanelServices};
use gpui::{App, Window};
use qust_shell::menu::{Menu, Position};
use qust_shell::recorder::Recorded;
use qust_ui::fixture::Fixture;
use qust_ui::{Background, Binding, Models, StreamInfo};
use serde_json::Value;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type KeySink = Rc<RefCell<Option<Box<dyn FnMut(Recorded)>>>>;

pub struct FakeServices {
    models: Models,
    /// Every call, in order ("save_setting visualizer-colors \"white\"", "pause_shortcuts true", …).
    pub log: RefCell<Vec<String>>,
    pub now: Cell<u64>,
    pub version: RefCell<String>,
    /// Commands that fail, with their message (a fixture's `errors`: `set_shortcut`,
    /// `set_background`).
    pub errors: RefCell<HashMap<String, String>>,
    /// What `stream_info` answers.
    pub info: RefCell<StreamInfo>,
    /// Whether `capture_keys` gives a native capture (false: the view falls back to GPUI keys).
    pub native_capture: Cell<bool>,
    keys: KeySink,
    /// Menus asked for, with where.
    pub menus: RefCell<Vec<(Menu, Position)>>,
    pending_menu: RefCell<Option<MenuDone>>,
    pub clipboard: RefCell<Vec<String>>,
    pub closed: Cell<u32>,
    paused: Cell<bool>,
}

struct CaptureGuard(KeySink);

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.0.borrow_mut().take();
    }
}

impl FakeServices {
    pub fn new(models: &Models) -> Rc<FakeServices> {
        Rc::new(FakeServices {
            models: models.clone(),
            log: RefCell::new(Vec::new()),
            now: Cell::new(1_794_400_000),
            version: RefCell::new("1.1.5".into()),
            errors: RefCell::new(HashMap::new()),
            info: RefCell::new(StreamInfo::default()),
            native_capture: Cell::new(true),
            keys: Rc::new(RefCell::new(None)),
            menus: RefCell::new(Vec::new()),
            pending_menu: RefCell::new(None),
            clipboard: RefCell::new(Vec::new()),
            closed: Cell::new(0),
            paused: Cell::new(false),
        })
    }

    /// Answers like the page's stub did for `fixture`.
    pub fn for_fixture(models: &Models, fixture: &Fixture) -> Rc<FakeServices> {
        let fake = FakeServices::new(models);
        if let Some(now) = fixture.now {
            fake.now.set(now);
        }
        if let Some(version) = &fixture.version {
            *fake.version.borrow_mut() = version.clone();
        }
        for (command, message) in &fixture.errors {
            fake.errors.borrow_mut().insert(command.clone(), message.as_str().unwrap_or_default().to_string());
        }
        if let Some(info) = &fixture.info {
            *fake.info.borrow_mut() = info.clone();
        }
        fake
    }

    fn record(&self, entry: String) {
        self.log.borrow_mut().push(entry);
    }

    /// The calls logged so far.
    pub fn calls(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    /// Whether a key capture is running.
    pub fn capturing(&self) -> bool {
        self.keys.borrow().is_some()
    }

    /// A key the native capture saw (the view handles it on its next turn of the executor).
    pub fn press(&self, recorded: Recorded) -> bool {
        match self.keys.borrow_mut().as_mut() {
            Some(on_key) => {
                on_key(recorded);
                true
            }
            None => false,
        }
    }

    /// Answers the menu shown last (`Some(id)` chosen, None dismissed).
    pub fn answer_menu(&self, chosen: Option<u32>, cx: &mut App) -> bool {
        let done = self.pending_menu.borrow_mut().take();
        match done {
            Some(done) => {
                done(chosen, cx);
                true
            }
            None => false,
        }
    }

    pub fn paused(&self) -> bool {
        self.paused.get()
    }

    fn update_binding(&self, action: &str, change: impl FnOnce(&mut Binding), cx: &mut App) {
        let mut shortcuts = self.models.shortcuts.read(cx).shortcuts.clone();
        let binding = match action {
            "play" => &mut shortcuts.play,
            "stop" => &mut shortcuts.stop,
            _ => &mut shortcuts.toggle,
        };
        change(binding);
        self.models.apply_shortcuts(shortcuts, cx);
    }
}

fn default_accelerator(action: &str) -> &'static str {
    match action {
        "play" => "Ctrl+Alt+Insert",
        "stop" => "Ctrl+Alt+KeyH",
        _ => "",
    }
}

impl PanelServices for FakeServices {
    fn now(&self) -> u64 {
        self.now.get()
    }

    fn version(&self) -> String {
        self.version.borrow().clone()
    }

    fn save_setting(&self, key: &str, value: Value, _: &mut App) {
        self.record(format!("save_setting {key} {value}"));
    }

    fn broadcast_setting(&self, key: &str, value: Value, cx: &mut App) {
        self.record(format!("broadcast_setting {key} {value}"));
        self.models.apply_setting(key, &value, cx);
    }

    fn set_theme(&self, theme: &str, _: &mut App) {
        self.record(format!("set_theme {theme}"));
    }

    fn pause_shortcuts(&self, paused: bool, cx: &mut App) {
        self.record(format!("pause_shortcuts {paused}"));
        self.paused.set(paused);
        if !paused {
            // Resuming registers every assigned shortcut again (the fake OS owns none).
            let mut shortcuts = self.models.shortcuts.read(cx).shortcuts.clone();
            for binding in [&mut shortcuts.play, &mut shortcuts.stop, &mut shortcuts.toggle] {
                binding.registered = binding.assigned();
            }
            self.models.apply_shortcuts(shortcuts, cx);
        }
    }

    fn set_shortcut(&self, action: &str, accelerator: &str, cx: &mut App) -> Result<(), String> {
        self.record(format!("set_shortcut {action} {accelerator}"));
        if let Some(error) = self.errors.borrow().get("set_shortcut") {
            return Err(error.clone());
        }
        let accelerator = if accelerator.is_empty() { default_accelerator(action).to_string() } else { accelerator.to_string() };
        self.update_binding(action, |binding| *binding = Binding { accelerator, registered: false }, cx);
        Ok(())
    }

    fn capture_keys(&self, _: &Window, on_key: Box<dyn FnMut(Recorded)>) -> Option<Box<dyn Any>> {
        self.record("capture_keys".into());
        if !self.native_capture.get() {
            return None;
        }
        *self.keys.borrow_mut() = Some(on_key);
        Some(Box::new(CaptureGuard(self.keys.clone())))
    }

    fn set_background(&self, enabled: bool, cx: &mut App) -> Result<Background, String> {
        self.record(format!("set_background {enabled}"));
        if let Some(error) = self.errors.borrow().get("set_background") {
            return Err(error.clone());
        }
        let background = Background { available: true, enabled, starts_with_windows: enabled };
        self.models.apply_background(background.clone(), cx);
        Ok(background)
    }

    fn check_for_updates(&self, _: &mut App) {
        self.record("check_for_updates".into());
    }

    fn install_update(&self, _: &mut App) {
        self.record("install_update".into());
    }

    fn popup_menu(&self, _: &Window, menu: Menu, position: Position, done: MenuDone, _: &mut App) {
        self.record(format!("popup_menu {position:?}"));
        self.menus.borrow_mut().push((menu, position));
        *self.pending_menu.borrow_mut() = Some(done);
    }

    fn copy_text(&self, text: String, _: &mut App) {
        self.record(format!("copy_text {text}"));
        self.clipboard.borrow_mut().push(text);
    }

    fn stream_info(&self, url: String, _: &mut App) -> InfoRequest {
        self.record(format!("stream_info {url}"));
        let info = self.info.borrow().clone();
        Box::pin(async move { Some(info) })
    }

    fn close_window(&self, _: &mut Window, _: &mut App) {
        self.record("close_window".into());
        self.closed.set(self.closed.get() + 1);
    }
}
