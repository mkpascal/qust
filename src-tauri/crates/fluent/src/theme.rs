//! The theme every element reads: the palette of the current light/dark mode and the accent.
//!
//! One [`Theme`] is a GPUI global (`cx.global::<Theme>()`, or [`theme(cx)`]). Elements read it in
//! `render`; nothing caches colors, so switching theme or accent is `Theme::install(...)`, which
//! replaces the global and refreshes every window (the page had no transition between themes
//! either). Which mode and accent to install is `qust-ui`'s business (system theme, the stored
//! `theme` setting, the Windows accent palette).
//!
//! The accent follows style.css exactly:
//! - `--accent-base` / `--accent-light` / `--accent-dark` are the system's palette entries
//!   (`AccentPalette` 3 / 1 / 4 on Windows); the CSS fallback `#0078d4` / `#60cdff` / `#005fb8`
//!   when the system gives none (macOS).
//! - `--accent` is the **light** shade in dark mode and the **dark** shade in light mode.
//! - `--on-accent` (text on accent fills) is black in dark mode, white in light mode.
//! - The analyzer's accent palette and the clock's state icon use `--accent-light` in **both**
//!   modes ([`Theme::accent_light`]).

use crate::color::Color;
use crate::tokens::{Palette, CSS_ACCENT, DARK, LIGHT, REFERENCE_ACCENT};
use gpui::{App, Global};
use std::ops::Deref;

/// The system accent's three shades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accent {
    pub base: Color,
    pub light: Color,
    pub dark: Color,
}

impl Accent {
    /// From `#rrggbb` strings (the format `qust_player::accent()` and `window.__QUST__` use).
    pub fn parse(base: &str, light: &str, dark: &str) -> Option<Accent> {
        Some(Accent { base: Color::hex(base)?, light: Color::hex(light)?, dark: Color::hex(dark)? })
    }

    /// What style.css declares, used when nothing is injected.
    pub fn css_fallback() -> Accent {
        Accent::parse(CSS_ACCENT[0], CSS_ACCENT[1], CSS_ACCENT[2]).unwrap_or(Accent { base: Color::BLACK, light: Color::WHITE, dark: Color::BLACK })
    }

    /// The Windows default blue, as the reference renders were made with.
    pub fn reference() -> Accent {
        Accent::parse(REFERENCE_ACCENT[0], REFERENCE_ACCENT[1], REFERENCE_ACCENT[2]).unwrap_or_else(Accent::css_fallback)
    }
}

/// The resolved theme. Derefs to the [`Palette`] (`theme.text`, `theme.card_stroke`, …).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub palette: Palette,
    pub accent_base: Color,
    pub accent_light: Color,
    pub accent_dark: Color,
    /// `--accent`: light shade in dark mode, dark shade in light mode.
    pub accent: Color,
}

impl Theme {
    pub fn new(dark: bool, accent: Accent) -> Theme {
        Theme {
            dark,
            palette: if dark { DARK } else { LIGHT },
            accent_base: accent.base,
            accent_light: accent.light,
            accent_dark: accent.dark,
            accent: if dark { accent.light } else { accent.dark },
        }
    }

    /// The accent the theme was built from.
    pub fn accent_shades(&self) -> Accent {
        Accent { base: self.accent_base, light: self.accent_light, dark: self.accent_dark }
    }

    /// Replaces the global theme and redraws every window. Also gives GPUI Kit's theme the colors
    /// its `Input` reads (text, placeholder, caret, selection), so text boxes follow.
    pub fn install(self, cx: &mut App) {
        cx.set_global(self);
        crate::textbox::sync_component_theme(&self, cx);
        cx.refresh_windows();
    }

    /// `color-mix(in srgb, var(--accent) p%, transparent)`.
    pub fn accent_mix(&self, p: f32) -> Color {
        self.accent.mix(p)
    }

    /// `color-mix(in srgb, var(--on-accent) p%, transparent)`.
    pub fn on_accent_mix(&self, p: f32) -> Color {
        self.palette.on_accent.mix(p)
    }
}

impl Deref for Theme {
    type Target = Palette;
    fn deref(&self) -> &Palette {
        &self.palette
    }
}

impl Global for Theme {}

/// The installed theme, or dark with the reference accent before anything was installed.
pub fn theme(cx: &App) -> &Theme {
    static DEFAULT: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    cx.try_global::<Theme>().unwrap_or_else(|| DEFAULT.get_or_init(|| Theme::new(true, Accent::reference())))
}
