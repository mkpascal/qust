//! Fonts, the text styles of style.css, and text laid out the way Chromium laid it out where
//! GPUI differs.
//!
//! The page asked for `"Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system,
//! BlinkMacSystemFont, sans-serif` (and `… Display` for the clock, cartridge names, dialog and
//! info titles). Chromium picked Segoe UI Variable Text / Display on Windows 11 (tokens.json
//! `fonts`); on Windows 10 it would have fallen back to Segoe UI, and on macOS `system-ui` is the
//! system font (SF Pro), which GPUI calls `.SystemUIFont`.
//!
//! Text is drawn with grayscale antialiasing (`qust_ui::init` sets it). The real WebView2 window
//! drew most text in grayscale too, and some with ClearType: see `reference/README.md` and
//! `reference/renders-gray/`.
//!
//! **Where GPUI differs from Chromium, and what this module does about it** (each measured on the
//! reference renders):
//!
//! - **Kerning.** GPUI's DirectWrite shaping attaches an `IDWriteTypography` with only the
//!   features it is given (liga, clig, calt by default), and DirectWrite then applies only those:
//!   kerning, which Chromium applies by default, is off unless asked for, and `tnum` alone drops
//!   it too. [`TextStyles::font_text`] / [`TextStyles::font_display`] turn `kern` on (unless the
//!   element already chose features) and [`TextStyles::tabular_nums`] asks for `tnum` + `kern`.
//! - **Baseline.** Chromium rounds the font's ascent and descent to whole (device) pixels, then
//!   adds half the leading rounded down (`FontHeight::AddLeading`) and draws the glyphs on the
//!   pixel nearest that baseline. GPUI centres the exact ascent + descent in the line box and
//!   rounds the result toward zero. For Segoe UI Variable that puts 14/20, 11/14, 11/16 and 11/20
//!   text 1 px lower in GPUI (12/16, 12/20, 13/18, 13/20, 15/20 land on the same pixel).
//!   [`chromium_baseline`] is Chromium's rule; [`Line`] applies it at paint time from its real
//!   position and the window's scale (so a line box at a half pixel or at 125 % lands where
//!   Chromium's did), and [`on_chromium_baseline`] shifts a plain text block laid out at a whole
//!   pixel.
//! - **Ellipsis.** GPUI's `text-overflow: ellipsis` cuts one character early (it measures
//!   characters one by one, unkerned) and trims trailing spaces and punctuation before the "…".
//!   Chromium cuts after the last grapheme whose end, in the whole kerned line, fits in the width
//!   less the ellipsis's, and keeps spaces and punctuation ("supported, …"). [`chromium_cut`],
//!   [`ellipsized`] and [`Line::ellipsis`].
//! - **Letter-spacing** (unsupported in GPUI): the 9 px MONO / STEREO badges (0.04 em) and the
//!   info card headings (0.06 em). Chromium adds the spacing after every character, the last
//!   one included, at fractional positions, kerning kept: [`Line::letter_spacing`] shapes the line
//!   whole and moves each glyph right by n × spacing.
//! - **Fallback fonts.** For scripts Segoe UI Variable lacks, Chromium picks a font per script
//!   (Windows' `FontFallbackWin`); GPUI hands the rest of the string to DirectWrite's system
//!   fallback, which picks differently for Thai and CJK. [`fallback_fonts`] lists Chromium's
//!   choices ahead of the system fallback.

use gpui::{
    point, px, size, App, AvailableSpace, Bounds, Element, ElementId, FontFallbacks, FontFeatures, FontWeight, GlobalElementId, HighlightStyle,
    InspectorElementId, IntoElement, LayoutId, Pixels, Refineable, ShapedLine, SharedString, StrikethroughStyle, Style, StyleRefinement, Styled,
    TextAlign, TextRun, TextStyle, Window,
};
use std::ops::Range;
use std::sync::{Arc, OnceLock};
use unicode_segmentation::UnicodeSegmentation;

use crate::color::Color;

