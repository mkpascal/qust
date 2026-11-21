//! The sliders island (`#sliders`, BEHAVIOUR.md 2.3): the seek row on top, volume and balance
//! side by side under it. The frozen page put volume over balance for a stream; Qust now gives a
//! stream the file's layout, so nothing moves when switching, with a line saying LIVE where a
//! file's seek bar is.
//!
//! The seek row changes four times a second while a file plays, and the LIVE label animates
//! while a stream plays, so the row is a live layer ([`SeekView`]) drawn in a slot this view
//! leaves for it.
//!
//! **Values.** `qust_fluent::Slider` shows the player's value unless the user is dragging it,
//! and reports only the user's changes (`on_input`), so nothing echoes back as a command. The
//! labels next to the sliders ("70%", "L 30", the seek time) show what the slider shows, the
//! user's value while dragging included: a `qust_fluent::Mirror` per slider follows the
//! slider's own rule from its `on_input` / `on_drag` / `on_commit` and the player's values.

use crate::view::{forced, Shared};
use gpui::{div, prelude::*, px, Context, Entity, FocusHandle, IntoElement, Render, Window};
use qust_fluent::motion::{live_breathe_opacity, live_wave, motion};
use qust_fluent::text::spaced_text;
use qust_fluent::{icon, line, surface, theme, Frame, Icon, Slider, SliderSize, TextStyles};
use qust_ui::{format, portal_slot, Playback};
use std::rc::Rc;
use std::time::Instant;

pub use qust_fluent::Mirror;

/// The seek bar's value is the position in ten-thousandths.
pub const SEEK_STEPS: i64 = 10000;

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
        let levels = shared.models.levels.read(cx).clone();
        SlidersView {
            shared,
            volume: Mirror::new(levels.volume.clamp(0, 100)),
            balance: Mirror::new(levels.balance.clamp(-100, 100)),
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

/// A `.slider-row` of the pair: icon, slider, value.
fn row(icon_name: Icon, slider: impl IntoElement, value: String, cx: &gpui::App) -> impl IntoElement {
    let theme = theme(cx);
    let (icon_size, value_width, value_size, gap) = (14.0, 30.0, 11.0, 4.0);
    div()
        .flex()
        .items_center()
        .gap(px(gap))
        .h(px(28.0))
        .flex_1()
        .min_w_0()
        .child(icon(icon_name, icon_size).text_color(theme.text_2))
        .child(slider)
        .child(
            // Right-aligned in its box, on Chromium's baseline (11/20 sits 1 px low in GPUI's).
            div()
                .flex()
                .justify_end()
                .flex_none()
                .w(px(value_width))
                .font_text()
                .type_size(value_size, 20.0)
                .tabular_nums()
                .text_color(theme.text_2)
                .child(line(value)),
        )
}

impl Render for SlidersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let levels = self.shared.models.levels.read(cx).clone();
        let this = cx.entity().downgrade();
        let size = SliderSize::Pair;

        let slider_box = |id: &'static str, slider: Slider| div().flex_1().min_w_0().child(slider.w_full()).id(id);

        let commands = self.shared.commands.clone();
        let input = this.clone();
        let commit = this.clone();
        let drag = this.clone();
        let volume = Slider::new("volume", levels.volume, 0, 100, "Volume")
            .size(size)
            .focus(&self.volume_focus)
            .forced(forced(cx, "#volumeSlider"))
            .on_input(move |value, _, cx| {
                let _ = input.update(cx, |view, cx| {
                    let owner = view.shared.models.levels.read(cx).volume.clamp(0, 100);
                    if view.volume.input(value, owner) {
                        cx.notify();
                    }
                });
                commands.set_volume(value);
            })
            .on_drag(move |dragging, _, cx| {
                let _ = drag.update(cx, |view, _| view.volume.drag(dragging));
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
        let drag = this.clone();
        let balance = Slider::new("balance", levels.balance, -100, 100, "Balance")
            .centered(0)
            .size(size)
            .tooltip("Double-click to center")
            .focus(&self.balance_focus)
            .forced(forced(cx, "#balanceSlider"))
            .on_input(move |value, _, cx| {
                let _ = input.update(cx, |view, cx| {
                    let owner = view.shared.models.levels.read(cx).balance.clamp(-100, 100);
                    if view.balance.input(value, owner) {
                        cx.notify();
                    }
                });
                commands.set_balance(value);
            })
            .on_drag(move |dragging, _, cx| {
                let _ = drag.update(cx, |view, _| view.balance.drag(dragging));
            })
            .on_commit(move |value, _, cx| {
                let _ = commit.update(cx, |view, cx| {
                    view.balance.commit(value);
                    cx.notify();
                });
            });

        let volume_row = row(volume_icon(self.volume.shown), slider_box("volume-wrap", volume), format!("{}%", self.volume.shown), cx);
        let balance_row = row(Icon::Balance, slider_box("balance-wrap", balance), format::balance(self.balance.shown), cx);
        let root = self.shared.root.clone();

        surface(Frame::island(&theme))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .py(px(9.0))
            .px(px(14.0))
            .child(div().w_full().h(px(28.0)).flex_none().child(portal_slot(root, "seek")))
            .child(div().flex().flex_row().gap(px(14.0)).child(volume_row).child(balance_row))
    }
}

/// The seek row (`.seek-row`) as a live layer: the seek bar and "pos / dur" for a file, the
/// LIVE line for a stream, a plain line otherwise (nothing loaded, a file whose length isn't
/// known yet).
pub struct SeekView {
    shared: Rc<Shared>,
    pub mirror: Mirror,
    focus: FocusHandle,
    /// When the stream started playing: the LIVE label's animation runs from there.
    playing_since: Option<Instant>,
    pub renders: u64,
}

impl SeekView {
    pub(crate) fn new(shared: Rc<Shared>, cx: &mut Context<Self>) -> SeekView {
        cx.observe(&shared.models.seek, |this: &mut SeekView, seek, cx| {
            let seek = seek.read(cx);
            this.mirror.owner(seek_value(seek.duration, seek.position, seek.seekable));
            let playing = seek.state == Playback::Playing;
            if playing != this.playing_since.is_some() {
                this.playing_since = playing.then(Instant::now);
            }
            cx.notify();
        })
        .detach();
        let seek = shared.models.seek.read(cx).clone();
        let playing_since = (seek.state == Playback::Playing).then(Instant::now);
        // Before everything else in the Tab order, as the first control in the page: the row is
        // painted after the window's content (it is a live layer), so it sorts by index.
        let focus = cx.focus_handle().tab_index(-1).tab_stop(true);
        SeekView { shared, mirror: Mirror::new(seek_value(seek.duration, seek.position, seek.seekable)), focus, playing_since, renders: 0 }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }
}

impl SeekView {
    /// A stream's line: the rail broken around LIVE, dimmed unless playing. While playing, the
    /// label breathes (1 to 0.6) and the accent runs through its letters (`motion::live_*`).
    /// Without `live`, the rail alone. Not a control: no focus, no pointer handling.
    fn live_line(&self, live: bool, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *theme(cx);
        let motion = motion(cx);
        let elapsed = self.playing_since.filter(|_| live && !motion.reduced).map(|since| motion.elapsed(since, None));
        if elapsed.is_some() && motion.wants_frames() {
            window.request_animation_frame();
        }
        let rail = || div().flex_1().min_w_0().h(px(4.0)).rounded(px(2.0)).bg(theme.slider_track);
        let text = "LIVE";
        let mut label = spaced_text(text, 10.0 * 0.08);
        if let Some(elapsed) = elapsed {
            for letter in 0..text.len() {
                let color = theme.text_2.lerp(theme.accent, live_wave(elapsed, letter) as f32);
                label = label.highlight(letter..letter + 1, gpui::HighlightStyle { color: Some(color.into()), ..Default::default() });
            }
        }
        let label = div()
            .flex_none()
            .font_text()
            .type_size(10.0, 14.0)
            .semibold()
            .text_color(theme.text_2)
            .opacity(elapsed.map_or(1.0, |elapsed| live_breathe_opacity(elapsed) as f32))
            .child(label);
        div().size_full().flex().items_center().child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(24.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(8.0))
                .pr(px(10.0))
                .when(self.playing_since.is_none(), |line| line.opacity(0.5))
                .child(rail())
                .when(live, |line| line.child(label).child(rail())),
        )
    }
}

