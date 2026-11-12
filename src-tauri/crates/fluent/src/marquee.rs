//! The title marquee (`setTitle` in script.js; `.title-row`, BEHAVIOUR.md 2.2.2).
//!
//! It measures only when the title text or its dim flag changes (not on resize or DPI change):
//! `overflow = round(text width) − row width` (`offsetWidth − clientWidth`, both whole px). If
//! the text overflows, it scrolls: the text gets `padding: 0 6px`, the row a mask that fades
//! the left 6 px in and the right 12 px out, and the text moves by `distance = −(overflow + 18)`
//! px over `duration = max(6, overflow / 20 + 4)` s, `ease-in-out infinite alternate`, holding
//! 12 % at each end. A title that fits is static, with no padding and no mask.
//!
//! GPUI has no CSS mask: the fades are gradients of the row's (opaque) background color over the
//! text, which gives the same pixels on a flat background (the display island's fill over the
//! window background: pass it as `backdrop`).
//!
//! It lives in its own view so that animating it re-renders nothing else: embed it with
//! `.cached(StyleRefinement::default().w_full().h(px(20.)))`. During the holds it draws nothing
//! (it waits for the hold's end), and while moving it draws one frame per vsync.

use crate::color::Color;
use crate::motion::{marquee_offset, marquee_still_for, motion};
use crate::paint::bounds_probe;
use crate::text::{TextStyles, FONT_TEXT};
use gpui::prelude::FluentBuilder;
use gpui::{
    div, linear_color_stop, linear_gradient, px, Context, Font, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, TextRun, Window,
};
use std::time::{Duration, Instant};

/// What scrolls, once measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    pub overflow: f32,
    /// `--distance`, negative.
    pub distance: f32,
    /// `--duration`.
    pub duration: Duration,
}

impl Sweep {
    /// `setTitle`'s numbers for a text `text_width` wide in a row `row_width` wide.
    pub fn measure(text_width: f32, row_width: f32) -> Option<Sweep> {
        let overflow = text_width.round() - row_width.round();
        if overflow <= 0.0 {
            return None;
        }
        Some(Sweep { overflow, distance: -(overflow + 18.0), duration: Duration::from_secs_f32((overflow / 20.0 + 4.0).max(6.0)) })
    }
}

pub struct Marquee {
    text: SharedString,
    dim: bool,
    colors: (Color, Color),
    backdrop: Color,
    // (text, dim) of the last measure, the result, and when the animation started.
    measured: Option<(SharedString, bool)>,
    sweep: Option<Sweep>,
    started: Instant,
    waiting: bool,
}

impl Marquee {
    /// `colors`: (title, dim title) colors (`--text`, `--text-3`); `backdrop`: the opaque color
    /// behind the row.
    pub fn new(text: impl Into<SharedString>, dim: bool, colors: (Color, Color), backdrop: Color) -> Marquee {
        Marquee { text: text.into(), dim, colors, backdrop, measured: None, sweep: None, started: Instant::now(), waiting: false }
    }

    /// A new title (from the player state). Re-measures only when it differs.
    pub fn set(&mut self, text: impl Into<SharedString>, dim: bool, cx: &mut Context<Self>) {
        let text = text.into();
        if text != self.text || dim != self.dim {
            self.text = text;
            self.dim = dim;
            cx.notify();
        }
    }

    /// New theme colors.
    pub fn set_colors(&mut self, colors: (Color, Color), backdrop: Color, cx: &mut Context<Self>) {
        if colors != self.colors || backdrop != self.backdrop {
            self.colors = colors;
            self.backdrop = backdrop;
            cx.notify();
        }
    }

    pub fn sweep(&self) -> Option<Sweep> {
        self.sweep
    }

    fn font(&self) -> Font {
        Font { weight: if self.dim { FontWeight::NORMAL } else { FontWeight::SEMIBOLD }, ..gpui::font(FONT_TEXT) }
    }
}

impl Render for Marquee {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let motion = motion(cx);
        let (bounds, probe) = bounds_probe("marquee-row", window, cx);
        let row_width = bounds.read(cx).map(|b| b.size.width.as_f32());
        let key = (self.text.clone(), self.dim);
        if self.measured.as_ref() != Some(&key) {
            match row_width {
                Some(row_width) => {
                    let run = TextRun { len: self.text.len(), font: self.font(), color: self.colors.0.into(), background_color: None, underline: None, strikethrough: None };
                    let width = window.text_system().shape_line(self.text.clone(), px(14.0), &[run], None).width.as_f32();
                    self.sweep = Sweep::measure(width, row_width);
                    self.measured = Some(key);
                    self.started = Instant::now();
                }
                // Not laid out yet: measure on the next frame.
                None => window.request_animation_frame(),
            }
        }
        let color = if self.dim { self.colors.1 } else { self.colors.0 };
        let mut offset = 0.0;
        if let Some(sweep) = self.sweep.filter(|_| !motion.reduced) {
            let elapsed = motion.elapsed(self.started, None);
            offset = marquee_offset(elapsed, sweep.duration, sweep.distance as f64) as f32;
            if motion.wants_frames() {
                match marquee_still_for(elapsed, sweep.duration) {
                    // Holding: nothing moves until the hold ends.
                    Some(still) if !self.waiting => {
                        self.waiting = true;
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(still).await;
                            let _ = this.update(cx, |this, cx| {
                                this.waiting = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                    Some(_) => {}
                    None => window.request_animation_frame(),
                }
            }
        }
        let scrolling = self.sweep.is_some();
        let backdrop = self.backdrop;
        div()
            .relative()
            .w_full()
            .h(px(20.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .font_text()
            .type_size(14.0, 20.0)
            .font_weight(if self.dim { FontWeight::NORMAL } else { FontWeight::SEMIBOLD })
            .text_color(color)
            .child(probe)
            .child(div().absolute().top_0().left(px(offset)).when(scrolling, |t| t.px(px(6.0))).child(self.text.clone()))
            // mask-image: linear-gradient(to right, transparent, #000 6px, #000 calc(100% − 12px),
            // transparent). Reduced motion stops the animation but keeps the padding and mask.
            .when(scrolling, |row| {
                row.child(div().absolute().top_0().bottom_0().left_0().w(px(6.0)).bg(linear_gradient(90.0, linear_color_stop(backdrop, 0.0), linear_color_stop(backdrop.alpha(0.0), 1.0))))
                    .child(div().absolute().top_0().bottom_0().right_0().w(px(12.0)).bg(linear_gradient(90.0, linear_color_stop(backdrop.alpha(0.0), 0.0), linear_color_stop(backdrop, 1.0))))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_like_set_title() {
        // The reference's long title: 580.75 px of text in a 195 px row.
        let sweep = Sweep::measure(580.75, 195.0).unwrap();
        assert_eq!(sweep.overflow, 386.0);
        assert_eq!(sweep.distance, -404.0);
        assert!((sweep.duration.as_secs_f32() - 23.3).abs() < 1e-4);
        assert!(Sweep::measure(194.6, 195.0).is_none());
        assert_eq!(Sweep::measure(200.0, 195.0).unwrap().duration, Duration::from_secs(6));
    }
}
