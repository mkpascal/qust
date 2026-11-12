//! Boxes drawn the way Chromium draws the page's: a 1 px border (per-side colors allowed,
//! solid or dashed), a background clipped to the padding box (`background-clip: padding-box`:
//! nothing under a semi-transparent border) or not, and an optional inset 1 px ring inside the
//! border (`box-shadow: inset 0 0 0 1px …`, the islands' second, lighter edge).
//!
//! A GPUI quad has one border color and paints its background under the border, so a box is a
//! few absolutely positioned layers behind the content. The box itself keeps a real but
//! transparent 1 px border, so **layout is CSS's**: give it the page's `padding`, `gap` and
//! size, and children land where Chromium put them (border-box sizing, like the page's
//! `box-sizing: border-box`). Absolute children are placed from the padding box (inside the
//! border), as in CSS; the layers reach out over the border with negative insets.
//!
//! [`surface`] returns such a box (a `relative()` div) for a [`Frame`]; add children, padding,
//! flex and size as usual. Border widths are whole CSS px, so at 100 % / 200 % they land on
//! device pixels like Chromium's; at 125 % / 150 % GPUI and Chromium snap a little differently.

use crate::color::Color;
use crate::theme::Theme;
use gpui::{canvas, div, point, prelude::FluentBuilder, px, size, AnyElement, Bounds, Div, IntoElement, ParentElement, PathBuilder, Pixels, Styled, Window};

/// A 1 px border's colors, per side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub top: Color,
    pub right: Color,
    pub bottom: Color,
    pub left: Color,
}

impl Border {
    pub fn all(color: Color) -> Border {
        Border { top: color, right: color, bottom: color, left: color }
    }

    /// `border: 1px solid c; border-top-color: top`.
    pub fn top(color: Color, top: Color) -> Border {
        Border { top, ..Border::all(color) }
    }

    /// `border: 1px solid c; border-bottom-color: bottom`.
    pub fn bottom(color: Color, bottom: Color) -> Border {
        Border { bottom, ..Border::all(color) }
    }

    fn uniform(&self) -> bool {
        self.top == self.right && self.right == self.bottom && self.bottom == self.left
    }
}

/// What a box looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// `border-radius` of the border box.
    pub radius: f32,
    /// A 1 px border, or none.
    pub border: Option<Border>,
    pub dashed: bool,
    /// The background.
    pub fill: Option<Color>,
    /// `background-clip: padding-box` (the default here, as most of the page's boxes); false:
    /// the fill goes under the border too (`.textbox`, `.dialog`, `.toggle-track`).
    pub clip_to_padding: bool,
    /// `box-shadow: inset 0 0 0 1px ring`.
    pub ring: Option<Color>,
}

impl Frame {
    pub const fn new(radius: f32) -> Frame {
        Frame { radius, border: None, dashed: false, fill: None, clip_to_padding: true, ring: None }
    }

    pub fn border(mut self, border: Border) -> Frame {
        self.border = Some(border);
        self
    }

    pub fn dashed(mut self, color: Color) -> Frame {
        self.border = Some(Border::all(color));
        self.dashed = true;
        self
    }

    pub fn fill(mut self, fill: Color) -> Frame {
        self.fill = Some(fill);
        self
    }

    /// The fill under the border too.
    pub fn fill_border_box(mut self) -> Frame {
        self.clip_to_padding = false;
        self
    }

    pub fn ring(mut self, ring: Color) -> Frame {
        self.ring = Some(ring);
        self
    }

    /// `.island` (and `.cartridge`, `.info-card`): radius 8, `--card-stroke` border, `--card`
    /// fill in the padding box, `--card-edge` inset ring.
    pub fn island(theme: &Theme) -> Frame {
        Frame::new(8.0).border(Border::all(theme.card_stroke)).fill(theme.card).ring(theme.card_edge)
    }

    /// `.setting`: the island look at radius 6.
    pub fn setting(theme: &Theme) -> Frame {
        Frame { radius: 6.0, ..Frame::island(theme) }
    }

    fn border_width(&self) -> f32 {
        if self.border.is_some() {
            1.0
        } else {
            0.0
        }
    }

    /// The layers, back to front, positioned from the padding box of a box whose layout
    /// border is this frame's: the fill, the ring (painted over the fill as the inset shadow
    /// is), then the border.
    pub fn layers(&self) -> Vec<AnyElement> {
        let mut layers = Vec::with_capacity(3);
        let border = self.border_width();
        let padding_radius = (self.radius - border).max(0.0);
        if let Some(fill) = self.fill {
            let layer = if self.clip_to_padding { inset_layer(0.0, padding_radius) } else { inset_layer(-border, self.radius) };
            layers.push(layer.bg(fill).into_any_element());
        }
        if let Some(ring) = self.ring {
            layers.push(inset_layer(0.0, padding_radius).border_1().border_color(ring).into_any_element());
        }
        if let Some(colors) = self.border {
            layers.extend(border_layers(self.radius, colors, self.dashed, -border));
        }
        layers
    }
}

