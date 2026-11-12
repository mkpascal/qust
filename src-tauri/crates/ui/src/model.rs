//! The player's messages as types, deserialised from the same JSON `qust-player` emits and the
//! reference fixtures (`reference/states/*.json`) contain, so one code path serves both.
//!
//! Field names follow the JSON (camelCase there, snake_case here). Everything a view shows is
//! derived from these, the way the page derived it (enabled buttons, formatted times, a URL's
//! host); [`format`] has the page's formatting functions.

use serde::{Deserialize, Serialize};

/// `state` in `player-state`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Playback {
    #[default]
    Stopped,
    Connecting,
    Playing,
    Buffering,
    Paused,
}

impl Playback {
    pub fn name(self) -> &'static str {
        match self {
            Playback::Stopped => "stopped",
            Playback::Connecting => "connecting",
            Playback::Playing => "playing",
            Playback::Buffering => "buffering",
            Playback::Paused => "paused",
        }
    }

    /// Playing, connecting or buffering: what the page called "playing" for the equalizer and
    /// the Play / Pause buttons (`!['stopped', 'paused'].includes(state)`).
    pub fn is_running(self) -> bool {
        !matches!(self, Playback::Stopped | Playback::Paused)
    }
}

/// A cartridge's instant switch bolt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preload {
    Loading,
    Live,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cartridge {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub preload: Option<Preload>,
}

/// `player-state`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerState {
    pub state: Playback,
    /// 6 slots.
    pub cartridges: Vec<Option<Cartridge>>,
    pub active_slot: Option<usize>,
    pub active_id: Option<u64>,
    pub instant: bool,
    pub station: String,
    pub title: String,
    pub title_dim: bool,
    pub bitrate: Option<f64>,
    pub samplerate: Option<f64>,
    pub channels: Option<f64>,
    pub volume: i64,
    pub balance: i64,
    pub colors: String,
    pub elapsed: f64,
    pub duration: Option<f64>,
    pub position: Option<f64>,
    pub seekable: bool,
}

impl PlayerState {
    pub fn cartridge(&self, slot: usize) -> Option<&Cartridge> {
        self.cartridges.get(slot).and_then(Option::as_ref)
    }

    /// Something is active (a cartridge or an entry): Play can start it.
    pub fn has_active(&self) -> bool {
        self.active_slot.is_some() || self.active_id.is_some()
    }

    /// The clock in whole seconds (`Math.floor(elapsed)`, 0 while stopped).
    pub fn clock_seconds(&self) -> u64 {
        if self.state == Playback::Stopped {
            0
        } else {
            self.elapsed.max(0.0).floor() as u64
        }
    }

    /// The same state without the fields that tick (`elapsed`, `position`): what decides
    /// whether anything but the clock changed.
    pub fn without_clock(&self) -> PlayerState {
        PlayerState { elapsed: 0.0, position: None, ..self.clone() }
    }
}

/// A playlist entry (`player-playlist` entries, `player-entry`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: u64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub duration: Option<f64>,
    pub missing: bool,
}

/// `player-playlist`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub entries: Vec<Entry>,
}

/// `player-status` (the Stream information window).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Status {
    pub state: Playback,
    pub station: Option<String>,
    pub url: Option<String>,
    pub buffer: Option<f64>,
    pub stalls: u64,
    pub status_title: Option<String>,
}

/// `stream_info` (`qust_player::stream_info`): every field may be missing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamInfo {
    pub title: Option<String>,
    pub name: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub website: Option<String>,
    pub server: Option<String>,
    pub content_type: Option<String>,
    pub format: Option<String>,
    pub bitrate: Option<f64>,
    pub rate: Option<f64>,
    pub samplerate: Option<f64>,
    pub channels: Option<f64>,
    pub metaint: Option<f64>,
    pub size: Option<f64>,
    pub bytes: Option<f64>,
    pub meta_bytes: Option<f64>,
    pub title_changes: Option<f64>,
    pub reconnects: Option<f64>,
    pub connected_seconds: Option<f64>,
}

/// `settings-changed` `{ key, value }` (`colors` from the analyzer click, `shortcuts`, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingChange {
    pub key: String,
    #[serde(default)]
    pub value: serde_json::Value,
}

/// One global shortcut (`get_shortcuts`): an accelerator built from `KeyboardEvent.code`
/// (`Ctrl+Alt+KeyH`, empty for none) and whether it is registered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Binding {
    pub accelerator: String,
    pub registered: bool,
}

impl Binding {
    pub fn assigned(&self) -> bool {
        !self.accelerator.is_empty()
    }

    /// The keys a tooltip shows ("Play (Ctrl+Alt+Insert)"), when registered.
    pub fn tooltip_keys(&self) -> Option<String> {
        (self.assigned() && self.registered).then(|| qust_fluent::shortcut_keys(&self.accelerator).join("+"))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub play: Binding,
    pub stop: Binding,
    pub toggle: Binding,
}

/// The updater's status as the Settings page knows it (`update_status`, `update-status`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateStatus {
    pub installed: bool,
    pub current: String,
    /// "idle", "checking", "current", "downloading", "ready", "installing", "error".
    pub status: String,
    pub version: Option<String>,
    pub progress: Option<u32>,
    pub error: Option<String>,
    pub last_check: Option<u64>,
    pub auto: bool,
}

