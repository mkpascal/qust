//! Small pieces the panels needed that the shared crates don't have (yet). Each is a candidate to
//! move to `qust-fluent` once another window needs it.

use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, point, px, App, Div, ElementId, FontFeatures, FontWeight, InteractiveElement, IntoElement, ParentElement, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    TextRun,
};
use qust_fluent::scrollbar::{Scrollbar, ScrollbarKind};
use qust_fluent::text::{FONT_DISPLAY, FONT_TEXT};
use qust_fluent::{Color, TextStyles, Theme};
use std::sync::Arc;

/// The panels' scrolling body (`body.settings-window { overflow-y: auto }`): the content in a
/// vertical scroll area and, while it overflows, the 8 px classic scrollbar beside it, taking its
/// width from the content as Chromium's does (the Settings list is 512 wide, not 520).
///
/// Whether it overflows is only known after layout: `overflow` is the caller's current belief and
/// `on_measured` is called (during prepaint) when the last layout says otherwise, so the caller
/// stores the new value and re-renders once. Candidate for `qust-fluent` (the `Scrollbar` there
/// only draws the thumb).
pub fn scroll_body(id: impl Into<ElementId>, handle: &ScrollHandle, overflow: bool, content: impl IntoElement, on_measured: impl Fn(bool, &mut App) + 'static) -> Div {
    let id: ElementId = id.into();
    let probe_handle = handle.clone();
    div()
        .size_full()
        .relative()
        .flex()
        .flex_row()
        .child(div().id(id.clone()).flex_1().min_w_0().h_full().overflow_y_scroll().track_scroll(handle).child(div().flex_none().w_full().child(content)))
        .when(overflow, |body| body.child(Scrollbar::new((id.clone(), "scrollbar"), handle, ScrollbarKind::Window)))
        // After the scroll area's prepaint (its maximum offset is known): a sibling painted later.
        .child(
            canvas(
                move |_, _, cx| {
                    let laid_out = probe_handle.bounds().size.height > px(0.0);
                    let over = probe_handle.max_offset().y > px(0.5);
                    if laid_out && over != overflow {
                        on_measured(over, cx);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0(),
        )
}

/// `kern` on. GPUI's DirectWrite shaping attaches an `IDWriteTypography` with liga / clig / calt
/// (and any features asked for), and DirectWrite then applies only those: kerning, which Chromium
/// applies by default, is off unless asked for.
pub fn kerning() -> FontFeatures {
    FontFeatures(Arc::new(vec![("kern".into(), 1)]))
}

/// `font-variant-numeric: tabular-nums` with kerning (`TextStyles::tabular_nums` asks for `tnum`
/// alone, which turns kerning off: see [`kerning`]).
pub fn tabular_kerning() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1), ("kern".into(), 1)]))
}

/// The ascent and descent GPUI lays a line out with, for the page's text or display font.
fn metrics(display: bool, size: f32, cx: &App) -> (f32, f32) {
    let family = if display { FONT_DISPLAY } else { FONT_TEXT };
    let system = cx.text_system();
    let id = system.resolve_font(&gpui::font(family));
    (system.ascent(id, px(size)).as_f32(), system.descent(id, px(size)).as_f32().abs())
}

/// Where Chromium puts the baseline in a line box of `line` px (from its top): ascent and descent
/// rounded to whole pixels (Skia's metrics on Windows), then half the leading, rounded down.
fn chromium_baseline(a: f32, d: f32, line: f32) -> f32 {
    a.round() + ((line - a.round() - d.round()) / 2.0).floor()
}

/// How far GPUI's baseline must move to sit where Chromium's does, in whole pixels.
///
/// GPUI centres the font's exact ascent + descent in the line box and rounds the baseline to a
/// device pixel; Chromium rounds ascent and descent first. For Segoe UI Variable Text that puts
/// 14/20 text (setting titles, combos, buttons, toggle labels) and 11/16 text (the info card
/// headings) 1 px lower in GPUI; 12/16, 12/20, 13/20 and 15/20 land on the same pixel. A fix for
/// `qust_fluent::TextStyles::type_size` (every 14 px text of the foundation's elements is 1 px low
/// too); applied here to the panels' own texts.
pub fn baseline_shift(size: f32, line: f32, display: bool, cx: &App) -> f32 {
    let (a, d) = metrics(display, size, cx);
    let gpui = ((line - a - d) / 2.0 + a).round();
    chromium_baseline(a, d, line) - gpui
}