/// An absolute layer `inset` px inside the padding box (negative: outside it), with corner
/// radius `radius`.
pub fn inset_layer(inset: f32, radius: f32) -> Div {
    div().absolute().top(px(inset)).left(px(inset)).right(px(inset)).bottom(px(inset)).rounded(px(radius))
}

/// A 1 px border drawn as layers at `inset` from the padding box (−1 for a box with a 1 px
/// layout border): one quad when every side has the same color, else one quad per color with
/// the other sides at width 0 (GPUI tapers a corner between a side of width 1 and one of width
/// 0, which is how Chromium splits a corner between two colors).
pub fn border_layers(radius: f32, border: Border, dashed: bool, inset: f32) -> Vec<AnyElement> {
    let layer = || inset_layer(inset, radius);
    if dashed {
        // Chromium's dashes ([`dashed_border`]); the page's dashed borders are one color.
        if border.top.a <= 0.0 {
            return Vec::new();
        }
        return vec![dashed_border(radius, border.top).absolute().top(px(inset)).left(px(inset)).right(px(inset)).bottom(px(inset)).into_any_element()];
    }
    if border.uniform() {
        if border.top.a <= 0.0 {
            return Vec::new();
        }
        return vec![layer().border_1().border_color(border.top).into_any_element()];
    }
    let mut groups: Vec<(Color, [bool; 4])> = Vec::new();
    for (side, color) in [border.top, border.right, border.bottom, border.left].into_iter().enumerate() {
        match groups.iter_mut().find(|(c, _)| *c == color) {
            Some((_, sides)) => sides[side] = true,
            None => {
                let mut sides = [false; 4];
                sides[side] = true;
                groups.push((color, sides));
            }
        }
    }
    groups
        .into_iter()
        .filter(|(color, _)| color.a > 0.0)
        .map(|(color, [top, right, bottom, left])| {
            layer()
                .when(top, |l| l.border_t_1())
                .when(right, |l| l.border_r_1())
                .when(bottom, |l| l.border_b_1())
                .when(left, |l| l.border_l_1())
                .border_color(color)
                .into_any_element()
        })
        .collect()
}

/// A 1 px dashed border of corner radius `radius` filling its bounds (the border box), drawn as
/// Chromium draws `border: 1px dashed` around a rounded box: the box snapped to device pixels,
/// its border's centre line as one closed path (Skia's `addRRect`: from the start of the top edge,
/// clockwise, corners included), dashed 3 × the width on and a gap near 2 × the width chosen so
/// the dashes fill the whole path evenly (Blink's `StrokeData::SelectBestDashGap`), butt ends.
/// GPUI's own `border_dashed` uses another pattern (and restarts it per side).
pub fn dashed_border(radius: f32, color: Color) -> gpui::Canvas<()> {
    canvas(|_, _, _| {}, move |bounds, _, window, _| paint_dashed(bounds, radius, color, window))
}

/// The dash and gap lengths for a closed path of `length` (Blink's `SelectBestDashGap`).
pub fn dash_gap(length: f32, dash: f32, gap: f32) -> f32 {
    let min = (length / (dash + gap)).floor().max(1.0);
    let max = min + 1.0;
    let min_gap = (length - min * dash) / min;
    let max_gap = (length - max * dash) / max;
    if max_gap <= 0.0 || (min_gap - gap).abs() < (max_gap - gap).abs() {
        min_gap
    } else {
        max_gap
    }
}

