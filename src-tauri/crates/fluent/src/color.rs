// Colors as CSS has them: r, g, b in 0–255 and alpha in 0–1, so values can be compared with the
// reference's layout files ([r, g, b, a]) and `color-mix()` computed the way Chromium does.

use gpui::{Hsla, Rgba};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0.0);
    pub const BLACK: Color = Color::rgba(0, 0, 0, 1.0);
    pub const WHITE: Color = Color::rgba(255, 255, 255, 1.0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color { r: r as f32, g: g as f32, b: b as f32, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::rgba(r, g, b, 1.0)
    }

    /// `#rrggbb` (the accent's format), else None.
    pub fn hex(text: &str) -> Option<Color> {
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    /// `color-mix(in srgb, <self> p%, transparent)`. Chromium interpolates premultiplied, and
    /// `transparent` contributes nothing, so the color stays and its alpha scales (not rounded to
    /// a byte: color-mix results keep their fractional alpha).
    pub fn mix(self, p: f32) -> Color {
        Color { a: self.a * p, ..self }
    }

    /// The same color at another alpha.
    pub fn alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    /// `self` painted over the opaque `under`, as a byte-exact sRGB result (source-over, no
    /// gamma, like Chromium). For tests and for flattening layers that must not double-blend.
    pub fn over(self, under: Color) -> Color {
        let mix = |top: f32, bottom: f32| top * self.a + bottom * under.a * (1.0 - self.a);
        let a = self.a + under.a * (1.0 - self.a);
        if a <= 0.0 {
            return Color::TRANSPARENT;
        }
        Color { r: mix(self.r, under.r) / a, g: mix(self.g, under.g) / a, b: mix(self.b, under.b) / a, a }
    }

    /// The 8-bit pixel this color gives when painted over an opaque background.
    pub fn pixel_over(self, under: Color) -> [u8; 3] {
        let c = self.over(under);
        [c.r.round() as u8, c.g.round() as u8, c.b.round() as u8]
    }

    pub fn rgba_f32(self) -> Rgba {
        Rgba { r: self.r / 255.0, g: self.g / 255.0, b: self.b / 255.0, a: self.a }
    }
}

impl From<Color> for Hsla {
    fn from(color: Color) -> Hsla {
        color.rgba_f32().into()
    }
}

impl From<Color> for Rgba {
    fn from(color: Color) -> Rgba {
        color.rgba_f32()
    }
}

impl From<Color> for gpui::Fill {
    fn from(color: Color) -> gpui::Fill {
        Hsla::from(color).into()
    }
}

impl From<Color> for gpui::Background {
    fn from(color: Color) -> gpui::Background {
        Hsla::from(color).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_and_parsing() {
        let accent = Color::hex("#4cc2ff").unwrap();
        assert_eq!(accent, Color::rgb(76, 194, 255));
        assert_eq!(accent.mix(0.9).a, 0.9);
        assert!(Color::hex("4cc2ff").is_none() && Color::hex("#4cc2f").is_none() && Color::hex("#4cc2fz").is_none());
        // --card over --bg (dark): 13/255 white over #202020.
        assert_eq!(Color::rgba(255, 255, 255, 13.0 / 255.0).pixel_over(Color::rgb(32, 32, 32)), [43, 43, 43]);
    }
}