/// `--font` as declared (tokens.json checks it).
pub const FONT_STACK: &str = "\"Segoe UI Variable Text\", \"Segoe UI\", system-ui, -apple-system, BlinkMacSystemFont, sans-serif";
/// `--font-display` as declared.
pub const FONT_STACK_DISPLAY: &str = "\"Segoe UI Variable Display\", \"Segoe UI\", system-ui, -apple-system, BlinkMacSystemFont, sans-serif";

#[cfg(not(target_os = "macos"))]
pub const FONT_TEXT: &str = "Segoe UI Variable Text";
#[cfg(not(target_os = "macos"))]
pub const FONT_DISPLAY: &str = "Segoe UI Variable Display";
#[cfg(target_os = "macos")]
pub const FONT_TEXT: &str = ".SystemUIFont";
#[cfg(target_os = "macos")]
pub const FONT_DISPLAY: &str = ".SystemUIFont";

/// The fonts tried, in order, for characters the page's font lacks, before the system's own
/// fallback. GPUI maps each listed family to every character it has (the first listed family
/// that has a character wins), so the order decides:
///
/// - `Segoe UI`: the page's second family (Arabic, Hebrew, and anything the variable font lacks
///   that Segoe UI has), as Chromium.
/// - `Tahoma`: Chromium's font for Thai (DirectWrite's would be Leelawadee UI). Segoe UI has
///   every other script Tahoma covers, so Tahoma takes only Thai.
/// - `Microsoft YaHei`: Chromium's font for Han characters (with an English UI locale), in
///   Japanese titles too. It also has kana, which Chromium drew in Yu Gothic: no order can give
///   kana to Yu Gothic without also giving it the Han characters it has (and Yu Gothic lacks
///   many simplified ones), so kana come out in YaHei (same advances, a different design). Named
///   explicitly, its Bold face is used for semibold text, as Chromium's (DirectWrite's own
///   fallback drew CJK lighter there).
///
/// Everything else (Korean, Devanagari, symbols, emoji) goes to DirectWrite's system fallback,
/// which picks what Chromium picked (Malgun Gothic, Nirmala UI, Segoe UI Symbol, Segoe UI
/// Emoji). On macOS the list is the system font, as before.
pub fn fallback_fonts() -> &'static [&'static str] {
    #[cfg(not(target_os = "macos"))]
    {
        &["Segoe UI", "Tahoma", "Microsoft YaHei", ".SystemUIFont"]
    }
    #[cfg(target_os = "macos")]
    {
        &[".SystemUIFont"]
    }
}

fn fallbacks() -> FontFallbacks {
    static FALLBACKS: OnceLock<FontFallbacks> = OnceLock::new();
    FALLBACKS.get_or_init(|| FontFallbacks::from_fonts(fallback_fonts().iter().map(|f| f.to_string()).collect())).clone()
}

/// `kern` on: the default Chromium applied (see the module docs).
pub fn kerning() -> FontFeatures {
    static KERN: OnceLock<FontFeatures> = OnceLock::new();
    KERN.get_or_init(|| FontFeatures(Arc::new(vec![("kern".into(), 1)]))).clone()
}

/// `font-variant-numeric: tabular-nums`, kerning kept.
pub fn tabular() -> FontFeatures {
    static TNUM: OnceLock<FontFeatures> = OnceLock::new();
    TNUM.get_or_init(|| FontFeatures(Arc::new(vec![("tnum".into(), 1), ("kern".into(), 1)]))).clone()
}

/// A font of the page: the text or display family with the page's fallbacks.
pub fn font(display: bool, weight: FontWeight, features: FontFeatures) -> gpui::Font {
    gpui::Font { weight, features, fallbacks: Some(fallbacks()), ..gpui::font(if display { FONT_DISPLAY } else { FONT_TEXT }) }
}

/// The text styles of style.css, on any styled element.
pub trait TextStyles: Styled + Sized {
    /// `var(--font)`, kerning on.
    fn font_text(mut self) -> Self {
        let style = self.text_style();
        style.font_family = Some(FONT_TEXT.into());
        style.font_fallbacks = Some(fallbacks());
        if style.font_features.is_none() {
            style.font_features = Some(kerning());
        }
        self
    }

