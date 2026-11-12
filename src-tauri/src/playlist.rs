// The playlist drawer under the player (the window's height), and the Add file picker. The sizes
// and turning files, folders and lists into entries are qust_player::playlist's.

use crate::store::Store;
use qust_player::playlist::{expand, AUDIO_EXTENSIONS, DRAWER_HEIGHT, LIST_EXTENSIONS, PLAYER_HEIGHT};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;
use tauri::{LogicalPosition, LogicalSize, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

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
