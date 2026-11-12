use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// Keys the Settings window writes that the player applies at once.
const WATCHED: [&str; 3] = ["visualizer-colors", "reconnect-minutes", "autoplay"];

// The bundle identifier (`identifier` in tauri.conf.json), which names the app's config dir.
const IDENTIFIER: &str = "com.pascalmuangkhot.qust";

type Watcher = Box<dyn Fn(&str, Option<&Value>) + Send + Sync>;

// The settings file, where Tauri's app_config_dir() put it: the user's config dir and the bundle
// identifier (%APPDATA%\com.pascalmuangkhot.qust\settings.json on Windows,
// ~/Library/Application Support/com.pascalmuangkhot.qust/settings.json on macOS).
pub fn settings_path() -> Option<PathBuf> {
    app_config_dir().map(|dir| dir.join("settings.json"))
}

// The Electron version's electron-store file.
pub fn electron_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("qust").join("config.json"))
}

fn app_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(IDENTIFIER))
}

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
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let dir = app_config_dir().ok_or("unknown path")?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");
        let data = read(&path).or_else(|| read(&electron_config_path()?)).unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;

    // The same files the Tauri versions resolved: their app_config_dir() was dirs::config_dir()
    // joined with the bundle identifier, and their config_dir() was dirs::config_dir(). Changing
    // the identifier would lose every installed copy's settings.
    #[test]
    fn identifier_is_the_tauri_versions() {
        assert_eq!(IDENTIFIER, "com.pascalmuangkhot.qust");
    }

    #[cfg(windows)]
    #[test]
    fn paths_on_windows() {
        let roaming = PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA"));
        assert_eq!(settings_path(), Some(roaming.join("com.pascalmuangkhot.qust").join("settings.json")));
        assert_eq!(electron_config_path(), Some(roaming.join("qust").join("config.json")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn paths_on_macos() {
        let support = PathBuf::from(std::env::var_os("HOME").expect("HOME")).join("Library/Application Support");
        assert_eq!(settings_path(), Some(support.join("com.pascalmuangkhot.qust/settings.json")));
        assert_eq!(electron_config_path(), Some(support.join("qust/config.json")));
    }
}
