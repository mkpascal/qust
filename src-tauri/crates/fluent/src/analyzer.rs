//! The analyzer: the 75 × 16 pixel grid painted from a 77-byte frame, exactly as `paint()` in
//! visualizer.js painted its canvas.
//!
//! Frame: byte 0 the loader level (0–255); for bar i (19 of them): `1+4i` lit rows (0–15),
//! `2+4i` peak row (0 = none), `3+4i` the loader block's half height, `4+4i` its alpha. Bar i
//! is 3 px wide at x = 4i; row r (0 = bottom) is at y = 15 − r. With level L = byte0 / 255:
//! bars and peaks at alpha 1 − L when L < 1, then the loader on top when L > 0: per bar rows
//! 8 − half … 7 + half (half ≤ 8) at alpha L × alpha / 255, in the row colors.
//!
//! Palettes: classic (fixed row colors, grey peak), white and accent (one color, rows at alpha
//! `(0.45 + 0.55 r / 15).toFixed(3)`, solid peak), off (no bars come; the loader uses white).
//! The accent palette uses `--accent-light` in both themes.
//!
//! The canvas was a 2D canvas (8-bit premultiplied backing store) composited over the screen's
//! `--display` background, so the element computes each pixel's final 8-bit color the same way
//! ([`rasterize`]) and paints opaque device pixels: the result is exact by construction, and the
//! unit test checks it against every analyzer reference render.
//!
//! Scaling (`Spectrum.fit()`): with r the device scale and s = max(1, round(r)), the element is
//! 75·s/r × 16·s/r CSS px and each grid pixel is exactly s × s device pixels, its origin snapped
//! to a device pixel ([`grid_scale`], [`css_size`]).
//!
//! Repainting only itself: the analyzer is meant to live in its own view (`qust_ui`'s
//! `AnalyzerView`), the only thing a new frame notifies.

use crate::color::Color;
use gpui::{canvas, fill, point, px, size, App, Bounds, IntoElement, RenderOnce, Styled, Window};

pub const BARS: usize = 19;
pub const ROWS: usize = 15;
pub const GRID_WIDTH: usize = BARS * 4 - 1;
pub const GRID_HEIGHT: usize = ROWS + 1;
pub const FRAME_LEN: usize = 1 + BARS * 4;

pub type Frame = [u8; FRAME_LEN];

pub const BLANK: Frame = [0; FRAME_LEN];

const CLASSIC: [[u8; 3]; 16] = [
    [24, 132, 8],
    [41, 148, 0],
    [49, 156, 8],
    [57, 181, 16],
    [50, 190, 16],
    [41, 206, 16],
    [148, 222, 33],
    [189, 222, 41],
    [214, 181, 33],
    [222, 165, 24],
    [198, 123, 8],
    [214, 115, 0],
    [214, 102, 0],
    [214, 90, 0],
    [206, 41, 16],
    [239, 49, 16],
];

/// The `visualizer-colors` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Colors {
    #[default]
    Classic,
    White,
    Accent,
    Off,
}

impl Colors {
    pub fn parse(name: &str) -> Colors {
        match name {
            "white" => Colors::White,
            "accent" => Colors::Accent,
            "off" => Colors::Off,
            _ => Colors::Classic,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Colors::Classic => "classic",
            Colors::White => "white",
            Colors::Accent => "accent",
            Colors::Off => "off",
        }
    }
}

/// The row colors (bottom to top) and the peak color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub rows: [Color; GRID_HEIGHT],
    pub peak: Color,
}

impl Palette {
    /// `palette(name, color)` in visualizer.js; `accent_light` is `--accent-light`.
    pub fn new(colors: Colors, accent_light: Color) -> Palette {
        if colors == Colors::Classic {
            return Palette { rows: CLASSIC.map(|[r, g, b]| Color::rgb(r, g, b)), peak: Color::rgb(150, 150, 150) };
        }
        let base = if colors == Colors::Accent { accent_light } else { Color::WHITE };
        let mut rows = [base; GRID_HEIGHT];
        for (r, row) in rows.iter_mut().enumerate() {
            // (0.45 + 0.55 * r / 15).toFixed(3): three decimals, as the page wrote them.
            let alpha = ((0.45 + 0.55 * r as f64 / (GRID_HEIGHT - 1) as f64) * 1000.0).round() / 1000.0;
            *row = base.alpha(alpha as f32);
        }
        Palette { rows, peak: base.alpha(1.0) }
    }
}

