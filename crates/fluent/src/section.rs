//! The Settings and Stream information windows' text blocks: group headings, the setting card,
//! description lists and the state badge (style.css "Settings", "The Stream information
//! window"). Functions returning divs; the window crate fills them. Single-line texts are
//! [`Line`]s (Chromium's baseline, ellipsis and letter-spacing).

use crate::color::Color;
use crate::surface::{surface, Border, Frame};
use crate::text::{line, spaced_text, Line, TextStyles};
use crate::theme::Theme;
use gpui::prelude::FluentBuilder;
use gpui::{div, px, Div, ParentElement, SharedString, Styled};

/// `.settings-group`: semibold 14/20, margin 14 0 6 (`first`: no top margin).
pub fn settings_group(text: impl Into<SharedString>, first: bool, theme: &Theme) -> Div {
    div().font_text().semibold().type_size(14.0, 20.0).text_color(theme.text).mt(px(if first { 0.0 } else { 14.0 })).mb(px(6.0)).child(line(text))
}

/// `.setting`: the card (radius 6, the island look), a row with gap 14, min-height 56,
/// padding 8 16, 3 px below. Children: [`setting_icon`], [`setting_text`], the control.
pub fn setting_card(theme: &Theme) -> Div {
    surface(Frame::setting(theme)).flex().flex_row().items_center().gap(px(14.0)).min_h(px(56.0)).mb(px(3.0)).py(px(8.0)).px(px(16.0))
}

/// `.setting-icon`: 18 px in `--text` (opacity 0.45 when the setting is disabled).
pub fn setting_icon(icon: crate::Icon, disabled: bool, theme: &Theme) -> Div {
    crate::icon(icon, 18.0).text_color(theme.text).when(disabled, |i| i.opacity(0.45))
}

/// `.setting-text`: the title (14/20, one line) over the description (12/16 `--text-2`, or
/// `--error`; it may wrap).
pub fn setting_text(title: impl Into<SharedString>, description: impl Into<SharedString>, error: bool, disabled: bool, theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .font_text()
        .child(div().type_size(14.0, 20.0).text_color(theme.text).when(disabled, |d| d.opacity(0.45)).child(line(title)))
        .child(div().type_size(12.0, 16.0).text_color(if error { theme.error } else { theme.text_2 }).child(description.into()))
}

/// `.info-card`: the island look with padding 12 14 14.
pub fn info_card(theme: &Theme) -> Div {
    surface(Frame::island(theme)).pt(px(12.0)).px(px(14.0)).pb(px(14.0))
}

/// `.info-card h2`: semibold 11/16, uppercase, letter-spacing 0.06 em, accent, 8 px below.
pub fn info_heading(text: &str, theme: &Theme) -> Div {
    div().mb(px(8.0)).child(spaced_text(text.to_uppercase(), 11.0 * 0.06).font_text().semibold().type_size(11.0, 16.0).text_color(theme.accent))
}

/// `.info-card dt`: 12/16 `--text-2` (13/20 in the ICY pairs).
pub fn info_label(text: impl Into<SharedString>, pairs: bool, theme: &Theme) -> Div {
    let (size, line_height) = if pairs { (13.0, 20.0) } else { (12.0, 16.0) };
    div().font_text().type_size(size, line_height).text_color(theme.text_2).child(line(text))
}

/// `.info-card dd`'s text: semibold 15/20 tabular (13/20 regular in the ICY pairs), one line,
/// cut with Chromium's ellipsis, as wide as its box. `selection`: highlighted (background, text).
/// Put it in [`info_value`] (or a box of the caller's) to get the margin.
pub fn info_value_line(text: impl Into<SharedString>, pairs: bool, selection: Option<(Color, Color)>, theme: &Theme) -> Line {
    let value = line(text).ellipsis().w_full().font_text().tabular_nums().text_color(theme.text).selected(selection);
    if pairs {
        value.regular().type_size(13.0, 20.0)
    } else {
        value.semibold().type_size(15.0, 20.0)
    }
}

/// `.info-card dd`: the value's line, 7 px below unless `last` (no margin in the ICY pairs).
/// The value is selectable and has its text as tooltip: the window crate adds those.
pub fn info_value(text: impl Into<SharedString>, pairs: bool, last: bool, theme: &Theme) -> Div {
    div().w_full().when(!pairs && !last, |d| d.mb(px(7.0))).child(info_value_line(text, pairs, None, theme))
}

/// `.info-state`: the pill (padding 2 10, radius 11, 1 px `--control-stroke`, 12/16 `--text-2`;
/// "Playing" in accent, text and border).
pub fn state_badge(text: impl Into<SharedString>, playing: bool, theme: &Theme) -> Div {
    let color = if playing { theme.accent } else { theme.text_2 };
    let border = if playing { theme.accent } else { theme.control_stroke };
    surface(Frame::new(11.0).border(Border::all(border))).py(px(2.0)).px(px(10.0)).font_text().type_size(12.0, 16.0).text_color(color).child(line(text))
}

/// `.channel`: the MONO / STEREO badge (9 px uppercase, letter-spacing 0.04 em, 1 px border,
/// radius 3, padding 0 4; lit: accent text and border; unlit: `--text-disabled` on
/// `--control-stroke`).
pub fn channel_badge(text: &str, lit: bool, theme: &Theme) -> Div {
    let (color, border) = if lit { (theme.accent, theme.accent) } else { (theme.text_disabled, theme.control_stroke) };
    surface(Frame::new(3.0).border(Border::all(border)))
        .px(px(4.0))
        .font_text()
        .type_size(9.0, 14.0)
        .text_color(color)
        .child(spaced_text(text.to_uppercase(), 9.0 * 0.04).relative())
}
