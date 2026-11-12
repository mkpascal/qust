//! The playing equalizer (`.equalizer` on a cartridge, `.row-eq` on a playlist row): three
//! bars 2 × 10 px, gap 2, radius 1, in the current color, each `scaleY(0.25 → 1)` from the
//! bottom, 0.9 s ease-in-out infinite alternate, phases 0 / −0.3 / −0.6 s. Under reduced motion
//! the animation is removed and the bars stand at full height (`animation: none`).
//!
//! It animates on every frame. In GPUI a view that notifies (or asks for the next frame) marks
//! every ancestor view dirty, and a dirty cached view re-renders its whole subtree: embedding
//! an animating view as a cached child of a cartridge or a row re-renders the grid or the list
//! at the frame rate. So a running equalizer is a **live layer** ([`EqualizerLayer`]): a sibling
//! of the window's content (`qust_ui::WindowRoot::set_live`), which reads where to draw from the
//! view that owns the row or cartridge, observes it, and paints at window coordinates, clipped
//! like the row. Only it and the window root re-render per frame.
//!
//! [`EqualizerView`] (a plain view) and [`equalizer`] (a div) remain for still pictures and
//! for places that re-render anyway.

use crate::color::Color;
use crate::motion::{equalizer_scale, motion};
use gpui::{canvas, div, fill, point, px, size, App, Bounds, ContentMask, Context, Div, Entity, IntoElement, ParentElement, Pixels, Render, Styled, Window};
use std::time::{Duration, Instant};

/// The bars after `elapsed` of the animation (`None`: reduced motion, full height).
pub fn equalizer(elapsed: Option<Duration>, color: Color) -> Div {
    div().flex().flex_row().items_end().gap(px(2.0)).w(px(10.0)).h(px(10.0)).children((0..3).map(|bar| {
        let scale = elapsed.map_or(1.0, |elapsed| equalizer_scale(elapsed, bar)) as f32;
        div().w(px(2.0)).h(px(10.0 * scale)).rounded(px(1.0)).bg(color)
    }))
}

/// Paints the bars in a 10 × 10 box at `origin` (window coordinates), each standing on the
/// bottom, its height whole px as Chromium's raster of the scaled bar.
pub fn paint_equalizer(origin: gpui::Point<Pixels>, elapsed: Option<Duration>, color: Color, window: &mut Window) {
    for bar in 0..3 {
        let scale = elapsed.map_or(1.0, |elapsed| equalizer_scale(elapsed, bar)) as f32;
        let height = (10.0 * scale).round();
        let bounds = Bounds::new(point(origin.x + px(4.0 * bar as f32), origin.y + px(10.0 - height)), size(px(2.0), px(height)));
        window.paint_quad(fill(bounds, color).corner_radii(px(1.0)));
    }
}

/// A running equalizer (a plain view: see the module docs before embedding it). `started`:
/// when the element appeared (the page's animation started with the `is-playing` class); set a
/// new one with [`EqualizerView::restart`].
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

/// Where a live equalizer draws this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqualizerPlacement {
    /// The 10 × 10 box, window coordinates.
    pub bars: Bounds<Pixels>,
    /// What clips it (the list's viewport), window coordinates.
    pub clip: Option<Bounds<Pixels>>,
    /// When its animation started.
    pub started: Instant,
    /// Whether it moves (a closed drawer's row stands still).
    pub animate: bool,
    /// The bars' color; None: the accent.
    pub color: Option<Color>,
}

type Placement = Box<dyn Fn(&App) -> Option<EqualizerPlacement>>;

/// An equalizer as a live layer of the window root (see the module docs): it observes `source`
/// (the view owning the row or cartridge) and asks it where to draw on every render.
pub struct EqualizerLayer {
    placement: Placement,
    /// Renders (measurements and tests).
    pub renders: u64,
}

impl EqualizerLayer {
    pub fn new<V: 'static>(source: &Entity<V>, place: impl Fn(&V, &App) -> Option<EqualizerPlacement> + 'static, cx: &mut Context<Self>) -> EqualizerLayer {
        cx.observe(source, |_, _, cx| cx.notify()).detach();
        let source = source.clone();
        EqualizerLayer { placement: Box::new(move |cx| place(source.read(cx), cx)), renders: 0 }
    }
}

impl Render for EqualizerLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let placement = (self.placement)(cx);
        let accent = crate::theme::theme(cx).accent;
        let motion = motion(cx);
        let elapsed = placement.map(|p| if motion.reduced { None } else { Some(motion.elapsed(p.started, None)) });
        if placement.is_some_and(|p| p.animate) && !motion.reduced && motion.wants_frames() {
            window.request_animation_frame();
        }
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (Some(p), Some(elapsed)) = (placement, elapsed) else { return };
                let color = p.color.unwrap_or(accent);
                match p.clip {
                    Some(clip) => window.with_content_mask(Some(ContentMask { bounds: clip }), |window| paint_equalizer(p.bars.origin, elapsed, color, window)),
                    None => paint_equalizer(p.bars.origin, elapsed, color, window),
                }
            },
        )
        .size_full()
    }
}