    /// `var(--font-display)`, kerning on.
    fn font_display(mut self) -> Self {
        let style = self.text_style();
        style.font_family = Some(FONT_DISPLAY.into());
        style.font_fallbacks = Some(fallbacks());
        if style.font_features.is_none() {
            style.font_features = Some(kerning());
        }
        self
    }

    /// `font-size: size; line-height: line` in CSS px.
    fn type_size(self, size: f32, line: f32) -> Self {
        self.text_size(px(size)).line_height(px(line))
    }

    fn semibold(self) -> Self {
        self.font_weight(FontWeight::SEMIBOLD)
    }

    fn regular(self) -> Self {
        self.font_weight(FontWeight::NORMAL)
    }

    /// `font-variant-numeric: tabular-nums` (kerning kept).
    fn tabular_nums(self) -> Self {
        self.font_features(tabular())
    }
}

impl<T: Styled> TextStyles for T {}

// --- Baseline -------------------------------------------------------------------------------------

/// Where Chromium puts the baseline in a line box of `line` px, from its top: the ascent and
/// descent rounded to whole pixels (Skia's metrics), plus half the leading rounded down
/// (`FontHeight::AddLeading`). All in device pixels at the window's scale.
pub fn chromium_baseline(ascent: f32, descent: f32, line: f32) -> f32 {
    ascent.round() + ((line - ascent.round() - descent.round()) / 2.0).floor()
}

/// Where GPUI puts it: the exact ascent + descent centred in the line box.
fn gpui_baseline(ascent: f32, descent: f32, line: f32) -> f32 {
    (line - ascent - descent) / 2.0 + ascent
}

/// GPUI rounds glyph origins half toward zero (`Window::paint_glyph`).
fn round_half_toward_zero(value: f32) -> f32 {
    if value >= 0.0 {
        (value - 0.5).ceil()
    } else {
        (value + 0.5).floor()
    }
}

/// The device pixel row Chromium draws the baseline of a line box at `top` (CSS px) on, for the
/// primary font's `ascent` and `descent` (CSS px; fallback fonts don't move it).
pub fn chromium_row(top: f32, ascent: f32, descent: f32, line: f32, scale: f32) -> f32 {
    (top * scale + chromium_baseline(ascent * scale, descent * scale, line * scale) + 0.5).floor()
}

/// The origin to give GPUI's line painting (`ShapedLine::paint`, which centres the shaped
/// line's exact `ascent` + `descent` and rounds) so that its baseline lands on device row `row`.
pub fn origin_for_row(row: f32, ascent: f32, descent: f32, line: f32, scale: f32) -> f32 {
    row / scale - gpui_baseline(ascent, descent, line)
}

/// [`origin_for_row`] of [`chromium_row`] when the shaped line has the primary font's metrics.
pub fn chromium_line_origin(top: f32, ascent: f32, descent: f32, line: f32, scale: f32) -> f32 {
    origin_for_row(chromium_row(top, ascent, descent, line, scale), ascent, descent, line, scale)
}

/// How far GPUI's baseline must move, in whole CSS px, for a line box laid out at a whole pixel
/// at scale 1 (what [`on_chromium_baseline`] applies).
pub fn baseline_shift(size: f32, line: f32, display: bool, cx: &App) -> f32 {
    let system = cx.text_system();
    let id = system.resolve_font(&gpui::font(if display { FONT_DISPLAY } else { FONT_TEXT }));
    let (a, d) = (system.ascent(id, px(size)).as_f32(), system.descent(id, px(size)).as_f32().abs());
    chromium_baseline(a, d, line) - round_half_toward_zero(gpui_baseline(a, d, line))
}

/// A block of plain text (`size`/`line`) moved onto Chromium's baseline (`relative` offsets
/// don't affect layout). For text laid out at a whole pixel; [`Line`] handles any position.
pub fn on_chromium_baseline<E: Styled>(text: E, size: f32, line: f32, display: bool, cx: &App) -> E {
    let shift = baseline_shift(size, line, display, cx);
    if shift == 0.0 {
        text
    } else {
        text.relative().top(px(shift))
    }
}

// --- Ellipsis ---------------------------------------------------------------------------------------

