//! Text the way Chromium laid it out, where GPUI differs. Candidates for `qust-fluent` (the same
//! findings as `qust-ui-panels`' `local.rs`, from which `kerning`, the baseline rule and the
//! ellipsis rule are taken; the orchestrator consolidates them):
//!
//! - **Kerning.** GPUI's DirectWrite shaping applies only the features it is given (liga, clig,
//!   calt), so kerning, which Chromium applies by default, is off unless asked for; `tnum` alone
//!   drops it too. The view's roots set [`kerning`]; tabular text uses [`tabular_kerning`].
//! - **Ellipsis.** GPUI's `text-overflow: ellipsis` cuts one character early and trims trailing
//!   spaces and punctuation before the "…"; Chromium keeps them ("(320 kbps A…" against GPUI's
//!   "(320 kbps…"). [`ellipsized`] cuts where Chromium cuts, and the view shows that text
//!   without GPUI's truncation.
//! - **Letter-spacing.** `qust_fluent::text::spaced_text` puts a margin after each character,
//!   which GPUI rounds to whole pixels (0.36 px becomes 0) and which loses kerning;
//!   [`spaced_line`] paints the kerned line glyph by glyph at fractional offsets.

use gpui::{canvas, point, px, FontFeatures, FontWeight, SharedString, Styled, TextRun, Window};
use qust_fluent::text::{FONT_DISPLAY, FONT_TEXT};
use qust_fluent::Color;
use std::sync::Arc;

/// What a title, a station name, a host or a file name shows: the text itself for now. The one
/// place to apply `qust_text::shown` (right-to-left titles) to every such text of this crate.
pub fn shown(text: &str) -> SharedString {
    SharedString::from(text.to_string())
}

/// `kern` on (see the module docs).
pub fn kerning() -> FontFeatures {
    FontFeatures(Arc::new(vec![("kern".into(), 1)]))
}

/// `font-variant-numeric: tabular-nums` with kerning kept.
pub fn tabular_kerning() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1), ("kern".into(), 1)]))
}

/// A font of the page: the text or display family, a weight, kerning on.
pub fn page_font(display: bool, weight: FontWeight, features: FontFeatures) -> gpui::Font {
    gpui::Font { weight, features, ..gpui::font(if display { FONT_DISPLAY } else { FONT_TEXT }) }
}

/// Where Chromium's ellipsis cuts `text` for `available` px: after the last character whose end
/// (its kerned position in the whole line) fits in `available` minus the ellipsis, keeping
/// spaces and punctuation. None when the text fits.
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

/// `text` as Chromium shows it in `available` px with `text-overflow: ellipsis`, and whether it
/// was cut.
pub fn ellipsized(text: &str, font: &gpui::Font, size: f32, available: f32, window: &Window) -> (SharedString, bool) {
    let run = |len: usize| TextRun { len, font: font.clone(), color: gpui::black(), background_color: None, underline: None, strikethrough: None };
    let system = window.text_system();
    let full = system.layout_line(text, px(size), &[run(text.len())], None);
    let starts: Vec<(usize, f32)> = full.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.index, g.position.x.as_f32()))).collect();
    let ellipsis = system.layout_line("\u{2026}", px(size), &[run(3)], None).width.as_f32();
    match chromium_cut(text, &starts, full.width.as_f32(), ellipsis, available) {
        Some(cut) => (format!("{}\u{2026}", &text[..cut]).into(), true),
        None => (SharedString::from(text.to_string()), false),
    }
}

/// The width of one line of text.
pub fn line_width(text: &str, font: &gpui::Font, size: f32, window: &Window) -> f32 {
    let run = TextRun { len: text.len(), font: font.clone(), color: gpui::black(), background_color: None, underline: None, strikethrough: None };
    window.text_system().layout_line(text, px(size), &[run], None).width.as_f32()
}

/// Chromium's baseline in a line box of `line` px: ascent and descent rounded first, then half
/// the leading, rounded down.
fn chromium_baseline(ascent: f32, descent: f32, line: f32) -> f32 {
    ascent.round() + ((line - ascent.round() - descent.round()) / 2.0).floor()
}

/// One line with CSS `letter-spacing` (see the module docs): `width` × `line` px; the caller
/// sizes it with [`spaced_width`].
pub fn spaced_line(text: impl Into<SharedString>, size: f32, line: f32, spacing: f32, color: Color) -> gpui::Canvas<(Arc<gpui::LineLayout>, Vec<usize>)> {
    let text: SharedString = text.into();
    let chars: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    canvas(
        move |_, window, _| {
            let font = page_font(false, FontWeight::NORMAL, kerning());
            let run = TextRun { len: text.len(), font, color: color.into(), background_color: None, underline: None, strikethrough: None };
            (window.text_system().layout_line(text.as_ref(), px(size), &[run], None), chars.clone())
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
    .h(px(line))
}

/// The width Chromium gives a spaced line: the kerned text plus the spacing after every
/// character (the last one included).
pub fn spaced_width(text: &str, size: f32, spacing: f32, window: &Window) -> f32 {
    line_width(text, &page_font(false, FontWeight::NORMAL, kerning()), size, window) + spacing * text.chars().count() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_like_chromium() {
        let text = "ab, cd";
        let starts: Vec<(usize, f32)> = (0..6).map(|i| (i, i as f32 * 10.0)).collect();
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 60.0), None);
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 46.0), Some(4));
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 45.0), Some(3));
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 5.0), Some(0));
    }
}

/// How far GPUI's baseline must move to sit where Chromium's does, in whole pixels, for text of
/// `size` px in a line box of `line` px laid out at a whole pixel (from `qust-ui-panels`'
/// `local.rs`). GPUI centres the font's exact ascent + descent in the line box and rounds the
/// baseline; Chromium rounds ascent and descent first. Text laid out at a half pixel (the station
/// line, centred at 22.5) rounds the other way in GPUI's snapping and needs no shift.
pub fn baseline_shift(size: f32, line: f32, display: bool, window: &Window) -> f32 {
    let system = window.text_system();
    let id = system.resolve_font(&gpui::font(if display { FONT_DISPLAY } else { FONT_TEXT }));
    let (a, d) = (system.ascent(id, px(size)).as_f32(), system.descent(id, px(size)).as_f32().abs());
    let gpui = ((line - a - d) / 2.0 + a).round();
    chromium_baseline(a, d, line) - gpui
}
