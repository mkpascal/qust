// The settings store lives in the qust-player crate; these are the commands the pages call.

pub use qust_player::Store;
use serde_json::Value;
use tauri::State;

#[tauri::command]
pub fn get_data(store: State<Store>, key: String) -> Option<Value> {
    store.get(&key)
}

#[tauri::command]
pub fn set_data(store: State<Store>, key: String, value: Value) {
    store.write(&key, Some(value));
}

#[tauri::command]
pub fn delete_data(store: State<Store>, key: String) {
    store.write(&key, None);
}
