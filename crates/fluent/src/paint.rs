//! Small painting helpers shared by the elements.

use crate::color::Color;
use gpui::{canvas, point, px, App, BoxShadow, Bounds, ElementId, Entity, Hsla, IntoElement, Pixels, Styled, Window};

/// A CSS `box-shadow: x y blur spread color` as GPUI's. GPUI's `blur_radius` is the Gaussian's
/// sigma, while CSS's blur radius is twice the sigma (CSS Backgrounds 3: "a Gaussian blur with a
/// standard deviation equal to half the blur radius"): the blur is halved here.
pub fn css_shadow(x: f32, y: f32, blur: f32, spread: f32, color: Color) -> BoxShadow {
    BoxShadow { color: Hsla::from(color), offset: point(px(x), px(y)), blur_radius: px(blur / 2.0), spread_radius: px(spread), inset: false }
}

/// The same, `inset`.
pub fn css_inset_shadow(x: f32, y: f32, blur: f32, spread: f32, color: Color) -> BoxShadow {
    BoxShadow { inset: true, ..css_shadow(x, y, blur, spread, color) }
}

/// Where an element was laid out in the last frame (for menus opened under a combo, for
/// mapping a press to a slider value, …): `let (bounds, probe) = bounds_probe(id, window, cx)`,
/// add `probe` as an absolute child filling the element, read `bounds.read(cx)` in handlers.
pub fn bounds_probe(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> (Entity<Option<Bounds<Pixels>>>, impl IntoElement) {
    let memory = window.use_keyed_state(id, cx, |_, _| None);
    let writer = memory.clone();
    let probe = canvas(
        move |bounds, _, cx| {
            writer.update(cx, |stored, _| *stored = Some(bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();
    (memory, probe)
}

/// CSS `opacity` on a group (the dragged cartridge at 0.35, a disabled setting's toggle at 0.45):
/// the group is drawn, then composited at that opacity, so parts drawn over each other inside it
/// don't show through one another. GPUI applies opacity to each primitive instead (a faded
/// thumb would let the faded track show through it), so a faded group is drawn with its colors
/// flattened: what sits straight on the backdrop at `opacity`, and what sits on something else
/// in the group as the opaque color the group composites to over the backdrop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub opacity: f32,
    /// The opaque color behind the group.
    pub backdrop: Color,
}

impl Fade {
    pub fn new(opacity: f32, backdrop: Color) -> Fade {
        Fade { opacity, backdrop }
    }

    /// A part painted straight on the backdrop (the group's own fill, ring, border).
    pub fn layer(fade: Option<Fade>, color: Color) -> Color {
        match fade {
            Some(f) => color.mix(f.opacity),
            None => color,
        }
    }

    /// A part painted on `under` (a part of the group: text on a fill, a thumb on a track): the
    /// opaque color the group gives it.
    pub fn on(fade: Option<Fade>, color: Color, under: Color) -> Color {
        match fade {
            Some(f) => color.over(under).mix(f.opacity).over(f.backdrop),
            None => color,
        }
    }
}

/// Chromium's pixel snapping of an edge at `value` CSS px: to the nearest device pixel, halves
/// rounded up (GPUI snaps halves toward zero, so an edge laid out at x.5 lands a pixel lower or
/// further left than Chromium's). Use it for lengths that land on halves in the page's layout
/// (the cartridge grid's 86.5 px rows).
pub fn snap_half_up(value: f32, scale: f32) -> f32 {
    (value * scale + 0.5).floor() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_flattens_a_group() {
        let bg = Color::rgba(32, 32, 32, 1.0);
        let fill = Color::rgba(255, 255, 255, 0.5);
        // Straight on the backdrop: just faded.
        assert_eq!(Fade::layer(Some(Fade::new(0.5, bg)), fill), fill.mix(0.5));
        // On the fill: opaque, between the backdrop and the composited group.
        let text = Fade::on(Some(Fade::new(0.5, bg)), Color::WHITE, fill);
        assert_eq!(text.a, 1.0);
        assert_eq!(Fade::on(None, Color::WHITE, fill), Color::WHITE);
        assert_eq!(snap_half_up(92.5, 1.0), 93.0);
        assert_eq!(snap_half_up(102.5, 1.0), 103.0);
    }
}
