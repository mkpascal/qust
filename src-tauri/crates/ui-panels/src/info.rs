//! The Stream information window (info.html, info.js): the station and title, the state badge,
//! the Stream / Network / Stability cards and the ICY metadata card.
//!
//! It follows `models.status` (`player-status`, every second and on changes) and, while something
//! plays, asks for the stream's details once a second (`PanelServices::stream_info`), and at once
//! when the URL or the state changed. It re-renders only when a text it shows changed.
//!
//! Every value has its text as tooltip (`title`), so a truncated one can be read; right-clicking
//! a value (`dd`) selects all of it and offers Copy (bridge.js). The page let the mouse select
//! part of a value too; here a value is selected whole (see the crate docs).
//!
//! The texts come from the stream and can be in any script: what the window draws is
//! `qust_text::shown` of each (right-to-left text in the order GPUI needs), computed when the
//! texts change; tooltips, Copy and accessibility labels keep the logical text.

use crate::format::{bytes, duration, format_from_content_type, js_number, locale_en, present, to_fixed, truthy};
use crate::services::PanelServices;
use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, Role, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window,
};
use qust_fluent::scrollbar::scroll_area;
use qust_fluent::section::{info_card, info_heading, info_label, info_value_line, state_badge};
use qust_fluent::text::{line, selection_colors};
use qust_fluent::{theme, TextStyles, TooltipExt};
use qust_shell::menu::{copy_menu, Position};
use qust_ui::{format, Models, Playback, Status, StreamInfo, WindowRoot};
use std::rc::Rc;
use std::time::Duration;

/// The client size the app opens the window at (main.rs: 580 × 590).
pub const SIZE: (f32, f32) = (580.0, 590.0);
/// The window title.
pub const TITLE: &str = "Stream information";
/// How often the details are asked for while something plays.
pub const POLL: Duration = Duration::from_secs(1);

/// The value ids, in page order (info.html's `dd` ids).
pub const VALUES: [&str; 21] = [
    "format", "bitrate", "samplerate", "channels", "rate", "buffer", "received", "connected", "stalls", "reconnects", "titleChanges", "host", "icyName", "icyTitle",
    "icyGenre", "icyDescription", "icyWebsite", "icyServer", "icyContentType", "icyMetaint", "icyMetaBytes",
];

const CARDS: [(&str, [(&str, &str); 4]); 3] = [
    ("Stream", [("Format", "format"), ("Bitrate", "bitrate"), ("Sample rate", "samplerate"), ("Channels", "channels")]),
    ("Network", [("Receiving", "rate"), ("Buffer", "buffer"), ("Received", "received"), ("Connected for", "connected")]),
    ("Stability", [("Dropouts", "stalls"), ("Reconnections", "reconnects"), ("Title changes", "titleChanges"), ("Host", "host")]),
];

const PAIRS: [(&str, &str); 9] = [
    ("Stream name", "icyName"),
    ("Current title", "icyTitle"),
    ("Genre", "icyGenre"),
    ("Description", "icyDescription"),
    ("Website", "icyWebsite"),
    ("Server", "icyServer"),
    ("Content type", "icyContentType"),
    ("Metadata interval", "icyMetaint"),
    ("Metadata received", "icyMetaBytes"),
];

/// Every text the window shows (and has as tooltips), as info.js's `render` computed them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InfoTexts {
    pub station: String,
    pub title: String,
    pub state: String,
    /// The badge in accent (`data-state="playing"`).
    pub playing: bool,
    /// One per [`VALUES`] id, in that order.
    pub values: Vec<String>,
}

impl InfoTexts {
    pub fn value(&self, id: &str) -> &str {
        VALUES.iter().position(|v| *v == id).and_then(|i| self.values.get(i)).map_or("—", String::as_str)
    }

    /// What the window draws for these texts: each through `qust_text::shown`.
    pub fn shown(&self) -> InfoTexts {
        InfoTexts {
            station: qust_text::shown(&self.station),
            title: qust_text::shown(&self.title),
            state: self.state.clone(),
            playing: self.playing,
            values: self.values.iter().map(qust_text::shown).collect(),
        }
    }
}

