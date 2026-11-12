// Global shortcuts, customizable in Settings: what shortcuts.rs did in the Tauri app. Stored as
// accelerators ("Ctrl+Alt+Insert", see `accelerator`) under shortcut-play / shortcut-stop /
// shortcut-toggle; an empty string means no shortcut. A shortcut another program owns can't be
// registered: it is kept but reported as not registered (Settings shows it struck through).

use crate::accelerator::{self, Accelerator};
use crate::hotkeys::HotkeyBackend;
use qust_player::Store;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Play,
    Stop,
    // Play / Stop / Start: plays when stopped or paused and stops otherwise.
    Toggle,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Play, Action::Stop, Action::Toggle];

    // The player's name for it (`Player::action`).
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Play => "play",
            Action::Stop => "stop",
            Action::Toggle => "toggle",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.as_str() == name)
    }

    pub fn store_key(self) -> &'static str {
        match self {
            Action::Play => "shortcut-play",
            Action::Stop => "shortcut-stop",
            Action::Toggle => "shortcut-toggle",
        }
    }

    // Play / Stop / Start starts unassigned: one key that follows Qust's state, for a single
    // Stream Deck button that can't get out of sync.
    pub fn default_accelerator(self) -> &'static str {
        match self {
            Action::Play => "Ctrl+Alt+Insert",
            Action::Stop => "Ctrl+Alt+KeyH",
            Action::Toggle => "",
        }
    }

    // The hotkey id it registers under (WM_HOTKEY's wParam).
    pub fn hotkey_id(self) -> i32 {
        match self {
            Action::Play => 1,
            Action::Stop => 2,
            Action::Toggle => 3,
        }
    }

    pub fn from_hotkey_id(id: i32) -> Option<Action> {
        Action::ALL.into_iter().find(|action| action.hotkey_id() == id)
    }
}

// Used only to test whether a new shortcut is free; never left registered.
const PROBE_ID: i32 = 0x5100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub action: Action,
    // As stored ("" when unassigned).
    pub accelerator: String,
    // What Settings shows: false for an unassigned shortcut and for one another app holds.
    pub registered: bool,
}

impl Binding {
    pub fn assigned(&self) -> bool {
        !self.accelerator.is_empty()
    }

    // The key chips ("Ctrl", "Alt", "H").
    pub fn keys(&self) -> Vec<String> {
        if self.assigned() {
            accelerator::keys(&self.accelerator)
        } else {
            Vec::new()
        }
    }

    // For tooltips ("Play (Ctrl+Alt+Insert)") and menus: only a registered shortcut is shown.
    pub fn label(&self) -> Option<String> {
        (self.assigned() && self.registered).then(|| accelerator::label(&self.accelerator))
    }

    // The accelerator text for a native menu item, as the old menus showed it (see
    // `accelerator::menu_text`): None when not registered or when muda couldn't have parsed it.
    pub fn menu_text(&self) -> Option<String> {
        self.label().and_then(|_| accelerator::menu_text(&self.accelerator))
    }
}

struct Slot {
    binding: Binding,
    parsed: Option<Accelerator>,
    // Registered with the OS right now (false while paused).
    active: bool,
}

type Listener = Box<dyn Fn(&[Binding]) + Send + Sync>;

pub struct Shortcuts {
    backend: Arc<dyn HotkeyBackend>,
    store: Store,
    slots: Mutex<Vec<Slot>>,
    listener: Mutex<Option<Listener>>,
}

impl Shortcuts {
    pub fn new(backend: Arc<dyn HotkeyBackend>, store: Store) -> Self {
        Shortcuts { backend, store, slots: Mutex::new(Vec::new()), listener: Mutex::new(None) }
    }

    // Hears the changes the Tauri app broadcast as `settings-changed` { key: "shortcuts" }: after
    // set_shortcut, and after resuming.
    pub fn on_change(&self, listener: impl Fn(&[Binding]) + Send + Sync + 'static) {
        *lock(&self.listener) = Some(Box::new(listener));
    }