/// One canvas pixel: premultiplied 8-bit, as Skia stores a 2D canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Premul {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

fn byte(value: f32) -> f32 {
    (value.clamp(0.0, 255.0)).round()
}

impl Premul {
    // fillRect with `color` at global alpha `global`, source-over, stored as bytes.
    fn fill(&mut self, color: Color, global: f32) {
        // The fill style's alpha is parsed to a byte (rgba(…, 0.633) paints at 161/255).
        let a = (color.a * 255.0).round() / 255.0 * global;
        // Skia's 8-bit pipeline: the source premultiplied to bytes, then source-over with the
        // destination scaled by (255 − source alpha) / 255 and rounded.
        let sa = byte(a * 255.0);
        let inv = (255.0 - sa) / 255.0;
        *self = Premul {
            r: byte(color.r * a) + byte(self.r * inv),
            g: byte(color.g * a) + byte(self.g * inv),
            b: byte(color.b * a) + byte(self.b * inv),
            a: sa + byte(self.a * inv),
        };
    }

    // The canvas composited over the opaque background.
    fn over(self, background: Color) -> [u8; 3] {
        let inv = 1.0 - self.a / 255.0;
        [byte(self.r + background.r * inv) as u8, byte(self.g + background.g * inv) as u8, byte(self.b + background.b * inv) as u8]
    }
}

/// The grid's pixels for `frame`: `None` where the canvas stayed transparent (the background
/// shows), else the final color over `background`.
pub fn rasterize(frame: &Frame, palette: &Palette, background: Color) -> [[Option<[u8; 3]>; GRID_WIDTH]; GRID_HEIGHT] {
    let mut canvas = [[None::<Premul>; GRID_WIDTH]; GRID_HEIGHT];
    let mut fill_rect = |x: usize, y: usize, color: Color, global: f32| {
        for cell in canvas[y][x..x + 3].iter_mut() {
            cell.get_or_insert_with(Premul::default).fill(color, global);
        }
    };
    let level = frame[0] as f32 / 255.0;
    if level < 1.0 {
        let global = 1.0 - level;
        for i in 0..BARS {
            let x = i * 4;
            let rows = (frame[1 + i * 4] as usize).min(ROWS);
            for r in 0..rows {
                fill_rect(x, GRID_HEIGHT - 1 - r, palette.rows[r], global);
            }
            let peak = (frame[2 + i * 4] as usize).min(ROWS);
            if peak >= 1 {
                fill_rect(x, GRID_HEIGHT - 1 - peak, palette.peak, global);
            }
        }
    }
    if level > 0.0 {
        let middle = GRID_HEIGHT / 2;
        for i in 0..BARS {
            let half = (frame[3 + i * 4] as usize).min(middle);
            let x = i * 4;
            let global = level * frame[4 + i * 4] as f32 / 255.0;
            for r in middle - half..middle + half {
                fill_rect(x, GRID_HEIGHT - 1 - r, palette.rows[r], global);
            }
        }
    }
    canvas.map(|row| row.map(|cell| cell.map(|pixel| pixel.over(background))))
}

/// Device pixels per grid pixel at device scale `scale_factor`: max(1, round(r)).
pub fn grid_scale(scale_factor: f32) -> f32 {
    (scale_factor.round()).max(1.0)
}

/// The element's CSS size at `scale_factor` (75·s/r × 16·s/r).
pub fn css_size(scale_factor: f32) -> (f32, f32) {
    let s = grid_scale(scale_factor);
    (GRID_WIDTH as f32 * s / scale_factor, GRID_HEIGHT as f32 * s / scale_factor)
}

/// Where the grid lands in device pixels, per axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// The first device pixel painted.
    pub start: i32,
    /// Grid pixel k starts at `origin + k · grid_scale` (pixels before it repeat grid pixel 0).
    pub origin: i32,
    /// One past the last device pixel painted.
    pub end: i32,
}

impl Span {
    /// The grid pixel a device pixel shows, if any.
    pub fn cell(&self, device: i32, scale: i32) -> Option<usize> {
        (device >= self.start && device < self.end).then(|| ((device - self.origin).max(0) / scale) as usize)
    }
}

