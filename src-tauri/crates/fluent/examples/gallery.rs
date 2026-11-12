// The element gallery: every qust-fluent element drawn where the reference layout files have
// its HTML counterpart, in the states the fixtures force, then compared with the reference
// renders element by element (`compare.mjs --region <element>`).
//
//   cargo run -p qust-fluent --example gallery -- [out-dir] [state …]
//
// The view reads `reference/layout/<window>/<state>.json` and places, at each element's box:
// islands and setting / info cards, buttons (plain, accent, icon, toggle, text with chevron,
// the update dot), sliders, MONO / STEREO badges, combos, shortcut boxes with key caps, Settings
// toggles, headings and description lists, the info badge, playlist rows and the divider, the
// dialog (backdrop, card, text boxes, footer). Text the window crates own (titles, values) is
// left out unless it is part of an element. Prints one line per element and state, and a
// summary per element kind.

use gpui::{div, prelude::*, px, AnyElement, AnyView, App, Entity, Window};
use gpui_component::input::InputState;
use qust_fluent::section::{channel_badge, info_card, info_heading, info_label, info_value, settings_group, setting_card, state_badge};
use qust_fluent::{icon, list, Button, Combo, Forced, Icon, ShortcutBox, ShortcutDisplay, Slider, SliderSize, Switch, TextBox, TextStyles};
use qust_ui::capture::{self, Job, Setup};
use qust_ui::fixture::{layout, Fixture};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

const STATES: &[&str] = &[
    "stopped",
    "buttons-hover",
    "buttons-pressed",
    "buttons-focus",
    "instant-playing",
    "update-dot",
    "slider-hover",
    "slider-dragging",
    "seek-middle",
    "seek-disabled",
    "playing",
    "specs-mono",
    "dialog-load",
    "dialog-edit-track-file",
    "drawer-entries",
    "drawer-select-single",
    "drawer-row-hover",
    "settings-default",
    "settings-hover",
    "settings-pressed",
    "settings-focus",
    "settings-shortcut-recording",
    "settings-shortcut-unavailable",
    "settings-update-ready",
    "settings-background-unavailable",
    "info-playing-stream",
    "info-stopped",
];

fn boxed(e: &Value) -> Option<[f32; 4]> {
    let b = e["box"].as_array()?;
    Some([b.first()?.as_f64()? as f32, b.get(1)?.as_f64()? as f32, b.get(2)?.as_f64()? as f32, b.get(3)?.as_f64()? as f32])
}

fn classes(e: &Value, theme: &str) -> Vec<String> {
    let class = match &e["class"] {
        Value::Object(by_theme) => by_theme.get(theme).and_then(Value::as_str).unwrap_or(""),
        other => other.as_str().unwrap_or(""),
    };
    class.split_whitespace().map(str::to_string).collect()
}

fn icon_for(symbol: &str, window: &str) -> Option<Icon> {
    let symbol = symbol.trim_start_matches('#');
    let settings = window == "settings";
    Icon::ALL.into_iter().find(|icon| icon.symbol() == symbol && (settings == matches!(icon, Icon::SettingsChevron | Icon::Theme | Icon::Bars | Icon::Autoplay | Icon::Keyboard | Icon::Update | Icon::Clock | Icon::Tray | Icon::Signal | Icon::SettingsInfo)))
}

// GPUI rounds authored lengths to device pixels before layout, while the page's boxes are
// fractional (flex distribution): fractions of the window aren't rounded, so the boxes land where
// Chromium had them and only their painted edges snap.
thread_local! {
    static WINDOW: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((860.0, 267.0)) };
}

fn at(b: [f32; 4], child: impl IntoElement) -> AnyElement {
    let (w, h) = WINDOW.with(|cell| cell.get());
    div()
        .absolute()
        .left(gpui::relative(b[0] / w))
        .top(gpui::relative(b[1] / h))
        .w(gpui::relative(b[2] / w))
        .h(gpui::relative(b[3] / h))
        .flex()
        .child(child)
        .into_any_element()
}

