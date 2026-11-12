// Shows the dynamic text of the qust-text fixtures (`reference/states/text-*.json`) with GPUI, in
// place, over the reference render, and compares it with what Chromium drew there.
//
//   cargo run -p qust-text --example text-capture -- [out-dir] [fixture ...]
//
// For each fixture variant (dark theme) it opens one hidden capture window per mode: the window
// shows the reference render itself, and over every element that holds a title, a station name,
// a cartridge name, a host or a value (the boxes of `reference/layout/...`), a patch of that
// element's background with the element's text drawn by GPUI in the element's font, size, weight,
// color, alignment and ellipsis (the title row with qust-fluent's `Marquee`, frozen like the
// reference). Modes: `raw` hands GPUI the logical string, `text` what `qust_text::display` returns.
// Text boxes (the dialog's fields) get qust-fluent's `TextBox` with the logical value in both
// modes: edited text can't be reordered, so they show what GPUI itself does.
//
// Writes `<variant>.raw.png`, `<variant>.text.png` and `<variant>.sbs.png`: for each element, the
// reference above the raw capture above the qust-text capture (2x, magenta separators). Prints,
// per element, the normalized cross-correlation of the ink with the reference (best over
// ±3 px × ±2 px shifts): glyph rasterisation differs, so a correct order and shaping scores high
// but not 1, a reversed or unjoined one much lower.

use gpui::{div, img, prelude::*, px, AnyView, App, FontWeight, ImageSource, RenderImage, Window};
use gpui_component::input::InputState;
use image::{Rgba, RgbaImage};
use qust_fluent::{Color, Marquee, TextBox, TextStyles};
use qust_ui::capture::{self, Job, Setup};
use qust_ui::fixture::{layout, Fixture};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FIXTURES: &[&str] = &["text-rtl-player", "text-rtl-playlist", "text-rtl-marquee", "text-mixed-marquee", "text-rtl-info", "text-rtl-dialog", "text-scripts-player"];

#[derive(Clone)]
struct Item {
    name: String,
    text: String,
    bounds: [f32; 4],
    visible: [f32; 4],
    style: Value,
    marquee: Option<bool>,
    // A text box: its value stays logical (it's edited), so this is GPUI's own rendering.
    input: bool,
}

fn rect(value: &Value) -> Option<[f32; 4]> {
    let a = value.as_array()?;
    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32, a.get(3)?.as_f64()? as f32])
}

// The elements whose text comes from a stream, a file or the user.
fn items(window: &str, variant: &str) -> Vec<Item> {
    let Some(layout) = layout(window, variant) else { return Vec::new() };
    let Some(elements) = layout["elements"].as_array() else { return Vec::new() };
    let title = elements.iter().find(|e| e["name"] == "#title");
    let mut out = Vec::new();
    for e in elements {
        let name = e["name"].as_str().unwrap_or_default();
        let wanted = match window {
            "player" => {
                name == "#station"
                    || name == "#titleRow"
                    || name.ends_with(".cartridge-name")
                    || name.ends_with(".cartridge-host")
                    || name.ends_with(".row-title")
                    || name.ends_with(".row-detail")
                    || e["tag"] == "input"
            }
            _ => name == "#station" || name == "#title" || e["tag"] == "dd",
        };
        if !wanted {
            continue;
        }
        let Some(mut bounds) = rect(&e["box"]) else { continue };
        // A cartridge's name and host are centered and as wide as their text (at most the
        // card's body): give GPUI the body's width, or its own measure decides the ellipsis.
        if name.ends_with(".cartridge-name") || name.ends_with(".cartridge-host") {
            let body = name.rsplit_once(">.").map(|(body, _)| body).unwrap_or_default();
            if let Some(body) = elements.iter().find(|p| p["name"] == body) {
                if let Some(b) = rect(&body["box"]) {
                    // Its content box: `padding: 4px 12px`.
                    let side = body["style"]["padding"].get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    bounds[0] = b[0] + side;
                    bounds[2] = b[2] - 2.0 * side;
                }
            }
        }
        let visible = if name.contains(".cartridge-") { bounds } else { rect(&e["visible"]).unwrap_or(bounds) };
        if visible[2] < 1.0 || visible[3] < 1.0 {
            continue;
        }
        let input = e["tag"] == "input";
        let (text, style, marquee) = if input {
            (e["attrs"]["value"].as_str().unwrap_or_default().to_string(), e["style"].clone(), None)
        } else if name == "#titleRow" {
            let Some(title) = title else { continue };
            let dim = title["class"].as_str().is_some_and(|c| c.contains("is-dim"));
            (title["text"].as_str().unwrap_or_default().to_string(), title["style"].clone(), Some(dim))
        } else {
            (e["text"].as_str().unwrap_or_default().to_string(), e["style"].clone(), None)
        };
        if text.is_empty() {
            continue;
        }
        out.push(Item { name: name.to_string(), text, bounds, visible, style, marquee, input });
    }
    out
}