/// Where the grid lands in device pixels for an element laid out at CSS (`x`, `y`), as Chromium
/// drew the pixelated canvas. Exact at 100 % and 200 %. At fractional scales the rules are
/// what the reference renders show (canvas at CSS 45, 58), not derived from Chromium's code:
/// - the grid origin is the device position rounded half up (125 %: 56.25, 72.5 → 56, 73;
///   150 %: 67.5, 87 → 68, 87);
/// - a canvas edge on exactly half a device pixel also paints that pixel, repeating the first
///   grid row or column (125 %: row 72; 150 %: column 67);
/// - the drawn size is the grid's size in device pixels (75·s × 16·s); horizontally it counts
///   from the first painted pixel (150 %: 67 + 150, so the last column is 1 pixel wide),
///   vertically from the origin (125 %: 73 + 16, so the repeated row adds one);
/// - at 150 % Chromium drew 31 rows, not 32, so the bottom grid row is 1 device pixel tall.
///   The style height there is `21.333333333333332px`, the only one of these sizes that isn't
///   a finite decimal; no rounding rule tried (layout units of 1/64, float lengths) explains
///   the missing row together with the 125 % render, so it is applied as measured
///   ([`MEASURED_ROW_CUT`]).
///
/// 175 %, 225 % and 250 % follow the same rules but have no reference render to check.
pub fn device_placement(x: f32, y: f32, scale_factor: f32) -> (Span, Span) {
    let s = grid_scale(scale_factor) as i32;
    let cut = MEASURED_ROW_CUT.iter().find(|(scale, _)| (*scale - scale_factor).abs() < 1e-4).map_or(0, |(_, rows)| *rows);
    let axis = |position: f32, size: i32, from_start: bool| {
        let device = position as f64 * scale_factor as f64;
        let origin = (device + 0.5).floor() as i32;
        let half = (device - device.floor() - 0.5).abs() < 1e-6;
        let start = if half { origin - 1 } else { origin };
        let end = if from_start { start } else { origin } + size;
        Span { start, origin, end }
    };
    (axis(x, GRID_WIDTH as i32 * s, true), axis(y, GRID_HEIGHT as i32 * s - cut, false))
}

/// Device rows the reference drew fewer than 16·s, by scale (see [`device_placement`]).
pub const MEASURED_ROW_CUT: [(f32, i32); 1] = [(1.5, 1)];

/// A frame as `harness/frames.mjs` makes them for the fixtures: a named shape ("music",
/// "full", "quiet", "empty"), the loader level (0–1) and the loader's time (s); `off` leaves
/// out the bars (the `off` colors).
pub fn fixture_frame(shape: &str, loader: f64, loader_time: f64, off: bool) -> Frame {
    let (bars, peaks): ([u8; BARS], [u8; BARS]) = match shape {
        "music" => ([11, 13, 14, 12, 9, 10, 11, 8, 9, 7, 8, 6, 7, 5, 4, 4, 3, 2, 1], [13, 14, 15, 14, 12, 11, 12, 11, 10, 9, 9, 8, 8, 7, 6, 5, 4, 3, 2]),
        "full" => ([15; BARS], [15; BARS]),
        "quiet" => ([3, 4, 4, 3, 2, 2, 3, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0], [9, 10, 11, 10, 8, 7, 8, 7, 6, 5, 5, 4, 4, 3, 3, 2, 2, 1, 0]),
        _ => ([0; BARS], [0; BARS]),
    };
    fixture_frame_from(&bars, &peaks, loader, loader_time, off)
}

/// The same with explicit bars and peaks.
pub fn fixture_frame_from(bars: &[u8], peaks: &[u8], loader: f64, loader_time: f64, off: bool) -> Frame {
    let mut frame = BLANK;
    let level = loader.clamp(0.0, 1.0);
    frame[0] = js_round(level * 255.0) as u8;
    if !off {
        for i in 0..BARS {
            frame[1 + i * 4] = bars.get(i).copied().unwrap_or(0).min(15);
            frame[2 + i * 4] = peaks.get(i).copied().unwrap_or(0).min(15);
        }
    }
    if loader > 0.0 {
        let head = ((loader_time / 1.1) % 1.0) * (BARS + 6) as f64 - 3.0;
        for i in 0..BARS {
            let pulse = (1.0 - (i as f64 - head).abs() / 3.0).max(0.0);
            frame[3 + i * 4] = (1.0 + js_round(pulse * 5.0)) as u8;
            frame[4 + i * 4] = js_round((0.3 + 0.7 * pulse) * 255.0) as u8;
        }
    }
    frame
}

