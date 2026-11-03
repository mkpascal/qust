// Global shortcuts, customizable in the settings window. Stored as accelerator strings
// ("Ctrl+Alt+Insert") under shortcut-play / shortcut-stop. A shortcut another program already owns
// can't be registered: it is kept but reported as unavailable.

use crate::store::Store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

// (action, storage key, default). Pressing the shortcut emits "<action>-action" to the windows.
const ACTIONS: [(&str, &str, &str); 2] = [
    ("play", "shortcut-play", "Ctrl+Alt+Insert"),
    ("stop", "shortcut-stop", "Ctrl+Alt+KeyH"),
];

struct Binding {
    accelerator: String,
    shortcut: Option<Shortcut>,
    registered: bool,
}

#[derive(Default)]
pub struct Shortcuts(Mutex<HashMap<&'static str, Binding>>);

pub fn handle(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let bindings = app.state::<Shortcuts>();
    let action = bindings.0.lock().unwrap().iter().find(|(_, binding)| binding.shortcut.as_ref() == Some(shortcut)).map(|(action, _)| *action);
    if let Some(action) = action {
        let _ = app.emit(&format!("{action}-action"), ());
    }
}

pub fn register_saved(app: &AppHandle) {
    let store = app.state::<Store>();
    let mut bindings = HashMap::new();
    for (action, key, default) in ACTIONS {
        let accelerator = store.get(key).and_then(|value| value.as_str().map(str::to_string)).unwrap_or_else(|| default.to_string());
        let shortcut = accelerator.parse::<Shortcut>().ok();
        let registered = shortcut.is_some_and(|shortcut| app.global_shortcut().register(shortcut).is_ok());
        bindings.insert(action, Binding { accelerator, shortcut, registered });
    }
    *app.state::<Shortcuts>().0.lock().unwrap() = bindings;
}

fn describe(bindings: &HashMap<&'static str, Binding>) -> Value {
    let mut out = serde_json::Map::new();
    for (action, _, _) in ACTIONS {
        if let Some(binding) = bindings.get(action) {
            out.insert(action.to_string(), json!({ "accelerator": binding.accelerator, "registered": binding.registered }));
        }
    }
    Value::Object(out)
}

#[tauri::command]
pub fn get_shortcuts(bindings: State<Shortcuts>) -> Value {
    describe(&bindings.0.lock().unwrap())
}

#[tauri::command]
pub fn set_shortcut(app: AppHandle, bindings: State<Shortcuts>, store: State<Store>, action: String, accelerator: String) -> Result<Value, String> {
    let Some((action, key, default)) = ACTIONS.into_iter().find(|(name, _, _)| *name == action) else {
        return Err("Unknown action".into());
    };
    let accelerator = if accelerator.is_empty() { default.to_string() } else { accelerator };
    let shortcut = accelerator.parse::<Shortcut>().map_err(|_| "That isn't a valid shortcut".to_string())?;
    let mut bindings = bindings.0.lock().unwrap();
    if bindings.iter().any(|(other, binding)| *other != action && binding.shortcut == Some(shortcut)) {
        return Err("Already used for another action".into());
    }

    // Shortcuts are paused while one is being recorded, so nothing needs unregistering here.
    let registered = app.global_shortcut().register(shortcut).is_ok();
    let _ = app.global_shortcut().unregister(shortcut);
    if !registered {
        return Err("Another app is already using this shortcut".into());
    }
    store.set(key, Value::String(accelerator.clone()));
    bindings.insert(action, Binding { accelerator, shortcut: Some(shortcut), registered: false });
    let _ = app.emit("settings-changed", json!({ "key": "shortcuts", "value": describe(&bindings) }));
    Ok(describe(&bindings))
}

// Paused while the settings window records a shortcut: a registered shortcut never reaches the page.
#[tauri::command]
pub fn pause_shortcuts(app: AppHandle, paused: bool) -> Value {
    set_paused(&app, paused)
}

pub fn set_paused(app: &AppHandle, paused: bool) -> Value {
    let state = app.state::<Shortcuts>();
    let mut bindings = state.0.lock().unwrap();
    for binding in bindings.values_mut() {
        let Some(shortcut) = binding.shortcut else { continue };
        if paused {
            if binding.registered {
                let _ = app.global_shortcut().unregister(shortcut);
            }
        } else if !app.global_shortcut().is_registered(shortcut) {
            binding.registered = app.global_shortcut().register(shortcut).is_ok();
        }
    }
    let value = describe(&bindings);
    if !paused {
        let _ = app.emit("settings-changed", json!({ "key": "shortcuts", "value": value }));
    }
    value
}