    // At startup: registers the saved shortcuts (or the defaults).
    pub fn register_saved(&self) {
        let mut slots = lock(&self.slots);
        for slot in slots.iter_mut() {
            if slot.active {
                self.backend.unregister(slot.binding.action.hotkey_id());
            }
        }
        *slots = Action::ALL
            .into_iter()
            .map(|action| {
                let accelerator = self.store.get(action.store_key()).and_then(|value| value.as_str().map(str::to_string)).unwrap_or_else(|| action.default_accelerator().to_string());
                let parsed = Accelerator::parse(&accelerator).ok();
                let active = parsed.is_some_and(|parsed| self.backend.register(action.hotkey_id(), &parsed).is_ok());
                Slot { binding: Binding { action, accelerator, registered: active }, parsed, active }
            })
            .collect();
    }

    pub fn bindings(&self) -> Vec<Binding> {
        lock(&self.slots).iter().map(|slot| slot.binding.clone()).collect()
    }

    pub fn binding(&self, action: Action) -> Option<Binding> {
        lock(&self.slots).iter().find(|slot| slot.binding.action == action).map(|slot| slot.binding.clone())
    }

    // A WM_HOTKEY id: which action it is.
    pub fn action_for(&self, id: i32) -> Option<Action> {
        Action::from_hotkey_id(id)
    }

    // What `get_shortcuts` returned: { play: { accelerator, registered }, stop: …, toggle: … }.
    pub fn describe(&self) -> Value {
        describe(&self.bindings())
    }

    // Settings sets a shortcut (recorded while paused). An empty accelerator resets to the
    // default. The error texts are the ones Settings shows.
    pub fn set_shortcut(&self, action: &str, accelerator: &str) -> Result<Vec<Binding>, String> {
        let Some(action) = Action::from_name(action) else {
            return Err("Unknown action".into());
        };
        let accelerator = if accelerator.is_empty() { action.default_accelerator().to_string() } else { accelerator.to_string() };
        let bindings = {
            let mut slots = lock(&self.slots);
            let index = match slots.iter().position(|slot| slot.binding.action == action) {
                Some(index) => index,
                None => {
                    slots.push(Slot { binding: Binding { action, accelerator: String::new(), registered: false }, parsed: None, active: false });
                    slots.len() - 1
                }
            };
            if accelerator.is_empty() {
                self.release(&mut slots[index]);
                self.store.set(action.store_key(), Value::String(String::new()));
                slots[index] = Slot { binding: Binding { action, accelerator, registered: false }, parsed: None, active: false };
            } else {
                let parsed = Accelerator::parse(&accelerator).map_err(|_| "That isn't a valid shortcut".to_string())?;
                if slots.iter().any(|slot| slot.binding.action != action && slot.parsed == Some(parsed)) {
                    return Err("Already used for another action".into());
                }
                // Shortcuts are paused while one is recorded, so the probe only meets other apps.
                let free = self.backend.register(PROBE_ID, &parsed).is_ok();
                self.backend.unregister(PROBE_ID);
                if !free {
                    return Err("Another app is already using this shortcut".into());
                }
                self.release(&mut slots[index]);
                self.store.set(action.store_key(), Value::String(accelerator.clone()));
                // Registered when the shortcuts resume.
                slots[index] = Slot { binding: Binding { action, accelerator, registered: false }, parsed: Some(parsed), active: false };
            }
            slots.iter().map(|slot| slot.binding.clone()).collect::<Vec<_>>()
        };
        self.notify(&bindings);
        Ok(bindings)
    }

    // While Settings records a shortcut, every shortcut is unregistered: a registered one never
    // reaches the window. Resuming registers them again; closing Settings always resumes.
    pub fn set_paused(&self, paused: bool) -> Vec<Binding> {
        let bindings = {
            let mut slots = lock(&self.slots);
            for slot in slots.iter_mut() {
                let Some(parsed) = slot.parsed else { continue };
                if paused {
                    if slot.active {
                        self.backend.unregister(slot.binding.action.hotkey_id());
                        slot.active = false;
                    }
                } else if !slot.active {
                    slot.active = self.backend.register(slot.binding.action.hotkey_id(), &parsed).is_ok();
                    slot.binding.registered = slot.active;
                }
            }
            slots.iter().map(|slot| slot.binding.clone()).collect::<Vec<_>>()
        };
        if !paused {
            self.notify(&bindings);
        }
        bindings
    }