fn dark_color(style: &Value) -> Color {
    let c = &style["color"];
    let c = if c.get("dark").is_some() { &c["dark"] } else { c };
    let v = |i: usize| c.get(i).and_then(Value::as_f64).unwrap_or(if i == 3 { 1.0 } else { 255.0 });
    Color { r: v(0) as f32, g: v(1) as f32, b: v(2) as f32, a: v(3) as f32 }
}

fn pixel(image: &RgbaImage, x: f32, y: f32) -> Color {
    let x = (x.max(0.0) as u32).min(image.width() - 1);
    let y = (y.max(0.0) as u32).min(image.height() - 1);
    let Rgba([r, g, b, _]) = *image.get_pixel(x, y);
    Color::rgb(r, g, b)
}

// The background under an element: the most common color along the inside of its edges (text
// can touch an edge, it doesn't cover it).
fn background(image: &RgbaImage, item: &Item) -> Color {
    let [x, y, w, h] = item.visible;
    let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    let mut count = |px: f32, py: f32| {
        let c = pixel(image, px, py);
        *counts.entry([c.r as u8, c.g as u8, c.b as u8]).or_default() += 1;
    };
    for i in 0..w.max(1.0) as i32 {
        count(x + i as f32, y + 0.5);
        count(x + i as f32, y + h - 1.0);
    }
    for j in 0..h.max(1.0) as i32 {
        count(x + 0.5, y + j as f32);
        count(x + w - 1.0, y + j as f32);
    }
    counts.into_iter().max_by_key(|&(_, n)| n).map(|([r, g, b], _)| Color::rgb(r, g, b)).unwrap_or(Color::BLACK)
}

fn render_image(image: &RgbaImage) -> Arc<RenderImage> {
    let mut bgra = image.clone();
    for p in bgra.pixels_mut() {
        p.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(bgra)]))
}

struct Transplant {
    reference: Arc<RenderImage>,
    size: (f32, f32),
    texts: Vec<(Item, Color, String)>,
    marquees: Vec<(Item, Color, gpui::Entity<Marquee>)>,
    inputs: Vec<(Item, Color, gpui::Entity<InputState>)>,
}

impl Render for Transplant {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let mut root = div().relative().w(px(self.size.0)).h(px(self.size.1)).child(
            img(ImageSource::Render(self.reference.clone())).absolute().left_0().top_0().w(px(self.size.0)).h(px(self.size.1)),
        );
        for (item, bg, shown) in &self.texts {
            let s = &item.style;
            let [x, y, w, h] = item.bounds;
            let [vx, vy, vw, vh] = item.visible;
            let mut text = div()
                .absolute()
                .left(px(x - vx))
                .top(px(y - vy))
                .w(px(w))
                .h(px(h))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(dark_color(s))
                .child(shown.clone());
            text = if s["font-family"].as_str().is_some_and(|f| f.starts_with("\"Segoe UI Variable Display")) { text.font_display() } else { text.font_text() };
            let size = s["font-size"].as_f64().unwrap_or(14.0) as f32;
            let line = s["line-height"].as_f64().map(|l| l as f32).unwrap_or(h);
            text = text.type_size(size, line).font_weight(if s["font-weight"].as_f64().unwrap_or(400.0) >= 600.0 { FontWeight::SEMIBOLD } else { FontWeight::NORMAL });
            if s["text-overflow"] == "ellipsis" {
                text = text.text_ellipsis();
            }
            if s["text-align"] == "center" {
                text = text.text_center();
            }
            if s["font-variant-numeric"] == "tabular-nums" {
                text = text.tabular_nums();
            }
            if s["text-decoration-line"] == "line-through" {
                text = text.line_through();
            }
            root = root.child(div().absolute().left(px(vx)).top(px(vy)).w(px(vw)).h(px(vh)).overflow_hidden().bg(*bg).child(text));
        }
        for (item, bg, marquee) in &self.marquees {
            let [x, y, w, h] = item.bounds;
            root = root.child(div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).bg(*bg).child(marquee.clone()));
        }
        for (k, (item, bg, state)) in self.inputs.iter().enumerate() {
            let [x, y, w, h] = item.bounds;
            root = root.child(div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).bg(*bg).child(TextBox::new(("box", k), state).w(px(w))));
        }
        root
    }
}

