//! The transport island (`.transport`, BEHAVIOUR.md 2.4): Play, Pause, Stop, Instant switch,
//! Stream information, Settings, sharing the width equally, enabled as `renderTransport()` set
//! them, with the page's tooltips (the global shortcut's keys on Play and Stop, the update on
//! Settings, the Instant switch's long texts).

use crate::host::Panel;
use crate::menus::has_active;
use crate::view::{forced, Shared};
use gpui::{prelude::*, px, Context, FocusHandle, IntoElement, Render, Window};
use qust_fluent::{surface, theme, Button, Frame, Icon};
use qust_ui::Playback;
use std::rc::Rc;

pub const INSTANT_OFF: &str = "Instant switch: keep every cartridge streaming in the background, even when stopped, for immediate starting and switching (uses more data)";
pub const INSTANT_ON: &str = "Instant switch is on: every cartridge streams in the background, even when stopped, so starting or switching is immediate (uses more data)";

/// "Play" or "Play (Ctrl+Alt+Insert)".
pub fn with_keys(name: &str, keys: Option<String>) -> String {
    match keys {
        Some(keys) => format!("{name} ({keys})"),
        None => name.to_string(),
    }
}

pub struct TransportView {
    shared: Rc<Shared>,
    pub focus: [FocusHandle; 6],
    pub renders: u64,
}

impl TransportView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> TransportView {
        cx.observe(&shared.models.deck, |_, _, cx| cx.notify()).detach();
        cx.observe(&shared.models.shortcuts, |_, _, cx| cx.notify()).detach();
        cx.observe(&shared.models.update, |_, _, cx| cx.notify()).detach();
        let focus = std::array::from_fn(|_| qust_fluent::focus::tab_stop(cx));
        TransportView { shared, focus, renders: 0 }
    }
}

impl Render for TransportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let deck = self.shared.models.deck.read(cx).clone();
        let shortcuts = self.shared.models.shortcuts.read(cx).shortcuts.clone();
        let update = self.shared.models.update.read(cx).update.clone();
        let state = deck.state;
        let play_disabled = !has_active(&deck) || state.is_running();
        let pause_disabled = matches!(state, Playback::Stopped | Playback::Paused);
        let stop_disabled = state == Playback::Stopped;
        let settings_tip = match (update.ready(), &update.version) {
            (true, Some(version)) => format!("Settings: version {version} is ready to install"),
            (true, None) => "Settings: version undefined is ready to install".to_string(),
            _ => "Settings".to_string(),
        };

        let s = self.shared.clone();
        let play = Button::new("playButton")
            .icon_only(Icon::Play)
            .accent()
            .aria_label("Play")
            .tooltip(with_keys("Play", shortcuts.play.tooltip_keys()))
            .disabled(play_disabled)
            .focus(&self.focus[0])
            .forced(forced(cx, "#playButton"))
            .on_click(move |_, _, _| s.commands.play())
            .flex_1();
        let s = self.shared.clone();
        let pause = Button::new("pauseButton")
            .icon_only(Icon::Pause)
            .aria_label("Pause")
            .tooltip("Pause")
            .disabled(pause_disabled)
            .focus(&self.focus[1])
            .forced(forced(cx, "#pauseButton"))
            .on_click(move |_, _, _| s.commands.pause())
            .flex_1();
        let s = self.shared.clone();
        let stop = Button::new("stopButton")
            .icon_only(Icon::Stop)
            .aria_label("Stop")
            .tooltip(with_keys("Stop", shortcuts.stop.tooltip_keys()))
            .disabled(stop_disabled)
            .focus(&self.focus[2])
            .forced(forced(cx, "#stopButton"))
            .on_click(move |_, _, _| s.commands.stop())
            .flex_1();
        let s = self.shared.clone();
        let instant_on = deck.instant;
        let instant = Button::new("instantButton")
            .icon_only(Icon::Bolt)
            .toggle(instant_on)
            .aria_label("Instant switch")
            .tooltip(if instant_on { INSTANT_ON } else { INSTANT_OFF })
            .focus(&self.focus[3])
            .forced(forced(cx, "#instantButton"))
            .on_click(move |_, _, cx| {
                let on = s.models.deck.read(cx).instant;
                s.commands.set_instant(!on);
            })
            .flex_1();
        let s = self.shared.clone();
        let info = Button::new("infoButton")
            .icon_only(Icon::Info)
            .aria_label("Stream information")
            .tooltip("Stream information")
            .focus(&self.focus[4])
            .forced(forced(cx, "#infoButton"))
            .on_click(move |_, window, cx| s.open_panel(Panel::Info, window, cx))
            .flex_1();
        let s = self.shared.clone();
        let settings = Button::new("settingsButton")
            .icon_only(Icon::Settings)
            .aria_label("Settings")
            .tooltip(settings_tip)
            .update_dot(update.ready())
            .focus(&self.focus[5])
            .forced(forced(cx, "#settingsButton"))
            .on_click(move |_, window, cx| s.open_panel(Panel::Settings, window, cx))
            .flex_1();

        surface(Frame::island(&theme))
            .size_full()
            .flex()
            .gap(px(8.0))
            .py(px(9.0))
            .px(px(10.0))
            .child(play)
            .child(pause)
            .child(stop)
            .child(instant)
            .child(info)
            .child(settings)
    }
}