/// `Math.round`.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// The frame a reference fixture's `frame` field describes (`harness/frames.mjs`): a raw array
/// of 77 numbers or `{ shape, loader, loaderTime }`, with `off` for the off colors.
pub fn frame_from_fixture(spec: &FixtureFrame, off: bool) -> Frame {
    match spec {
        FixtureFrame::Raw(bytes) => {
            let mut frame = BLANK;
            for (slot, value) in frame.iter_mut().zip(bytes) {
                *slot = *value;
            }
            frame
        }
        FixtureFrame::Shape { shape, bars, peaks, loader, loader_time, off: spec_off } => match (bars, peaks) {
            (Some(bars), Some(peaks)) => fixture_frame_from(bars, peaks, *loader, *loader_time, off || *spec_off),
            _ => fixture_frame(shape.as_deref().unwrap_or("empty"), *loader, *loader_time, off || *spec_off),
        },
    }
}

/// A fixture's `frame`, parsed.
#[derive(Clone, Debug, PartialEq)]
pub enum FixtureFrame {
    Raw(Vec<u8>),
    Shape { shape: Option<String>, bars: Option<Vec<u8>>, peaks: Option<Vec<u8>>, loader: f64, loader_time: f64, off: bool },
}

/// The analyzer element: paints `frame` over `background` (the screen's `--display`) at its
/// layout position, snapped to device pixels. Size it with [`css_size`] (`qust_ui`'s view does).
#[derive(IntoElement)]
pub struct Analyzer {
    frame: Frame,
    palette: Palette,
    background: Color,
}

impl Analyzer {
    pub fn new(frame: Frame, colors: Colors, accent_light: Color, background: Color) -> Analyzer {
        Analyzer { frame, palette: Palette::new(colors, accent_light), background }
    }
}