/// Where Chromium's `text-overflow: ellipsis` cuts `text` for `available` px (a byte index), or
/// None when it fits: after the last grapheme whose end (its kerned position in the whole line,
/// `glyph_starts` = each glyph's text index and x) fits in `available` minus the ellipsis,
/// keeping spaces and punctuation. A grapheme is never split (a base letter keeps its marks, an
/// emoji sequence stays whole); letters whose shape depends on their neighbours are measured in
/// the whole line, as Chromium measured them before cutting.
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
    for (index, grapheme) in text.grapheme_indices(true) {
        let end = index + grapheme.len();
        if start_of(end) <= limit + 0.01 {
            cut = end;
        } else {
            break;
        }
    }
    Some(cut)
}

fn plain_run(len: usize, font: &gpui::Font) -> TextRun {
    TextRun { len, font: font.clone(), color: gpui::black(), background_color: None, underline: None, strikethrough: None }
}

/// The width of one line of text (kerned as the font's features say).
pub fn line_width(text: &str, font: &gpui::Font, size: f32, window: &Window) -> f32 {
    window.text_system().layout_line(text, px(size), &[plain_run(text.len(), font)], None).width.as_f32()
}

/// The width Chromium gives a line with `letter-spacing`: the text plus the spacing after every
/// character (the last one included).
pub fn spaced_width(text: &str, font: &gpui::Font, size: f32, spacing: f32, window: &Window) -> f32 {
    line_width(text, font, size, window) + spacing * text.chars().count() as f32
}

/// `text` as Chromium shows it in `available` px with `text-overflow: ellipsis`, and whether it
/// was cut.
pub fn ellipsized(text: &str, font: &gpui::Font, size: f32, available: f32, window: &Window) -> (SharedString, bool) {
    let system = window.text_system();
    let full = system.layout_line(text, px(size), &[plain_run(text.len(), font)], None);
    let starts = glyph_starts(&full);
    let ellipsis = system.layout_line("\u{2026}", px(size), &[plain_run(3, font)], None).width.as_f32();
    match chromium_cut(text, &starts, full.width.as_f32(), ellipsis, available) {
        Some(cut) => (format!("{}\u{2026}", &text[..cut]).into(), true),
        None => (SharedString::from(text.to_string()), false),
    }
}

fn glyph_starts(layout: &gpui::LineLayout) -> Vec<(usize, f32)> {
    layout.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.index, g.position.x.as_f32()))).collect()
}

// --- The line element -----------------------------------------------------------------------------

/// One line of text laid out as Chromium laid it out: drop it where a string child would go.
/// It inherits the text style like a string child (family, size, line height, weight, color,
/// features, strikethrough), and its own style can refine it (`line(t).type_size(12., 16.)`).
/// Its box is as wide as its text (letter-spacing included) and one line tall; give it
/// `w_full()` / `flex_1().min_w_0()` to constrain it.
///
/// - The glyphs sit on Chromium's baseline, computed when it paints from its real position and
///   the window's scale ([`chromium_line_origin`]).
/// - [`Line::ellipsis`]: cut where Chromium cuts ([`chromium_cut`]) when the box is narrower than
///   the text.
/// - [`Line::letter_spacing`]: CSS `letter-spacing`.
/// - [`Line::align_end`]: the text set against the box's right edge (a box as wide as its text
///   that GPUI rounded to whole pixels: Chromium's fractional box ended where the text ends).
/// - [`Line::highlight`]: a range in another weight or color (`<b>` in the specs line).
/// - [`Line::selected`]: a selection highlight behind the text, and its text color.
///
/// Not in the accessibility tree by itself: give the element around it a role and label.
pub struct Line {
    text: SharedString,
    ellipsis: bool,
    spacing: f32,
    end: bool,
    translate: f32,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    selection: Option<(Color, Color)>,
    style: StyleRefinement,
}

/// A [`Line`] showing `text`.
pub fn line(text: impl Into<SharedString>) -> Line {
    Line { text: text.into(), ellipsis: false, spacing: 0.0, end: false, translate: 0.0, highlights: Vec::new(), selection: None, style: StyleRefinement::default() }
}

