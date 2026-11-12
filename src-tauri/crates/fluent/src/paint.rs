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