    // Unregisters everything (quitting).
    pub fn unregister_all(&self) {
        let mut slots = lock(&self.slots);
        for slot in slots.iter_mut() {
            self.release(slot);
        }
    }

    fn release(&self, slot: &mut Slot) {
        if slot.active {
            self.backend.unregister(slot.binding.action.hotkey_id());
            slot.active = false;
        }
    }

    fn notify(&self, bindings: &[Binding]) {
        if let Some(listener) = lock(&self.listener).as_ref() {
            listener(bindings);
        }
    }
}

pub fn describe(bindings: &[Binding]) -> Value {
    let mut out = serde_json::Map::new();
    for action in Action::ALL {
        if let Some(binding) = bindings.iter().find(|binding| binding.action == action) {
            out.insert(action.as_str().to_string(), json!({ "accelerator": binding.accelerator, "registered": binding.registered }));
        }
    }
    Value::Object(out)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::HotkeyError;
    use std::collections::HashMap;

    // An OS where `taken` belongs to other apps.
    #[derive(Default)]
    struct FakeOs {
        taken: Vec<String>,
        held: Mutex<HashMap<i32, String>>,
        log: Mutex<Vec<String>>,
    }

    impl HotkeyBackend for FakeOs {
        fn register(&self, id: i32, accelerator: &Accelerator) -> Result<(), HotkeyError> {
            let key = accelerator.to_stored();
            let mut held = self.held.lock().unwrap();
            self.log.lock().unwrap().push(format!("register {id} {key}"));
            if self.taken.contains(&key) || held.values().any(|other| *other == key) {
                return Err(HotkeyError::AlreadyRegistered);
            }
            held.insert(id, key);
            Ok(())
        }

        fn unregister(&self, id: i32) {
            self.log.lock().unwrap().push(format!("unregister {id}"));
            self.held.lock().unwrap().remove(&id);
        }
    }

    fn store(name: &str) -> Store {
        let path = std::env::temp_dir().join(format!("qust-shell-shortcuts-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Store::open(path)
    }

    fn setup(name: &str, taken: &[&str]) -> (Arc<FakeOs>, Shortcuts) {
        let os = Arc::new(FakeOs { taken: taken.iter().map(|t| t.to_string()).collect(), ..Default::default() });
        let shortcuts = Shortcuts::new(os.clone(), store(name));
        (os, shortcuts)
    }

    #[test]
    fn defaults_register_and_report() {
        let (os, shortcuts) = setup("defaults", &[]);
        shortcuts.register_saved();
        assert_eq!(
            shortcuts.describe(),
            json!({
                "play": { "accelerator": "Ctrl+Alt+Insert", "registered": true },
                "stop": { "accelerator": "Ctrl+Alt+KeyH", "registered": true },
                "toggle": { "accelerator": "", "registered": false },
            })
        );
        assert_eq!(os.held.lock().unwrap().len(), 2);
        assert_eq!(shortcuts.binding(Action::Play).unwrap().label().as_deref(), Some("Ctrl+Alt+Insert"));
        assert_eq!(shortcuts.binding(Action::Stop).unwrap().label().as_deref(), Some("Ctrl+Alt+H"));
        assert_eq!(shortcuts.binding(Action::Play).unwrap().menu_text().as_deref(), Some("Ctrl+Alt+Ins"));
        assert_eq!(shortcuts.binding(Action::Toggle).unwrap().label(), None);
        assert_eq!(shortcuts.action_for(2), Some(Action::Stop));
    }

    #[test]
    fn taken_at_startup_is_kept_but_unregistered() {
        let (_os, shortcuts) = setup("taken", &["Ctrl+Alt+Insert"]);
        shortcuts.register_saved();
        let play = shortcuts.binding(Action::Play).unwrap();
        assert_eq!(play.accelerator, "Ctrl+Alt+Insert");
        assert!(!play.registered);
        assert_eq!(play.label(), None);
        assert_eq!(play.keys(), ["Ctrl", "Alt", "Insert"]);
    }

    #[test]
    fn saved_values_are_used() {
        let (_os, shortcuts) = setup("saved", &[]);
        shortcuts.store.set("shortcut-play", json!("ctrl+shift+f9"));
        shortcuts.store.set("shortcut-stop", json!(""));
        shortcuts.store.set("shortcut-toggle", json!("Ctrl+Nonsense"));
        shortcuts.register_saved();
        let bindings = shortcuts.bindings();
        assert_eq!((bindings[0].accelerator.as_str(), bindings[0].registered), ("ctrl+shift+f9", true));
        assert_eq!((bindings[1].accelerator.as_str(), bindings[1].registered), ("", false));
        // Kept as stored, never registered.
        assert_eq!((bindings[2].accelerator.as_str(), bindings[2].registered), ("Ctrl+Nonsense", false));
    }

    #[test]
    fn recording_flow() {
        let (os, shortcuts) = setup("flow", &["Ctrl+Alt+KeyB"]);
        shortcuts.register_saved();
        let changes = Arc::new(Mutex::new(0));
        let counter = changes.clone();
        shortcuts.on_change(move |_| *counter.lock().unwrap() += 1);

        // Pausing unregisters but still reports them as registered.
        let paused = shortcuts.set_paused(true);
        assert!(paused[0].registered);
        assert!(os.held.lock().unwrap().is_empty());
        assert_eq!(*changes.lock().unwrap(), 0);

        assert_eq!(shortcuts.set_shortcut("nope", "F1"), Err("Unknown action".into()));
        assert_eq!(shortcuts.set_shortcut("play", "Ctrl+"), Err("That isn't a valid shortcut".into()));
        assert_eq!(shortcuts.set_shortcut("play", "Ctrl+Alt+KeyH"), Err("Already used for another action".into()));
        assert_eq!(shortcuts.set_shortcut("play", "ctrl+alt+h"), Err("Already used for another action".into()));
        assert_eq!(shortcuts.set_shortcut("play", "Ctrl+Alt+KeyB"), Err("Another app is already using this shortcut".into()));
        // The probe never stays registered.
        assert!(os.held.lock().unwrap().is_empty());

        let set = shortcuts.set_shortcut("toggle", "Ctrl+Alt+KeyP").unwrap();
        assert_eq!((set[2].accelerator.as_str(), set[2].registered), ("Ctrl+Alt+KeyP", false));
        assert_eq!(shortcuts.store.get("shortcut-toggle"), Some(json!("Ctrl+Alt+KeyP")));
        assert_eq!(*changes.lock().unwrap(), 1);

        // Resuming registers everything again and broadcasts.
        let resumed = shortcuts.set_paused(false);
        assert!(resumed.iter().all(|binding| binding.registered));
        assert_eq!(os.held.lock().unwrap().len(), 3);
        assert_eq!(*changes.lock().unwrap(), 2);

        // Resetting: Play back to its default, toggle to none.
        shortcuts.set_paused(true);
        shortcuts.set_shortcut("play", "F2").unwrap();
        let reset = shortcuts.set_shortcut("play", "").unwrap();
        assert_eq!(reset[0].accelerator, "Ctrl+Alt+Insert");
        let cleared = shortcuts.set_shortcut("toggle", "").unwrap();
        assert_eq!((cleared[2].accelerator.as_str(), cleared[2].registered), ("", false));
        assert_eq!(shortcuts.store.get("shortcut-toggle"), Some(json!("")));
        let resumed = shortcuts.set_paused(false);
        assert_eq!(resumed[0].accelerator, "Ctrl+Alt+Insert");
        assert!(resumed[0].registered);
        assert_eq!(os.held.lock().unwrap().len(), 2);

        // Resuming when not paused changes nothing.
        let before = os.log.lock().unwrap().len();
        shortcuts.set_paused(false);
        assert_eq!(os.log.lock().unwrap().len(), before);

        shortcuts.unregister_all();
        assert!(os.held.lock().unwrap().is_empty());
    }
}
