// The playlist drawer under the player, and turning dropped or picked files, folders and
// .m3u / .pls lists into playlist entries (the player adds them, player/commands.rs). Files are
// played where they are; nothing is copied.

use crate::store::Store;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

pub const PLAYER_WIDTH: f64 = 860.0;
pub const PLAYER_HEIGHT: f64 = 267.0;
// The drawer's height, added under the player when it's open. Matches --drawer-height in style.css.
pub const DRAWER_HEIGHT: f64 = 320.0;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "mp4", "ogg", "oga", "opus", "flac", "wav", "webm", "weba"];
const LIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls"];
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: u32 = 16;

// The player's height, with the drawer as it was left (`playlist-open` in the store).
pub fn player_height(app: &AppHandle) -> f64 {
    let open = app.state::<Store>().get("playlist-open") == Some(Value::Bool(true));
    PLAYER_HEIGHT + if open { DRAWER_HEIGHT } else { 0.0 }
}

// Opens or closes the drawer: the window grows or shrinks over ~200 ms. Opening near the bottom
// of the screen also moves the window up, so the drawer stays on screen.
#[tauri::command]
pub fn set_playlist_open(window: WebviewWindow, store: State<Store>, open: bool) {
    store.set("playlist-open", Value::Bool(open));
    animate(window, PLAYER_HEIGHT + if open { DRAWER_HEIGHT } else { 0.0 });
}

// Bumped by every animation, so a newer one (a quick second click) stops the one running.
static ANIMATION: AtomicU64 = AtomicU64::new(0);

fn animate(window: WebviewWindow, to: f64) {
    let id = ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
    thread::spawn(move || {
        let Ok(scale) = window.scale_factor() else { return };
        let Ok(inner) = window.inner_size() else { return };
        let inner = inner.to_logical::<f64>(scale);
        let from = inner.height;
        let mut moving = None;
        if to > from {
            if let (Ok(position), Ok(outer), Ok(Some(monitor))) = (window.outer_position(), window.outer_size(), window.current_monitor()) {
                let position = position.to_logical::<f64>(scale);
                let outer = outer.to_logical::<f64>(scale);
                let area = monitor.work_area();
                let top = area.position.to_logical::<f64>(scale).y;
                let bottom = top + area.size.to_logical::<f64>(scale).height;
                let overflow = position.y + outer.height + (to - from) - bottom;
                if overflow > 0.0 {
                    moving = Some((position.x, position.y, (position.y - overflow).max(top)));
                }
            }
        }
        const STEPS: u32 = 14;
        for step in 1..=STEPS {
            if ANIMATION.load(Ordering::SeqCst) != id {
                return;
            }
            let t = step as f64 / STEPS as f64;
            let eased = 1.0 - (1.0 - t).powi(3);
            if let Some((x, y0, y1)) = moving {
                let _ = window.set_position(LogicalPosition::new(x, y0 + (y1 - y0) * eased));
            }
            let _ = window.set_size(LogicalSize::new(inner.width, (from + (to - from) * eased).round()));
            thread::sleep(Duration::from_millis(14));
        }
    });
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

// "Add file": the system file picker, then the same as dropping the picked files. Blocking.
pub fn pick(window: &WebviewWindow) -> Vec<Value> {
    let all: Vec<&str> = AUDIO_EXTENSIONS.iter().chain(LIST_EXTENSIONS).copied().collect();
    let picked = window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("Add to playlist")
        .add_filter("Audio files and playlists", &all)
        .add_filter("All files", &["*"])
        .blocking_pick_files()
        .unwrap_or_default();
    let paths: Vec<PathBuf> = picked.into_iter().filter_map(|file| file.into_path().ok()).collect();
    expand(&paths)
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
pub fn decode(text: &str) -> Option<String> {
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
