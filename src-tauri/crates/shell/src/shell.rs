// Everything above, started in the right order for the app's main(), before GPUI:
//
//   let launch = Launch::from_env();
//   let shell = match Shell::start(store.clone(), launch) {
//       Started::Primary(shell, request) => shell,     // `request`: what to do once GPUI runs
//       Started::Secondary => return,                    // handed over to the running Qust
//   };
//
// then GPUI (headless when `request` is `Request::Nothing`, i.e. tray only), and in it
// `gpui_glue::forward_events(&shell.native, …)` with `background::route` for each event.

use crate::background::{self, Launch, Request};
use crate::hotkeys::Hotkeys;
use crate::instance::{self, Instance, InstanceLock};
use crate::shortcuts::Shortcuts;
use crate::startup::StartupEntry;
use crate::theme::{self, Theme};
use crate::tray::Tray;
use crate::NativeThread;
use qust_player::Store;
use serde_json::Value;
use std::sync::Arc;

pub struct Shell {
    pub native: NativeThread,
    pub shortcuts: Arc<Shortcuts>,
    pub tray: Tray,
    pub startup: StartupEntry,
    store: Store,
    instance: Option<InstanceLock>,
}

pub enum Started {
    Primary(Shell, Request),
    // Another Qust runs and got this launch's arguments: exit.
    Secondary,
}

impl Shell {
    pub fn start(store: Store, launch: Launch) -> std::io::Result<Started> {
        // First, so a second launch only wakes the running Qust.
        let mut lock = match instance::acquire(&instance::app_id()) {
            Instance::Primary(lock) => lock,
            Instance::Secondary { .. } => return Ok(Started::Secondary),
        };
        crate::set_app_identity();
        theme::set_app_theme(Theme::from_store(&store));
        let native = NativeThread::start()?;
        lock.listen(&native);
        let shortcuts = Arc::new(Shortcuts::new(Arc::new(Hotkeys::new(&native)), store.clone()));
        shortcuts.register_saved();
        let tray = Tray::new(&native);
        let (show_tray, request) = background::at_startup(launch, background::enabled(&store));
        if show_tray {
            tray.show();
        }
        let startup = StartupEntry::for_app().unwrap_or_else(|_| StartupEntry::new(crate::startup::NAME, "qust.exe", &[background::LAUNCH_ARG]));
        Ok(Started::Primary(Shell { native, shortcuts, tray, startup, store, instance: Some(lock) }, request))
    }

    // Settings' "Background helper": get_background and set_background.
    pub fn background(&self) -> Value {
        background::describe(&self.store, &self.startup)
    }

    pub fn set_background(&self, on: bool) -> Result<Value, String> {
        background::set_background(&self.store, &self.startup, &self.tray, on)
    }

    // Whether closing the last window leaves Qust in the tray.
    pub fn keep_running_without_windows(&self) -> bool {
        background::keep_running_without_windows(&self.store)
    }

    // Before the updater's installer starts Qust again: the new copy must be the first instance.
    pub fn release_instance(&mut self) {
        if let Some(lock) = self.instance.take() {
            lock.release();
        }
    }

    // "Quit Qust", or the last window closed without the helper: everything this owns let go.
    // Then quit GPUI.
    pub fn stop(&mut self) {
        self.shortcuts.unregister_all();
        self.tray.hide();
        self.release_instance();
        self.native.stop();
    }
}
