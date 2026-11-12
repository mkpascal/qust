//! The player's state split into small GPUI models, so that what changes often re-renders only
//! what shows it. Each model is an `Entity` notified only when its own part changed:
//!
//! | model | holds | notified |
//! |---|---|---|
//! | [`Clock`] | state, whole seconds | once a second while playing, on state changes |
//! | [`Seek`] | duration, position, seekable | 4 × a second while a file plays |
//! | [`NowPlaying`] | station, title, dim, bitrate, sample rate, channels | on title / spec changes |
//! | [`Deck`] | state, cartridges, active slot / entry, instant | on cartridge / state changes |
//! | [`Levels`] | volume, balance | on volume / balance changes |
//! | [`AnalyzerFrame`] | the 77-byte frame, the colors | at up to 60 Hz |
//! | [`PlayerSnapshot`] | the whole `PlayerState` | when anything but the clock fields changed |
//! | [`PlaylistModel`] | the entries | on `player-playlist`, and per entry on `player-entry` |
//! | [`StatusModel`] | `player-status` | every second and on changes |
//! | [`SettingsModel`] | the settings the windows show | on `settings-changed` and local writes |
//! | [`ShortcutsModel`], [`UpdateModel`], [`BackgroundModel`] | | when they change |
//!
//! A view keeps the `Entity` it shows and observes it (`cx.observe(&models.clock, |_, _, cx|
//! cx.notify())`), and reads it in `render` (`self.clock.read(cx)`). Views never write models;
//! the bridge does, on the UI thread.

