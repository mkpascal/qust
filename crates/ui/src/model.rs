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

    /// The player's `host(url)` (script.js): `new URL(url).host`, or the text itself when it
    /// isn't a URL (the cartridge host line and the playlist's detail).
    pub fn host(url: &str) -> String {
        url_host(url).unwrap_or_else(|| url.to_string())
    }

    /// `new URL(url).host`, None where the constructor throws (info.js shows "—" then): the
    /// host, lowercased and with internationalized labels in their punycode form for the special
    /// schemes (`http://Bücher.example` → `xn--bcher-kva.example`), and the port unless it is
    /// the scheme's default. A Windows path parses as a URL with a one-letter scheme and no host
    /// (`""`), as in the browser.
    pub fn url_host(url: &str) -> Option<String> {
        let url = url.trim_matches(|c: char| c <= ' ');
        let colon = url.find(':')?;
        let scheme = &url[..colon];
        let mut chars = scheme.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return None;
        }
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "http" | "ws" => Some(80),
            "https" | "wss" => Some(443),
            "ftp" => Some(21),
            "file" => Some(0),
            _ => None,
        };
        let special = default_port.is_some();
        let rest = &url[colon + 1..];
        let authority_start = if special {
            // Special schemes ignore any number of slashes and backslashes.
            rest.trim_start_matches(['/', '\\'])
        } else if let Some(after) = rest.strip_prefix("//") {
            after
        } else {
            return Some(String::new());
        };
        let end = authority_start.find(|c: char| matches!(c, '/' | '?' | '#') || (special && c == '\\')).unwrap_or(authority_start.len());
        let authority = &authority_start[..end];
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        // host[:port], an IPv6 address in brackets.
        let (host, port) = if authority.starts_with('[') {
            let close = authority.find(']')?;
            (&authority[..=close], authority[close + 1..].strip_prefix(':'))
        } else {
            match authority.rfind(':') {
                Some(i) => (&authority[..i], Some(&authority[i + 1..])),
                None => (authority, None),
            }
        };
        let port = match port.filter(|p| !p.is_empty()) {
            Some(p) if p.chars().all(|c| c.is_ascii_digit()) => Some(p.parse::<u32>().ok().filter(|p| *p <= 65535)?),
            Some(_) => return None,
            None => None,
        };
        if scheme == "file" {
            return Some(String::new());
        }
        let host = if special {
            if host.is_empty() {
                return None;
            }
            domain_to_ascii(host)?
        } else {
            host.to_string()
        };
        Some(match port {
            Some(p) if Some(p) != default_port => format!("{host}:{p}"),
            _ => host,
        })
    }

    /// The URL standard's domain to ASCII for the hosts streams use: lowercased, each label with
    /// non-ASCII characters in punycode (`xn--`). (No full UTS #46 mapping: width and
    /// compatibility forms stay as typed.)
    fn domain_to_ascii(host: &str) -> Option<String> {
        if host.starts_with('[') {
            return Some(host.to_ascii_lowercase());
        }
        let lower = host.to_lowercase();
        let labels: Option<Vec<String>> = lower
            .split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}'])
            .map(|label| if label.is_ascii() { Some(label.to_string()) } else { punycode(label).map(|p| format!("xn--{p}")) })
            .collect();
        let host = labels?.join(".");
        // Forbidden host code points.
        if host.chars().any(|c| matches!(c, ' ' | '#' | '%' | '/' | ':' | '<' | '>' | '?' | '@' | '[' | '\\' | ']' | '^' | '|')) {
            return None;
        }
        Some(host)
    }

    /// RFC 3492 punycode of one label.
    fn punycode(input: &str) -> Option<String> {
        const BASE: u32 = 36;
        const T_MIN: u32 = 1;
        const T_MAX: u32 = 26;
        const SKEW: u32 = 38;
        const DAMP: u32 = 700;
        fn adapt(mut delta: u32, points: u32, first: bool) -> u32 {
            delta /= if first { DAMP } else { 2 };
            delta += delta / points;
            let mut k = 0;
            while delta > ((BASE - T_MIN) * T_MAX) / 2 {
                delta /= BASE - T_MIN;
                k += BASE;
            }
            k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
        }
        fn digit(d: u32) -> char {
            (if d < 26 { b'a' + d as u8 } else { b'0' + (d - 26) as u8 }) as char
        }
        let code_points: Vec<u32> = input.chars().map(|c| c as u32).collect();
        let mut output: String = input.chars().filter(|c| c.is_ascii()).collect();
        let basic = output.len() as u32;
        let mut handled = basic;
        if basic > 0 {
            output.push('-');
        }
        let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
        while (handled as usize) < code_points.len() {
            let m = code_points.iter().copied().filter(|&c| c >= n).min()?;
            delta = delta.checked_add((m - n).checked_mul(handled + 1)?)?;
            n = m;
            for &c in &code_points {
                if c < n {
                    delta = delta.checked_add(1)?;
                }
                if c == n {
                    let mut q = delta;
                    let mut k = BASE;
                    loop {
                        let t = if k <= bias { T_MIN } else if k >= bias + T_MAX { T_MAX } else { k - bias };
                        if q < t {
                            break;
                        }
                        output.push(digit(t + (q - t) % (BASE - t)));
                        q = (q - t) / (BASE - t);
                        k += BASE;
                    }
                    output.push(digit(q));
                    bias = adapt(delta, handled + 1, handled == basic);
                    delta = 0;
                    handled += 1;
                }
            }
            delta += 1;
            n += 1;
        }
        Some(output)
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
        // As `new URL(url).host` (checked against Node): punycode, default ports, non-URLs.
        assert_eq!(host("http://Bücher.example:80/x"), "xn--bcher-kva.example");
        assert_eq!(host("https://www.例え.jp:8443/x"), "www.xn--r8jz45g.jp:8443");
        assert_eq!(host("https://straße.de/"), "xn--strae-oqa.de");
        assert_eq!(host("http://ÉCOLE.fr"), "xn--cole-9oa.fr");
        assert_eq!(host("http://user:pw@HOST.com:8000/"), "host.com:8000");
        assert_eq!(host("rtsp://x.com:554/"), "x.com:554");
        assert_eq!(host("http://[::1]:8080/"), "[::1]:8080");
        assert_eq!(host("ftp://x.com:21/"), "x.com");
        assert_eq!(host("http://a.com:/x"), "a.com");
        assert_eq!(host("HTTP://EXAMPLE.COM:0080/"), "example.com");
        assert_eq!(host("http://xn--bcher-kva.example/"), "xn--bcher-kva.example");
        assert_eq!(host("foo"), "foo");
        assert_eq!(host("/home/a.mp3"), "/home/a.mp3");
        assert_eq!(host("C:\\Music\\a.mp3"), "");
        assert_eq!(url_host("foo"), None);
        assert_eq!(url_host("http://"), None);
        assert_eq!(url_host("C:\\Music\\a.mp3").as_deref(), Some(""));
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
