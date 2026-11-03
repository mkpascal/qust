#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod proxy;
mod store;

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn main() {
    let play = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Insert);
    let stop = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut == &play {
                        let _ = app.emit("play-action", ());
                    } else if shortcut == &stop {
                        let _ = app.emit("stop-action", ());
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            store::get_data,
            store::set_data,
            store::delete_data
        ])
        .setup(move |app| {
            app.manage(store::Store::load(app.handle())?);
            let proxy = proxy::start()?;

            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Qust")
                .inner_size(964.0, 283.0)
                .resizable(false)
                .maximizable(false)
                .initialization_script(&format!("window.__QUST_PROXY__ = {:?};", proxy));
            // Media may start from a global shortcut, without a click in the window.
            #[cfg(windows)]
            let window = window.additional_browser_args(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required",
            );
            window.build()?;

            for shortcut in [play, stop] {
                if let Err(error) = app.global_shortcut().register(shortcut) {
                    eprintln!("could not register {shortcut}: {error}");
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Qust");
}
