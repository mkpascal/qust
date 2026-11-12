//! Settings and Stream information: opened owned by the player (above it, no taskbar button),
//! centered over it and kept on its monitor, not minimizable, hidden until placed, title bar in
//! the app's theme, Esc closing them; an open one is brought to the front instead of opening a
//! second (open_panel in main.rs). And [`AppServices`], what both views ask of the app.

use crate::app::{self, qust};
use crate::compose::PanelKind;
use crate::config::WindowMode;
use crate::win;
use gpui::{point, px, size, App, Bounds, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowId, WindowKind, WindowOptions};
use qust_shell::gpui_glue;
use qust_shell::menu::{Menu, Position};
use qust_shell::recorder::Recorded;
use qust_shell::theme::{self as shell_theme, Theme};
use qust_ui::{Background, Bridge, WindowRoot};
use qust_ui_panels::{info, info_window, settings, settings_window, InfoRequest, MenuDone, PanelServices};
use serde_json::{json, Value};
use std::any::Any;
use std::rc::Rc;

#[derive(Clone)]
pub struct PanelWindow {
    pub kind: PanelKind,
    pub handle: WindowHandle<WindowRoot>,
    pub id: WindowId,
    pub hwnd: isize,
}

fn spec(kind: PanelKind) -> ((f32, f32), &'static str) {
    match kind {
        PanelKind::Settings => (settings::SIZE, settings::TITLE),
        PanelKind::Info => (info::SIZE, info::TITLE),
    }
}

