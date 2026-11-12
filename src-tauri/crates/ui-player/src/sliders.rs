//! The sliders island (`#sliders`, BEHAVIOUR.md 2.3): volume over balance for a stream; for a
//! file with a duration, the seek row on top and volume and balance side by side.
//!
//! The seek row changes four times a second while a file plays, so it is a live layer
//! ([`SeekView`]) drawn in a slot this view leaves for it.
//!
//! **Values.** `qust_fluent::Slider` shows the player's value unless the user is dragging it,
//! and reports only the user's changes (`on_input`), so nothing echoes back as a command. The
//! labels next to the sliders ("70%", "L 30", the seek time) show what the slider shows, the
//! user's value while dragging included; the slider doesn't expose that, so [`Mirror`] follows
//! the same rule beside it: a press on the slider starts a drag (seen in the capture phase,
//! before the slider handles it), the commit at the release ends it, user values come through
//! `on_input`, the player's only while not dragging. Candidate for `qust-fluent`: a way for the
//! owner to read the shown value (see the report).

use crate::view::{forced, Shared};
use gpui::{div, prelude::*, px, Context, Entity, FocusHandle, IntoElement, MouseButton, MouseDownEvent, Render, SharedString, Window};
use qust_fluent::{icon, surface, theme, Frame, Icon, Slider, SliderSize, TextStyles};
use qust_ui::{format, portal_slot};
use std::rc::Rc;

/// The seek bar's value is the position in ten-thousandths.
pub const SEEK_STEPS: i64 = 10000;

/// What a slider shows, kept beside it (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mirror {
    pub shown: i64,
    pub owner: i64,
    pub dragging: bool,
}

impl Mirror {
    pub fn new(value: i64) -> Mirror {
        Mirror { shown: value, owner: value, dragging: false }
    }

    /// A value from the player. True when the shown value changed.
    pub fn owner(&mut self, value: i64) -> bool {
        if value == self.owner {
            return false;
        }
        self.owner = value;
        if self.dragging || self.shown == value {
            return false;
        }
        self.shown = value;
        true
    }

    pub fn input(&mut self, value: i64) -> bool {
        let changed = self.shown != value;
        self.shown = value;
        changed
    }

    /// The press that starts a drag.
    pub fn press(&mut self) {
        self.dragging = true;
    }

    /// The commit (end of a drag, or a wheel or key step).
    pub fn commit(&mut self, value: i64) {
        self.dragging = false;
        self.shown = value;
    }
}

pub fn volume_icon(value: i64) -> Icon {
    if value == 0 {
        Icon::Volume0
    } else if value < 50 {
        Icon::Volume1
    } else {
        Icon::Volume2
    }
}

/// `--duration` truthy: the island shows the seek row.
pub fn seekable_layout(duration: Option<f64>) -> bool {
    duration.is_some_and(|d| d != 0.0 && !d.is_nan())
}

/// The seek bar's value from the player's position.
pub fn seek_value(duration: Option<f64>, position: Option<f64>, seekable: bool) -> i64 {
    match (duration, position) {
        (Some(duration), Some(position)) if seekable && duration != 0.0 => {
            let value = (position / duration).min(1.0) * SEEK_STEPS as f64;
            qust_fluent::slider::Range { min: 0, max: SEEK_STEPS, center: None }.clamp(value)
        }
        _ => 0,
    }
}

pub struct SlidersView {
    shared: Rc<Shared>,
    pub volume: Mirror,
    pub balance: Mirror,
    seekable: bool,
    volume_focus: FocusHandle,
    balance_focus: FocusHandle,
    pub renders: u64,
}

