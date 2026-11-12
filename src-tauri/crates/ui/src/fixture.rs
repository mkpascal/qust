//! The reference fixtures (`reference/states/*.json`): what the player sent plus the user
//! actions that lead to each state. [`Fixture::apply`] puts a fixture's state into the
//! [`Models`] exactly as the bridge would from the live player, so a view rendered for a
//! capture runs the same code as in the app.
//!
//! What a fixture can't express as player state is for the window crate to interpret:
//! `forced` pseudo-states by selector ([`Fixture::forced`]), and `actions` (opening the dialog,
//! a drag, a drop, a key) as raw JSON.

use crate::model::{Background, Entry, PlayerState, Playlist, Shortcuts, Status, StreamInfo, UpdateStatus};
use crate::models::{Models, SettingsModel};
use gpui::App;
use qust_fluent::analyzer::{frame_from_fixture, FixtureFrame, Frame, BLANK};
use qust_fluent::{Accent, Forced};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `reference/`: `QUST_REFERENCE` if set, else the repository's.
pub fn reference_dir() -> PathBuf {
    std::env::var_os("QUST_REFERENCE").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../reference"))
}

/// A fixture's `time` (or one of its `times`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeSpec {
    Ms(f64),
    /// Percent of the longest infinite animation's iteration.
    Percent(f64),
}

impl TimeSpec {
    fn parse(value: &Value) -> Option<TimeSpec> {
        match value {
            Value::Number(n) => n.as_f64().map(TimeSpec::Ms),
            Value::String(s) => s.strip_suffix('%').and_then(|p| p.trim().parse().ok()).map(TimeSpec::Percent),
            _ => None,
        }
    }

