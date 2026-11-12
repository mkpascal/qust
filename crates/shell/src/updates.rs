// The updater's schedule and settings around qust-updater's `Updates`, as updater.rs kept them:
// "Check automatically" (`auto-update`, on unless false), `update-last-check` (seconds since
// 1970), a first check 30 s after launch and then whenever the last one is 8 hours old (looked at
// every half hour), and the status Settings shows (`update-status`).
//
// Installing: `install` starts the installer; then the app must let go of the single-instance lock
// (the installer starts Qust again, which must not hand over to this one) and quit at once — on
// Windows the installer closes whatever still runs and reopens Qust; on macOS the app starts itself
// again (`relaunch`).

use qust_player::Store;
use qust_updater::{Snapshot, Updater, Updates};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::sync::Arc;

type Listener = Arc<dyn Fn(Value) + Send + Sync>;

pub struct UpdateService {
    updates: Updates,
    store: Store,
    current: String,
}

impl UpdateService {
    // `current` is the app's version (the release version, "1.1.5"; not a crate's 0.1.0).
    // `listener` gets every status change, as the `update-status` payload.
    pub fn new(store: Store, current: &str, listener: impl Fn(Value) + Send + Sync + 'static) -> Result<Arc<UpdateService>, String> {
        let updater = Updater::new(qust_updater::ENDPOINT, qust_updater::PUBLIC_KEY, current).map_err(|error| error.to_string())?;
        Self::with_updater(store, updater, listener)
    }

    pub fn with_updater(store: Store, updater: Updater, listener: impl Fn(Value) + Send + Sync + 'static) -> Result<Arc<UpdateService>, String> {
        let listener: Listener = Arc::new(listener);
        let current = updater.current_version();
        // The service's own status reaches the listener through a weak link, set once built.
        let slot: Arc<std::sync::OnceLock<std::sync::Weak<UpdateService>>> = Arc::new(std::sync::OnceLock::new());
        let hook = slot.clone();
        let saving = store.clone();
        let updates = Updates::new(updater, move |snapshot: &Snapshot| {
            if let Some(checked) = snapshot.checked_at {
                if saving.get("update-last-check").and_then(|value| value.as_u64()) != Some(checked) {
                    saving.set("update-last-check", json!(checked));
                }
            }
            if let Some(service) = hook.get().and_then(|weak| weak.upgrade()) {
                listener(service.describe_snapshot(snapshot));
            }
        });
        let service = Arc::new(UpdateService { updates, store, current });
        let _ = slot.set(Arc::downgrade(&service));
        Ok(service)
    }

    pub fn auto_enabled(&self) -> bool {
        self.store.get("auto-update") != Some(Value::Bool(false))
    }

    // update_status: { installed, current, status, version, progress, error, lastCheck, auto }.
    pub fn describe(&self) -> Value {
        self.describe_snapshot(&self.updates.snapshot())
    }

    fn describe_snapshot(&self, snapshot: &Snapshot) -> Value {
        json!({
            "installed": qust_updater::installed(),
            "current": self.current,
            "status": snapshot.status.as_str(),
            "version": snapshot.version,
            "progress": snapshot.progress,
            "error": snapshot.error,
            "lastCheck": self.store.get("update-last-check"),
            "auto": self.auto_enabled(),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.updates.snapshot()
    }

    // check_for_updates. Blocking (network): call it from a background thread.
    pub fn check(&self) -> Result<Value, String> {
        self.updates.check()?;
        Ok(self.describe())
    }

    // install_update, up to starting the installer. Blocking. On Ok, release the instance lock
    // and quit (Windows), or `relaunch` (macOS).
    pub fn install(&self) -> Result<(), String> {
        self.updates.install(&relaunch_args())
    }

    // The background check: 30 s after launch, then every half hour while due and "Check
    // automatically" is on. Only the installed version checks.
    pub fn schedule(self: &Arc<Self>) {
        if !qust_updater::installed() {
            return;
        }
        let service = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("qust-updates".into())
            .spawn(move || {
                std::thread::sleep(qust_updater::FIRST_CHECK_DELAY);
                loop {
                    let Some(service) = service.upgrade() else { return };
                    if service.due(qust_updater::now()) {
                        let _ = service.updates.check();
                    }
                    drop(service);
                    std::thread::sleep(qust_updater::SCHEDULE_TICK);
                }
            })
            .ok();
    }

    // Whether a scheduled check should run now.
    pub fn due(&self, now: u64) -> bool {
        self.auto_enabled() && qust_updater::check_due(self.store.get("update-last-check").and_then(|value| value.as_u64()), now)
    }
}

// The arguments the installer starts Qust with again: this launch's (so a tray-only helper comes
// back as one), as the Tauri updater passed them.
pub fn relaunch_args() -> Vec<OsString> {
    std::env::args_os().skip(1).collect()
}

// macOS, after `install` replaced the bundle: start the new copy (the instance lock released
// first), then quit.
pub fn relaunch() -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?).args(relaunch_args()).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn store(name: &str) -> Store {
        let path = std::env::temp_dir().join(format!("qust-shell-updates-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Store::open(path)
    }

    #[test]
    fn settings_and_schedule() {
        let store = store("schedule");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let updater = Updater::new("http://127.0.0.1:9/latest.json", qust_updater::PUBLIC_KEY, "1.1.5").unwrap();
        let service = UpdateService::with_updater(store.clone(), updater, move |value| log.lock().unwrap().push(value)).unwrap();
        let status = service.describe();
        assert_eq!(status["current"], "1.1.5");
        assert_eq!(status["status"], "idle");
        assert_eq!(status["auto"], true);
        assert_eq!(status["lastCheck"], Value::Null);
        assert_eq!(status["installed"], false);

        // Due when never checked, or 8 hours after the last check, and only with auto on.
        let now = 1_800_000_000;
        assert!(service.due(now));
        store.set("update-last-check", json!(now - 8 * 3600 + 1));
        assert!(!service.due(now));
        store.set("update-last-check", json!(now - 8 * 3600));
        assert!(service.due(now));
        store.set("auto-update", json!(false));
        assert!(!service.due(now));
        assert_eq!(service.describe()["auto"], false);

        // A test build isn't installed: checking says so, and nothing is broadcast.
        assert_eq!(service.check(), Err("Updates work in the installed version of Qust".into()));
        assert!(seen.lock().unwrap().is_empty());
    }
}