impl SlidersView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> SlidersView {
        cx.observe(&shared.models.levels, |this: &mut SlidersView, levels, cx| {
            let (volume, balance) = { let l = levels.read(cx); (l.volume, l.balance) };
            let a = this.volume.owner(volume.clamp(0, 100));
            let b = this.balance.owner(balance.clamp(-100, 100));
            // The sliders themselves follow the model at the next render either way.
            let _ = (a, b);
            cx.notify();
        })
        .detach();
        cx.observe(&shared.models.seek, |this: &mut SlidersView, seek, cx| {
            let seekable = seekable_layout(seek.read(cx).duration);
            if seekable != this.seekable {
                this.seekable = seekable;
                cx.notify();
            }
        })
        .detach();
        let levels = shared.models.levels.read(cx).clone();
        let seekable = seekable_layout(shared.models.seek.read(cx).duration);
        SlidersView {
            shared,
            volume: Mirror::new(levels.volume.clamp(0, 100)),
            balance: Mirror::new(levels.balance.clamp(-100, 100)),
            seekable,
            volume_focus: qust_fluent::focus::tab_stop(cx),
            balance_focus: qust_fluent::focus::tab_stop(cx),
            renders: 0,
        }
    }

    pub fn volume_focus(&self) -> &FocusHandle {
        &self.volume_focus
    }

    pub fn balance_focus(&self) -> &FocusHandle {
        &self.balance_focus
    }
}

/// A `.slider-row`: icon, slider, value.
fn row(icon_name: Icon, slider: impl IntoElement, value: String, pair: bool, window: &Window, cx: &gpui::App) -> impl IntoElement {
    let theme = theme(cx);
    let (icon_size, value_width, value_size, gap) = if pair { (14.0, 30.0, 11.0, 4.0) } else { (16.0, 34.0, 12.0, 10.0) };
    div()
        .flex()
        .items_center()
        .gap(px(gap))
        .h(px(28.0))
        .when(pair, |r| r.flex_1().min_w_0())
        .child(icon(icon_name, icon_size).text_color(theme.text_2))
        .child(slider)
        .child(
            div()
                .flex_none()
                .w(px(value_width))
                .font_text()
                .type_size(value_size, 20.0).font_features(crate::text::tabular_kerning())
                .text_color(theme.text_2)
                .text_right()
                // On Chromium's baseline: 11/20 sits 1 px low in GPUI.
                .relative()
                .top(px(crate::text::baseline_shift(value_size, 20.0, false, window)))
                .child(SharedString::from(value)),
        )
}

impl Render for SlidersView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let pair = self.seekable;
        let levels = self.shared.models.levels.read(cx).clone();
        let this = cx.entity().downgrade();
        let size = if pair { SliderSize::Pair } else { SliderSize::Regular };

        let mirror_slider = |id: &'static str, slider: Slider, which: fn(&mut SlidersView) -> &mut Mirror| {
            let press = this.clone();
            div().flex_1().min_w_0().child(slider.w_full()).capture_any_mouse_down(move |event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left {
                    let _ = press.update(cx, |view, _| which(view).press());
                }
            }).id(id)
        };

        let commands = self.shared.commands.clone();
        let input = this.clone();
        let commit = this.clone();
        let volume = Slider::new("volume", levels.volume, 0, 100, "Volume")
            .size(size)
            .focus(&self.volume_focus)
            .forced(forced(cx, "#volumeSlider"))
            .on_input(move |value, _, cx| {
                let _ = input.update(cx, |view, cx| {
                    if view.volume.input(value) {
                        cx.notify();
                    }
                });
                commands.set_volume(value);
            })
            .on_commit(move |value, _, cx| {
                let _ = commit.update(cx, |view, cx| {
                    view.volume.commit(value);
                    cx.notify();
                });
            });

        let commands = self.shared.commands.clone();
        let input = this.clone();
        let commit = this.clone();
        let balance = Slider::new("balance", levels.balance, -100, 100, "Balance")
            .centered(0)
            .size(size)
            .tooltip("Double-click to center")
            .focus(&self.balance_focus)
            .forced(forced(cx, "#balanceSlider"))
            .on_input(move |value, _, cx| {
                let _ = input.update(cx, |view, cx| {
                    if view.balance.input(value) {
                        cx.notify();
                    }
                });
                commands.set_balance(value);
            })
            .on_commit(move |value, _, cx| {
                let _ = commit.update(cx, |view, cx| {
                    view.balance.commit(value);
                    cx.notify();
                });
            });

        let volume_row = row(volume_icon(self.volume.shown), mirror_slider("volume-wrap", volume, |v| &mut v.volume), format!("{}%", self.volume.shown), pair, window, cx);
        let balance_row = row(Icon::Balance, mirror_slider("balance-wrap", balance, |v| &mut v.balance), format::balance(self.balance.shown), pair, window, cx);
        let root = self.shared.root.clone();

        surface(Frame::island(&theme))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .py(px(9.0))
            .px(px(14.0))
            .when(pair, |s| s.child(div().w_full().h(px(28.0)).flex_none().child(portal_slot(root, "seek"))))
            .child(div().flex().when(pair, |p| p.flex_row().gap(px(14.0))).when(!pair, |p| p.flex_col().gap(px(4.0))).child(volume_row).child(balance_row))
    }
}

