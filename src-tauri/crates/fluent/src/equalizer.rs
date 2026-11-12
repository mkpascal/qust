//! The playing equalizer (`.equalizer` on a cartridge, `.row-eq` on a playlist row): three
//! bars 2 × 10 px, gap 2, radius 1, in the current color, each `scaleY(0.25 → 1)` from the
//! bottom, 0.9 s ease-in-out infinite alternate, phases 0 / −0.3 / −0.6 s. Under reduced motion
//! the animation is removed and the bars stand at full height (`animation: none`).
//!
//! It animates on every frame, so it lives in its own tiny view ([`EqualizerView`]): embed it
//! with `.cached(..)` at its fixed 10 × 10 size and the cartridge or row around it never
//! re-renders for it.

use crate::color::Color;
use crate::motion::{equalizer_scale, motion};
use gpui::{div, px, Context, Div, IntoElement, ParentElement, Render, Styled, Window};
use std::time::{Duration, Instant};

/// The bars after `elapsed` of the animation (`None`: reduced motion, full height).
pub fn equalizer(elapsed: Option<Duration>, color: Color) -> Div {
    div().flex().flex_row().items_end().gap(px(2.0)).w(px(10.0)).h(px(10.0)).children((0..3).map(|bar| {
        let scale = elapsed.map_or(1.0, |elapsed| equalizer_scale(elapsed, bar)) as f32;
        div().w(px(2.0)).h(px(10.0 * scale)).rounded(px(1.0)).bg(color)
    }))
}

/// A running equalizer. `started`: when the element appeared (the page's animation started with
/// the `is-playing` class); set a new one with [`EqualizerView::restart`].
pub struct EqualizerView {
    pub started: Instant,
    pub color: Color,
}

impl EqualizerView {
    pub fn new(color: Color) -> EqualizerView {
        EqualizerView { started: Instant::now(), color }
    }

    pub fn restart(&mut self, cx: &mut Context<Self>) {
        self.started = Instant::now();
        cx.notify();
    }

    pub fn set_color(&mut self, color: Color, cx: &mut Context<Self>) {
        if self.color != color {
            self.color = color;
            cx.notify();
        }
    }
}

impl Render for EqualizerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let motion = motion(cx);
        if motion.reduced {
            return equalizer(None, self.color);
        }
        if motion.wants_frames() {
            window.request_animation_frame();
        }
        equalizer(Some(motion.elapsed(self.started, None)), self.color)
    }
}