/// A text block moved by [`baseline_shift`] (nothing else moves: `relative` offsets don't affect
/// layout).
pub fn on_chromium_baseline(text: Div, size: f32, line: f32, display: bool, cx: &App) -> Div {
    let shift = baseline_shift(size, line, display, cx);
    if shift == 0.0 {
        text
    } else {
        text.relative().top(px(shift))
    }
}

/// `.setting-text` (the title over the description), with the title on Chromium's baseline.
/// `qust_fluent::section::setting_text` otherwise.
pub fn setting_text(title: impl Into<SharedString>, description: impl Into<SharedString>, error: bool, disabled: bool, theme: &Theme, cx: &App) -> Div {
    let title = div().type_size(14.0, 20.0).text_color(theme.text).when(disabled, |d| d.opacity(0.45)).child(title.into());
    let description = div().type_size(12.0, 16.0).text_color(if error { theme.error } else { theme.text_2 }).child(description.into());
    div().flex_1().min_w_0().font_text().child(on_chromium_baseline(title, 14.0, 20.0, false, cx)).child(on_chromium_baseline(description, 12.0, 16.0, false, cx))
}

/// `.settings-group` on Chromium's baseline.
pub fn settings_group(text: &str, first: bool, theme: &Theme, cx: &App) -> Div {
    on_chromium_baseline(qust_fluent::section::settings_group(text.to_string(), first, theme), 14.0, 20.0, false, cx)
}

/// One line of text with CSS `letter-spacing`, painted glyph by glyph: the line is shaped whole
/// (kerning kept, as Chromium keeps it), then every glyph after the n-th character moves right by
/// n × `spacing`, at fractional positions, on Chromium's baseline.
///
/// `qust_fluent::text::spaced_text` puts each character in its own box followed by a margin of
/// `spacing`; GPUI rounds authored lengths, so 0.66 px becomes 1 px (the info headings come out
/// 2–4 px wider) and 0.36 px becomes 0 (the MONO / STEREO badges lose their spacing), and kerning
/// is lost. Candidate to replace it.
pub fn spaced_line(text: impl Into<SharedString>, size: f32, line: f32, weight: FontWeight, spacing: f32, color: Color) -> impl IntoElement {
    let text: SharedString = text.into();
    let chars: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    canvas(
        move |_, window, _| {
            let mut font = gpui::font(FONT_TEXT);
            font.weight = weight;
            font.features = kerning();
            let run = TextRun { len: text.len(), font, color: color.into(), background_color: None, underline: None, strikethrough: None };
            let layout = window.text_system().layout_line(text.as_ref(), px(size), &[run], None);
            (layout, chars.clone())
        },
        move |bounds, (layout, chars), window, _| {
            let baseline = chromium_baseline(layout.ascent.as_f32(), layout.descent.as_f32().abs(), line);
            for run in &layout.runs {
                for glyph in &run.glyphs {
                    let n = chars.iter().position(|i| *i == glyph.index).unwrap_or(0) as f32;
                    let origin = point(bounds.origin.x + glyph.position.x + px(n * spacing), bounds.origin.y + px(baseline));
                    let _ = window.paint_glyph(origin, run.font_id, glyph.id, px(size), color.into());
                }
            }
        },
    )
    .w_full()
    .h(px(line))
}

/// How a [`truncated_line`] looks.
#[derive(Clone)]
pub struct LineStyle {
    pub display: bool,
    pub weight: FontWeight,
    pub features: FontFeatures,
    pub size: f32,
    pub line: f32,
    pub color: Color,
    /// A selection highlight behind the shown text: (background, text color).
    pub highlight: Option<(Color, Color)>,
}

