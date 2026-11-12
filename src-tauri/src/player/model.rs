// What the player holds: cartridges, playlist entries, and the names given to them, with the same
// rules and store formats as the old page (script.js).

use crate::playlist::AUDIO_EXTENSIONS;
use serde_json::{json, Value};

pub const SLOTS: usize = 6;
pub const COLOR_CYCLE: [&str; 4] = ["classic", "white", "accent", "off"];
pub const URL_ERROR: &str = "Enter an address starting with http:// or https://";

#[derive(Clone, Debug, PartialEq)]
pub struct Cartridge {
    pub title: String,
    pub url: String,
}

// A playlist row: a local file (`path`) or a stream or online file (`url`). `id` lives as long as
// the app runs; `probed` means its duration has been looked for.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub title: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub duration: Option<f64>,
    pub missing: bool,
    pub probed: bool,
}

impl Entry {
    // The engine key, as keyOf() in the old page: the URL, or "file:" and the path.
    pub fn key(&self) -> String {
        match &self.path {
            Some(path) => format!("file:{path}"),
            None => self.url.clone().unwrap_or_default(),
        }
    }

    // For the page (`player-playlist`, `player-entry`).
    pub fn json(&self) -> Value {
        let mut value = json!({ "id": self.id, "title": self.title, "duration": self.duration, "missing": self.missing });
        if let Some(path) = &self.path {
            value["path"] = json!(path);
        }
        if let Some(url) = &self.url {
            value["url"] = json!(url);
        }
        value
    }

    // For the store (`playlist`).
    pub fn saved(&self) -> Value {
        match &self.path {
            Some(path) => json!({ "title": self.title, "path": path, "duration": self.duration }),
            None => json!({ "title": self.title, "url": self.url, "duration": self.duration }),
        }
    }

    // The name an entry gets when left without one.
    pub fn default_title(&self) -> String {
        match (&self.path, &self.url) {
            (Some(path), _) => file_title(path),
            (None, Some(url)) => url_title(url),
            _ => String::new(),
        }
    }
}

// { path | url, title?, duration? } (saved, dropped or from a list) as an entry, as toEntry().
pub fn to_entry(item: &Value, id: u64) -> Option<Entry> {
    let duration = item["duration"].as_f64().filter(|duration| *duration > 0.0);
    let title = text(&item["title"]);
    if let Some(path) = text(&item["path"]) {
        let title = title.unwrap_or_else(|| file_title(&path));
        return Some(Entry { id, title, path: Some(path), url: None, duration, missing: false, probed: false });
    }
    let url = text(&item["url"])?;
    let title = title.unwrap_or_else(|| url_title(&url));
    Some(Entry { id, title, path: None, url: Some(url), duration, missing: false, probed: false })
}

// A non-empty string (what JS took as truthy), else None.
pub fn text(value: &Value) -> Option<String> {
    value.as_str().filter(|text| !text.is_empty()).map(str::to_string)
}

pub fn valid_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
}

// `new URL(url).host`: the host, with the port unless it's the scheme's default.
pub fn host(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else { return url.to_string() };
    let host = parsed.host_str().unwrap_or("").to_string();
    match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

// The file's name without its extension.
pub fn file_title(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    strip_extension(name).to_string()
}

fn strip_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot + 1 < name.len() => &name[..dot],
        _ => name,
    }
}

// An online file's name, or a stream's host.
pub fn url_title(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else { return url.to_string() };
    let last = parsed.path().rsplit('/').next().unwrap_or("");
    let Some(name) = crate::playlist::decode(last) else { return url.to_string() };
    let audio = name.rsplit_once('.').is_some_and(|(_, extension)| AUDIO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()));
    if audio {
        strip_extension(&name).to_string()
    } else {
        host(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(host("http://Example.com:8000/live"), "example.com:8000");
        assert_eq!(host("https://example.com:443/live"), "example.com");
        assert_eq!(file_title(r"C:\Music\01 Song.mp3"), "01 Song");
        assert_eq!(file_title("/a/b.c/track"), "track");
        assert_eq!(url_title("http://host/files/My%20Song.MP3?x=1"), "My Song");
        assert_eq!(url_title("http://host:8000/stream"), "host:8000");
        assert!(valid_url("https://a.b/x") && !valid_url("ftp://a.b") && !valid_url("nope"));
        let entry = to_entry(&json!({ "url": "http://h/a.ogg", "title": "", "duration": 0 }), 1).unwrap();
        assert_eq!((entry.title.as_str(), entry.duration), ("a", None));
        assert!(to_entry(&json!({ "title": "x" }), 2).is_none());
    }
}
