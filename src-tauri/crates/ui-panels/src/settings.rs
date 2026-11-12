//! The Settings window (settings.html, settings.js): Appearance, Playback, Global shortcuts,
//! Updates and About, every card in the page's order with its texts.
//!
//! - The three combos open a native menu under the button (its bottom-left + 2 px), one item per
//!   choice with the current one checked. Theme → `set_theme`; Visualizer colors → saved
//!   (`visualizer-colors`) and broadcast (`colors`); Reconnect → saved (`reconnect-minutes`) and
//!   broadcast (`reconnectMinutes`). The colors combo follows `settings-changed` from the player
//!   (the analyzer click), through `models.settings`.
//! - Play on launch → saved (`autoplay`) and broadcast.
//! - The shortcuts record a new key combination (see [`SettingsView::start_recording`]).
//! - The background helper and the updater act through the services; their state comes from
//!   `models.background` / `models.update`, and a failed helper change shows its error until the
//!   next change, as the page did.
//! - Esc closes the window, unless a shortcut is being recorded (then it cancels the recording).

use crate::services::PanelServices;
use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, App, AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, Modifiers, ParentElement, Pixels,
    Render, ScrollHandle, Styled, Subscription, Window,
};
use qust_fluent::focus::{tab_stop, FocusNext, FocusPrev};
use qust_fluent::scrollbar::scroll_area;
use qust_fluent::section::{setting_card, setting_icon, setting_text, settings_group};
use qust_fluent::{shortcut_keys, theme, Button, ButtonKind, Combo, Forced, Icon, ShortcutBox, ShortcutDisplay, Switch, TextStyles, Theme};
use qust_shell::menu::{Menu, Position};
use qust_shell::recorder::{record_gpui, Recorded};
use qust_ui::{format, Background, Binding, Models, UpdateStatus, WindowRoot};
use serde_json::json;
use std::any::Any;
use std::rc::Rc;

/// The client size the app opens the window at (main.rs: 520 × 850). The content is taller
/// (931 px): it scrolls.
pub const SIZE: (f32, f32) = (520.0, 850.0);
/// The window title.
pub const TITLE: &str = "Qust settings";

pub const THEMES: [(&str, &str); 3] = [("system", "Use system setting"), ("light", "Light"), ("dark", "Dark")];
pub const COLORS: [(&str, &str); 4] = [("classic", "Classic"), ("white", "White"), ("accent", "Accent color"), ("off", "Off")];
/// In menu order: settings.js listed `{ 1: …, 5: …, 15: …, 30: …, 60: …, 0: … }`, and JavaScript
/// enumerates integer keys in ascending order, so "Until stopped" came first.
pub const RECONNECT: [(f64, &str); 6] = [(0.0, "Until stopped"), (1.0, "1 minute"), (5.0, "5 minutes"), (15.0, "15 minutes"), (30.0, "30 minutes"), (60.0, "1 hour")];

/// The shortcuts: (action, title, hint).
pub const SHORTCUTS: [(&str, &str, &str); 3] = [
    ("play", "Play", "Works even when Qust isn't focused"),
    ("stop", "Stop", "Works even when Qust isn't focused"),
    ("toggle", "Play / Stop / Start", "Plays or stops; opens Qust and plays when it's closed. One key for a Stream Deck button."),
];

pub const RECORDING_HINT: &str = "Esc to cancel, Backspace to reset";
pub const UNAVAILABLE_HINT: &str = "Another app is using this shortcut. Pick a different one.";
pub const BACKGROUND_HINT: &str = "Starts with Windows in the tray, so shortcuts work while Qust is closed";
pub const BACKGROUND_UNAVAILABLE: &str = "Works in the installed version of Qust.";

/// The three combos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboKind {
    Theme,
    Colors,
    Reconnect,
}

/// What a shortcut card shows: the box and the status line under the title.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutRow {
    pub display: ShortcutDisplay,
    pub status: String,
    pub error: bool,
}

