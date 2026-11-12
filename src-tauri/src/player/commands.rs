// The player's Tauri side: the commands the pages call (argument names become camelCase in JS),
// and the Host that emits to the windows. The commands run on the async runtime, not the main
// thread, and return once the player has handled them and emitted what they changed (the player
// thread never waits on anything, so that is quick).

use super::{Host, Player};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub struct TauriHost(pub AppHandle);

impl Host for TauriHost {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.0.emit(event, payload);
    }
}

fn player(app: &AppHandle) -> Player {
    app.state::<Player>().inner().clone()
}

#[tauri::command(async)]
pub fn player_attach(app: AppHandle, frames: Channel<InvokeResponseBody>) -> Value {
    let sink = Arc::new(move |frame: &[u8]| {
        let _ = frames.send(InvokeResponseBody::Raw(frame.to_vec()));
    });
    player(&app).attach(sink)
}

#[tauri::command(async)]
pub fn player_play(app: AppHandle) {
    player(&app).call(|core| core.play());
}

#[tauri::command(async)]
pub fn player_pause(app: AppHandle) {
    player(&app).call(|core| core.pause());
}

#[tauri::command(async)]
pub fn player_stop(app: AppHandle) {
    player(&app).call(|core| core.stop());
}

#[tauri::command(async)]
pub fn player_seek(app: AppHandle, seconds: f64) {
    player(&app).call(move |core| core.seek(seconds));
}

#[tauri::command(async)]
pub fn player_set_volume(app: AppHandle, value: i64) {
    player(&app).call(move |core| core.set_volume(value));
}

#[tauri::command(async)]
pub fn player_set_balance(app: AppHandle, value: i64) {
    player(&app).call(move |core| core.set_balance(value));
}

#[tauri::command(async)]
pub fn player_set_instant(app: AppHandle, on: bool) {
    player(&app).call(move |core| core.set_instant(on));
}

#[tauri::command(async)]
pub fn player_cycle_colors(app: AppHandle) {
    player(&app).call(|core| core.cycle_colors());
}

#[tauri::command(async)]
pub fn player_status(app: AppHandle) -> Value {
    player(&app).status()
}

#[tauri::command(async)]
pub fn cartridge_play(app: AppHandle, slot: usize) {
    player(&app).call(move |core| core.select_slot(slot));
}

#[tauri::command(async)]
pub fn cartridge_save(app: AppHandle, slot: usize, title: String, url: String) -> Result<(), String> {
    player(&app).call(move |core| core.save_cartridge(slot, title, url)).unwrap_or(Ok(()))
}

#[tauri::command(async)]
pub fn cartridge_eject(app: AppHandle, slot: usize) {
    player(&app).call(move |core| core.eject(slot));
}

#[tauri::command(async)]
pub fn cartridge_move(app: AppHandle, from: usize, to: usize) {
    player(&app).call(move |core| core.move_cartridge(from, to));
}

#[tauri::command(async)]
pub fn playlist_play(app: AppHandle, id: u64) {
    player(&app).call(move |core| core.select_entry(id));
}

#[tauri::command(async)]
pub fn playlist_add_url(app: AppHandle, url: String, title: String) -> Result<Vec<u64>, String> {
    player(&app).call(move |core| core.add_url(url, title)).unwrap_or(Ok(Vec::new()))
}

// "Add file": the system file picker (blocking, so on a thread of its own), then the files,
// folders and lists picked, expanded as when dropped.
#[tauri::command]
pub async fn playlist_add_files(window: WebviewWindow, index: Option<usize>) -> Vec<u64> {
    let app = window.app_handle().clone();
    let items = tauri::async_runtime::spawn_blocking(move || crate::playlist::pick(&window)).await.unwrap_or_default();
    add(&app, items, index)
}

#[tauri::command]
pub async fn playlist_drop(app: AppHandle, paths: Vec<String>, index: usize) -> Vec<u64> {
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let items = tauri::async_runtime::spawn_blocking(move || crate::playlist::expand(&paths)).await.unwrap_or_default();
    add(&app, items, Some(index))
}

fn add(app: &AppHandle, items: Vec<Value>, index: Option<usize>) -> Vec<u64> {
    if items.is_empty() {
        return Vec::new();
    }
    player(app).call(move |core| core.add_entries(&items, index)).unwrap_or_default()
}

#[tauri::command(async)]
pub fn playlist_remove(app: AppHandle, ids: Vec<u64>) {
    player(&app).call(move |core| core.remove_entries(&ids));
}

#[tauri::command(async)]
pub fn playlist_move(app: AppHandle, ids: Vec<u64>, index: usize) {
    player(&app).call(move |core| core.move_entries(&ids, index));
}

#[tauri::command(async)]
pub fn playlist_edit(app: AppHandle, id: u64, title: String, url: Option<String>) -> Result<(), String> {
    player(&app).call(move |core| core.edit_entry(id, title, url)).unwrap_or(Ok(()))
}

// The details of a stream or file (the Stream information window), by its bare URL or path.
#[tauri::command]
pub fn stream_info(url: String) -> Value {
    let lower = url.to_ascii_lowercase();
    let key = if lower.starts_with("http://") || lower.starts_with("https://") { url } else { format!("file:{url}") };
    qust_audio::info(&key)
}
