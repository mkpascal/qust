// The background helper: with "background" on, Qust starts with Windows in the system tray with no
// window (--background), keeps running in the tray when the player is closed, and opens the player
// when a Play or Play / Stop / Start shortcut is pressed. The helper is this same process, just
// without windows: the shortcuts, store and proxy keep running.

use crate::store::Store;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

pub const LAUNCH_ARG: &str = "--background";
pub const TOGGLE_ARG: &str = "--toggle";
const TRAY_ID: &str = "qust";

// Set when the player is opened to play: its page asks for it once loaded (take_play_request).
#[derive(Default)]
pub struct PlayRequest(AtomicBool);

pub fn enabled(app: &AppHandle) -> bool {
    app.state::<Store>().get("background") == Some(Value::Bool(true))
}

pub fn launched_with(arg: &str) -> bool {
    std::env::args().any(|given| given == arg)
}

// Shows the player, opening it when closed. With `play`, it also starts the last cartridge.
pub fn show_player(app: &AppHandle, play: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if play {
            let _ = app.emit("play-action", ());
        }
        return;
    }
    app.state::<PlayRequest>().0.store(play, Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::build_player(&app) {
            eprintln!("could not open the player: {error}");
        }
    });
}

// A shortcut or tray action. With the player closed, Play and Play / Stop / Start open it and
// play; Stop has nothing to do.
pub fn player_action(app: &AppHandle, action: &str) {
    if app.get_webview_window("main").is_some() {
        let _ = app.emit(&format!("{action}-action"), ());
    } else if action == "play" || action == "toggle" {
        show_player(app, true);
    }
}

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open Qust", true, None::<&str>)?,
            &MenuItem::with_id(app, "toggle", "Play / Stop", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit Qust", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Qust")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_player(app, false),
            "toggle" => player_action(app, "toggle"),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_player(tray.app_handle(), false);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn describe(app: &AppHandle) -> Value {
    json!({
        "enabled": enabled(app),
        "startsWithWindows": app.autolaunch().is_enabled().unwrap_or(false),
    })
}

#[tauri::command]
pub fn get_background(app: AppHandle) -> Value {
    describe(&app)
}

#[tauri::command]
pub fn set_background(app: AppHandle, enabled: bool) -> Result<Value, String> {
    let autolaunch = app.autolaunch();
    let result = if enabled { autolaunch.enable() } else { autolaunch.disable() };
    result.map_err(|error| format!("Couldn't change the Windows startup entry: {error}"))?;
    app.state::<Store>().set("background", Value::Bool(enabled));
    if enabled {
        create_tray(&app).map_err(|error| error.to_string())?;
    } else {
        let _ = app.remove_tray_by_id(TRAY_ID);
    }
    Ok(describe(&app))
}

#[tauri::command]
pub fn take_play_request(app: AppHandle) -> bool {
    app.state::<PlayRequest>().0.swap(false, Ordering::SeqCst)
}
