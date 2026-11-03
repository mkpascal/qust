#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod proxy;
mod shortcuts;
mod store;

use tauri::{AppHandle, Manager, State, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};


// Injected into every window as window.__QUST__ (see src/js/bridge.js).
struct PageConfig {
    script: String,
    mica: bool,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(shortcuts::handle).build())
        .manage(shortcuts::Shortcuts::default())
        .invoke_handler(tauri::generate_handler![
            store::get_data,
            store::set_data,
            store::delete_data,
            shortcuts::get_shortcuts,
            shortcuts::set_shortcut,
            shortcuts::pause_shortcuts,
            open_panel,
            set_theme
        ])
        .on_window_event(|window, event| {
            // Closing settings mid-recording must not leave the shortcuts paused.
            if window.label() == "settings" && matches!(event, WindowEvent::Destroyed) {
                shortcuts::set_paused(window.app_handle(), false);
            }
        })
        .setup(|app| {
            app.manage(store::Store::load(app.handle())?);
            let mica = mica_supported();
            let config = serde_json::json!({
                "proxy": proxy::start()?,
                "mica": mica,
                "accent": accent(),
                "debug": cfg!(debug_assertions),
            });
            app.manage(PageConfig { script: format!("window.__QUST__ = {config};"), mica });

            build_window(app.handle(), "main", "index.html", "Qust", (860.0, 267.0), None)?;
            shortcuts::register_saved(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Qust");
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
        // Every webview shares one browser process, so they all need the same arguments. Media may
        // start from a global shortcut, without a click in the window.
        builder = builder.additional_browser_args(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required",
        );
        if config.mica {
            use tauri::window::{Effect, EffectsBuilder};
            builder = builder.transparent(true).effects(EffectsBuilder::new().effect(Effect::Mica).build());
        }
    }
    builder.build()
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
        "settings" => ("settings.html", "Qust settings", (520.0, 560.0)),
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
    }
}

// Mica needs Windows 11 (build 22000). Elsewhere the page keeps its solid background.
fn mica_supported() -> bool {
    #[cfg(windows)]
    return windows_version::OsVersion::current().build >= 22000;
    #[cfg(not(windows))]
    return false;
}

// The user's accent from the Windows palette: the base color, the lighter shade Windows uses in
// dark mode (Light 2) and the darker one used in light mode (Dark 1).
#[cfg(windows)]
fn accent() -> Option<serde_json::Value> {
    let key = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
        .ok()?;
    let palette = key.get_value("AccentPalette").ok()?;
    let color = |index: usize| {
        let rgb = palette.get(index * 4..index * 4 + 3)?;
        Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
    };
    Some(serde_json::json!({ "base": color(3)?, "light": color(1)?, "dark": color(4)? }))
}

#[cfg(not(windows))]
fn accent() -> Option<serde_json::Value> {
    None
}