fn mode_of(job: &Job) -> &'static str {
    if job.out.to_string_lossy().ends_with(".raw.png") {
        "raw"
    } else {
        "text"
    }
}

fn shown(text: &str, mode: &str) -> String {
    if mode == "raw" {
        text.to_string()
    } else {
        qust_text::display(text).text.into_owned()
    }
}

fn build(setup: &Setup, window: &mut Window, cx: &mut App) -> AnyView {
    let job = setup.job;
    let mode = mode_of(job);
    let reference_path = job.fixture.render_path(&job.variant, "dark", 1.0);
    let reference = image::open(&reference_path).map(|i| i.to_rgba8()).unwrap_or_else(|_| RgbaImage::new(1, 1));
    let size = job.fixture.window_size();
    let mut texts = Vec::new();
    let mut marquees = Vec::new();
    let mut inputs = Vec::new();
    for item in items(&job.fixture.window, &job.variant) {
        let bg = background(&reference, &item);
        if item.input {
            let state = qust_fluent::input_state("", item.text.clone(), window, cx);
            inputs.push((item, bg, state));
        } else if let Some(dim) = item.marquee {
            let colors = (Color::WHITE, Color::rgba(255, 255, 255, 0.545));
            let text = shown(&item.text, mode);
            marquees.push((item.clone(), bg, cx.new(|_| Marquee::new(text, dim, colors, bg))));
        } else {
            let text = shown(&item.text, mode);
            texts.push((item, bg, text));
        }
    }
    cx.new(|_| Transplant { reference: render_image(&reference), size, texts, marquees, inputs }).into()
}

// Ink: how far each pixel's luminance is from the background's.
fn ink(image: &RgbaImage, bg: Color, x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || x as u32 >= image.width() || y as u32 >= image.height() {
        return 0.0;
    }
    let Rgba([r, g, b, _]) = *image.get_pixel(x as u32, y as u32);
    let lum = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    (lum(r as f32, g as f32, b as f32) - lum(bg.r, bg.g, bg.b)).abs()
}

// Normalized cross-correlation of the ink in `area`, best over small shifts of the capture.
fn correlation(reference: &RgbaImage, capture: &RgbaImage, bg: Color, area: [f32; 4]) -> f32 {
    let (x0, y0) = (area[0].floor() as i32, area[1].floor() as i32);
    let (x1, y1) = ((area[0] + area[2]).ceil() as i32, (area[1] + area[3]).ceil() as i32);
    let a: Vec<f32> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| ink(reference, bg, x, y)).collect();
    let mean_a = a.iter().sum::<f32>() / a.len().max(1) as f32;
    let mut best = -1.0f32;
    for dy in -2..=2 {
        for dx in -3..=3 {
            let b: Vec<f32> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| ink(capture, bg, x + dx, y + dy)).collect();
            let mean_b = b.iter().sum::<f32>() / b.len().max(1) as f32;
            let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
            for (p, q) in a.iter().zip(&b) {
                ab += (p - mean_a) * (q - mean_b);
                aa += (p - mean_a) * (p - mean_a);
                bb += (q - mean_b) * (q - mean_b);
            }
            if aa > 0.0 && bb > 0.0 {
                best = best.max(ab / (aa * bb).sqrt());
            }
        }
    }
    best
}

fn crop(image: &RgbaImage, area: [i32; 4]) -> RgbaImage {
    let mut out = RgbaImage::new(area[2] as u32, area[3] as u32);
    for y in 0..area[3] {
        for x in 0..area[2] {
            let (sx, sy) = (area[0] + x, area[1] + y);
            if sx >= 0 && sy >= 0 && (sx as u32) < image.width() && (sy as u32) < image.height() {
                out.put_pixel(x as u32, y as u32, *image.get_pixel(sx as u32, sy as u32));
            }
        }
    }
    out
}

