// What is remembered of the plugins: the list in the store (`audio-plugins`, in order: path, name,
// vendor, on or off), and each plugin's own state (its settings, as the plugin saves them) in a
// file of its own next to the settings file (`audio-plugins/<key>.state`), so the settings file
// stays small and quick to rewrite.

use qust_player::Store;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The store key of the list.
pub const KEY: &str = "audio-plugins";
/// The plugin being started, while it starts: found at launch, it is what made Qust close.
pub const LOADING: &str = "audio-plugins-loading";

const MAGIC: &[u8; 4] = b"QVST";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    /// Names the state file. Stable for as long as the plugin is in the list.
    pub key: String,
    /// The `.vst3` the user picked: a bundle folder, or a single file.
    pub path: PathBuf,
    pub name: String,
    pub vendor: String,
    pub enabled: bool,
    /// Qust closed while starting it: it stays off until turned on again.
    pub crashed: bool,
}

impl Saved {
    pub fn new(path: PathBuf, name: String, vendor: String) -> Saved {
        Saved { key: new_key(), path, name, vendor, enabled: true, crashed: false }
    }

    fn to_json(&self) -> Value {
        let mut value = json!({
            "key": self.key,
            "path": self.path.to_string_lossy(),
            "name": self.name,
            "vendor": self.vendor,
            "enabled": self.enabled,
        });
        if self.crashed {
            value["crashed"] = json!(true);
        }
        value
    }

    fn from_json(value: &Value) -> Option<Saved> {
        let path = value.get("path")?.as_str().filter(|path| !path.is_empty())?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let key = Some(text("key")).filter(|key| valid_key(key)).unwrap_or_else(new_key);
        Some(Saved {
            key,
            path: PathBuf::from(path),
            name: Some(text("name")).filter(|name| !name.is_empty()).unwrap_or_else(|| crate::bundle::display_name(Path::new(path))),
            vendor: text("vendor"),
            enabled: value.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            crashed: value.get("crashed").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// The list, in processing order.
pub fn load(store: &Store) -> Vec<Saved> {
    match store.get(KEY) {
        Some(Value::Array(items)) => items.iter().filter_map(Saved::from_json).collect(),
        _ => Vec::new(),
    }
}

pub fn save(store: &Store, plugins: &[Saved]) {
    store.set(KEY, Value::Array(plugins.iter().map(Saved::to_json).collect()));
}

// Keys are file names: only what `new_key` makes.
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 32 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

fn new_key() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos() as u64);
    let mixed = nanos ^ (std::process::id() as u64).rotate_left(40) ^ COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    format!("{mixed:016x}")
}

/// Where the states go: `audio-plugins` next to the settings file.
pub fn state_dir(store: &Store) -> PathBuf {
    store.path().parent().map_or_else(|| PathBuf::from("audio-plugins"), |dir| dir.join("audio-plugins"))
}

fn state_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.state"))
}

/// The plugin's saved state: (its processor's, its controller's).
pub fn read_state(dir: &Path, key: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    if !valid_key(key) {
        return None;
    }
    decode(&std::fs::read(state_path(dir, key)).ok()?)
}

pub fn write_state(dir: &Path, key: &str, component: &[u8], controller: &[u8]) {
    if !valid_key(key) || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    // Written aside, then moved over: a crash mid-write never leaves half a state.
    let path = state_path(dir, key);
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, encode(component, controller)).is_ok() && std::fs::rename(&temporary, &path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

pub fn remove_state(dir: &Path, key: &str) {
    if valid_key(key) {
        let _ = std::fs::remove_file(state_path(dir, key));
    }
}

fn encode(component: &[u8], controller: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(24 + component.len() + controller.len());
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&VERSION.to_le_bytes());
    for part in [component, controller] {
        data.extend_from_slice(&(part.len() as u64).to_le_bytes());
        data.extend_from_slice(part);
    }
    data
}

fn decode(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if data.get(..4)? != MAGIC || u32::from_le_bytes(data.get(4..8)?.try_into().ok()?) != VERSION {
        return None;
    }
    let mut at = 8;
    let mut part = || -> Option<Vec<u8>> {
        let len = usize::try_from(u64::from_le_bytes(data.get(at..at + 8)?.try_into().ok()?)).ok()?;
        let bytes = data.get(at + 8..at.checked_add(8 + len)?)?.to_vec();
        at += 8 + len;
        Some(bytes)
    };
    let component = part()?;
    let controller = part()?;
    Some((component, controller))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("qust-vst-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(dir.join("settings.json"))
    }

    #[test]
    fn the_list_round_trips_in_order() {
        let store = store("list");
        let mut first = Saved::new(PathBuf::from(r"C:\VST3\EQ.vst3"), "EQ".into(), "Acme".into());
        first.enabled = false;
        let mut second = Saved::new(PathBuf::from(r"C:\VST3\Limiter.vst3"), "Limiter".into(), String::new());
        second.crashed = true;
        save(&store, &[first.clone(), second.clone()]);
        assert_eq!(load(&store), [first, second]);
        assert_eq!(load(&Store::open(store.path().to_path_buf())).len(), 2);
    }

    #[test]
    fn a_hand_edited_list_is_read_with_defaults() {
        let store = store("defaults");
        store.set(KEY, json!([{ "path": r"C:\VST3\Gate.vst3" }, { "name": "no path" }, { "path": "x.vst3", "key": "../evil" }]));
        let plugins = load(&store);
        assert_eq!(plugins.len(), 2);
        assert_eq!((plugins[0].name.as_str(), plugins[0].enabled, plugins[0].crashed), ("Gate", true, false));
        assert!(valid_key(&plugins[1].key), "{}", plugins[1].key);
        assert_ne!(plugins[0].key, plugins[1].key);
    }

    #[test]
    fn states_round_trip_and_reject_damage() {
        let store = store("state");
        let dir = state_dir(&store);
        write_state(&dir, "00ff", b"component", b"");
        assert_eq!(read_state(&dir, "00ff"), Some((b"component".to_vec(), Vec::new())));
        let data = encode(b"abc", b"de");
        assert_eq!(decode(&data[..data.len() - 1]), None);
        assert_eq!(decode(b"QVST\x02\0\0\0"), None);
        assert_eq!(read_state(&dir, "../settings"), None);
        remove_state(&dir, "00ff");
        assert_eq!(read_state(&dir, "00ff"), None);
    }
}
