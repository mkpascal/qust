//! Fonts and the text styles of style.css.
//!
//! The page asked for `"Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system,
//! BlinkMacSystemFont, sans-serif` (and `… Display` for the clock, cartridge names, dialog and
//! info titles). Chromium picked Segoe UI Variable Text / Display on Windows 11 (tokens.json
//! `fonts`); on Windows 10 it would have fallen back to Segoe UI, and on macOS `system-ui` is the
//! system font (SF Pro), which GPUI calls `.SystemUIFont`.
//!
//! Text is drawn with grayscale antialiasing (`qust_ui::init` sets it): the real WebView2 window
//! drew some text with ClearType and some in grayscale; positions, sizes and weights are what has
//! to match.

use gpui::{px, Div, FontFallbacks, FontFeatures, FontWeight, IntoElement, ParentElement, SharedString, Styled};
use std::sync::{Arc, OnceLock};

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

fn fallbacks() -> FontFallbacks {
    static FALLBACKS: OnceLock<FontFallbacks> = OnceLock::new();
    FALLBACKS.get_or_init(|| FontFallbacks::from_fonts(vec!["Segoe UI".into(), ".SystemUIFont".into()])).clone()
}

/// `font-variant-numeric: tabular-nums`.
pub fn tabular() -> FontFeatures {
    static TNUM: OnceLock<FontFeatures> = OnceLock::new();
    TNUM.get_or_init(|| FontFeatures(Arc::new(vec![("tnum".into(), 1)]))).clone()
}

/// The text styles of style.css, on any styled element.
pub trait TextStyles: Styled + Sized {
    /// `var(--font)`.
    fn font_text(mut self) -> Self {
        let style = self.text_style();
        style.font_family = Some(FONT_TEXT.into());
        style.font_fallbacks = Some(fallbacks());
        self
    }

    /// `var(--font-display)`.
    fn font_display(mut self) -> Self {
        let style = self.text_style();
        style.font_family = Some(FONT_DISPLAY.into());
        style.font_fallbacks = Some(fallbacks());
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

    /// `font-variant-numeric: tabular-nums`.
    fn tabular_nums(self) -> Self {
        self.font_features(tabular())
    }
}

impl<T: Styled> TextStyles for T {}

/// Text with CSS `letter-spacing` (GPUI has none): each character in its own box followed by
/// `spacing` px, as Chromium adds the spacing after every character, the last one included.
/// Used by the 9 px MONO / STEREO badges (0.04 em = 0.36 px) and the info card headings
/// (0.06 em). Kerning between the characters is lost; at these sizes it moves nothing visibly.
pub fn spaced_text(text: impl Into<SharedString>, spacing: f32) -> Div {
    let text: SharedString = text.into();
    gpui::div().flex().flex_row().whitespace_nowrap().children(text.chars().map(|c| gpui::div().mr(px(spacing)).child(SharedString::from(c.to_string())).into_any_element()))
}
