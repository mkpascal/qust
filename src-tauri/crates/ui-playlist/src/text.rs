//! Text as Chromium lays it out, where GPUI differs. Copied from `qust-ui-panels`'s `local.rs`
//! (found there first) and extended with strikethrough for missing tracks; **candidates for
//! `qust-fluent`** (`TextStyles`, `list::row_title` / `row_detail`, the status line):
//!
//! - **Kerning.** GPUI's DirectWrite shaping applies only the features it is given (liga, clig,
//!   calt, and e.g. `tnum`), so kerning, which Chromium applies by default, is off: the drawer
//!   asks for `kern` on its root (inherited), and `tnum` + `kern` where numbers are tabular.
//! - **Baseline.** Chromium rounds the font's ascent and descent before centring them in the
//!   line box; GPUI centres the exact values and rounds the baseline. For Segoe UI Variable Text
//!   that puts 11/16 text (the status line) 1 px lower in GPUI; 12/18, 13/18, 12/16 and 13/16
//!   land on the same pixel (measured on the drawer's captures).
//! - **Ellipsis.** GPUI's `text-overflow` cuts one character early (it measures characters one
//!   by one, unkerned) and trims trailing spaces and punctuation before the "…"; Chromium cuts
//!   after the last character whose kerned end fits and keeps them. Row titles and details
//!   truncate all the time, so they are drawn by [`truncated_line`].

use gpui::{canvas, point, px, App, FontFeatures, FontWeight, IntoElement, SharedString, StrikethroughStyle, Styled, TextRun, Window};
use qust_fluent::text::FONT_TEXT;
use qust_fluent::Color;
use std::sync::{Arc, OnceLock};

/// `kern` on (see the module docs).
pub fn kerning() -> FontFeatures {
    static FEATURES: OnceLock<FontFeatures> = OnceLock::new();
    FEATURES.get_or_init(|| FontFeatures(Arc::new(vec![("kern".into(), 1)]))).clone()
}

/// `font-variant-numeric: tabular-nums` with kerning (`tnum` alone turns kerning off).
pub fn tabular_kerning() -> FontFeatures {
    static FEATURES: OnceLock<FontFeatures> = OnceLock::new();
    FEATURES.get_or_init(|| FontFeatures(Arc::new(vec![("tnum".into(), 1), ("kern".into(), 1)]))).clone()
}

fn metrics(size: f32, cx: &App) -> (f32, f32) {
    let system = cx.text_system();
    let id = system.resolve_font(&gpui::font(FONT_TEXT));
    (system.ascent(id, px(size)).as_f32(), system.descent(id, px(size)).as_f32().abs())
}

/// Where Chromium puts the baseline in a line box of `line` px: ascent and descent rounded,
/// then half the leading, rounded down.
fn chromium_baseline(a: f32, d: f32, line: f32) -> f32 {
    a.round() + ((line - a.round() - d.round()) / 2.0).floor()
}

/// How far GPUI's baseline must move (whole px) to sit where Chromium's does, for the text font.
pub fn baseline_shift(size: f32, line: f32, cx: &App) -> f32 {
    let (a, d) = metrics(size, cx);
    chromium_baseline(a, d, line) - ((line - a - d) / 2.0 + a).round()
}

/// Where Chromium's ellipsis cuts `text` for `available` px (a byte index), or None when it fits:
/// after the last character whose kerned end fits in `available` minus the ellipsis.
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

/// How a [`truncated_line`] looks.
#[derive(Clone)]
pub struct LineStyle {
    pub weight: FontWeight,
    pub features: FontFeatures,
    pub size: f32,
    pub line: f32,
    pub color: Color,
    /// `text-decoration: line-through`.
    pub strikethrough: bool,
    /// For a box as wide as its text (the row detail): the width the text may take, used for the
    /// cut instead of the box's (GPUI rounds the box to whole px), and the text set against the
    /// box's right edge, where Chromium's fractional box ends.
    pub max_width: Option<f32>,
}

impl LineStyle {
    fn run(&self, len: usize) -> TextRun {
        let mut font = gpui::font(FONT_TEXT);
        font.weight = self.weight;
        font.features = self.features.clone();
        let strikethrough = self.strikethrough.then(|| StrikethroughStyle { thickness: px(1.0), color: Some(self.color.into()) });
        TextRun { len, font, color: self.color.into(), background_color: None, underline: None, strikethrough }
    }
}

/// The width a line of `text` takes (kerned), for boxes sized by their text (the row detail).
pub fn line_width(text: &str, style: &LineStyle, window: &Window) -> f32 {
    window.text_system().layout_line(text, px(style.size), &[style.run(text.len())], None).width.as_f32()
}

/// One line cut with an ellipsis the way Chromium cuts it, on Chromium's baseline; fills its
/// parent's width, `line` tall.
pub fn truncated_line(text: impl Into<SharedString>, style: LineStyle) -> impl IntoElement {
    let text: SharedString = text.into();
    let line = style.line;
    canvas(
        move |bounds, window, _| {
            let system = window.text_system().clone();
            let size = px(style.size);
            let full = system.layout_line(text.as_ref(), size, &[style.run(text.len())], None);
            let starts: Vec<(usize, f32)> = full.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.index, g.position.x.as_f32()))).collect();
            let ellipsis = system.layout_line("\u{2026}", size, &[style.run(3)], None).width.as_f32();
            let available = style.max_width.unwrap_or(bounds.size.width.as_f32());
            let shown: SharedString = match chromium_cut(&text, &starts, full.width.as_f32(), ellipsis, available) {
                Some(cut) => format!("{}\u{2026}", &text[..cut]).into(),
                None => text.clone(),
            };
            let shaped = system.shape_line(shown.clone(), size, &[style.run(shown.len())], None);
            let (a, d) = (full.ascent.as_f32(), full.descent.as_f32().abs());
            let gpui_baseline = ((line - a - d) / 2.0 + a).round();
            let x = if style.max_width.is_some() { bounds.size.width - shaped.width } else { px(0.0) };
            (shaped, chromium_baseline(a, d, line) - gpui_baseline, x)
        },
        move |bounds, (shaped, shift, x), window, cx| {
            let _ = shaped.paint(point(bounds.origin.x + x, bounds.origin.y + px(shift)), px(line), gpui::TextAlign::Left, None, window, cx);
        },
    )
    .w_full()
    .h(px(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_like_chromium() {
        // Glyphs 5 px wide: "abcdef" is 30 px; with a 6 px ellipsis in 20 px, "ab" fits (10 + 6).
        let starts: Vec<(usize, f32)> = (0..6).map(|i| (i, i as f32 * 5.0)).collect();
        assert_eq!(chromium_cut("abcdef", &starts, 30.0, 6.0, 20.0), Some(2));
        assert_eq!(chromium_cut("abcdef", &starts, 30.0, 6.0, 30.0), None);
        // Trailing spaces are kept ("a b…", not "a…").
        assert_eq!(chromium_cut("a bcde", &starts, 30.0, 6.0, 21.0), Some(3));
    }
}