impl UpdateStatus {
    /// From `qust_updater`'s snapshot and what the app knows around it.
    pub fn from_snapshot(snapshot: &qust_updater::Snapshot, installed: bool, current: &str, last_check: Option<u64>, auto: bool) -> UpdateStatus {
        UpdateStatus {
            installed,
            current: current.to_string(),
            status: snapshot.status.as_str().to_string(),
            version: snapshot.version.clone(),
            progress: snapshot.progress,
            error: snapshot.error.clone(),
            last_check: snapshot.checked_at.or(last_check),
            auto,
        }
    }

    pub fn ready(&self) -> bool {
        self.status == "ready"
    }
}

/// The background helper (`get_background`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Background {
    pub available: bool,
    pub enabled: bool,
    pub starts_with_windows: bool,
}

/// The page's formatting functions.
pub mod format {
    /// The display clock (`renderTransport`): `MM:SS` under an hour (minutes padded), `H:MM:SS`
    /// from one hour (hours not padded). Seconds are already floored.
    pub fn clock(seconds: u64) -> String {
        let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m:02}:{s:02}")
        }
    }

    /// `formatTime` (seek text, playlist durations): rounded, `M:SS` (minutes not padded),
    /// `H:MM:SS` from an hour.
    pub fn time(seconds: f64) -> String {
        let total = (seconds.max(0.0) + 0.5).floor() as u64;
        let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
    }

    /// kHz as the specs line shows it: an integer when whole, else one decimal (`toFixed(1)`).
    pub fn khz(samplerate: f64) -> String {
        let khz = samplerate / 1000.0;
        if khz.fract() == 0.0 {
            format!("{}", khz as i64)
        } else {
            to_fixed(khz, 1)
        }
    }

    /// JavaScript's `Number.prototype.toFixed` for the values the pages format (round half
    /// away from zero on the decimal value).
    pub fn to_fixed(value: f64, digits: usize) -> String {
        let factor = 10f64.powi(digits as i32);
        let rounded = (value * factor).round() / factor;
        format!("{rounded:.digits$}")
    }

    /// `new URL(url).host`: the host with the port unless it is the scheme's default.
    pub fn host(url: &str) -> String {
        let Some((scheme, rest)) = url.split_once("://") else { return String::new() };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let authority = authority.rsplit('@').next().unwrap_or(authority).to_ascii_lowercase();
        let default = match scheme.to_ascii_lowercase().as_str() {
            "http" => Some(":80"),
            "https" => Some(":443"),
            _ => None,
        };
        match default {
            Some(port) if authority.ends_with(port) => authority[..authority.len() - port.len()].to_string(),
            _ => authority,
        }
    }

    /// The balance label: "C" at 0, "L N" left, "R N" right.
    pub fn balance(value: i64) -> String {
        match value {
            0 => "C".to_string(),
            v if v < 0 => format!("L {}", -v),
            v => format!("R {v}"),
        }
    }

    /// "Last checked …" (`ago` in settings.js): under 60 s "just now", under an hour "N min
    /// ago", under a day "N h ago", else "N days ago".
    pub fn ago(timestamp: u64, now: u64) -> String {
        let seconds = now.saturating_sub(timestamp);
        if seconds < 60 {
            "just now".to_string()
        } else if seconds < 3600 {
            format!("{} min ago", seconds / 60)
        } else if seconds < 86400 {
            format!("{} h ago", seconds / 3600)
        } else {
            format!("{} days ago", seconds / 86400)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format::*;
    use super::*;

    #[test]
    fn formats_like_the_page() {
        assert_eq!(clock(8), "00:08");
        assert_eq!(clock(754), "12:34");
        assert_eq!(clock(3725), "1:02:05");
        assert_eq!(time(75.4), "1:15");
        assert_eq!(time(301.2), "5:01");
        assert_eq!(time(3725.0), "1:02:05");
        assert_eq!(khz(44100.0), "44.1");
        assert_eq!(khz(22050.0), "22.1");
        assert_eq!(khz(48000.0), "48");
        assert_eq!(host("https://kexp-mp3-128.streamguys1.com:8443/kexp128.mp3"), "kexp-mp3-128.streamguys1.com:8443");
        assert_eq!(host("https://example.com:443/live"), "example.com");
        assert_eq!(balance(-30), "L 30");
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 86400), "1 days ago");
    }

    #[test]
    fn parses_player_json() {
        let state: PlayerState = serde_json::from_str(r#"{"state":"playing","cartridges":[{"title":"A","url":"http://a","preload":"live"},null],"activeSlot":0,"activeId":null,"instant":true,"station":"A","title":"T","titleDim":false,"bitrate":128,"samplerate":44100,"channels":2,"volume":70,"balance":0,"colors":"classic","elapsed":754.3,"duration":null,"position":null,"seekable":false}"#).unwrap();
        assert_eq!(state.state, Playback::Playing);
        assert_eq!(state.cartridge(0).unwrap().preload, Some(Preload::Live));
        assert!(state.cartridge(1).is_none());
        assert_eq!(state.clock_seconds(), 754);
        let entry: Entry = serde_json::from_str(r#"{"id":4,"title":"Radio Nova","url":"http://n","duration":null,"missing":false}"#).unwrap();
        assert_eq!(entry.url.as_deref(), Some("http://n"));
    }
}