/// `renderShortcuts` for one action.
pub fn shortcut_row(binding: &Binding, hint: &str, recording: bool, error: Option<&str>) -> ShortcutRow {
    let assigned = binding.assigned();
    let display = if recording {
        ShortcutDisplay::Recording
    } else if assigned {
        ShortcutDisplay::Keys { keys: shortcut_keys(&binding.accelerator), unavailable: !binding.registered }
    } else {
        ShortcutDisplay::Unassigned
    };
    let (status, error) = if recording {
        (RECORDING_HINT.to_string(), false)
    } else if let Some(error) = error {
        (error.to_string(), true)
    } else if assigned && !binding.registered {
        (UNAVAILABLE_HINT.to_string(), true)
    } else {
        (hint.to_string(), false)
    };
    ShortcutRow { display, status, error }
}

/// What the updates card shows (`renderUpdate`).
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateRow {
    pub text: String,
    pub error: bool,
    pub label: &'static str,
    pub busy: bool,
    pub accent: bool,
}

/// `renderUpdate`, at `now` (seconds).
pub fn update_row(update: &UpdateStatus, now: u64) -> UpdateRow {
    let checked = match update.last_check.filter(|t| *t != 0) {
        Some(t) => format!("Last checked {}.", format::ago(t, now)),
        None => "Not checked yet.".to_string(),
    };
    // JavaScript prints a missing value as "null" in a template string.
    let version = update.version.clone().unwrap_or_else(|| "null".into());
    let (mut text, mut label, mut busy, mut error) = (checked.clone(), "Check for updates", false, false);
    if !update.installed {
        text = "Updates work in the installed version of Qust.".into();
        busy = true;
    } else {
        match update.status.as_str() {
            "checking" => {
                text = "Checking for updates…".into();
                busy = true;
            }
            "downloading" => {
                text = format!("Downloading version {version}… {}%", update.progress.unwrap_or(0));
                busy = true;
            }
            "ready" => {
                text = format!("Version {version} is ready to install.");
                label = "Restart and update";
            }
            "installing" => {
                text = "Installing…".into();
                busy = true;
            }
            "error" => {
                text = format!("Couldn't update: {}", update.error.clone().unwrap_or_else(|| "null".into()));
                label = "Try again";
                error = true;
            }
            "current" => text = format!("You're up to date. {checked}"),
            _ => {}
        }
    }
    UpdateRow { text, error, label, busy, accent: update.status == "ready" }
}

struct Focus {
    root: FocusHandle,
    theme: FocusHandle,
    colors: FocusHandle,
    autoplay: FocusHandle,
    reconnect: FocusHandle,
    background: FocusHandle,
    shortcuts: [FocusHandle; 3],
    update: FocusHandle,
    auto: FocusHandle,
}

pub struct SettingsView {
    models: Models,
    services: Rc<dyn PanelServices>,
    // The page's `settings`: read once, then changed here (and the colors from the player).
    theme: String,
    colors: String,
    autoplay: bool,
    reconnect: f64,
    recording: Option<usize>,
    // Which recording a captured key belongs to (a late key of an earlier one is dropped).
    generation: u64,
    capture: Option<Box<dyn Any>>,
    errors: [Option<String>; 3],
    // The helper's state after a change made here, with its error; None: the model's.
    background: Option<(Background, Option<String>)>,
    // "Check automatically" switched here, until the next update status.
    auto: Option<bool>,
    forced: Vec<(String, Forced)>,
    focus: Focus,
    scroll: ScrollHandle,
    overflow: bool,
    _subscriptions: Vec<Subscription>,
}

fn valid_colors(colors: &str) -> bool {
    COLORS.iter().any(|(value, _)| *value == colors)
}