fn paint_dashed(bounds: Bounds<Pixels>, radius: f32, color: Color, window: &mut Window) {
    let scale = window.scale_factor();
    // The border box snapped to device pixels (round half up, as Chromium's pixel snapping).
    let snap = |v: f32| (v * scale + 0.5).floor() / scale;
    let (left, top) = (snap(bounds.origin.x.as_f32()), snap(bounds.origin.y.as_f32()));
    let (right, bottom) = (snap(bounds.right().as_f32()), snap(bounds.bottom().as_f32()));
    let width = (scale.floor().max(1.0)) / scale;
    let half = width / 2.0;
    let (l, t, r, b) = (left + half, top + half, right - half, bottom - half);
    let rad = (radius - half).max(0.0).min((r - l) / 2.0).min((b - t) / 2.0);
    let (straight_x, straight_y) = ((r - l - 2.0 * rad).max(0.0), (b - t - 2.0 * rad).max(0.0));
    let arc = std::f32::consts::FRAC_PI_2 * rad;
    let length = 2.0 * straight_x + 2.0 * straight_y + 4.0 * arc;
    if length <= 0.0 {
        return;
    }
    let dash = 3.0 * width;
    if length <= dash * 2.0 {
        return;
    }
    let gap = dash_gap(length, dash, 2.0 * width);
    // The path, clockwise from the start of the top edge: (kind, length): 0 line, 1 arc.
    // Lines: start point and direction; arcs: centre and start angle.
    struct Piece {
        len: f32,
        line: Option<((f32, f32), (f32, f32))>,
        arc: Option<((f32, f32), f32)>,
    }
    use std::f32::consts::{FRAC_PI_2, PI};
    let pieces = [
        Piece { len: straight_x, line: Some(((l + rad, t), (1.0, 0.0))), arc: None },
        Piece { len: arc, line: None, arc: Some(((r - rad, t + rad), -FRAC_PI_2)) },
        Piece { len: straight_y, line: Some(((r, t + rad), (0.0, 1.0))), arc: None },
        Piece { len: arc, line: None, arc: Some(((r - rad, b - rad), 0.0)) },
        Piece { len: straight_x, line: Some(((r - rad, b), (-1.0, 0.0))), arc: None },
        Piece { len: arc, line: None, arc: Some(((l + rad, b - rad), FRAC_PI_2)) },
        Piece { len: straight_y, line: Some(((l, b - rad), (0.0, -1.0))), arc: None },
        Piece { len: arc, line: None, arc: Some(((l + rad, t + rad), PI)) },
    ];
    let mut start = 0.0f32;
    let mut on = 0.0f32;
    while on < length {
        let off = (on + dash).min(length);
        // The parts of [on, off) on each piece.
        let mut at = start;
        for piece in &pieces {
            let (from, to) = (on.max(at), off.min(at + piece.len));
            if to > from && piece.len > 0.0 {
                let (a, z) = (from - at, to - at);
                if let Some(((x, y), (dx, dy))) = piece.line {
                    // A straight dash: a quad across the stroke's width.
                    let (x0, y0) = (x + dx * a, y + dy * a);
                    let (x1, y1) = (x + dx * z, y + dy * z);
                    let (minx, maxx) = (x0.min(x1) - if dy != 0.0 { half } else { 0.0 }, x0.max(x1) + if dy != 0.0 { half } else { 0.0 });
                    let (miny, maxy) = (y0.min(y1) - if dx != 0.0 { half } else { 0.0 }, y0.max(y1) + if dx != 0.0 { half } else { 0.0 });
                    window.paint_quad(gpui::fill(Bounds::new(point(px(minx), px(miny)), size(px(maxx - minx), px(maxy - miny))), color));
                } else if let Some(((cx, cy), angle)) = piece.arc {
                    let mut path = PathBuilder::stroke(px(width));
                    let steps = (((z - a) / 0.5).ceil() as usize).max(2);
                    for i in 0..=steps {
                        let s = a + (z - a) * i as f32 / steps as f32;
                        let theta = angle + s / rad;
                        let p = point(px(cx + rad * theta.cos()), px(cy + rad * theta.sin()));
                        if i == 0 {
                            path.move_to(p);
                        } else {
                            path.line_to(p);
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
            }
            at += piece.len;
        }
        on += dash + gap;
        start = 0.0;
    }
}

/// A `relative()` div drawn as `frame`, with the frame's border in its layout.
pub fn surface(frame: Frame) -> Div {
    div().relative().when(frame.border.is_some(), |d| d.border_1().border_color(Color::TRANSPARENT)).children(frame.layers())
}

/// CSS `outline: <width>px solid color; outline-offset: offset` around a box of corner radius
/// `radius` whose layout border is `border` px, as an absolute layer to add as the box's last
/// child. Chromium's outline follows the border radius: its outer edge has radius
/// `radius + offset + width` (never below 0). The page's focus ring is [`focus_ring`]; the
/// cartridge body and eject strip use offset −2 (ring inside), the drop target offset 2 in
/// accent.
pub fn outline(radius: f32, border: f32, offset: f32, width: f32, color: Color) -> Div {
    let outset = offset + width;
    inset_layer(-(outset + border), (radius + outset).max(0.0)).border(px(width)).border_color(color)
}

/// The page's `:focus-visible` ring (2 px `--focus`, offset 1) around a box of radius `radius`
/// with a 1 px layout border (every bordered element here).
pub fn focus_ring(radius: f32, theme: &Theme) -> Div {
    outline(radius, 1.0, 1.0, 2.0, theme.focus)
}