use crate::model::{Background, Cartridge, Entry, Playback, PlayerState, Playlist, Shortcuts, Status, UpdateStatus};
use gpui::{App, AppContext, Entity};
use qust_fluent::analyzer::{Colors, Frame, BLANK};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clock {
    pub state: Playback,
    /// `Math.floor(elapsed)`, 0 while stopped.
    pub seconds: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seek {
    pub state: Playback,
    pub duration: Option<f64>,
    pub position: Option<f64>,
    pub seekable: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub state: Playback,
    pub station: String,
    pub title: String,
    pub title_dim: bool,
    pub bitrate: Option<f64>,
    pub samplerate: Option<f64>,
    pub channels: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deck {
    pub state: Playback,
    pub cartridges: Vec<Option<Cartridge>>,
    pub active_slot: Option<usize>,
    pub active_id: Option<u64>,
    pub instant: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Levels {
    pub volume: i64,
    pub balance: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalyzerFrame {
    pub frame: Frame,
    pub colors: Colors,
    /// Frames received (tests and measurements).
    pub received: u64,
}

impl Default for AnalyzerFrame {
    fn default() -> Self {
        AnalyzerFrame { frame: BLANK, colors: Colors::Classic, received: 0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerSnapshot {
    pub state: PlayerState,
    /// False until the first state arrived (`player_attach`'s answer or a fixture).
    pub attached: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistModel {
    pub entries: Vec<Entry>,
    /// Bumped on every `player-playlist` (a full rebuild); single-entry changes bump
    /// `entry_versions` for that id only.
    pub version: u64,
    /// Per entry, the `player-entry` changes since the last rebuild: only ids in `entries` (an
    /// entry the playlist no longer has is never added), and cleared by every rebuild, so it
    /// never holds more than the playlist's length (see `apply_entry`).
    pub entry_versions: HashMap<u64, u64>,
}

impl PlaylistModel {
    pub fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusModel {
    pub status: Status,
}

/// The settings the windows show, from the store (`settings.json`) and `settings-changed`.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsModel {
    /// "system", "light" or "dark".
    pub theme: String,
    pub colors: String,
    pub autoplay: bool,
    pub reconnect_minutes: f64,
    pub auto_update: bool,
    pub instant: bool,
    pub playlist_open: bool,
}

impl Default for SettingsModel {
    fn default() -> Self {
        SettingsModel { theme: "system".into(), colors: "classic".into(), autoplay: false, reconnect_minutes: 5.0, auto_update: true, instant: false, playlist_open: false }
    }
}

impl SettingsModel {
    /// From the store's values (`get_data`); missing keys keep their defaults, as the pages did.
    pub fn from_values(get: impl Fn(&str) -> Option<serde_json::Value>) -> SettingsModel {
        let mut settings = SettingsModel::default();
        if let Some(theme) = get("theme").and_then(|v| v.as_str().map(str::to_string)).filter(|t| ["system", "light", "dark"].contains(&t.as_str())) {
            settings.theme = theme;
        }
        if let Some(colors) = get("visualizer-colors").and_then(|v| v.as_str().map(str::to_string)) {
            settings.colors = colors;
        }
        settings.autoplay = get("autoplay") == Some(serde_json::Value::Bool(true));
        if let Some(minutes) = get("reconnect-minutes").and_then(|v| v.as_f64()).filter(|m| *m >= 0.0) {
            settings.reconnect_minutes = minutes;
        }
        settings.auto_update = get("auto-update") != Some(serde_json::Value::Bool(false));
        settings.instant = get("instant-switch") == Some(serde_json::Value::Bool(true));
        settings.playlist_open = get("playlist-open") == Some(serde_json::Value::Bool(true));
        settings
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShortcutsModel {
    pub shortcuts: Shortcuts,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateModel {
    pub update: UpdateStatus,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackgroundModel {
    pub background: Background,
}

/// Every model, created once per app (the player has one state for all windows). Cheap to
/// clone (entity handles).
#[derive(Clone)]
pub struct Models {
    pub clock: Entity<Clock>,
    pub seek: Entity<Seek>,
    pub now: Entity<NowPlaying>,
    pub deck: Entity<Deck>,
    pub levels: Entity<Levels>,
    pub analyzer: Entity<AnalyzerFrame>,
    pub snapshot: Entity<PlayerSnapshot>,
    pub playlist: Entity<PlaylistModel>,
    pub status: Entity<StatusModel>,
    pub settings: Entity<SettingsModel>,
    pub shortcuts: Entity<ShortcutsModel>,
    pub update: Entity<UpdateModel>,
    pub background: Entity<BackgroundModel>,
}

/// Replaces `target` with `value` and notifies, only when they differ.
fn set<T: PartialEq + 'static>(entity: &Entity<T>, value: T, cx: &mut App) -> bool {
    entity.update(cx, |current, cx| {
        if *current == value {
            return false;
        }
        *current = value;
        cx.notify();
        true
    })
}

impl Models {
    pub fn new(cx: &mut App) -> Models {
        Models {
            clock: cx.new(|_| Clock::default()),
            seek: cx.new(|_| Seek::default()),
            now: cx.new(|_| NowPlaying::default()),
            deck: cx.new(|_| Deck::default()),
            levels: cx.new(|_| Levels::default()),
            analyzer: cx.new(|_| AnalyzerFrame::default()),
            snapshot: cx.new(|_| PlayerSnapshot::default()),
            playlist: cx.new(|_| PlaylistModel::default()),
            status: cx.new(|_| StatusModel::default()),
            settings: cx.new(|_| SettingsModel::default()),
            shortcuts: cx.new(|_| ShortcutsModel::default()),
            update: cx.new(|_| UpdateModel::default()),
            background: cx.new(|_| BackgroundModel::default()),
        }
    }

    /// A `player-state` (or the attach snapshot, or a fixture's `state`): each model gets its
    /// part and is notified only if that part changed.
    pub fn apply_state(&self, state: PlayerState, cx: &mut App) {
        set(&self.clock, Clock { state: state.state, seconds: state.clock_seconds() }, cx);
        set(&self.seek, Seek { state: state.state, duration: state.duration, position: state.position, seekable: state.seekable }, cx);
        set(
            &self.now,
            NowPlaying {
                state: state.state,
                station: state.station.clone(),
                title: state.title.clone(),
                title_dim: state.title_dim,
                bitrate: state.bitrate,
                samplerate: state.samplerate,
                channels: state.channels,
            },
            cx,
        );
        set(&self.deck, Deck { state: state.state, cartridges: state.cartridges.clone(), active_slot: state.active_slot, active_id: state.active_id, instant: state.instant }, cx);
        set(&self.levels, Levels { volume: state.volume, balance: state.balance }, cx);
        let colors = Colors::parse(&state.colors);
        self.analyzer.update(cx, |analyzer, cx| {
            if analyzer.colors != colors {
                analyzer.colors = colors;
                cx.notify();
            }
        });
        self.snapshot.update(cx, |snapshot, cx| {
            let changed = !snapshot.attached || snapshot.state.without_clock() != state.without_clock();
            snapshot.state = state;
            snapshot.attached = true;
            if changed {
                cx.notify();
            }
        });
    }

    /// An analyzer frame.
    pub fn apply_frame(&self, frame: Frame, cx: &mut App) {
        self.analyzer.update(cx, |analyzer, cx| {
            analyzer.received += 1;
            if analyzer.frame != frame {
                analyzer.frame = frame;
                cx.notify();
            }
        });
    }

    pub fn apply_playlist(&self, playlist: Playlist, cx: &mut App) {
        self.playlist.update(cx, |model, cx| {
            model.entries = playlist.entries;
            model.version += 1;
            model.entry_versions.clear();
            cx.notify();
        });
    }

    pub fn apply_entry(&self, entry: Entry, cx: &mut App) {
        self.playlist.update(cx, |model, cx| {
            if let Some(slot) = model.entries.iter_mut().find(|e| e.id == entry.id) {
                if *slot != entry {
                    *slot = entry.clone();
                    let version = model.entry_versions.entry(entry.id).or_default();
                    *version = version.wrapping_add(1);
                    cx.notify();
                }
            }
            // Bounded by the entries (ids only get in when found above): drop any stale id a
            // caller might have put there by hand.
            if model.entry_versions.len() > model.entries.len() {
                let ids: std::collections::HashSet<u64> = model.entries.iter().map(|e| e.id).collect();
                model.entry_versions.retain(|id, _| ids.contains(id));
            }
        });
    }

    pub fn apply_status(&self, status: Status, cx: &mut App) {
        set(&self.status, StatusModel { status }, cx);
    }

    pub fn apply_shortcuts(&self, shortcuts: Shortcuts, cx: &mut App) {
        set(&self.shortcuts, ShortcutsModel { shortcuts }, cx);
    }

    pub fn apply_update(&self, update: UpdateStatus, cx: &mut App) {
        set(&self.update, UpdateModel { update }, cx);
    }

    pub fn apply_background(&self, background: Background, cx: &mut App) {
        set(&self.background, BackgroundModel { background }, cx);
    }

    pub fn apply_settings(&self, settings: SettingsModel, cx: &mut App) {
        set(&self.settings, settings, cx);
    }

    /// `settings-changed`: the keys the windows follow.
    pub fn apply_setting(&self, key: &str, value: &serde_json::Value, cx: &mut App) {
        self.settings.update(cx, |settings, cx| {
            let before = settings.clone();
            match key {
                "colors" | "visualizer-colors" => {
                    if let Some(colors) = value.as_str() {
                        settings.colors = colors.to_string();
                    }
                }
                "autoplay" => settings.autoplay = value.as_bool().unwrap_or(false),
                "reconnectMinutes" | "reconnect-minutes" => {
                    if let Some(minutes) = value.as_f64() {
                        settings.reconnect_minutes = minutes;
                    }
                }
                "theme" => {
                    if let Some(theme) = value.as_str() {
                        settings.theme = theme.to_string();
                    }
                }
                "auto-update" => settings.auto_update = value.as_bool().unwrap_or(true),
                _ => {}
            }
            if *settings != before {
                cx.notify();
            }
        });
    }
}