/// Text with CSS `letter-spacing` of `spacing` px: a [`Line`] with [`Line::letter_spacing`].
pub fn spaced_text(text: impl Into<SharedString>, spacing: f32) -> Line {
    line(text).letter_spacing(spacing)
}

impl Line {
    /// `overflow: hidden; text-overflow: ellipsis; white-space: nowrap`.
    pub fn ellipsis(mut self) -> Self {
        self.ellipsis = true;
        self
    }

    /// CSS `letter-spacing` in px.
    pub fn letter_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn align_end(mut self) -> Self {
        self.end = true;
        self
    }

    /// CSS `transform: translateX(x)`: the text drawn `x` px to the right of its box, fractions
    /// kept (GPUI snaps a box's position to whole pixels; Chromium moved the marquee's text by
    /// fractions of one).
    pub fn translate_x(mut self, x: f32) -> Self {
        self.translate = x;
        self
    }

    /// A byte range of the text in another weight or color.
    pub fn highlight(mut self, range: Range<usize>, style: HighlightStyle) -> Self {
        self.highlights.push((range, style));
        self
    }

    /// A selection highlight: (background, text color).
    pub fn selected(mut self, colors: Option<(Color, Color)>) -> Self {
        self.selection = colors;
        self
    }

    fn runs(&self, text_style: &TextStyle, len: usize) -> Vec<TextRun> {
        let base = text_style.to_run(len);
        if self.highlights.is_empty() {
            return vec![base];
        }
        let mut runs = Vec::new();
        let mut at = 0;
        let mut highlights = self.highlights.clone();
        highlights.sort_by_key(|(r, _)| r.start);
        for (range, highlight) in highlights {
            let (start, end) = (range.start.min(len), range.end.min(len));
            if start > at {
                runs.push(TextRun { len: start - at, ..base.clone() });
            }
            if end > start.max(at) {
                let mut run = TextRun { len: end - start.max(at), ..base.clone() };
                if let Some(color) = highlight.color {
                    run.color = color;
                }
                if let Some(weight) = highlight.font_weight {
                    run.font.weight = weight;
                }
                runs.push(run);
                at = end;
            }
        }
        if at < len {
            runs.push(TextRun { len: len - at, ..base });
        }
        runs
    }
}