/// Where Chromium's `text-overflow: ellipsis` cuts `text` for `available` px: after the last
/// character whose end (its kerned position in the whole line) fits in `available` minus the
/// ellipsis, keeping spaces and punctuation. GPUI's own ellipsis cuts one character earlier
/// (it measures characters one by one, unkerned) and then trims trailing spaces and punctuation
/// ("supported, …" in Chromium is "supported…" in GPUI).
pub fn chromium_cut(text: &str, glyph_starts: &[(usize, f32)], width: f32, ellipsis: f32, available: f32) -> Option<usize> {
    if width <= available + 0.01 {
        return None;
    }
    let limit = available - ellipsis;
    let start_of = |index: usize| -> f32 {
        if index >= text.len() {
            return width;
        }
        glyph_starts.iter().filter(|(i, _)| *i >= index).map(|(_, x)| *x).fold(width, f32::min)
    };
    let mut cut = 0;
    for (index, c) in text.char_indices() {
        let end = index + c.len_utf8();
        if start_of(end) <= limit + 0.01 {
            cut = end;
        } else {
            break;
        }
    }
    Some(cut)
}

/// One line of text cut with an ellipsis the way Chromium cuts it ([`chromium_cut`]), painted on
/// Chromium's baseline. Fills its parent's width; `line` tall.
pub fn truncated_line(text: impl Into<SharedString>, style: LineStyle) -> impl IntoElement {
    let text: SharedString = text.into();
    let line = style.line;
    canvas(
        move |bounds, window, _| {
            let mut font = gpui::font(if style.display { FONT_DISPLAY } else { FONT_TEXT });
            font.weight = style.weight;
            font.features = style.features.clone();
            let color = style.highlight.map_or(style.color, |(_, text)| text);
            let run = |len: usize| TextRun { len, font: font.clone(), color: color.into(), background_color: None, underline: None, strikethrough: None };
            let system = window.text_system().clone();
            let size = px(style.size);
            let full = system.layout_line(text.as_ref(), size, &[run(text.len())], None);
            let starts: Vec<(usize, f32)> = full.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.index, g.position.x.as_f32()))).collect();
            let ellipsis = system.layout_line("\u{2026}", size, &[run(3)], None).width.as_f32();
            let shown: SharedString = match chromium_cut(&text, &starts, full.width.as_f32(), ellipsis, bounds.size.width.as_f32()) {
                Some(cut) => format!("{}\u{2026}", &text[..cut]).into(),
                None => text.clone(),
            };
            let shaped = system.shape_line(shown.clone(), size, &[run(shown.len())], None);
            let (a, d) = (full.ascent.as_f32(), full.descent.as_f32().abs());
            let gpui_baseline = ((line - a - d) / 2.0 + a).round();
            (shaped, chromium_baseline(a, d, line) - gpui_baseline)
        },
        move |bounds, (shaped, shift), window, cx| {
            if let Some((background, _)) = style.highlight {
                let width = shaped.width().min(bounds.size.width);
                window.paint_quad(gpui::fill(gpui::Bounds::new(bounds.origin, gpui::size(width, bounds.size.height)), background));
            }
            let _ = shaped.paint(point(bounds.origin.x, bounds.origin.y + px(shift)), px(line), gpui::TextAlign::Left, None, window, cx);
        },
    )
    .w_full()
    .h(px(line))
}

/// The selection highlight of a selected value (Chromium's default `::selection` on Windows: the
/// system highlight color, white text). Not in any reference render: the page never styled it.
pub fn selection_colors() -> (Color, Color) {
    (Color::rgba(0, 120, 215, 1.0), Color::WHITE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_baselines() {
        // Segoe UI Variable Text: ascent 2210 / descent 514 per 2048.
        let at = |size: f32, line: f32| chromium_baseline(size * 2210.0 / 2048.0, size * 514.0 / 2048.0, line);
        assert_eq!(at(14.0, 20.0), 15.0);
        assert_eq!(at(12.0, 16.0), 13.0);
        assert_eq!(at(11.0, 16.0), 12.0);
        assert_eq!(at(15.0, 20.0), 16.0);
    }

    #[test]
    fn cut_like_chromium() {
        // Four 10 px characters "ab, " then "cd"; an ellipsis of 6.
        let text = "ab, cd";
        let starts: Vec<(usize, f32)> = (0..6).map(|i| (i, i as f32 * 10.0)).collect();
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 60.0), None);
        // 46 px: 40 for text → "ab, " kept whole, trailing space included.
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 46.0), Some(4));
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 45.0), Some(3));
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 5.0), Some(0));
    }
}