impl RenderOnce for Analyzer {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let (width, height) = css_size(window.scale_factor());
        let pixels = rasterize(&self.frame, &self.palette, self.background);
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let sf = window.scale_factor();
                let s = grid_scale(sf) as i32;
                let (h, v) = device_placement(bounds.origin.x.as_f32(), bounds.origin.y.as_f32(), sf);
                let device = |d: i32| px(d as f32 / sf);
                // A grid pixel's device range on an axis: whole device pixels from the origin,
                // the first one reaching back to the span's start, all cut at its end.
                let range = |span: Span, k: usize| {
                    let from = if k == 0 { span.start } else { span.origin + k as i32 * s };
                    let to = (span.origin + (k as i32 + 1) * s).min(span.end);
                    (from, to)
                };
                for (y, row) in pixels.iter().enumerate() {
                    let (top, bottom) = range(v, y);
                    if bottom <= top {
                        continue;
                    }
                    // Runs of equal color along a row as one quad.
                    let mut x = 0;
                    while x < GRID_WIDTH {
                        let Some(color) = row[x] else {
                            x += 1;
                            continue;
                        };
                        let first = x;
                        while x < GRID_WIDTH && row[x] == Some(color) {
                            x += 1;
                        }
                        let (left, _) = range(h, first);
                        let (_, right) = range(h, x - 1);
                        if right <= left {
                            continue;
                        }
                        let cell = Bounds::new(point(device(left), device(top)), size(device(right - left), device(bottom - top)));
                        window.paint_quad(fill(cell, Color::rgb(color[0], color[1], color[2])));
                    }
                }
            },
        )
        .w(px(width))
        .h(px(height))
        .flex_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::{Path, PathBuf};

    fn reference() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../reference")
    }

    fn fixture_spec(value: &Value) -> Option<FixtureFrame> {
        match value {
            Value::Array(bytes) => Some(FixtureFrame::Raw(bytes.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect())),
            Value::Object(spec) => {
                let list = |v: &Value| v.as_array().map(|a| a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect::<Vec<u8>>());
                let (shape, bars, peaks) = match &spec.get("shape") {
                    Some(Value::String(name)) => (Some(name.clone()), None, None),
                    Some(Value::Object(custom)) => (None, custom.get("bars").and_then(list), custom.get("peaks").and_then(list)),
                    _ => (None, None, None),
                };
                Some(FixtureFrame::Shape {
                    shape,
                    bars,
                    peaks,
                    loader: spec.get("loader").and_then(Value::as_f64).unwrap_or(0.0),
                    loader_time: spec.get("loaderTime").and_then(Value::as_f64).unwrap_or(0.0),
                    off: spec.get("off").and_then(Value::as_bool).unwrap_or(false),
                })
            }
            _ => None,
        }
    }

    // Every player render that shows a frame: the canvas region, device pixel by device pixel,
    // against `rasterize` (no GPUI involved: this checks the color maths and the scaling rule).
    #[test]
    fn rasterize_matches_every_reference_render() {
        let root = reference();
        let mut checked = 0;
        let mut renders = 0;
        for entry in std::fs::read_dir(root.join("renders/player")).expect("renders") {
            let path = entry.expect("entry").path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let Some((state, rest)) = name.split_once('.') else { continue };
            let Some((theme, scale)) = rest.trim_end_matches(".png").split_once('@') else { continue };
            let scale: f32 = scale.parse().expect("scale");
            let layout: Value = serde_json::from_str(&std::fs::read_to_string(root.join(format!("layout/player/{state}.json"))).expect("layout")).expect("json");
            let fixture_file = layout["fixture"].as_str().unwrap_or("").split(' ').next().unwrap_or("").to_string();
            let fixture: Value = serde_json::from_str(&std::fs::read_to_string(root.join(&fixture_file)).expect("fixture")).expect("json");
            // States with an open dialog dim the canvas under the backdrop; skip those.
            if layout["elements"].as_array().unwrap().iter().any(|e| e["name"] == "#dialog") {
                continue;
            }
            let colors = Colors::parse(fixture["state"]["colors"].as_str().unwrap_or("classic"));
            let frame = match fixture.get("frame").and_then(fixture_spec) {
                Some(spec) => frame_from_fixture(&spec, colors == Colors::Off),
                None => BLANK,
            };
            let accent = fixture["accent"]["light"].as_str().and_then(Color::hex).unwrap_or(Color::rgb(76, 194, 255));
            let background = if theme == "dark" { Color::rgb(18, 18, 18) } else { Color::rgb(28, 28, 28) };
            let pixels = rasterize(&frame, &Palette::new(colors, accent), background);
            let image = image::open(&path).expect("png").to_rgba8();
            let s = grid_scale(scale) as i32;
            // #canvas is at (45, 58) CSS px in every layout file.
            let (h, v) = device_placement(45.0, 58.0, scale);
            let bg = [background.r as u8, background.g as u8, background.b as u8];
            let mut wrong = Vec::new();
            // The drawn box and one device pixel around it (which must be the background).
            for y in v.start - 1..=v.end {
                for x in h.start - 1..=h.end {
                    let expected = match (h.cell(x, s), v.cell(y, s)) {
                        (Some(gx), Some(gy)) => pixels[gy][gx].unwrap_or(bg),
                        _ => bg,
                    };
                    let p = image.get_pixel(x as u32, y as u32).0;
                    if [p[0], p[1], p[2]] != expected {
                        wrong.push((x, y, [p[0], p[1], p[2]], expected));
                    }
                    checked += 1;
                }
            }
            assert!(wrong.is_empty(), "{name}: {} pixels differ, first {:?}", wrong.len(), &wrong[..wrong.len().min(5)]);
            renders += 1;
        }
        assert!(renders > 100, "{renders} renders");
        eprintln!("analyzer: {renders} renders, {checked} grid cells, all exact");
    }

    #[test]
    fn scaling_rule() {
        assert_eq!(css_size(1.0), (75.0, 16.0));
        assert_eq!(css_size(1.25), (60.0, 12.8));
        let (w, _) = css_size(1.5);
        assert!((w - 100.0).abs() < 1e-4);
        assert_eq!(css_size(2.0), (75.0, 16.0));
        assert_eq!(grid_scale(2.5), 3.0);
    }

    #[test]
    fn frames_like_frames_mjs() {
        let frame = fixture_frame("empty", 1.0, 0.05, false);
        assert_eq!(frame[0], 255);
        // head = 0.0454… × 25 − 3: bar 0 is the pulse's nearest.
        assert!(frame[3] >= 1 && frame[4] > 0);
        let music = fixture_frame("music", 0.0, 0.0, false);
        assert_eq!((music[1], music[2]), (11, 13));
        assert_eq!(fixture_frame("music", 0.0, 0.0, true)[1], 0);
    }
}