impl Render for SeekView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let theme = *theme(cx);
        let seek = self.shared.models.seek.read(cx).clone();
        let Some(duration) = seek.duration.filter(|_| seekable_layout(seek.duration)) else {
            return self.live_line(seek.live, window, cx).into_any_element();
        };
        let value = seek_value(seek.duration, seek.position, seek.seekable);
        let this = cx.entity().downgrade();
        let input = this.clone();
        let commit = this.clone();
        let drag = this.clone();
        let commands = self.shared.commands.clone();
        let models = self.shared.models.clone();
        let slider = Slider::new("seek", value, 0, SEEK_STEPS, "Position")
            .size(SliderSize::Seek)
            .disabled(!seek.seekable)
            .focus(&self.focus)
            .forced(forced(cx, "#seekSlider"))
            .on_input(move |value, _, cx| {
                let _ = input.update(cx, |view, cx| {
                    let seek = view.shared.models.seek.read(cx);
                    let owner = seek_value(seek.duration, seek.position, seek.seekable);
                    if view.mirror.input(value, owner) {
                        cx.notify();
                    }
                });
            })
            .on_drag(move |dragging, _, cx| {
                let _ = drag.update(cx, |view, _| view.mirror.drag(dragging));
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
        let position = format::time(self.mirror.shown as f64 / SEEK_STEPS as f64 * duration);
        let total = format::time(duration);
        let text = format!("{position} / {total}");
        let bold = gpui::HighlightStyle { color: Some(theme.text.into()), font_weight: Some(gpui::FontWeight::SEMIBOLD), ..Default::default() };
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(div().id("seek-wrap").flex_1().min_w_0().child(slider.w_full()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .flex_none()
                    .min_w(px(78.0))
                    .font_text()
                    .type_size(12.0, 20.0)
                    .tabular_nums()
                    .text_color(theme.text_2)
                    .child(line(text).highlight(0..position.len(), bold)),
            )
            .into_any_element()
    }
}

/// The seek row's view, for the root's live layers.
pub(crate) fn seek_layer(shared: &Rc<Shared>, cx: &mut gpui::App) -> Entity<SeekView> {
    let shared = shared.clone();
    cx.new(|cx| SeekView::new(shared, cx))
}
