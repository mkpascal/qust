//! The playlist's list row (`.row`) and its parts, as functions returning divs, so the playlist
//! adds its own handlers (selection, drag, double-click) and builds rows cheaply in a virtual
//! list.
//!
//! Row: 26 tall, radius 4, padding 0 10 0 4, gap 10, 13/18. Hover `--subtle-hover`; selected
//! `--row-selected` (hovered: `--row-selected-hover`) with Fluent's pill: 3 × 12 accent at the
//! left edge (top/bottom 7), radius 2. Number column 34 px, right-aligned, 12 px `--text-3`
//! tabular (cartridge rows: a 14 px icon in `--text-2`, even on the active row:
//! `.row.is-cartridge .row-number` comes after `.row.is-active .row-number` in style.css).
//! Title: flex, one line, ellipsis.
//! Detail: 12 px `--text-3` tabular, at most 40 % of the row, ellipsis. Active (the active
//! cartridge or entry): title and number in accent, semibold. Missing: title `--text-disabled`,
//! struck through. Playing: the number is replaced by the accent equalizer (10 px tall, margin
//! right 2). Drop line: 2 px accent, radius 1, inset 6 px left and right, at the top (−1 px) of
//! the row or at its bottom (−1 px) for the end.
//!
//! GPUI difference to know: GPUI suppresses hover styles while the last input was the keyboard
//! (Chromium keeps the row under the pointer highlighted). Forced hover (captures) is exact.

use crate::state::Forced;
use crate::text::{line, strikethrough, TextStyles};
use crate::theme::Theme;
use gpui::prelude::FluentBuilder;
use gpui::{div, px, relative, Div, ElementId, InteractiveElement, ParentElement, SharedString, Stateful, Styled};

/// The row box with its hover / selected fill and pill. Add the number, title and detail.
pub fn row(id: impl Into<ElementId>, selected: bool, forced: Forced, theme: &Theme) -> Stateful<Div> {
    let (fill, hover) = if selected { (theme.row_selected, theme.row_selected_hover) } else { (crate::Color::TRANSPARENT, theme.subtle_hover) };
    let fill = if forced.hover { hover } else { fill };
    div()
        .id(id)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(26.0))
        .pl(px(4.0))
        .pr(px(10.0))
        .rounded(px(4.0))
        .font_text()
        .type_size(13.0, 18.0)
        .text_color(theme.text)
        .bg(fill)
        .when(!forced.hover, |r| r.hover(move |s| s.bg(hover)))
        .when(selected, |r| r.child(pill(theme)))
}

/// `.row.is-selected::before`.
pub fn pill(theme: &Theme) -> Div {
    div().absolute().top(px(7.0)).bottom(px(7.0)).left_0().w(px(3.0)).rounded(px(2.0)).bg(theme.accent)
}

/// `.row.drop-before::after` (`after == false`) or `.row.drop-after::after`.
pub fn drop_line(after: bool, theme: &Theme) -> Div {
    div()
        .absolute()
        .left(px(6.0))
        .right(px(6.0))
        .h(px(2.0))
        .rounded(px(1.0))
        .bg(theme.accent)
        .when(after, |d| d.bottom(px(-1.0)))
        .when(!after, |d| d.top(px(-1.0)))
}

/// `.row-number`: 34 px, right-aligned; put the index text, the cartridge icon or the
/// equalizer in it.
pub fn row_number(active: bool, cartridge: bool, theme: &Theme) -> Div {
    let color = if cartridge { theme.text_2 } else if active { theme.accent } else { theme.text_3 };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_end()
        .w(px(34.0))
        .text_size(px(12.0))
        .tabular_nums()
        .text_color(color)
        .when(active, |d| d.semibold())
}

/// `.row-title`: one line cut with Chromium's ellipsis, on Chromium's baseline.
pub fn row_title(text: impl Into<SharedString>, active: bool, missing: bool, theme: &Theme) -> Div {
    let color = if missing { theme.text_disabled } else if active { theme.accent } else { theme.text };
    let mut title = line(text).ellipsis().w_full().text_color(color);
    if active {
        title = title.semibold();
    }
    if missing {
        title.text_style().strikethrough = Some(strikethrough(color));
    }
    div().flex_1().min_w_0().h(px(18.0)).child(title)
}

/// `.row-detail`: as wide as its text (a fractional width, as Chromium's box), at most 40 % of
/// the row's content box, cut with Chromium's ellipsis.
pub fn row_detail(text: impl Into<SharedString>, theme: &Theme) -> crate::text::Line {
    line(text).ellipsis().flex_none().max_w(relative(0.4)).text_size(px(12.0)).tabular_nums().text_color(theme.text_3)
}

/// `.playlist-divider`: 1 px `--card-stroke` with a 1 px `--card-edge` line under it
/// (`box-shadow: 0 1px 0`), margin 4 8.
pub fn divider(theme: &Theme) -> Div {
    div().relative().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(theme.card_stroke).child(div().absolute().top(px(1.0)).left_0().right_0().h(px(1.0)).bg(theme.card_edge))
}