impl SettingsView {
    pub fn new(models: &Models, services: Rc<dyn PanelServices>, window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        let settings = models.settings.read(cx).clone();
        let subscriptions = vec![
            // Also changed from the player (clicking the analyzer cycles its colors).
            cx.observe(&models.settings, |view: &mut SettingsView, settings, cx| {
                let colors = settings.read(cx).colors.clone();
                if colors != view.colors && valid_colors(&colors) {
                    view.colors = colors;
                    cx.notify();
                }
            }),
            cx.observe(&models.shortcuts, |_, _, cx| cx.notify()),
            cx.observe(&models.update, |view: &mut SettingsView, _, cx| {
                view.auto = None;
                cx.notify();
            }),
            cx.observe(&models.background, |view: &mut SettingsView, _, cx| {
                if view.background.as_ref().is_some_and(|(_, error)| error.is_none()) {
                    view.background = None;
                }
                cx.notify();
            }),
            // Losing the window's focus ends a recording (the native capture reports it too).
            cx.observe_window_activation(window, |view, window, cx| {
                if !window.is_window_active() && view.recording.is_some() {
                    view.finish(None, cx);
                }
            }),
            // Closing mid-recording must not leave the shortcuts paused.
            cx.on_release(|view: &mut SettingsView, cx| {
                if view.recording.take().is_some() {
                    view.capture = None;
                    view.services.pause_shortcuts(false, cx);
                }
            }),
        ];
        let focus = Focus {
            root: cx.focus_handle(),
            theme: tab_stop(cx),
            colors: tab_stop(cx),
            autoplay: tab_stop(cx),
            reconnect: tab_stop(cx),
            background: tab_stop(cx),
            shortcuts: [tab_stop(cx), tab_stop(cx), tab_stop(cx)],
            update: tab_stop(cx),
            auto: tab_stop(cx),
        };
        // Keys reach the view only along the focus path: the window starts with the view focused.
        focus.root.focus(window, cx);
        let reconnect = RECONNECT.iter().find(|(minutes, _)| *minutes == settings.reconnect_minutes).map_or(5.0, |(minutes, _)| *minutes);
        SettingsView {
            models: models.clone(),
            services,
            theme: settings.theme.clone(),
            colors: if valid_colors(&settings.colors) { settings.colors.clone() } else { "classic".into() },
            autoplay: settings.autoplay,
            reconnect,
            recording: None,
            generation: 0,
            capture: None,
            errors: [None, None, None],
            background: None,
            auto: None,
            forced: Vec::new(),
            focus,
            scroll: ScrollHandle::new(),
            // 931 px of content in 850: it starts with the scrollbar (corrected after layout).
            overflow: true,
            _subscriptions: subscriptions,
        }
    }

    /// Forces pseudo-states on the element a reference fixture names (`#themeCombo`,
    /// `[data-action="stop"]`, …): captures only.
    pub fn force(&mut self, selector: &str, forced: Forced, cx: &mut Context<Self>) {
        self.forced.push((selector.to_string(), forced));
        cx.notify();
    }

    fn forced(&self, selectors: &[&str]) -> Forced {
        self.forced.iter().filter(|(s, _)| selectors.contains(&s.as_str())).fold(Forced::NONE, |all, (_, f)| all.union(*f))
    }