struct Gallery {
    layout: Value,
    fixture: Fixture,
    theme: String,
    inputs: BTreeMap<String, Entity<InputState>>,
}

impl Gallery {
    fn children_of<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Value> + 'a {
        let prefix = format!("{name}>");
        self.layout["elements"].as_array().into_iter().flatten().filter(move |e| e["name"].as_str().is_some_and(|n| n.starts_with(&prefix)))
    }

    fn element(&self, name: &str) -> Option<&Value> {
        self.layout["elements"].as_array()?.iter().find(|e| e["name"] == name)
    }

    // The fixture's forced states for an element, by the selectors the fixtures use.
    fn forced(&self, name: &str) -> Forced {
        let mut forced = self.fixture.forced(name);
        for (selector, states) in self.fixture.forced_all() {
            let matches = match selector.as_str() {
                ".setting:nth-of-type(3) .toggle" => name == "body>.settings-list>.setting[2]>.toggle",
                ".setting:nth-of-type(5) .toggle" => name == "body>.settings-list>.setting[4]>.toggle",
                "[data-action=\"play\"]" => name == "body>.settings-list>.setting[5]>.shortcut",
                "[data-action=\"stop\"]" => name == "body>.settings-list>.setting[6]>.shortcut",
                "#autoplayToggle" => name == "body>.settings-list>.setting[2]>.toggle",
                s if s.starts_with("#playlist > .row:nth-of-type(") => {
                    let n: usize = s.trim_start_matches("#playlist > .row:nth-of-type(").trim_end_matches(')').parse().unwrap_or(0);
                    self.playlist_children().get(n.wrapping_sub(1)).is_some_and(|child| child == name)
                }
                _ => false,
            };
            if matches {
                forced = forced.union(states);
            }
        }
        forced
    }

    fn playlist_children(&self) -> Vec<String> {
        self.children_of("#playlist")
            .filter_map(|e| e["name"].as_str())
            .filter(|n| n.matches('>').count() == 1)
            .map(str::to_string)
            .collect()
    }

    fn build(&self, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
        WINDOW.with(|cell| cell.set(self.fixture.window_size()));
        let theme = *qust_fluent::theme(cx);
        let w = self.fixture.window.clone();
        let mut out = Vec::new();
        let elements = self.layout["elements"].as_array().cloned().unwrap_or_default();
        for e in &elements {
            let Some(name) = e["name"].as_str() else { continue };
            let Some(b) = boxed(e) else { continue };
            let class = classes(e, &self.theme);
            let has = |c: &str| class.iter().any(|k| k == c);
            let attr = |a: &str| e["attrs"].get(a).cloned().unwrap_or(Value::Null);
            let text = e["text"].as_str().unwrap_or("").to_string();
            // A script focus (the dialog's Name field) shows as :focus in the layout file.
            let forced = if e["focused"] == Value::Bool(true) { self.forced(name).union(Forced::FOCUS) } else { self.forced(name) };
            if has("island") && !has("cartridge") {
                out.push(at(b, qust_fluent::surface(qust_fluent::Frame::island(&theme)).size_full()));
            } else if has("button") {
                let mut button = Button::new(gpui::SharedString::from(name.to_string())).forced(forced).disabled(attr("disabled") == Value::Bool(true));
                if has("accent") {
                    button = button.accent();
                }
                if let Some(pressed) = attr("aria-pressed").as_str() {
                    button = button.toggle(pressed == "true");
                }
                let icon_child = self.children_of(name).find(|c| c["icon"].is_string() && !classes(c, &self.theme).iter().any(|k| k == "chevron"));
                let glyph = icon_child.and_then(|c| icon_for(c["icon"].as_str()?, &w));
                if has("icon") {
                    if let Some(glyph) = glyph {
                        button = button.icon_only(glyph);
                    }
                } else {
                    if let Some(glyph) = glyph {
                        button = button.icon(glyph);
                    }
                    if !text.is_empty() {
                        button = button.label(text.clone());
                    }
                }
                if self.children_of(name).any(|c| classes(c, &self.theme).iter().any(|k| k == "chevron")) {
                    button = button.trailing(icon(Icon::Chevron, 12.0).ml(px(1.0)).text_color(theme.text_2));
                }
                if name.starts_with("#playlistButton") || name.starts_with("#addFileButton") || name.starts_with("#addUrlButton") {
                    button = button.gap(7.0).px(px(8.0)).text_size(px(13.0));
                }
                if has("has-update") {
                    button = button.update_dot(true);
                }
                out.push(at(b, button.size_full()));
            } else if has("slider") {
                let value = attr("aria-valuenow").as_str().and_then(|v| v.parse().ok()).unwrap_or(0);
                let min = attr("aria-valuemin").as_str().and_then(|v| v.parse().ok()).unwrap_or(0);
                let max = attr("aria-valuemax").as_str().and_then(|v| v.parse().ok()).unwrap_or(100);
                let seekable = self.element("#sliders").is_some_and(|s| classes(s, &self.theme).iter().any(|k| k == "is-seekable"));
                let size = if name == "#seekSlider" {
                    SliderSize::Seek
                } else if seekable {
                    SliderSize::Pair
                } else {
                    SliderSize::Regular
                };
                let mut slider = Slider::new(gpui::SharedString::from(name.to_string()), value, min, max, "slider")
                    .size(size)
                    .forced(forced)
                    .forced_dragging(has("is-dragging"))
                    .disabled(attr("aria-disabled").as_str() == Some("true"));
                if has("is-centered") {
                    slider = slider.centered(0);
                }
                out.push(at(b, slider.size_full()));
            } else if has("channel") {
                out.push(at(b, channel_badge(&text, has("is-on"), &theme)));
            } else if has("setting") {
                out.push(at(b, setting_card(&theme).size_full()));
            } else if has("setting-icon") {
                if let Some(glyph) = e["icon"].as_str().and_then(|s| icon_for(s, &w)) {
                    out.push(at(b, icon(glyph, 18.0).text_color(theme.text)));
                }
            } else if has("settings-group") {
                out.push(at(b, settings_group(text.clone(), true, &theme).mt_0()));
            } else if has("combo") {
                let label = self.children_of(name).find_map(|c| c["text"].as_str().map(str::to_string)).unwrap_or_default();
                out.push(at(b, Combo::new(gpui::SharedString::from(name.to_string()), label).forced(forced).size_full()));
            } else if has("shortcut") {
                let keys: Vec<String> = self.children_of(name).filter_map(|c| c["text"].as_str().map(str::to_string)).collect();
                let display = if has("is-recording") {
                    ShortcutDisplay::Recording
                } else if has("is-unassigned") {
                    ShortcutDisplay::Unassigned
                } else {
                    ShortcutDisplay::Keys { keys, unavailable: has("is-unavailable") }
                };
                out.push(at(b, ShortcutBox::new(gpui::SharedString::from(name.to_string()), display, "shortcut").forced(forced)));
            } else if has("toggle") {
                let input = elements.iter().find(|i| i["tag"] == "input" && boxed(i).is_some_and(|ib| (ib[0] - b[0]).abs() < 0.01));
                let checked = input.is_some_and(|i| i["attrs"]["checked"] == Value::Bool(true));
                // .setting.is-disabled .toggle { opacity: 0.45 }: the setting around it says.
                let setting = name.rsplit_once('>').map(|(parent, _)| parent).and_then(|parent| self.element(parent));
                let disabled = input.is_some_and(|i| i["attrs"]["disabled"] == Value::Bool(true)) || setting.is_some_and(|s| classes(s, &self.theme).iter().any(|k| k == "is-disabled"));
                out.push(at(b, Switch::new(gpui::SharedString::from(name.to_string()), checked, "toggle").disabled(disabled).forced(forced)));
            } else if has("info-card") {
                out.push(at(b, info_card(&theme).size_full()));
            } else if e["tag"] == "h2" && name.contains("info-card") {
                out.push(at(b, info_heading(&text, &theme)));
            } else if e["tag"] == "dt" {
                out.push(at(b, info_label(text.clone(), name.contains("info-pairs"), &theme)));
            } else if e["tag"] == "dd" {
                out.push(at(b, info_value(text.clone(), name.contains("info-pairs"), true, &theme).w_full()));
            } else if has("info-state") {
                out.push(at(b, state_badge(text.clone(), e["attrs"]["data-state"] == "playing" || text == "Playing", &theme)));
            } else if has("row") {
                let number = self.children_of(name).find(|c| classes(c, &self.theme).iter().any(|k| k == "row-index"));
                let cartridge = has("is-cartridge");
                let active = has("is-active");
                let index_text = number.and_then(|n| n["text"].as_str()).unwrap_or("").to_string();
                let title = self.children_of(name).find(|c| classes(c, &self.theme).iter().any(|k| k == "row-title")).and_then(|c| c["text"].as_str()).unwrap_or("").to_string();
                let detail = self.children_of(name).find(|c| classes(c, &self.theme).iter().any(|k| k == "row-detail")).and_then(|c| c["text"].as_str()).map(str::to_string);
                let num = list::row_number(active, cartridge, &theme);
                let num = if cartridge { num.child(icon(Icon::Radio, 14.0).text_color(if active { theme.accent } else { theme.text_2 })) } else { num.child(index_text) };
                let row = list::row(gpui::SharedString::from(name.to_string()), has("is-selected"), forced, &theme)
                    .child(num)
                    .child(list::row_title(title, active, has("is-missing"), &theme))
                    .children(detail.map(|d| list::row_detail(d, &theme)));
                out.push(at(b, row.w_full()));
            } else if has("playlist-divider") {
                out.push(at(b, div().w_full().h(px(1.0)).bg(theme.card_stroke).relative().child(div().absolute().top(px(1.0)).left_0().right_0().h(px(1.0)).bg(theme.card_edge))));
            } else if has("dialog-backdrop") {
                out.push(at(b, div().size_full().bg(theme.smoke)));
            } else if has("dialog") {
                let card = qust_fluent::DialogCard::new("card", std::time::Instant::now(), "dialog");
                out.push(at(b, card.size_full()));
            } else if has("dialog-footer") {
                out.push(at(b, qust_fluent::dialog::dialog_footer(cx).size_full()));
            } else if name == "#dialogTitle" {
                out.push(at(b, qust_fluent::dialog::dialog_title(text.clone(), cx)));
            } else if name == "#nameLabel" || name == "#urlLabel" {
                out.push(at(b, div().font_text().type_size(13.0, 18.0).text_color(theme.text).child(text.clone())));
            } else if has("textbox") {
                if let Some(state) = self.inputs.get(name) {
                    let readonly = attr("readonly") == Value::Bool(true);
                    out.push(at(b, TextBox::new(gpui::SharedString::from(name.to_string()), state).readonly(readonly).forced(forced).w_full()));
                }
            }
        }
        let _ = window;
        out
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let children = self.build(window, cx);
        div().relative().size_full().children(children)
    }
}