/// Opens `kind` (or brings it to the front). Nothing without the player window: the panels
/// belong to it.
pub fn open(kind: PanelKind, cx: &mut App) {
    let inner = qust(cx);
    if let Some(panel) = inner.panel(kind) {
        if inner.mode == WindowMode::Normal {
            let hwnd = panel.hwnd;
            app::outside(cx, move || qust_shell::window::bring_to_front(hwnd));
        }
        return;
    }
    let Some(owner) = inner.player_hwnd() else { return };
    let models = inner.bridge.models.clone();
    if kind == PanelKind::Settings {
        // What the page asked for as it opened: their defaults read "not installed", "helper
        // unavailable", "no shortcuts".
        if let Some(updates) = &inner.updates {
            models.apply_update(serde_json::from_value(updates.describe()).unwrap_or_default(), cx);
        }
        let background = inner.shell.borrow().background();
        app::apply_background(background, cx);
        models.apply_shortcuts(serde_json::from_value(inner.shortcuts.describe()).unwrap_or_default(), cx);
    }
    let ((width, height), title) = spec(kind);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.0), px(0.0)), size(px(width), px(height))))),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), appears_transparent: false, traffic_light_position: None }),
        focus: inner.mode == WindowMode::Normal,
        // Shown once owned and placed.
        show: false,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some(qust_shell::instance::IDENTIFIER.into()),
        ..Default::default()
    };
    let services: Rc<dyn PanelServices> = inner.services.clone();
    let opened = cx.open_window(options, move |window, cx| match kind {
        PanelKind::Settings => settings_window(&models, services, window, cx).0,
        PanelKind::Info => info_window(&models, services, window, cx).0,
    });
    let handle = match opened {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("qust: could not open {title}: {error}");
            return;
        }
    };
    let hwnd = handle
        .update(cx, |_, window, cx| {
            for subscription in qust_ui::theme::follow(window, cx) {
                subscription.detach();
            }
            window.resize(size(px(width), px(height)));
            gpui_glue::hwnd(window).unwrap_or(0)
        })
        .unwrap_or(0);
    let panel = PanelWindow { kind, handle, id: handle.window_id(), hwnd };
    match kind {
        PanelKind::Settings => *inner.settings.borrow_mut() = Some(panel),
        PanelKind::Info => *inner.info.borrow_mut() = Some(panel),
    }
    inner.note(format!("{kind:?} opened"));
    let mode = inner.mode;
    let offscreen = (inner.offscreen_at.0, inner.offscreen_at.1 + 1000);
    // Owned, placed and shown once GPUI's resize (queued on its executor) has landed, outside
    // GPUI's handling (see `app::outside`).
    cx.spawn(async move |_cx| {
        if !win::is_window(hwnd) {
            return;
        }
        qust_shell::window::set_owner(hwnd, owner);
        qust_shell::window::set_minimizable(hwnd, false);
        qust_shell::window::follow_theme(hwnd);
        qust_shell::window::close_on_escape(hwnd);
        match mode {
            WindowMode::Offscreen => win::show_offscreen(hwnd, offscreen.0, offscreen.1),
            _ => {
                if let Some((_, (x, y))) = qust_shell::window::panel_position(owner, (width as f64, height as f64)) {
                    qust_shell::window::set_outer_position(hwnd, x, y);
                }
            }
        }
        if mode == WindowMode::Normal {
            #[cfg(windows)]
            qust_shell::window::bring_to_front(hwnd);
            #[cfg(not(windows))]
            let _ = handle.update(_cx, |_, window, _| window.activate_window());
        }
    })
    .detach();
}
/// Closes `kind` if open (as its close button would).
pub fn close(kind: PanelKind, cx: &mut App) {
    if let Some(panel) = qust(cx).panel(kind) {
        let _ = panel.handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// What the panels ask of the app.
pub struct AppServices;

impl PanelServices for AppServices {
    fn now(&self) -> u64 {
        qust_updater::now()
    }

    fn version(&self) -> String {
        crate::VERSION.to_string()
    }

    fn save_setting(&self, key: &str, value: Value, cx: &mut App) {
        Bridge::global(cx).player.write_setting(key, Some(value));
    }

    fn broadcast_setting(&self, key: &str, value: Value, cx: &mut App) {
        let models = Bridge::global(cx).models.clone();
        models.apply_setting(key, &value, cx);
    }

    fn set_theme(&self, theme: &str, cx: &mut App) {
        let bridge = Bridge::global(cx).clone();
        bridge.player.write_setting("theme", Some(json!(theme)));
        bridge.models.apply_setting("theme", &json!(theme), cx);
        qust_ui::theme::apply(qust_ui::theme::ThemeChoice::parse(theme), cx);
        // The menus and every window's title bar.
        let theme = Theme::from_setting(Some(theme));
        app::outside(cx, move || shell_theme::set_app_theme(theme));
    }

    fn pause_shortcuts(&self, paused: bool, cx: &mut App) {
        let inner = qust(cx);
        // Resuming reports through the shortcuts' listener; pausing doesn't, as before.
        inner.shortcuts.set_paused(paused);
    }

    fn set_shortcut(&self, action: &str, accelerator: &str, cx: &mut App) -> Result<(), String> {
        qust(cx).shortcuts.set_shortcut(action, accelerator).map(|_| ())
    }

    fn capture_keys(&self, window: &Window, on_key: Box<dyn FnMut(Recorded)>) -> Option<Box<dyn Any>> {
        #[cfg(windows)]
        {
            let hwnd = gpui_glue::hwnd(window)?;
            Some(Box::new(qust_shell::window::KeyCapture::start(hwnd, on_key)))
        }
        #[cfg(not(windows))]
        {
            let _ = (window, on_key);
            None
        }
    }

    fn set_background(&self, enabled: bool, cx: &mut App) -> Result<Background, String> {
        let value = qust(cx).shell.borrow().set_background(enabled)?;
        app::apply_background(value.clone(), cx);
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    fn check_for_updates(&self, cx: &mut App) {
        let Some(updates) = qust(cx).updates.clone() else { return };
        // The status reaches models.update through the service's listener.
        std::thread::spawn(move || {
            let _ = updates.check();
        });
    }

    fn install_update(&self, cx: &mut App) {
        let inner = qust(cx);
        let Some(updates) = inner.updates.clone() else { return };
        let main = inner.main.clone();
        std::thread::spawn(move || {
            if updates.install().is_ok() {
                main.post(app::installed_update);
            }
        });
    }

    fn popup_menu(&self, window: &Window, menu: Menu, position: Position, done: MenuDone, cx: &mut App) {
        app::show_menu(menu, position, window, done, cx);
    }

    fn copy_text(&self, text: String, cx: &mut App) {
        match qust(cx).hooks.clipboard.clone() {
            Some(take) => take(text),
            None => cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)),
        }
    }

    fn stream_info(&self, url: String, cx: &mut App) -> InfoRequest {
        let reply = Bridge::global(cx).player.stream_info(url);
        Box::pin(async move { reply.get().await })
    }

    fn close_window(&self, window: &mut Window, _cx: &mut App) {
        window.remove_window();
    }
}