impl Styled for Line {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Line {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// What a line measured at layout.
pub struct LineLayoutState {
    style: Style,
    text_style: TextStyle,
    size: f32,
    line: f32,
    natural: f32,
    /// The primary font's ascent and descent.
    metrics: (f32, f32),
}

/// What it shows, decided once its box is known.
pub struct LinePrepaint {
    shaped: ShapedLine,
    /// Glyph index → character number, for letter-spacing.
    chars: Vec<usize>,
    x: f32,
}

impl Element for Line {
    type RequestLayoutState = LineLayoutState;
    type PrepaintState = Option<LinePrepaint>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, _: &mut App) -> (LayoutId, LineLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let mut text_style = window.text_style();
        text_style.refine(&self.style.text);
        let rem = window.rem_size();
        let font_size = text_style.font_size.to_pixels(rem).as_f32();
        let line = text_style.line_height_in_pixels(rem).as_f32();
        let runs = self.runs(&text_style, self.text.len());
        let natural = window.text_system().layout_line(&self.text, px(font_size), &runs, None).width.as_f32() + self.spacing * self.text.chars().count() as f32;
        let ellipsis = self.ellipsis;
        let layout_id = window.request_measured_layout(style.clone(), move |known, available, window, _| {
            let width = known.width.map(|w| w.as_f32()).unwrap_or(match available.width {
                // An ellipsis box can shrink to nothing (`overflow: hidden` makes its min-content
                // contribution 0).
                AvailableSpace::MinContent if ellipsis => 0.0,
                // Chromium's box is as wide as the text, fractions kept; GPUI rounds a measured
                // size up to whole device pixels. Half a pixel less makes that the nearest whole
                // width (a centred or right-aligned box then sits within half a pixel of
                // Chromium's, not up to one).
                _ => {
                    let scale = window.scale_factor();
                    ((natural * scale - 0.5) / scale).max(0.0)
                }
            });
            size(px(width), known.height.unwrap_or(px(line)))
        });
        // Chromium's line box comes from the primary font's metrics.
        let system = window.text_system();
        let id = system.resolve_font(&text_style.font());
        let metrics = (system.ascent(id, px(font_size)).as_f32(), system.descent(id, px(font_size)).as_f32().abs());
        (layout_id, LineLayoutState { style, text_style, size: font_size, line, natural, metrics })
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, state: &mut LineLayoutState, window: &mut Window, _: &mut App) -> Option<LinePrepaint> {
        let text_style = &state.text_style;
        let system = window.text_system().clone();
        let mut shown = self.text.clone();
        let available = bounds.size.width.as_f32();
        // A box sized by its own text is the text's width rounded to the nearest device pixel
        // (see `request_layout`): that is not an overflow.
        if self.ellipsis && state.natural > available + 0.5 / window.scale_factor() + 0.01 {
            let runs = self.runs(text_style, self.text.len());
            let full = system.layout_line(&self.text, px(state.size), &runs, None);
            let spacing = self.spacing;
            // Letter-spacing moves every glyph after the n-th character n × spacing to the right.
            let starts: Vec<(usize, f32)> = glyph_starts(&full)
                .into_iter()
                .map(|(index, x)| (index, x + spacing * self.text[..index.min(self.text.len())].chars().count() as f32))
                .collect();
            let ellipsis = system.layout_line("\u{2026}", px(state.size), &[text_style.to_run(3)], None).width.as_f32() + spacing;
            if let Some(cut) = chromium_cut(&self.text, &starts, state.natural, ellipsis, available) {
                shown = format!("{}\u{2026}", &self.text[..cut]).into();
            }
        }
        let mut runs = self.runs(text_style, shown.len());
        if let Some((_, color)) = self.selection {
            for run in &mut runs {
                run.color = color.into();
            }
        }
        let shaped = system.shape_line(shown.clone(), px(state.size), &runs, None);
        let chars: Vec<usize> = if self.spacing != 0.0 { shown.char_indices().map(|(i, _)| i).collect() } else { Vec::new() };
        let width = shaped.width.as_f32() + self.spacing * shown.chars().count() as f32;
        let x = if self.end { available - width } else { 0.0 } + self.translate;
        Some(LinePrepaint { shaped, chars, x })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut LineLayoutState,
        prepaint: &mut Option<LinePrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepared) = prepaint.take() else { return };
        let line = state.line;
        let selection = self.selection;
        let spacing = self.spacing;
        let font_size = px(state.size);
        let color = state.text_style.color;
        let (font_ascent, font_descent) = state.metrics;
        state.style.paint(bounds, window, cx, |window, cx| {
            let shaped = &prepared.shaped;
            let (ascent, descent) = (shaped.ascent.as_f32(), shaped.descent.as_f32().abs());
            let scale = window.scale_factor();
            let row = chromium_row(bounds.origin.y.as_f32(), font_ascent, font_descent, line, scale);
            let y = origin_for_row(row, ascent, descent, line, scale);
            let x = bounds.origin.x.as_f32() + prepared.x;
            if let Some((background, _)) = selection {
                let width = (shaped.width.as_f32()).min(bounds.size.width.as_f32());
                window.paint_quad(gpui::fill(Bounds::new(point(px(x), bounds.origin.y), size(px(width), bounds.size.height)), background));
            }
            if spacing == 0.0 {
                let _ = shaped.paint(point(px(x), px(y)), px(line), TextAlign::Left, None, window, cx);
                return;
            }
            // Glyph by glyph, each moved by the spacing of the characters before it.
            let baseline = y + gpui_baseline(ascent, descent, line);
            for run in &shaped.runs {
                for glyph in &run.glyphs {
                    let n = prepared.chars.iter().position(|i| *i == glyph.index).unwrap_or(0) as f32;
                    let origin = point(px(x) + glyph.position.x + px(n * spacing), px(baseline) + glyph.position.y);
                    let _ = if glyph.is_emoji {
                        window.paint_emoji(origin, run.font_id, glyph.id, font_size)
                    } else {
                        window.paint_glyph(origin, run.font_id, glyph.id, font_size, color)
                    };
                }
            }
        });
    }
}

