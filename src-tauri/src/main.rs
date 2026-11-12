#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod background;
mod menu;
mod player;
mod playlist;
mod shortcuts;
mod store;
mod updater;

use qust_player::playlist::{player_height, PLAYER_WIDTH};
use tauri::window::Color;
use tauri::{AppHandle, Manager, RunEvent, State, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

// Injected into every window as window.__QUST__ (see src/js/bridge.js).
struct PageConfig {
    script: String,
}

fn main() {
    let builder = tauri::Builder::default();
    // First, so a second launch (Start menu while the helper runs) just shows this instance.
    // `qust.exe --toggle` acts like the Play / Stop / Start shortcut (e.g. from a Stream Deck
    // "Open" action). Not in debug builds: they share the installed app's identifier, so they would
    // just hand over to an installed Qust running in the tray instead of starting.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _| {
        if args.iter().any(|arg| arg == background::TOGGLE_ARG) {
            background::player_action(app, "toggle");
        } else if !args.iter().any(|arg| arg == background::LAUNCH_ARG) {
            background::show_player(app, false);
        }
    }));
    builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(shortcuts::handle).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![background::LAUNCH_ARG]),
        ))
        .manage(shortcuts::Shortcuts::default())
        .manage(updater::Updates::default())
        .invoke_handler(tauri::generate_handler![
            store::get_data,
            store::set_data,
            store::delete_data,
            shortcuts::get_shortcuts,
            shortcuts::set_shortcut,
            shortcuts::pause_shortcuts,
            updater::update_status,
            updater::check_for_updates,
            updater::install_update,
            background::get_background,
            background::set_background,
            player::player_attach,
            player::player_play,
            player::player_pause,
            player::player_stop,
            player::player_seek,
            player::player_set_volume,
            player::player_set_balance,
            player::player_set_instant,
            player::player_cycle_colors,
            player::player_status,
            player::cartridge_play,
            player::cartridge_save,
            player::cartridge_eject,
            player::cartridge_move,
            player::playlist_play,
            player::playlist_add_url,
            player::playlist_add_files,
            player::playlist_drop,
            player::playlist_remove,
            player::playlist_move,
            player::playlist_edit,
            player::stream_info,
            playlist::set_playlist_open,
            menu::popup_menu,
            open_panel,
            set_theme
        ])
        .on_window_event(|window, event| {
            // Closing settings mid-recording must not leave the shortcuts paused.
            if window.label() == "settings" && matches!(event, WindowEvent::Destroyed) {
                shortcuts::set_paused(window.app_handle(), false);
            }
            // Playback doesn't outlive the player window (Qust may stay in the tray).
            if window.label() == "main" && matches!(event, WindowEvent::Destroyed) {
                window.app_handle().state::<player::Player>().detach();
            }
            if let (WindowEvent::ThemeChanged(theme), Some(window)) = (event, window.app_handle().get_webview_window(window.label())) {
                let _ = window.set_background_color(Some(background(*theme)));
            }
        })
        .setup(|app| {
            let store = store::Store::load()?;
            let player = player::Player::start(store.clone(), player::TauriHost(app.handle().clone()), qust_player::OutputKind::Device);
            // Settings written by the Settings window take effect at once.
            let watcher = player.clone();
            store.watch(move |key, value| watcher.setting_changed(key, value.cloned()));
            app.manage(store);
            app.manage(player);
            let config = serde_json::json!({
                "accent": qust_player::accent(),
                "debug": cfg!(debug_assertions),
            });
            app.manage(PageConfig { script: format!("window.__QUST__ = {config};") });

            // Started by Windows with the helper on: tray only, the player opens on demand.
            if background::enabled(app.handle()) {
                background::create_tray(app.handle())?;
            }
            if background::launched_with(background::TOGGLE_ARG) {
                background::show_player(app.handle(), true);
            } else if !(background::enabled(app.handle()) && background::launched_with(background::LAUNCH_ARG)) {
                build_player(app.handle())?;
            }
            shortcuts::register_saved(app.handle());
            updater::schedule(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running Qust")
        .run(|app, event| {
            // With the helper on, closing the last window leaves Qust running in the tray; "Quit
            // Qust" in the tray menu exits with a code, which is let through.
            if let RunEvent::ExitRequested { api, code: None, .. } = event {
                if background::enabled(app) {
                    api.prevent_exit();
                }
            }
        });
}

pub fn build_player(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    build_window(app, "main", "index.html", "Qust", (PLAYER_WIDTH, player_height(&app.state::<store::Store>())), None)
}

fn build_window(
    app: &AppHandle,
    label: &str,
    page: &str,
    title: &str,
    size: (f64, f64),
    owner: Option<&WebviewWindow>,
) -> tauri::Result<WebviewWindow> {
    let config = app.state::<PageConfig>();
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title(title)
        .theme(stored_theme(app))
        .inner_size(size.0, size.1)
        .resizable(false)
        .maximizable(false)
        .initialization_script(&config.script);
    if let Some(owner) = owner {
        // Owned by the player: stays above it and gets no taskbar button of its own.
        builder = builder.parent(owner)?.minimizable(false);
        if let Some((x, y)) = centered_over(owner, size) {
            builder = builder.position(x, y);
        }
    }
    #[cfg(windows)]
    {
        // Every webview shares one browser process, so they all need the same arguments.
        builder = builder.additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    }
    let window = builder.build()?;
    if let Ok(theme) = window.theme() {
        let _ = window.set_background_color(Some(background(theme)));
    }
    Ok(window)
}

// The page's background (--bg in style.css), also shown where the window grows before the page
// repaints (the playlist drawer opening).
fn background(theme: Theme) -> Color {
    match theme {
        Theme::Light => Color(0xf3, 0xf3, 0xf3, 0xff),
        _ => Color(0x20, 0x20, 0x20, 0xff),
    }
}

// Logical position that centers a window of `size` over `owner`, kept on the owner's monitor.
fn centered_over(owner: &WebviewWindow, size: (f64, f64)) -> Option<(f64, f64)> {
    let scale = owner.scale_factor().ok()?;
    let position = owner.outer_position().ok()?.to_logical::<f64>(scale);
    let outer = owner.outer_size().ok()?.to_logical::<f64>(scale);
    let mut x = position.x + (outer.width - size.0) / 2.0;
    let mut y = position.y + (outer.height - size.1) / 2.0;
    if let Some(monitor) = owner.current_monitor().ok().flatten() {
        let area = monitor.work_area();
        let origin = area.position.to_logical::<f64>(scale);
        let extent = area.size.to_logical::<f64>(scale);
        x = x.clamp(origin.x, (origin.x + extent.width - size.0 - 16.0).max(origin.x));
        y = y.clamp(origin.y, (origin.y + extent.height - size.1 - 40.0).max(origin.y));
    }
    Some((x, y))
}

fn stored_theme(app: &AppHandle) -> Option<Theme> {
    match app.state::<store::Store>().get("theme").as_ref().and_then(|value| value.as_str()) {
        Some("light") => Some(Theme::Light),
        Some("dark") => Some(Theme::Dark),
        _ => None,
    }
}

// Opens (or focuses) one of the windows owned by the player: "settings" or "info" (Stream
// information). Async: creating a window from a synchronous command deadlocks on Windows.
#[tauri::command]
async fn open_panel(app: AppHandle, name: String) -> Result<(), String> {
    let (page, title, size) = match name.as_str() {
        "settings" => ("settings.html", "Qust settings", (520.0, 850.0)),
        "info" => ("info.html", "Stream information", (580.0, 590.0)),
        _ => return Err(format!("unknown panel {name}")),
    };
    if let Some(window) = app.get_webview_window(&name) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    let owner = app.get_webview_window("main");
    build_window(&app, &name, page, title, size, owner.as_ref())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_theme(app: AppHandle, store: State<store::Store>, theme: String) {
    store.set("theme", serde_json::Value::String(theme));
    let theme = stored_theme(&app);
    for window in app.webview_windows().values() {
        let _ = window.set_theme(theme);
        if let Ok(theme) = window.theme() {
            let _ = window.set_background_color(Some(background(theme)));
        }
    }
}
