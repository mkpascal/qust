// The player window's size with and without the playlist drawer, and turning dropped or picked
// files, folders and .m3u / .pls lists into playlist entries (the player adds them,
// `Player::add_entries`). Files are played where they are; nothing is copied.

use crate::store::Store;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const PLAYER_WIDTH: f64 = 860.0;
pub const PLAYER_HEIGHT: f64 = 267.0;
// The drawer's height, added under the player when it's open. Matches --drawer-height in style.css.
pub const DRAWER_HEIGHT: f64 = 320.0;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "mp4", "ogg", "oga", "opus", "flac", "wav", "webm", "weba"];
pub const LIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls"];
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: u32 = 16;

// The player's height, with the drawer as it was left (`playlist-open` in the store).
pub fn player_height(store: &Store) -> f64 {
    let open = store.get("playlist-open") == Some(Value::Bool(true));
    PLAYER_HEIGHT + if open { DRAWER_HEIGHT } else { 0.0 }
}

// Dropped paths (files, folders, .m3u / .pls lists) as playlist entries: { path, title } for a
// file, { url, title } for a stream from a list. Folders are added in name order, with their
// subfolders; lists found inside folders are skipped, so their files aren't added twice.
// Blocking: run it off the main thread.
pub fn expand(paths: &[PathBuf]) -> Vec<Value> {
    let mut entries = Vec::new();
    for path in paths {
        add(path, &mut entries, 0);
    }
    entries
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn add(path: &Path, entries: &mut Vec<Value>, depth: u32) {
    if entries.len() >= MAX_ENTRIES || depth > MAX_DEPTH {
        return;
    }
    if path.is_dir() {
        let Ok(dir) = fs::read_dir(path) else { return };
        let mut children: Vec<PathBuf> = dir.flatten().map(|entry| entry.path()).collect();
        children.sort_by_cached_key(|child| natural_key(&child.file_name().unwrap_or_default().to_string_lossy()));
        for child in children {
            add(&child, entries, depth + 1);
        }
        return;
    }
    let extension = extension(path);
    if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        entries.push(file_entry(path, None, None));
    } else if depth == 0 && LIST_EXTENSIONS.contains(&extension.as_str()) {
        read_list(path, entries);
    }
}

// Sorts "2 Song" before "10 Song", ignoring case.
fn natural_key(name: &str) -> Vec<(u8, u64, String)> {
    let mut key = Vec::new();
    let mut rest = name.to_lowercase();
    while !rest.is_empty() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            key.push((0, rest[..digits].parse().unwrap_or(u64::MAX), String::new()));
            rest = rest[digits..].to_string();
        } else {
            let text: String = rest.chars().take_while(|c| !c.is_ascii_digit()).collect();
            rest = rest[text.len()..].to_string();
            key.push((1, 0, text));
        }
    }
    key
}

fn file_entry(path: &Path, title: Option<String>, duration: Option<f64>) -> Value {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    json!({ "path": path.to_string_lossy(), "title": title.unwrap_or(stem), "duration": duration })
}

// An .m3u / .m3u8 (with #EXTINF titles) or .pls list: streams and files, relative paths taken
// from the list's folder.
fn read_list(path: &Path, entries: &mut Vec<Value>) {
    let Ok(bytes) = fs::read(path) else { return };
    let text = String::from_utf8_lossy(&bytes);
    let dir = path.parent().unwrap_or(Path::new(""));
    let pls = extension(path) == "pls";
    let mut pending: Option<(Option<String>, Option<f64>)> = None;
    let mut pls_titles = std::collections::HashMap::new();
    let mut pls_files = Vec::new();
    for line in text.lines().map(|line| line.trim_start_matches('\u{feff}').trim()) {
        if line.is_empty() {
            continue;
        }
        if pls {
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim().to_ascii_lowercase();
            if let Some(n) = key.strip_prefix("file") {
                pls_files.push((n.to_string(), value.trim().to_string()));
            } else if let Some(n) = key.strip_prefix("title") {
                pls_titles.insert(n.to_string(), value.trim().to_string());
            }
        } else if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (seconds, title) = info.split_once(',').unwrap_or((info, ""));
            let seconds = seconds.split_whitespace().next().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0);
            pending = Some(((!title.trim().is_empty()).then(|| title.trim().to_string()), seconds));
        } else if !line.starts_with('#') {
            let (title, duration) = pending.take().unwrap_or((None, None));
            list_entry(dir, line, title, duration, entries);
        }
    }
    for (n, location) in pls_files {
        list_entry(dir, &location, pls_titles.remove(&n), None, entries);
    }
}

fn list_entry(dir: &Path, location: &str, title: Option<String>, duration: Option<f64>, entries: &mut Vec<Value>) {
    if entries.len() >= MAX_ENTRIES {
        return;
    }
    let lower = location.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        entries.push(json!({ "url": location, "title": title, "duration": duration }));
        return;
    }
    // file:///C:/Music/a.mp3 on Windows, file:///Users/me/a.mp3 on macOS
    let location = match lower.strip_prefix("file://") {
        Some(_) => {
            let decoded = decode(&location[7..]).unwrap_or_default();
            if cfg!(windows) { decoded.trim_start_matches('/').to_string() } else { decoded }
        }
        None => location.to_string(),
    };
    let path = dir.join(location);
    if path.is_file() && AUDIO_EXTENSIONS.contains(&extension(&path).as_str()) {
        entries.push(file_entry(&path, title, duration));
    }
}

// Percent-decoding, as decodeURIComponent (None for a broken escape or bytes that aren't UTF-8).
pub(crate) fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