/// The selection highlight of a selected value (Chromium's default `::selection` on Windows: the
/// system highlight color, white text). Not in any reference render: the page never styled it.
pub fn selection_colors() -> (Color, Color) {
    (Color::rgba(0, 120, 215, 1.0), Color::WHITE)
}

/// A line struck through (`text-decoration: line-through`) in `color`.
pub fn strikethrough(color: Color) -> StrikethroughStyle {
    StrikethroughStyle { thickness: px(1.0), color: Some(color.into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Segoe UI Variable: ascent 2210 / descent 514 per 2048.
    fn metrics(size: f32) -> (f32, f32) {
        (size * 2210.0 / 2048.0, size * 514.0 / 2048.0)
    }

    #[test]
    fn baselines_like_chromium() {
        let at = |size: f32, line: f32| {
            let (a, d) = metrics(size);
            chromium_baseline(a, d, line)
        };
        assert_eq!(at(14.0, 20.0), 15.0);
        assert_eq!(at(12.0, 16.0), 13.0);
        assert_eq!(at(11.0, 16.0), 12.0);
        assert_eq!(at(11.0, 14.0), 11.0);
        assert_eq!(at(15.0, 20.0), 16.0);
        assert_eq!(at(13.0, 18.0), 14.0);
    }

    #[test]
    fn gpui_lands_on_chromium_rows() {
        // Every size / line height pair style.css uses, at whole and half pixel positions and
        // at the four scales: GPUI's own rounding of the returned origin lands on Chromium's row.
        let pairs = [(9.0, 14.0), (11.0, 14.0), (11.0, 16.0), (11.0, 20.0), (12.0, 16.0), (12.0, 18.0), (12.0, 20.0), (13.0, 18.0), (13.0, 20.0), (14.0, 20.0), (15.0, 20.0), (18.0, 24.0), (22.0, 24.0)];
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            for (size, line) in pairs {
                let (a, d) = metrics(size);
                for top in [0.0f32, 22.5, 39.5, 64.5, 117.75, 267.0] {
                    let origin = chromium_line_origin(top, a, d, line, scale);
                    let gpui_row = round_half_toward_zero((origin + gpui_baseline(a, d, line)) * scale);
                    let chromium_row = (top * scale + chromium_baseline(a * scale, d * scale, line * scale) + 0.5).floor();
                    assert_eq!(gpui_row, chromium_row, "{size}/{line} at {top} ×{scale}");
                }
            }
        }
        // The pairs found 1 px low in GPUI at a whole pixel, at scale 1.
        for (size, line, shift) in [(14.0, 20.0, -1.0), (11.0, 16.0, -1.0), (11.0, 14.0, -1.0), (11.0, 20.0, -1.0), (12.0, 16.0, 0.0), (13.0, 20.0, 0.0), (15.0, 20.0, 0.0)] {
            let (a, d) = metrics(size);
            assert_eq!(chromium_baseline(a, d, line) - round_half_toward_zero(gpui_baseline(a, d, line)), shift, "{size}/{line}");
        }
    }

    #[test]
    fn cuts_like_chromium() {
        // Four 10 px characters "ab, " then "cd"; an ellipsis of 6.
        let text = "ab, cd";
        let starts: Vec<(usize, f32)> = (0..6).map(|i| (i, i as f32 * 10.0)).collect();
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 60.0), None);
        // 46 px: 40 for text → "ab, " kept whole, trailing space included.
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 46.0), Some(4));
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 45.0), Some(3));
        assert_eq!(chromium_cut(text, &starts, 60.0, 6.0, 5.0), Some(0));
        // A base letter keeps its combining mark: "e\u{301}" is one grapheme (bytes 1..4).
        let text = "ae\u{301}bc";
        let starts = [(0, 0.0), (1, 10.0), (4, 20.0), (5, 30.0)];
        assert_eq!(chromium_cut(text, &starts, 40.0, 6.0, 25.0), Some(1));
        assert_eq!(chromium_cut(text, &starts, 40.0, 6.0, 27.0), Some(4));
    }
}