    /// The suffix the renders are named with (`-t0450`, `-p50`).
    pub fn suffix(&self) -> String {
        match self {
            TimeSpec::Ms(ms) => format!("-t{:04}", ms.round() as i64),
            TimeSpec::Percent(p) => format!("-p{}", p.round() as i64),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Fixture {
    /// The file's name without `.json`.
    pub name: String,
    pub raw: Value,
    pub window: String,
    pub description: String,
    pub drawer: bool,
    pub state: Option<PlayerState>,
    pub playlist: Vec<Entry>,
    pub frame: Option<FixtureFrame>,
    pub settings: Map<String, Value>,
    pub shortcuts: Option<Shortcuts>,
    pub update: Option<UpdateStatus>,
    pub background: Option<Background>,
    pub version: Option<String>,
    /// `Date.now()` in seconds.
    pub now: Option<u64>,
    pub status: Option<Status>,
    pub info: Option<StreamInfo>,
    pub errors: Map<String, Value>,
    pub actions: Vec<Value>,
    pub time: Option<TimeSpec>,
    pub times: Vec<TimeSpec>,
    pub transitions: bool,
    pub scales: Vec<f32>,
    pub themes: Vec<String>,
    pub accent: Option<Accent>,
    /// A window size override (CSS px).
    pub size: Option<(f32, f32)>,
}

fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                merge(base.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base, over) => *base = over.clone(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Option<T> {
    if value.is_null() {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

impl Fixture {
    /// By name (`stopped`, `settings-default`) or path.
    pub fn load(name_or_path: &str) -> Result<Fixture, String> {
        let path = if name_or_path.ends_with(".json") { PathBuf::from(name_or_path) } else { reference_dir().join("states").join(format!("{name_or_path}.json")) };
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut raw: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        // `"extends": "<state>"`: a deep merge over the base.
        if let Some(base) = raw.get("extends").and_then(Value::as_str).map(str::to_string) {
            let mut merged = Fixture::load(&base)?.raw;
            merge(&mut merged, &raw);
            raw = merged;
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        Ok(Fixture::from_value(name, raw))
    }

    pub fn from_value(name: String, raw: Value) -> Fixture {
        let frame = match &raw["frame"] {
            Value::Array(bytes) => Some(FixtureFrame::Raw(bytes.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect())),
            Value::Object(spec) => {
                let list = |v: &Value| v.as_array().map(|a| a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect::<Vec<u8>>());
                let (shape, bars, peaks) = match spec.get("shape") {
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
        };
        let accent = raw.get("accent").and_then(|a| Accent::parse(a["base"].as_str()?, a["light"].as_str()?, a["dark"].as_str()?));
        let size = raw.get("size").and_then(|s| Some((s.get(0)?.as_f64()? as f32, s.get(1)?.as_f64()? as f32)));
        Fixture {
            name,
            window: raw["window"].as_str().unwrap_or("player").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            drawer: raw["drawer"].as_bool().unwrap_or(false),
            state: parse(&raw["state"]),
            playlist: parse(&raw["playlist"]).unwrap_or_default(),
            frame,
            settings: raw["settings"].as_object().cloned().unwrap_or_default(),
            shortcuts: parse(&raw["shortcuts"]),
            update: parse(&raw["update"]),
            background: parse(&raw["background"]),
            version: raw["version"].as_str().map(str::to_string),
            now: raw["now"].as_u64(),
            status: parse(&raw["status"]),
            info: parse(&raw["info"]),
            errors: raw["errors"].as_object().cloned().unwrap_or_default(),
            actions: raw["actions"].as_array().cloned().unwrap_or_default(),
            time: TimeSpec::parse(&raw["time"]),
            times: raw["times"].as_array().map(|t| t.iter().filter_map(TimeSpec::parse).collect()).unwrap_or_default(),
            transitions: raw["transitions"].as_bool().unwrap_or(false),
            scales: raw["scales"].as_array().map(|s| s.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect()).unwrap_or_else(|| vec![1.0]),
            themes: raw["themes"].as_array().map(|t| t.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_else(|| vec!["dark".into(), "light".into()]),
            accent,
            size,
            raw,
        }
    }

    /// The window's client size in CSS px (as `main.rs` / `playlist.rs` built them, or the
    /// fixture's override).
    pub fn window_size(&self) -> (f32, f32) {
        if let Some(size) = self.size {
            return size;
        }
        match self.window.as_str() {
            "settings" => (520.0, 850.0),
            "info" => (580.0, 590.0),
            _ if self.drawer => (860.0, 587.0),
            _ => (860.0, 267.0),
        }
    }

    /// The analyzer frame it shows (blank when it has none).
    pub fn analyzer_frame(&self) -> Frame {
        let off = self.state.as_ref().is_some_and(|s| s.colors == "off");
        self.frame.as_ref().map_or(BLANK, |spec| frame_from_fixture(spec, off))
    }

    /// The accent the reference used for it.
    pub fn accent(&self) -> Accent {
        self.accent.unwrap_or_else(Accent::reference)
    }

    /// The pseudo-states forced on `selector` by `force` actions.
    pub fn forced(&self, selector: &str) -> Forced {
        let mut forced = Forced::NONE;
        for (sel, names) in self.forced_all() {
            if sel == selector {
                forced = forced.union(names);
            }
        }
        forced
    }

    /// Every `force` action: (selector, states).
    pub fn forced_all(&self) -> Vec<(String, Forced)> {
        let mut all = Vec::new();
        for action in &self.actions {
            if let Some(force) = action.get("force").and_then(Value::as_object) {
                for (selector, names) in force {
                    let names: Vec<&str> = names.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    all.push((selector.clone(), Forced::from_names(names)));
                }
            }
        }
        all
    }

    /// The render names this fixture produced: `name`, or one per `times` entry.
    pub fn variants(&self) -> Vec<(String, Option<TimeSpec>)> {
        if self.times.is_empty() {
            vec![(self.name.clone(), self.time)]
        } else {
            self.times.iter().map(|t| (format!("{}{}", self.name, t.suffix()), Some(*t))).collect()
        }
    }

    /// The frozen animation time for a variant: milliseconds, or a percentage of the longest
    /// infinite animation's iteration as the layout file recorded it.
    pub fn frozen_time(&self, variant: &str, time: Option<TimeSpec>) -> Option<Duration> {
        match time? {
            TimeSpec::Ms(ms) => Some(Duration::from_secs_f64(ms / 1000.0)),
            TimeSpec::Percent(p) => {
                let layout = layout(&self.window, variant)?;
                let longest = layout["animations"]
                    .as_array()?
                    .iter()
                    .filter(|a| a["iterations"].as_str() == Some("infinite") || a["iterations"].as_f64().is_some_and(f64::is_infinite))
                    .filter_map(|a| a["duration"].as_f64())
                    .fold(0.0, f64::max);
                Some(Duration::from_secs_f64(longest * p / 100.0 / 1000.0))
            }
        }
    }

    /// The reference render for a variant, theme and scale.
    ///
    /// The render set is `reference/renders/` (ClearType text, as headless Edge drew it), or the
    /// one `QUST_REFERENCE_SET` names: `renders-gray` has the same states with grayscale text,
    /// which is what the real window drew for most runs (the capture examples' `--gray`).
    pub fn render_path(&self, variant: &str, theme: &str, scale: f32) -> PathBuf {
        let set = std::env::var("QUST_REFERENCE_SET").ok().filter(|s| !s.is_empty() && !s.contains(['/', '\\', '.'])).unwrap_or_else(|| "renders".into());
        reference_dir().join(set).join(&self.window).join(format!("{variant}.{theme}@{}.png", scale_name(scale)))
    }

    /// Puts the fixture into the models, as the live player would have.
    pub fn apply(&self, models: &Models, cx: &mut App) {
        if let Some(state) = &self.state {
            models.apply_state(state.clone(), cx);
        }
        models.apply_playlist(Playlist { entries: self.playlist.clone() }, cx);
        models.apply_frame(self.analyzer_frame(), cx);
        let settings = &self.settings;
        models.apply_settings(SettingsModel::from_values(|key| settings.get(key).cloned()), cx);
        if let Some(shortcuts) = &self.shortcuts {
            models.apply_shortcuts(shortcuts.clone(), cx);
        }
        if let Some(update) = &self.update {
            models.apply_update(update.clone(), cx);
        }
        if let Some(background) = &self.background {
            models.apply_background(background.clone(), cx);
        }
        if let Some(status) = &self.status {
            models.apply_status(status.clone(), cx);
        }
    }
}

/// `1`, `1.25`, `1.5`, `2` as the render names write them.
pub fn scale_name(scale: f32) -> String {
    let text = format!("{scale}");
    text.trim_end_matches(".0").to_string()
}

/// A layout file (`reference/layout/<window>/<state>.json`).
pub fn layout(window: &str, state: &str) -> Option<Value> {
    let text = std::fs::read_to_string(reference_dir().join("layout").join(window).join(format!("{state}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

/// An element's box `[x, y, width, height]` in a layout file (at scale 1, or as measured at
/// `scale` when the file has it).
pub fn element_box(layout: &Value, name: &str, scale: f32) -> Option<[f32; 4]> {
    let scaled = layout["scales"][scale_name(scale)]["boxes"][name].as_array();
    let plain = layout["elements"].as_array()?.iter().find(|e| e["name"] == name).and_then(|e| e["box"].as_array());
    let values = scaled.or(plain)?;
    Some([values.first()?.as_f64()? as f32, values.get(1)?.as_f64()? as f32, values.get(2)?.as_f64()? as f32, values.get(3)?.as_f64()? as f32])
}

/// Every fixture name in `reference/states`.
pub fn all() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(reference_dir().join("states"))
        .map(|dir| dir.filter_map(|e| e.ok()).filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string())).collect())
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_parses() {
        let names = all();
        assert!(names.len() >= 121, "{} fixtures", names.len());
        for name in names {
            let fixture = Fixture::load(&name).unwrap_or_else(|e| panic!("{e}"));
            if fixture.window == "player" {
                assert!(fixture.state.is_some(), "{name}: player-state");
            }
            for (variant, time) in fixture.variants() {
                assert!(fixture.render_path(&variant, "dark", 1.0).exists(), "{name}: {variant} render");
                if let Some(TimeSpec::Percent(_)) = time {
                    assert!(fixture.frozen_time(&variant, time).is_some(), "{variant}: animation time");
                }
            }
        }
        let marquee = Fixture::load("playing-marquee").unwrap();
        assert_eq!(marquee.frozen_time("playing-marquee-p50", Some(TimeSpec::Percent(50.0))), Some(Duration::from_millis(11650)));
        let hover = Fixture::load("buttons-hover").unwrap();
        assert!(!hover.forced_all().is_empty());
    }
}