/// The seek row (`.seek-row`) as a live layer: the seek bar and "pos / dur".
pub struct SeekView {
    shared: Rc<Shared>,
    pub mirror: Mirror,
    focus: FocusHandle,
    pub renders: u64,
}

impl SeekView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> SeekView {
        cx.observe(&shared.models.seek, |this: &mut SeekView, seek, cx| {
            let seek = seek.read(cx);
            this.mirror.owner(seek_value(seek.duration, seek.position, seek.seekable));
            cx.notify();
        })
        .detach();
        let seek = shared.models.seek.read(cx).clone();
        // Before everything else in the Tab order, as the first control in the page: the row is
        // painted after the window's content (it is a live layer), so it sorts by index.
        let focus = cx.focus_handle().tab_index(-1).tab_stop(true);
        SeekView { shared, mirror: Mirror::new(seek_value(seek.duration, seek.position, seek.seekable)), focus, renders: 0 }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
}

impl Render for SeekView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let seek = self.shared.models.seek.read(cx).clone();
        let Some(duration) = seek.duration.filter(|_| seekable_layout(seek.duration)) else {
            return div().size_full().into_any_element();
        };
        let value = seek_value(seek.duration, seek.position, seek.seekable);
        let this = cx.entity().downgrade();
        let input = this.clone();
        let commit = this.clone();
        let press = this.clone();
        let commands = self.shared.commands.clone();
        let models = self.shared.models.clone();
        let slider = Slider::new("seek", value, 0, SEEK_STEPS, "Position")
            .size(SliderSize::Seek)
            .disabled(!seek.seekable)
            .focus(&self.focus)
            .forced(forced(cx, "#seekSlider"))
            .on_input(move |value, _, cx| {
                let _ = input.update(cx, |view, cx| {
                    if view.mirror.input(value) {
                        cx.notify();
                    }
                });
            })
            .on_commit(move |value, _, cx| {
                let _ = commit.update(cx, |view, cx| {
                    view.mirror.commit(value);
                    cx.notify();
                });
                // As the page: only when the player says the file is seekable, with its duration now.
                let seek = models.seek.read(cx);
                if let (true, Some(duration)) = (seek.seekable, seek.duration.filter(|d| *d != 0.0)) {
                    commands.seek(value as f64 / SEEK_STEPS as f64 * duration);
                }
            });
        let seekable = seek.seekable;
        let position = format::time(self.mirror.shown as f64 / SEEK_STEPS as f64 * duration);
        let total = format::time(duration);
        let text = format!("{position} / {total}");
        let bold = gpui::HighlightStyle { color: Some(theme.text.into()), font_weight: Some(gpui::FontWeight::SEMIBOLD), ..Default::default() };
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .id("seek-wrap")
                    .flex_1()
                    .min_w_0()
                    .child(slider.w_full())
                    .capture_any_mouse_down(move |event: &MouseDownEvent, _, cx| {
                        if event.button == MouseButton::Left && seekable {
                            let _ = press.update(cx, |view, _| view.mirror.press());
                        }
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .min_w(px(78.0))
                    .font_text()
                    .type_size(12.0, 20.0)
                    .font_features(crate::text::tabular_kerning())
                    .text_color(theme.text_2)
                    .text_right()
                    .whitespace_nowrap()
                    .child(gpui::StyledText::new(text).with_highlights([(0..position.len(), bold)])),
            )
            .into_any_element()
    }
}

/// The seek row's view, for the root's live layers.
pub(crate) fn seek_layer(shared: &Rc<Shared>, cx: &mut gpui::App) -> Entity<SeekView> {
    let shared = shared.clone();
    cx.new(|cx| SeekView::new(shared, cx))
}