/// `set(id, value)`: empty, null and undefined show an em dash.
fn shown(value: Option<String>) -> String {
    match value {
        Some(text) if !text.is_empty() => text,
        _ => "—".to_string(),
    }
}

/// Whether something is active (`status.url && status.state !== 'stopped'`).
pub fn is_active(status: &Status) -> bool {
    present(&status.url).is_some() && status.state != Playback::Stopped
}

/// info.js `render`, as data.
pub fn texts(status: &Status, info: &StreamInfo) -> InfoTexts {
    let active = is_active(status);
    let empty = StreamInfo::default();
    let details = if active { info } else { &empty };
    let title = present(&details.title).or(present(&status.status_title));
    let state = match status.state {
        Playback::Stopped => "Stopped",
        Playback::Connecting => "Connecting",
        Playback::Buffering => "Buffering",
        Playback::Playing => "Playing",
        Playback::Paused => "Paused",
    };
    let value = |id: &str| -> Option<String> {
        match id {
            "format" => present(&details.format).map(str::to_string).or_else(|| details.content_type.as_deref().and_then(format_from_content_type).map(str::to_string)),
            "bitrate" => truthy(details.bitrate).map(|b| format!("{} kbps", js_number(b))),
            "samplerate" => truthy(details.samplerate).map(|s| format!("{} kHz", locale_en(s / 1000.0, 1))),
            "channels" => match details.channels {
                Some(1.0) => Some("Mono".into()),
                Some(2.0) => Some("Stereo".into()),
                other => truthy(other).map(|c| format!("{} channels", js_number(c))),
            },
            "rate" => truthy(details.rate).map(|r| format!("{} kbps", js_number(r))),
            "buffer" => status.buffer.filter(|_| active).map(|b| format!("{} s", to_fixed(b, 1))),
            // `bytes + metaBytes`: the details always carry both together.
            "received" => details.bytes.map(|b| bytes(b + details.meta_bytes.unwrap_or(0.0))),
            "connected" => details.connected_seconds.map(duration),
            "stalls" => active.then(|| js_number(status.stalls as f64)),
            "reconnects" => details.reconnects.map(js_number),
            "titleChanges" => details.title_changes.map(js_number),
            // `host(url)` in info.js: null (an em dash) where `new URL` throws.
            "host" => status.url.as_deref().filter(|_| active).and_then(format::url_host),
            "icyName" => details.name.clone(),
            "icyTitle" => details.title.clone(),
            "icyGenre" => details.genre.clone(),
            "icyDescription" => details.description.clone(),
            "icyWebsite" => details.website.clone(),
            "icyServer" => details.server.clone(),
            "icyContentType" => details.content_type.clone(),
            "icyMetaint" => active.then(|| match truthy(details.metaint) {
                Some(m) => format!("{} bytes", locale_en(m, 3)),
                None => "None".to_string(),
            }),
            "icyMetaBytes" => match (details.meta_bytes, truthy(details.metaint)) {
                (Some(m), Some(_)) => Some(format!("{} bytes", locale_en(m, 3))),
                _ => None,
            },
            _ => None,
        }
    };
    InfoTexts {
        station: present(&status.station).unwrap_or("Not playing").to_string(),
        title: shown(if active {
            title.or(present(&details.name)).or(present(&status.station)).map(str::to_string)
        } else {
            Some("Start a cartridge to see its stream details".to_string())
        }),
        state: state.to_string(),
        playing: status.state == Playback::Playing,
        values: VALUES.iter().map(|id| shown(value(id))).collect(),
    }
}