fn build(setup: &Setup, window: &mut Window, cx: &mut App) -> AnyView {
    let fixture = setup.job.fixture.clone();
    let layout = layout(&fixture.window, &setup.job.variant).unwrap_or(Value::Null);
    let mut inputs = BTreeMap::new();
    for e in layout["elements"].as_array().into_iter().flatten() {
        if classes(e, &setup.job.theme).iter().any(|k| k == "textbox") {
            let name = e["name"].as_str().unwrap_or("").to_string();
            let placeholder = e["attrs"]["placeholder"].as_str().unwrap_or("").to_string();
            let value = e["attrs"]["value"].as_str().unwrap_or("").to_string();
            inputs.insert(name, qust_fluent::input_state(placeholder, value, window, cx));
        }
    }
    let theme = setup.job.theme.clone();
    cx.new(|_| Gallery { layout, fixture, theme, inputs }).into()
}

// The regions compared, per window: the element kinds the gallery draws.
fn regions(layout: &Value, theme: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in layout["elements"].as_array().into_iter().flatten() {
        let Some(name) = e["name"].as_str() else { continue };
        let class = classes(e, theme);
        let kind = ["button", "slider", "channel", "setting", "combo", "shortcut", "toggle-track", "info-card", "info-state", "row", "dialog", "textbox", "island"]
            .into_iter()
            .find(|k| class.iter().any(|c| c == k));
        let Some(kind) = kind else { continue };
        if class.iter().any(|c| c == "cartridge") || kind == "island" && name.starts_with("#cartridges") {
            continue;
        }
        let variant = if class.iter().any(|c| c == "accent") {
            format!("{kind}.accent")
        } else if class.iter().any(|c| c == "icon") {
            format!("{kind}.icon")
        } else {
            kind.to_string()
        };
        out.push((name.to_string(), variant));
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("qust-gallery"));
    let wanted: Vec<String> = if args.len() > 1 { args[1..].to_vec() } else { STATES.iter().map(|s| s.to_string()).collect() };
    let mut jobs = Vec::new();
    for state in &wanted {
        let Ok(fixture) = Fixture::load(state) else {
            eprintln!("no fixture {state}");
            continue;
        };
        for theme in ["dark", "light"] {
            jobs.extend(Job::all(&fixture, theme, &out));
        }
    }
    let results = capture::capture_all(jobs.clone(), build);
    // kind → (regions, regions matching, worst ratio)
    let mut summary: BTreeMap<String, (u32, u32, f64, u32, u32)> = BTreeMap::new();
    for (job, result) in jobs.iter().zip(results) {
        let (path, scale) = match result {
            Ok(r) => r,
            Err(error) => {
                println!("{} {}: {error}", job.variant, job.theme);
                continue;
            }
        };
        let reference = job.fixture.render_path(&job.variant, &job.theme, scale);
        let layout = layout(&job.fixture.window, &job.variant).unwrap_or(Value::Null);
        for (name, kind) in regions(&layout, &job.theme) {
            let region_path = path.with_file_name(format!("{}.{}.{}.png", job.variant, job.theme, name.replace(['#', '>', '[', ']', '.', ' '], "_")));
            let _ = std::fs::copy(&path, &region_path);
            match capture::compare(&reference, &region_path, &["--region", &name]) {
                Ok(c) => {
                    let mean = c.report["pixels"]["meanDifference"].as_f64().unwrap_or(0.0);
                    println!(
                        "{:<5} {:<34} {:<52} {:<14} {:>7.3} % px, mean {:>5.2}, text {}/{}",
                        job.theme,
                        job.variant,
                        name,
                        kind,
                        c.ratio * 100.0,
                        mean,
                        c.text_ok,
                        c.text_runs
                    );
                    let entry = summary.entry(format!("{kind} {}", job.theme)).or_default();
                    entry.0 += 1;
                    if c.ratio <= 0.001 {
                        entry.1 += 1;
                    }
                    entry.2 = entry.2.max(c.ratio);
                    entry.3 += c.text_ok as u32;
                    entry.4 += c.text_runs as u32;
                }
                Err(e) => println!("{} {} {name}: {e}", job.variant, job.theme),
            }
            let _ = std::fs::remove_file(region_path.with_extension("report.json"));
        }
    }
    println!("\nsummary (element kind, theme: regions within 0.1 % / all, worst non-text pixel ratio, text runs matching)");
    for (kind, (all, good, worst, text_ok, text_all)) in summary {
        println!("  {kind:<22} {good:>3}/{all:<3} worst {:>7.3} %  text {text_ok}/{text_all}", worst * 100.0);
    }
}