// Each element: reference, raw, qust-text, stacked, at 2x.
fn side_by_side(items: &[Item], images: [&RgbaImage; 3], out: &Path) {
    const SCALE: u32 = 2;
    // Around the ink of all three (a row's box is much wider than its text).
    let areas: Vec<[i32; 4]> = items
        .iter()
        .map(|i| {
            let v = i.visible;
            let bg = background(images[0], i);
            let (x0, x1) = (v[0].floor() as i32, (v[0] + v[2]).ceil() as i32);
            let (y0, y1) = (v[1].floor() as i32, (v[1] + v[3]).ceil() as i32);
            let inked = |x: i32| images.iter().any(|image| (y0..y1).any(|y| ink(image, bg, x, y) > 40.0));
            let left = (x0..x1).find(|&x| inked(x)).unwrap_or(x0);
            let right = (x0..x1).rev().find(|&x| inked(x)).unwrap_or(x1);
            [left - 4, y0 - 2, (right - left + 9).clamp(8, 480), y1 - y0 + 4]
        })
        .collect();
    let width = areas.iter().map(|a| a[2] as u32).max().unwrap_or(1) * SCALE;
    let height: u32 = areas.iter().map(|a| (a[3] as u32 * 3 + 2) * SCALE + 8).sum::<u32>().max(1);
    let mut sheet = RgbaImage::from_pixel(width, height, Rgba([40, 40, 40, 255]));
    let mut y = 0;
    for area in &areas {
        for (k, image) in images.iter().enumerate() {
            let piece = crop(image, *area);
            for py in 0..piece.height() * SCALE {
                for px in 0..piece.width() * SCALE {
                    sheet.put_pixel(px, y + py, *piece.get_pixel(px / SCALE, py / SCALE));
                }
            }
            y += piece.height() * SCALE;
            if k < 2 {
                for px in 0..width {
                    sheet.put_pixel(px, y, Rgba([255, 0, 255, 255]));
                }
                y += 1;
            }
        }
        y += 8;
    }
    let _ = sheet.save(out);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("qust-text-capture"));
    let names: Vec<String> = if args.len() > 1 { args[1..].to_vec() } else { FIXTURES.iter().map(|s| s.to_string()).collect() };
    let mut jobs = Vec::new();
    for name in &names {
        let fixture = match Fixture::load(name) {
            Ok(fixture) => fixture,
            Err(error) => {
                eprintln!("{name}: {error}");
                std::process::exit(2);
            }
        };
        for mut job in Job::all(&fixture, "dark", &out) {
            for mode in ["raw", "text"] {
                job.out = out.join(format!("{}.{mode}.png", job.variant));
                jobs.push(job.clone());
            }
        }
    }
    let results = capture::capture_all(jobs.clone(), build);
    for pair in jobs.chunks(2).zip(results.chunks(2)) {
        let (jobs, results) = pair;
        let job = &jobs[0];
        if let Some(Err(error)) = results.iter().find(|r| r.is_err()) {
            println!("{}: {error}", job.variant);
            continue;
        }
        let Ok(reference) = image::open(job.fixture.render_path(&job.variant, "dark", 1.0)).map(|i| i.to_rgba8()) else { continue };
        let Ok(raw) = image::open(&jobs[0].out).map(|i| i.to_rgba8()) else { continue };
        let Ok(text) = image::open(&jobs[1].out).map(|i| i.to_rgba8()) else { continue };
        let items = items(&job.fixture.window, &job.variant);
        println!("{} ({})", job.variant, out.join(format!("{}.sbs.png", job.variant)).display());
        for item in &items {
            let bg = background(&reference, item);
            let raw_score = correlation(&reference, &raw, bg, item.visible);
            let text_score = correlation(&reference, &text, bg, item.visible);
            let rtl = if qust_text::has_rtl(&item.text) { "rtl" } else { "   " };
            println!("  {rtl} raw {raw_score:5.2}  text {text_score:5.2}  {}  {}", item.name, item.text);
        }
        // The sheet shows the elements whose text isn't plain ASCII.
        let shown: Vec<Item> = items.into_iter().filter(|i| !i.text.is_ascii()).collect();
        side_by_side(&shown, [&reference, &raw, &text], &out.join(format!("{}.sbs.png", job.variant)));
    }
}