pub struct InfoView {
    models: Models,
    services: Rc<dyn PanelServices>,
    status: Status,
    info: StreamInfo,
    texts: InfoTexts,
    /// `texts` as drawn (`InfoTexts::shown`).
    shown: InfoTexts,
    ticker: Option<Task<()>>,
    // Requests sent and the newest one whose answer was applied (older answers are dropped).
    requested: u64,
    applied: u64,
    selected: Option<&'static str>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    overflow: bool,
    /// Details requests made (tests).
    pub polls: u64,
    /// Renders (tests: it re-renders only when a text changed).
    pub renders: u64,
    _subscriptions: Vec<Subscription>,
}

impl InfoView {
    pub fn new(models: &Models, services: Rc<dyn PanelServices>, window: &mut Window, cx: &mut Context<Self>) -> InfoView {
        let subscriptions = vec![cx.observe(&models.status, |view, _, cx| view.on_status(cx))];
        let focus = cx.focus_handle();
        // Keys reach the view only along the focus path: the window starts with the view focused.
        focus.focus(window, cx);
        let mut view = InfoView {
            models: models.clone(),
            services,
            status: Status::default(),
            info: StreamInfo::default(),
            texts: texts(&Status::default(), &StreamInfo::default()),
            shown: InfoTexts::default(),
            ticker: None,
            requested: 0,
            applied: 0,
            selected: None,
            focus,
            scroll: ScrollHandle::new(),
            overflow: false,
            polls: 0,
            renders: 0,
            _subscriptions: subscriptions,
        };
        // The status as the window opens (the page asked `player_status`; the bridge keeps the
        // model current from the start, so it is the same).
        view.status = models.status.read(cx).status.clone();
        view.texts = texts(&view.status, &view.info);
        view.shown = view.texts.shown();
        view.poll(cx);
        view.update_ticker(cx);
        view
    }

    /// What the window shows now.
    pub fn texts(&self) -> &InfoTexts {
        &self.texts
    }

    /// The value selected by a right-click, if any.
    pub fn selected(&self) -> Option<&'static str> {
        self.selected
    }

    /// The body's scroll position.
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// Whether the once-a-second poll is running.
    pub fn polling(&self) -> bool {
        self.ticker.is_some()
    }

    /// `update(next)`: a new URL or state asks for the details at once.
    fn on_status(&mut self, cx: &mut Context<Self>) {
        let next = self.models.status.read(cx).status.clone();
        let changed = next.url != self.status.url || next.state != self.status.state;
        self.status = next;
        if changed {
            self.poll(cx);
            self.update_ticker(cx);
        } else {
            self.refresh(cx);
        }
    }

    fn update_ticker(&mut self, cx: &mut Context<Self>) {
        if !is_active(&self.status) {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                break;
            }
        }));
    }

    /// `poll()`: nothing active clears the details; else they are asked for.
    fn poll(&mut self, cx: &mut Context<Self>) {
        let Some(url) = present(&self.status.url).filter(|_| is_active(&self.status)).map(str::to_string) else {
            self.info = StreamInfo::default();
            self.refresh(cx);
            return;
        };
        self.requested += 1;
        self.polls += 1;
        let id = self.requested;
        let request = self.services.stream_info(url.clone(), cx);
        cx.spawn(async move |this, cx| {
            let info = request.await;
            let _ = this.update(cx, |view, cx| {
                if id <= view.applied || view.status.url.as_deref() != Some(url.as_str()) {
                    return;
                }
                view.applied = id;
                view.info = info.unwrap_or_default();
                view.refresh(cx);
            });
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let next = texts(&self.status, &self.info);
        if next != self.texts {
            // A value whose text is replaced loses its selection, as a replaced text node does.
            if let Some(id) = self.selected {
                if next.value(id) != self.texts.value(id) {
                    self.selected = None;
                }
            }
            self.shown = next.shown();
            self.texts = next;
            cx.notify();
        }
    }

    /// Right-click on a value: select it whole, then the native Copy menu at the cursor.
    pub fn context_menu(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id);
        cx.notify();
        let text = self.texts.value(id).to_string();
        let services = self.services.clone();
        self.services.popup_menu(
            window,
            copy_menu(),
            Position::Cursor,
            Box::new(move |chosen, cx| {
                if chosen == Some(0xE002) {
                    services.copy_text(text, cx);
                }
            }),
            cx,
        );
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" {
            self.services.close_window(window, cx);
        } else if keystroke.key == "c" && (keystroke.modifiers.control || keystroke.modifiers.platform) {
            if let Some(id) = self.selected {
                self.services.copy_text(self.texts.value(id).to_string(), cx);
            }
        }
    }

    fn value(&self, id: &'static str, pairs: bool, last: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = *theme(cx);
        let text = SharedString::from(self.texts.value(id).to_string());
        let selected = self.selected == Some(id);
        // `.info-card dd`: semibold 15/20 tabular, 7 px below unless last; 13/20 regular in the
        // ICY pairs. One line, cut with an ellipsis as Chromium cuts it.
        div()
            .w_full()
            .when(!pairs && !last, |d| d.mb(px(7.0)))
            .child(info_value_line(self.shown.value(id).to_string(), pairs, selected.then(selection_colors), &theme))
            .id(id)
            .role(Role::Label)
            .aria_label(text.clone())
            .native_tooltip(text)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, _, window, cx| {
                    view.context_menu(id, window, cx);
                    cx.stop_propagation();
                }),
            )
            .into_any_element()
    }
}

