use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

// Keys the Settings window writes that the player applies at once.
const WATCHED: [&str; 3] = ["visualizer-colors", "reconnect-minutes", "autoplay"];

type Watcher = Box<dyn Fn(&str, Option<&Value>) + Send + Sync>;

// Shared by the commands and the player thread: clones are the same store.
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    data: Mutex<Map<String, Value>>,
    watcher: Mutex<Option<Watcher>>,
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
        Ok(Store::with(path, data))
    }

    // A store on any file (the tests use a temporary one).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open(path: PathBuf) -> Self {
        let data = read(&path).unwrap_or_default();
        Store::with(path, data)
    }

    fn with(path: PathBuf, data: Map<String, Value>) -> Self {
        Store { inner: Arc::new(Inner { path, data: Mutex::new(data), watcher: Mutex::new(None) }) }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.inner.data.lock().unwrap().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: Value) {
        let mut data = self.inner.data.lock().unwrap();
        data.insert(key.to_string(), value);
        save(&self.inner.path, &data);
    }

    pub fn remove(&self, key: &str) {
        let mut data = self.inner.data.lock().unwrap();
        if data.remove(key).is_some() {
            save(&self.inner.path, &data);
        }
    }

    // Called when a page writes one of the WATCHED keys (the player's own writes don't call it).
    pub fn watch(&self, watcher: impl Fn(&str, Option<&Value>) + Send + Sync + 'static) {
        *self.inner.watcher.lock().unwrap() = Some(Box::new(watcher));
    }

    // A write from a page (set_data / delete_data): the player hears about the keys it follows.
    pub fn write(&self, key: &str, value: Option<Value>) {
        match &value {
            Some(value) => self.set(key, value.clone()),
            None => self.remove(key),
        }
        if WATCHED.contains(&key) {
            if let Some(watcher) = self.inner.watcher.lock().unwrap().as_ref() {
                watcher(key, value.as_ref());
            }
        }
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
    store.write(&key, Some(value));
}

#[tauri::command]
pub fn delete_data(store: State<Store>, key: String) {
    store.write(&key, None);
}