    /// The scroll position (the reference's `scroll` action).
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// The action being recorded ("play", "stop", "toggle").
    pub fn recording(&self) -> Option<&'static str> {
        self.recording.map(|i| SHORTCUTS[i].0)
    }

    /// What shortcut card `index` shows now (tests).
    pub fn shortcut_row(&self, index: usize, cx: &App) -> ShortcutRow {
        let shortcuts = &self.models.shortcuts.read(cx).shortcuts;
        let binding = [&shortcuts.play, &shortcuts.stop, &shortcuts.toggle][index.min(2)];
        shortcut_row(binding, SHORTCUTS[index.min(2)].2, self.recording == Some(index), self.errors[index.min(2)].as_deref())
    }

    /// The helper card now: (toggle on, available, description, error) (tests).
    pub fn background_row(&self, cx: &App) -> (bool, bool, String, bool) {
        let (state, error) = self.background.clone().unwrap_or_else(|| (self.models.background.read(cx).background.clone(), None));
        let text = error.clone().unwrap_or_else(|| if state.available { BACKGROUND_HINT.to_string() } else { BACKGROUND_UNAVAILABLE.to_string() });
        (state.available && state.enabled, state.available, text, error.is_some())
    }

    /// The updates card now, and whether "Check automatically" is on (tests).
    pub fn update_state(&self, cx: &App) -> (UpdateRow, bool) {
        let update = &self.models.update.read(cx).update;
        (update_row(update, self.services.now()), self.auto.unwrap_or(update.auto))
    }

    /// Whether Play on launch is on.
    pub fn autoplay(&self) -> bool {
        self.autoplay
    }

    /// Which control has the focus: "theme", "colors", "autoplay", "reconnect", "background",
    /// "play", "stop", "toggle", "update", "auto", "root" (tests).
    pub fn focused(&self, window: &Window) -> Option<&'static str> {
        let f = &self.focus;
        [
            (&f.theme, "theme"),
            (&f.colors, "colors"),
            (&f.autoplay, "autoplay"),
            (&f.reconnect, "reconnect"),
            (&f.background, "background"),
            (&f.shortcuts[0], "play"),
            (&f.shortcuts[1], "stop"),
            (&f.shortcuts[2], "toggle"),
            (&f.update, "update"),
            (&f.auto, "auto"),
            (&f.root, "root"),
        ]
        .into_iter()
        .find(|(handle, _)| handle.is_focused(window))
        .map(|(_, name)| name)
    }

    /// The focus of the window's content itself (no control): where the window starts.
    pub fn root_focus(&self) -> FocusHandle {
        self.focus.root.clone()
    }

    /// The current choice of a combo, as stored ("system", "accent", "5").
    pub fn choice(&self, kind: ComboKind) -> String {
        match kind {
            ComboKind::Theme => self.theme.clone(),
            ComboKind::Colors => self.colors.clone(),
            ComboKind::Reconnect => format!("{}", self.reconnect as i64),
        }
    }

    fn combo_text(&self, kind: ComboKind) -> &'static str {
        match kind {
            ComboKind::Theme => THEMES.iter().find(|(v, _)| *v == self.theme).map_or("", |(_, t)| t),
            ComboKind::Colors => COLORS.iter().find(|(v, _)| *v == self.colors).map_or("", |(_, t)| t),
            ComboKind::Reconnect => RECONNECT.iter().find(|(v, _)| *v == self.reconnect).map_or("", |(_, t)| t),
        }
    }

    /// The menu a combo opens: one item per choice (ids 1…), the current one checked.
    pub fn combo_menu(&self, kind: ComboKind) -> Menu {
        let items: Vec<(String, &str)> = match kind {
            ComboKind::Theme => THEMES.iter().map(|(v, t)| (v.to_string(), *t)).collect(),
            ComboKind::Colors => COLORS.iter().map(|(v, t)| (v.to_string(), *t)).collect(),
            ComboKind::Reconnect => RECONNECT.iter().map(|(v, t)| (format!("{}", *v as i64), *t)).collect(),
        };
        let current = self.choice(kind);
        items.iter().enumerate().fold(Menu::new(), |menu, (i, (value, text))| menu.action(i as u32 + 1, text, true, *value == current, None))
    }

    /// A combo was clicked (`bounds`: its border box): its menu at the bottom-left of the
    /// button + 2 px.
    pub fn open_combo(&mut self, kind: ComboKind, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let x = f64::from(bounds.left().as_f32());
        let y = f64::from(bounds.bottom().as_f32()) + 2.0;
        let this = cx.entity().downgrade();
        self.services.popup_menu(
            window,
            self.combo_menu(kind),
            Position::Client { x, y },
            Box::new(move |chosen, cx| {
                if let Some(id) = chosen.filter(|id| *id > 0) {
                    let _ = this.update(cx, |view, cx| view.choose(kind, id as usize - 1, cx));
                }
            }),
            cx,
        );
    }

    /// The `index`th choice of a combo was picked.
    pub fn choose(&mut self, kind: ComboKind, index: usize, cx: &mut Context<Self>) {
        match kind {
            ComboKind::Theme => {
                let Some((value, _)) = THEMES.get(index) else { return };
                self.theme = value.to_string();
                self.services.set_theme(value, cx);
            }
            ComboKind::Colors => {
                let Some((value, _)) = COLORS.get(index) else { return };
                self.colors = value.to_string();
                self.change("colors", "visualizer-colors", json!(value), cx);
            }
            ComboKind::Reconnect => {
                let Some((minutes, _)) = RECONNECT.get(index) else { return };
                self.reconnect = *minutes;
                self.change("reconnectMinutes", "reconnect-minutes", json!(*minutes as i64), cx);
            }
        }
        cx.notify();
    }

    /// `change(key, storageKey, value)`: saved, then broadcast.
    fn change(&self, key: &str, store_key: &str, value: serde_json::Value, cx: &mut Context<Self>) {
        self.services.save_setting(store_key, value.clone(), cx);
        self.services.broadcast_setting(key, value, cx);
    }

    pub fn set_autoplay(&mut self, on: bool, cx: &mut Context<Self>) {
        self.autoplay = on;
        self.change("autoplay", "autoplay", json!(on), cx);
        cx.notify();
    }

    /// The helper toggled: Rust's answer, or the toggle back and the error in red.
    pub fn set_background(&mut self, on: bool, cx: &mut Context<Self>) {
        self.background = Some(match self.services.set_background(on, cx) {
            Ok(background) => (background, None),
            Err(error) => (Background { available: true, enabled: !on, starts_with_windows: false }, Some(qust_text::shown(&error))),
        });
        cx.notify();
    }

    pub fn set_auto_update(&mut self, on: bool, cx: &mut Context<Self>) {
        self.auto = Some(on);
        self.services.save_setting("auto-update", json!(on), cx);
        cx.notify();
    }

    /// The update button: install when an update is ready, else check.
    pub fn update_clicked(&mut self, cx: &mut Context<Self>) {
        if self.models.update.read(cx).update.ready() {
            self.services.install_update(cx);
        } else {
            self.services.check_for_updates(cx);
        }
    }

    /// A shortcut box was clicked: record a new key combination for it (one at a time). The
    /// shortcuts are paused (a registered one never reaches the window), the window's keys are
    /// captured, and the first key that means something ends it: Esc cancels, Backspace or Delete
    /// reset to the default, a combination (with a modifier, or F1–F24 / a media key alone) is
    /// set. Losing the window's focus cancels.
    pub fn start_recording(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.is_some() || index >= SHORTCUTS.len() {
            return;
        }
        self.recording = Some(index);
        self.errors = [None, None, None];
        self.generation += 1;
        let generation = self.generation;
        cx.notify();
        self.services.pause_shortcuts(true, cx);
        let (tx, rx) = smol::channel::unbounded::<Recorded>();
        self.capture = self.services.capture_keys(
            window,
            Box::new(move |recorded| {
                let _ = tx.try_send(recorded);
            }),
        );
        // Ends with the first key that counts, or when the capture is dropped (its sender goes).
        cx.spawn(async move |this, cx| {
            while let Ok(recorded) = rx.recv().await {
                if recorded == Recorded::Ignore {
                    continue;
                }
                let _ = this.update(cx, |view, cx| {
                    if view.generation == generation {
                        view.recorded(recorded, cx);
                    }
                });
                break;
            }
        })
        .detach();
    }

    /// A key while recording, from the native capture or from GPUI.
    pub fn recorded(&mut self, recorded: Recorded, cx: &mut Context<Self>) {
        match recorded {
            Recorded::Ignore => {}
            Recorded::Cancel => self.finish(None, cx),
            Recorded::Reset => self.finish(Some(String::new()), cx),
            Recorded::Set(accelerator) => self.finish(Some(accelerator), cx),
        }
    }

    /// `finishRecording(accelerator)`: None cancels.
    fn finish(&mut self, accelerator: Option<String>, cx: &mut Context<Self>) {
        let Some(index) = self.recording.take() else { return };
        self.capture = None;
        self.errors = [None, None, None];
        if let Some(accelerator) = accelerator {
            if let Err(error) = self.services.set_shortcut(SHORTCUTS[index].0, &accelerator, cx) {
                // Shown as it comes (the OS's text can be in a right-to-left language).
                self.errors[index] = Some(qust_text::shown(&error));
            }
        }
        self.services.pause_shortcuts(false, cx);
        cx.notify();
    }

    // GPUI's key events while recording (where there is no native capture, or it missed one).
    fn record_key(&mut self, key: &str, modifiers: Modifiers, cx: &mut Context<Self>) {
        let recorded = record_gpui(key, modifiers.control, modifiers.alt, modifiers.shift, modifiers.platform);
        self.recorded(recorded, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.is_some() {
            // Every key is swallowed while recording.
            let keystroke = event.keystroke.clone();
            self.record_key(&keystroke.key, keystroke.modifiers, cx);
            cx.stop_propagation();
        } else if event.keystroke.key == "escape" {
            self.services.close_window(window, cx);
        }
    }

    fn combo(&self, kind: ComboKind, cx: &Context<Self>) -> AnyElement {
        let (id, selector, title, handle) = match kind {
            ComboKind::Theme => ("themeCombo", "#themeCombo", "Theme", &self.focus.theme),
            ComboKind::Colors => ("colorsCombo", "#colorsCombo", "Visualizer colors", &self.focus.colors),
            ComboKind::Reconnect => ("reconnectCombo", "#reconnectCombo", "Reconnect", &self.focus.reconnect),
        };
        let this = cx.entity().downgrade();
        Combo::new(id, self.combo_text(kind))
            .aria_label(title)
            .forced(self.forced(&[selector]))
            .focus(handle)
            .on_click(move |bounds, window, cx| {
                let _ = this.update(cx, |view, cx| view.open_combo(kind, bounds, window, cx));
            })
            .into_any_element()
    }

    /// A toggle: interactive, or (`inert`) shown but neither focusable nor clickable, faded when
    /// `faded` (`.setting.is-disabled .toggle`: `Switch::disabled`, a group opacity). An inert
    /// toggle still grows its thumb under the pointer, as the page's label did.
    #[allow(clippy::too_many_arguments)]
    fn toggle(&self, id: &'static str, checked: bool, label: &'static str, selectors: &[&str], inert: bool, faded: bool, handle: &FocusHandle, on_change: impl Fn(&mut SettingsView, bool, &mut Context<SettingsView>) + 'static, cx: &Context<Self>) -> AnyElement {
        let mut switch = Switch::new(id, checked, label).forced(self.forced(selectors)).disabled(faded);
        if !inert {
            let this = cx.entity().downgrade();
            let on_change = Rc::new(on_change);
            switch = switch.focus(handle).on_change(move |on, _, cx| {
                let on_change = on_change.clone();
                let _ = this.update(cx, |view, cx| on_change(view, on, cx));
            });
        }
        switch.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn card(icon: Icon, title: impl Into<gpui::SharedString>, description: impl Into<gpui::SharedString>, error: bool, disabled: bool, control: Option<AnyElement>, theme: &Theme, _: &App) -> gpui::Div {
        setting_card(theme)
            .child(setting_icon(icon, disabled, theme))
            .child(setting_text(title, description, error, disabled, theme))
            .when_some(control, |card, control| card.child(control))
    }
}

/// A group heading after a card: CSS collapses the card's 3 px bottom margin into the heading's
/// 14 px top margin; GPUI doesn't collapse margins, so the heading adds only the difference.
fn group(text: &str, first: bool, theme: &Theme, _: &App) -> gpui::Div {
    let heading = settings_group(text.to_string(), first, theme);
    if first {
        heading
    } else {
        heading.mt(px(11.0))
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme(cx);
        let shortcuts = self.models.shortcuts.read(cx).shortcuts.clone();
        let update = self.models.update.read(cx).update.clone();
        let background = self.background.clone().unwrap_or_else(|| (self.models.background.read(cx).background.clone(), None));
        let this = cx.entity().downgrade();

        // Appearance
        let mut list = div()
            .flex()
            .flex_col()
            .pt(px(16.0))
            .px(px(20.0))
            .pb(px(20.0))
            .child(group("Appearance", true, &theme, cx))
            .child(Self::card(Icon::Theme, "Theme", "Follow Windows, or pick light or dark", false, false, Some(self.combo(ComboKind::Theme, cx)), &theme, cx))
            .child(Self::card(Icon::Bars, "Visualizer colors", "Bar colors, or off. Click the analyzer to cycle.", false, false, Some(self.combo(ComboKind::Colors, cx)), &theme, cx));

        // Playback
        let autoplay = self.toggle("autoplayToggle", self.autoplay, "Play on launch", &["#autoplayToggle", ".setting:nth-of-type(3) .toggle"], false, false, &self.focus.autoplay, |view, on, cx| view.set_autoplay(on, cx), cx);
        let (state, error) = background;
        let available = state.available;
        let helper = self.toggle(
            "backgroundToggle",
            available && state.enabled,
            "Background helper",
            &["#backgroundToggle", ".setting:nth-of-type(5) .toggle"],
            !available,
            !available,
            &self.focus.background,
            |view, on, cx| view.set_background(on, cx),
            cx,
        );
        let helper_text = error.clone().unwrap_or_else(|| if available { BACKGROUND_HINT.to_string() } else { BACKGROUND_UNAVAILABLE.to_string() });
        list = list
            .child(group("Playback", false, &theme, cx))
            .child(Self::card(Icon::Autoplay, "Play on launch", "Start the last cartridge when Qust opens", false, false, Some(autoplay), &theme, cx))
            .child(Self::card(Icon::Signal, "Reconnect", "How long to keep trying when a stream drops", false, false, Some(self.combo(ComboKind::Reconnect, cx)), &theme, cx))
            .child(Self::card(Icon::Tray, "Background helper", helper_text, error.is_some(), !available, Some(helper), &theme, cx));

        // Global shortcuts
        list = list.child(group("Global shortcuts", false, &theme, cx));
        for (index, (action, title, hint)) in SHORTCUTS.iter().enumerate() {
            let binding = match index {
                0 => &shortcuts.play,
                1 => &shortcuts.stop,
                _ => &shortcuts.toggle,
            };
            let row = shortcut_row(binding, hint, self.recording == Some(index), self.errors[index].as_deref());
            let selector = format!("[data-action=\"{action}\"]");
            let this = this.clone();
            let shortcut = ShortcutBox::new(*action, row.display.clone(), *title)
                .forced(self.forced(&[selector.as_str()]))
                .focus(&self.focus.shortcuts[index])
                .on_click(move |window, cx| {
                    let _ = this.update(cx, |view, cx| view.start_recording(index, window, cx));
                });
            list = list.child(Self::card(Icon::Keyboard, *title, row.status.clone(), row.error, false, Some(shortcut.into_any_element()), &theme, cx));
        }

        // Updates
        let row = update_row(&update, self.services.now());
        let update_button = {
            let this = this.clone();
            Button::new("updateButton")
                .label(row.label)
                .kind(if row.accent { ButtonKind::Accent } else { ButtonKind::Standard })
                .disabled(row.busy)
                .forced(self.forced(&["#updateButton"]))
                .focus(&self.focus.update)
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |view, cx| view.update_clicked(cx));
                })
                .into_any_element()
        };
        let auto = self.toggle(
            "autoUpdateToggle",
            self.auto.unwrap_or(update.auto),
            "Check automatically",
            &["#autoUpdateToggle"],
            !update.installed,
            false,
            &self.focus.auto,
            |view, on, cx| view.set_auto_update(on, cx),
            cx,
        );
        list = list
            .child(group("Updates", false, &theme, cx))
            .child(Self::card(Icon::Update, format!("Qust {}", self.services.version()), if row.error { qust_text::shown(&row.text) } else { row.text.clone() }, row.error, false, Some(update_button), &theme, cx))
            .child(Self::card(Icon::Clock, "Check automatically", "Every 8 hours. Updates install when you restart Qust.", false, false, Some(auto), &theme, cx));

        // About
        list = list
            .child(group("About", false, &theme, cx))
            .child(Self::card(Icon::SettingsInfo, "Qust", "Quick streaming player. GPL-3.0-or-later. github.com/mkpascal/qust", false, false, None, &theme, cx));

        let measured = this.clone();
        div()
            .id("settings-window")
            .size_full()
            // The page's font (kerning on) for every text inside.
            .font_text()
            .track_focus(&self.focus.root)
            .on_key_down(cx.listener(Self::key_down))
            // Tab and Shift+Tab are bindings, dispatched before key listeners: while recording
            // they are keys like any other (Shift+Tab can be a shortcut).
            .capture_action(cx.listener(|view, _: &FocusNext, window, cx| {
                if view.recording.is_some() {
                    view.record_key("tab", window.modifiers(), cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|view, _: &FocusPrev, window, cx| {
                if view.recording.is_some() {
                    // The binding is exactly Shift+Tab.
                    view.record_key("tab", Modifiers { shift: true, ..window.modifiers() }, cx);
                    cx.stop_propagation();
                }
            }))
            .child(scroll_area("settings-body", &self.scroll, self.overflow, list, move |over, cx| {
                let _ = measured.update(cx, |view, cx| {
                    view.overflow = over;
                    cx.notify();
                });
            }))
    }
}

/// The Settings window's root, for `cx.open_window(options, |window, cx| ...)`: open it at
/// [`SIZE`] with the title [`TITLE`].
pub fn settings_window(models: &Models, services: Rc<dyn PanelServices>, window: &mut Window, cx: &mut App) -> (Entity<WindowRoot>, Entity<SettingsView>) {
    let view = cx.new(|cx| SettingsView::new(models, services, window, cx));
    let root = cx.new(|_| WindowRoot::new(view.clone()));
    (root, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(accelerator: &str, registered: bool) -> Binding {
        Binding { accelerator: accelerator.into(), registered }
    }

    #[test]
    fn shortcut_rows_like_render_shortcuts() {
        let hint = SHORTCUTS[0].2;
        let row = shortcut_row(&binding("Ctrl+Alt+Insert", true), hint, false, None);
        assert_eq!(row, ShortcutRow { display: ShortcutDisplay::Keys { keys: vec!["Ctrl".into(), "Alt".into(), "Insert".into()], unavailable: false }, status: hint.into(), error: false });
        let row = shortcut_row(&binding("Ctrl+Alt+Insert", false), hint, false, None);
        assert_eq!((row.status.as_str(), row.error), (UNAVAILABLE_HINT, true));
        assert!(matches!(row.display, ShortcutDisplay::Keys { unavailable: true, .. }));
        let row = shortcut_row(&binding("", false), hint, false, None);
        assert_eq!((row.display, row.status.as_str(), row.error), (ShortcutDisplay::Unassigned, hint, false));
        // Recording wins over everything; an error over the unavailable hint.
        let row = shortcut_row(&binding("Ctrl+Alt+Insert", false), hint, true, Some("x"));
        assert_eq!((row.display, row.status.as_str(), row.error), (ShortcutDisplay::Recording, RECORDING_HINT, false));
        let row = shortcut_row(&binding("Ctrl+Alt+Insert", false), hint, false, Some("Already used for another action"));
        assert_eq!((row.status.as_str(), row.error), ("Already used for another action", true));
    }

    #[test]
    fn update_rows_like_render_update() {
        let now = 1_794_400_000;
        let base = UpdateStatus { installed: true, current: "1.1.5".into(), status: "idle".into(), last_check: Some(now - 7500), auto: true, ..Default::default() };
        let row = update_row(&base, now);
        assert_eq!((row.text.as_str(), row.label, row.busy, row.accent, row.error), ("Last checked 2 h ago.", "Check for updates", false, false, false));
        assert_eq!(update_row(&UpdateStatus { last_check: Some(0), ..base.clone() }, now).text, "Not checked yet.");
        assert_eq!(update_row(&UpdateStatus { last_check: Some(now - 3 * 86400), ..base.clone() }, now).text, "Last checked 3 days ago.");
        let row = update_row(&UpdateStatus { status: "downloading".into(), version: Some("1.2.0".into()), progress: None, ..base.clone() }, now);
        assert_eq!((row.text.as_str(), row.busy), ("Downloading version 1.2.0… 0%", true));
        let row = update_row(&UpdateStatus { status: "error".into(), error: None, ..base.clone() }, now);
        assert_eq!((row.text.as_str(), row.label, row.error), ("Couldn't update: null", "Try again", true));
        // Not installed wins over the status, but a ready status still colours the button.
        let row = update_row(&UpdateStatus { installed: false, status: "ready".into(), ..base.clone() }, now);
        assert_eq!((row.text.as_str(), row.label, row.busy, row.accent), ("Updates work in the installed version of Qust.", "Check for updates", true, true));
    }
}