impl Render for InfoView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let t = self.texts.clone();
        let shown = self.shown.clone();
        let head = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.0))
            .px(px(2.0))
            .pb(px(4.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_text()
                    .child(div().id("station").type_size(12.0, 16.0).text_color(theme.text_2).whitespace_nowrap().native_tooltip(t.station.clone()).child(shown.station.clone()))
                    .child(
                        div()
                            .id("title")
                            .role(Role::Label)
                            .aria_label(t.title.clone())
                            .native_tooltip(t.title.clone())
                            .child(line(shown.title.clone()).ellipsis().w_full().font_display().semibold().type_size(18.0, 24.0).text_color(theme.text)),
                    ),
            )
            .child(state_badge(t.state.clone(), t.playing, &theme).flex_none().id("state").native_tooltip(t.state.clone()));
        let cards = div().flex().flex_row().gap(px(10.0)).children(CARDS.iter().map(|(title, rows)| {
            let mut dl = div().flex().flex_col();
            for (i, (label, id)) in rows.iter().enumerate() {
                dl = dl.child(info_label(*label, false, &theme)).child(self.value(id, false, i == rows.len() - 1, cx));
            }
            info_card(&theme).flex_1().min_w_0().child(info_heading(title, &theme)).child(dl)
        }));
        let pairs = div().flex().flex_col().gap(px(5.0)).children(PAIRS.iter().map(|(label, id)| {
            div().flex().flex_row().gap(px(12.0)).child(info_label(*label, true, &theme).w(px(130.0)).flex_none()).child(div().flex_1().min_w_0().child(self.value(id, true, true, cx)))
        }));
        let icy = info_card(&theme).child(info_heading("ICY metadata", &theme)).child(pairs);
        let content = div().flex().flex_col().gap(px(10.0)).pt(px(14.0)).px(px(20.0)).pb(px(20.0)).child(head).child(cards).child(icy);
        let this = cx.entity().downgrade();
        div()
            .id("info-window")
            .size_full()
            // The page's font (kerning on) for every text inside.
            .font_text()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.selected.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(scroll_area("info-body", &self.scroll, self.overflow, content, move |over, cx| {
                let _ = this.update(cx, |view, cx| {
                    view.overflow = over;
                    cx.notify();
                });
            }))
    }
}

/// The Stream information window's root, for `cx.open_window(options, |window, cx| ...)`: open
/// it at [`SIZE`] with the title [`TITLE`].
pub fn info_window(models: &Models, services: Rc<dyn PanelServices>, window: &mut Window, cx: &mut App) -> (Entity<WindowRoot>, Entity<InfoView>) {
    let view = cx.new(|cx| InfoView::new(models, services, window, cx));
    let root = cx.new(|_| WindowRoot::new(view.clone()));
    (root, view)
}
