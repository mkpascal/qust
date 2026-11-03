use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

pub struct Store {
    path: PathBuf,
    data: Mutex<Map<String, Value>>,
}

impl Store {
    // Starts from the Electron version's electron-store file when there is no settings file yet,
    // so saved cartridges survive the switch.
    pub fn load(app: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let dir = app.path().app_config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        let data = read(&path)
            .or_else(|| read(&app.path().config_dir().ok()?.join("qust").join("config.json")))
            .unwrap_or_default();
        save(&path, &data);
        Ok(Store { path, data: Mutex::new(data) })
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.data.lock().unwrap().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: Value) {
        let mut data = self.data.lock().unwrap();
        data.insert(key.to_string(), value);
        save(&self.path, &data);
    }
}

fn read(path: &Path) -> Option<Map<String, Value>> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn save(path: &Path, data: &Map<String, Value>) {
    if let Ok(json) = serde_json::to_string_pretty(data) {
        let _ = fs::write(path, json);
    }
}

#[tauri::command]
pub fn get_data(store: State<Store>, key: String) -> Option<Value> {
    store.get(&key)
}

#[tauri::command]
pub fn set_data(store: State<Store>, key: String, value: Value) {
    store.set(&key, value);
}

#[tauri::command]
pub fn delete_data(store: State<Store>, key: String) {
    let mut data = store.data.lock().unwrap();
    data.remove(&key);
    save(&store.path, &data);
}
