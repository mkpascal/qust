// Auto-update from the GitHub releases (latest.json, signed by CI; see tauri.conf.json > plugins).
// An update is checked and downloaded in the background, then installed when the user chooses
// "Restart and update" in Settings, so playback is never cut without asking. Only the installed
// version updates itself: a portable or development copy has no installer to update.

use crate::store::Store;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_EVERY_SECONDS: u64 = 8 * 60 * 60;

#[derive(Default)]
pub struct Updates(Mutex<UpdateState>);

#[derive(Default)]
struct UpdateState {
    // "idle", "checking", "current", "downloading", "ready", "installing" or "error"
    status: &'static str,
    version: Option<String>,
    progress: Option<u32>,
    error: Option<String>,
    ready: Option<(Update, Vec<u8>)>,
}

// The NSIS installer puts uninstall.exe next to the app; the portable exe has none.
pub fn installed() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    #[cfg(windows)]
    return std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe").exists()))
        .unwrap_or(false);
    #[cfg(not(windows))]
    return true;
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

fn auto_enabled(app: &AppHandle) -> bool {
    app.state::<Store>().get("auto-update") != Some(Value::Bool(false))
}

fn describe(app: &AppHandle) -> Value {
    let updates = app.state::<Updates>();
    let state = updates.0.lock().unwrap();
    json!({
        "installed": installed(),
        "current": app.package_info().version.to_string(),
        "status": if state.status.is_empty() { "idle" } else { state.status },
        "version": state.version,
        "progress": state.progress,
        "error": state.error,
        "lastCheck": app.state::<Store>().get("update-last-check"),
        "auto": auto_enabled(app),
    })
}

fn set(app: &AppHandle, change: impl FnOnce(&mut UpdateState)) {
    change(&mut app.state::<Updates>().0.lock().unwrap());
    let _ = app.emit("update-status", describe(app));
}

// Checks GitHub and, when there is a newer version, downloads it so it's ready to install.
async fn check(app: &AppHandle) -> Result<(), String> {
    if !installed() {
        return Err("Updates work in the installed version of Qust".into());
    }
    let busy = matches!(app.state::<Updates>().0.lock().unwrap().status, "checking" | "downloading" | "ready" | "installing");
    if busy {
        return Ok(());
    }
    set(app, |state| {
        state.status = "checking";
        state.error = None;
    });

    let result = async {
        let update = app.updater().map_err(|error| error.to_string())?.check().await.map_err(|error| error.to_string())?;
        app.state::<Store>().set("update-last-check", json!(now()));
        let Some(update) = update else {
            set(app, |state| state.status = "current");
            return Ok(());
        };

        let version = update.version.clone();
        set(app, |state| {
            state.status = "downloading";
            state.version = Some(version);
            state.progress = Some(0);
        });
        let progress_app = app.clone();
        let mut received = 0u64;
        let mut shown = 0u32;
        let bytes = update
            .download(
                move |chunk, total| {
                    received += chunk as u64;
                    let percent = total.filter(|total| *total > 0).map_or(0, |total| (received * 100 / total) as u32);
                    if percent > shown {
                        shown = percent;
                        set(&progress_app, |state| state.progress = Some(percent));
                    }
                },
                || {},
            )
            .await
            .map_err(|error| error.to_string())?;
        set(app, |state| {
            state.status = "ready";
            state.progress = None;
            state.ready = Some((update, bytes));
        });
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = &result {
        set(app, |state| {
            state.status = "error";
            state.progress = None;
            state.error = Some(error.clone());
        });
    }
    result
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> Value {
    describe(&app)
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Value, String> {
    check(&app).await?;
    Ok(describe(&app))
}

// "Update now": checks and downloads first when needed, then installs and restarts. On Windows the
// installer closes Qust itself and opens it again when done.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if app.state::<Updates>().0.lock().unwrap().ready.is_none() {
        check(&app).await?;
    }
    let ready = app.state::<Updates>().0.lock().unwrap().ready.take();
    let Some((update, bytes)) = ready else {
        return Err("Qust is up to date".into());
    };
    set(&app, |state| state.status = "installing");
    if let Err(error) = update.install(bytes) {
        set(&app, |state| {
            state.status = "error";
            state.error = Some(error.to_string());
        });
        return Err(error.to_string());
    }
    app.restart();
}

// Background check: shortly after launch, then every 8 hours while "Check automatically" is on.
pub fn schedule(app: AppHandle) {
    if !installed() {
        return;
    }
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(30));
        loop {
            let last = app.state::<Store>().get("update-last-check").and_then(|value| value.as_u64()).unwrap_or(0);
            if auto_enabled(&app) && now().saturating_sub(last) >= CHECK_EVERY_SECONDS {
                let _ = tauri::async_runtime::block_on(check(&app));
            }
            thread::sleep(Duration::from_secs(30 * 60));
        }
    });
}
