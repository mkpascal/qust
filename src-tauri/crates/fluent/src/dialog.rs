//! The modal dialog (`.dialog-backdrop`, `.dialog`): Load / Edit cartridge, Add URL, Edit track.
//!
//! Backdrop: the whole window in `--smoke` (`rgba(0,0,0,.3)` in both themes), appearing at
//! once, swallowing the pointer (a click on it does nothing), centring the card. Card: 360 px
//! wide, 1 px `--dialog-stroke` border over a `--dialog` fill (no `background-clip`), radius 8,
//! `0 32px 64px rgba(0,0,0,.37), 0 2px 21px rgba(0,0,0,.37)`, content clipped. Opening:
//! `dialog-in` 0.25 s Fluent decelerate, from opacity 0 and scale 1.05. Closing is instant.
//!
//! GPUI can't scale an element with its text. The opening scale is applied to the card's
//! frame (border, fill, shadow grow about the centre) while the content stays at its final size;
//! opacity applies to everything. The scale is 1.0075 at 60 ms and under 1.002 from 125 ms
//! (`motion::dialog_in`), so text and fields sit at most ~1.4 px from where Chromium drew them
//! during the first 60 ms, and nowhere after.
//!
//! Pieces: [`backdrop`], [`DialogCard`] (frame + animation; add the body and footer), and the
//! body's parts: [`dialog_body`], [`dialog_title`], [`field`], [`dialog_error`],
//! [`dialog_footer`] (buttons `flex_1()`).

use crate::button::RefineStyle;
use crate::color::Color;
use crate::motion::{dialog_in, motion, timing};
use crate::paint::{bounds_probe, css_shadow};
use crate::surface::{border_layers, inset_layer, Border};
use crate::text::TextStyles;
use crate::theme::theme;
use gpui::{
    div, px, AnyElement, App, Div, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement, Styled,
    Window,
};
use std::time::Instant;

/// The backdrop: put it last in the window's root (it paints over everything before it) and the
/// card in it.
pub fn backdrop(cx: &App) -> gpui::Stateful<Div> {
    let theme = theme(cx);
    div()
        .id("dialog-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.smoke)
        .occlude()
}

#[derive(IntoElement)]
pub struct DialogCard {
    id: ElementId,
    opened_at: Instant,
    label: SharedString,
    children: Vec<AnyElement>,
    style: gpui::StyleRefinement,
}

impl Styled for DialogCard {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl DialogCard {
    /// `opened_at`: when the dialog opened (the animation runs from there). `label`: its title,
    /// the dialog's accessible name (`aria-labelledby="dialogTitle"`).
    pub fn new(id: impl Into<ElementId>, opened_at: Instant, label: impl Into<SharedString>) -> DialogCard {
        DialogCard { id: id.into(), opened_at, label: label.into(), children: Vec::new(), style: gpui::StyleRefinement::default() }
    }
}

impl ParentElement for DialogCard {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for DialogCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *theme(cx);
        let motion = motion(cx);
        let (opacity, scale) = if motion.reduced { (1.0, 1.0) } else { dialog_in(motion.elapsed(self.opened_at, Some(timing::DIALOG_IN))) };
        if motion.wants_frames() && opacity < 1.0 {
            window.request_animation_frame();
        }
        let (bounds, probe) = bounds_probe((self.id.clone(), "bounds"), window, cx);
        let height = bounds.read(cx).map_or(0.0, |b| b.size.height.as_f32());
        let grow_x = (scale as f32 - 1.0) * 360.0 / 2.0;
        let grow_y = (scale as f32 - 1.0) * height / 2.0;
        let shadow = Color::rgba(0, 0, 0, 94.0 / 255.0);
        // The frame covers the border box (the card has a 1 px layout border), grown about the
        // centre while opening.
        let frame = div()
            .absolute()
            .top(px(-1.0 - grow_y))
            .bottom(px(-1.0 - grow_y))
            .left(px(-1.0 - grow_x))
            .right(px(-1.0 - grow_x))
            .rounded(px(8.0 * scale as f32))
            .shadow(vec![css_shadow(0.0, 32.0, 64.0, 0.0, shadow), css_shadow(0.0, 2.0, 21.0, 0.0, shadow)])
            .child(inset_layer(0.0, 8.0 * scale as f32).bg(theme.dialog))
            .children(border_layers(8.0 * scale as f32, Border::all(theme.dialog_stroke), false, 0.0));
        div()
            .id(self.id)
            .relative()
            .w(px(360.0))
            .border_1()
            .border_color(Color::TRANSPARENT)
            .flex()
            .flex_col()
            .opacity(opacity as f32)
            .role(Role::Dialog)
            .aria_label(self.label)
            .child(probe)
            .child(frame)
            .children(self.children)
            .refine_style(&self.style)
    }
}

/// `.dialog-body`: padding 16 20 14, gap 8.
pub fn dialog_body() -> Div {
    div().relative().flex().flex_col().gap(px(8.0)).pt(px(16.0)).px(px(20.0)).pb(px(14.0))
}

/// `.dialog h2`: Display semibold 18/24, 2 px below.
pub fn dialog_title(text: impl Into<SharedString>, cx: &App) -> Div {
    div().font_display().semibold().type_size(18.0, 24.0).mb(px(2.0)).text_color(theme(cx).text).child(text.into())
}

/// `.field`: the label (13/18 `--text`) over the text box, gap 4.
pub fn field(label: impl Into<SharedString>, textbox: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .font_text()
        .type_size(13.0, 18.0)
        .text_color(theme(cx).text)
        .child(div().child(label.into()))
        .child(textbox)
}

/// `.dialog-error`: 12/16 `--error`.
pub fn dialog_error(text: impl Into<SharedString>, cx: &App) -> Div {
    div().font_text().type_size(12.0, 16.0).text_color(theme(cx).error).child(text.into())
}

/// `.dialog-footer`: padding 16 20, gap 8, a `--card-stroke` top border, `--dialog-footer` fill,
/// rounded at the bottom like the card's clip (inside its 1 px border). Give the buttons
/// `flex_1()`.
pub fn dialog_footer(cx: &App) -> Div {
    let theme = theme(cx);
    div()
        .relative()
        .flex()
        .flex_row()
        .gap(px(8.0))
        .py(px(16.0))
        .px(px(20.0))
        // Its background goes under its own top border (border-box), as in CSS.
        .border_t_1()
        .border_color(theme.card_stroke)
        .rounded_b(px(7.0))
        .bg(theme.dialog_footer)
}
