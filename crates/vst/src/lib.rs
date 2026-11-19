//! Qust's audio processing plugins: a host for 64-bit VST3 effects, run in order on everything
//! Qust plays (after the mix, before the analyzer, the volume and the balance).
//!
//! [`Plugins::start`] starts the plugin thread with the store (the list is `audio-plugins`, each
//! plugin's settings a file next to the settings file) and the engines' [`EffectSlot`]. The
//! plugins run while an engine does (the player window is open): when one starts, the enabled
//! plugins start for its rate and become its chain; when it goes, their settings are saved and
//! they are let go of. A plugin that is off isn't started until it is turned on or its window is
//! asked for.
//!
//! Everything the UI shows comes through `on_change` as a [`Snapshot`], called on the plugin
//! thread whenever it changes. The commands never wait.
//!
//! A plugin runs inside Qust, so one that crashes takes Qust with it. The plugin being started is
//! marked in the store meanwhile: if Qust closed then, the next launch turns that plugin off and
//! says why.
//!
//! VST3 hosting is Windows only: elsewhere [`AVAILABLE`] is false and the list is only shown.

pub mod bundle;
mod ring;
pub mod saved;
#[cfg(windows)]
mod win;

use qust_audio::EffectSlot;
use qust_player::Store;
use std::path::PathBuf;
use std::sync::Arc;

/// Whether plugins can run here.
pub const AVAILABLE: bool = cfg!(windows);

#[derive(Clone, Debug, PartialEq)]
pub enum PluginState {
    /// Not running: off, or no engine (no player window).
    Off,
    /// Starting.
    Loading,
    /// Running (in the chain when it is on).
    Ready,
    /// Couldn't start, or made Qust close while starting: why.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginRow {
    /// Lasts as long as the app runs; not saved.
    pub id: u64,
    pub name: String,
    pub vendor: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub state: PluginState,
    /// Its window is open.
    pub editor_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// In processing order.
    pub plugins: Vec<PluginRow>,
    /// Why the last thing asked for didn't work ("Couldn't add …"), until the next change.
    pub notice: Option<String>,
}

pub struct Plugins {
    #[cfg(windows)]
    handle: Option<win::Handle>,
}

impl Plugins {
    pub fn start(store: Store, slot: Arc<EffectSlot>, on_change: impl Fn(Snapshot) + Send + 'static) -> Plugins {
        #[cfg(windows)]
        {
            Plugins { handle: win::Handle::start(store, slot, Box::new(on_change)) }
        }
        #[cfg(not(windows))]
        {
            let _ = slot;
            let plugins = saved::load(&store)
                .into_iter()
                .enumerate()
                .map(|(index, saved)| PluginRow { id: index as u64 + 1, name: saved.name, vendor: saved.vendor, path: saved.path, enabled: saved.enabled, state: PluginState::Off, editor_open: false })
                .collect();
            on_change(Snapshot { plugins, notice: None });
            Plugins {}
        }
    }

    #[cfg(windows)]
    fn send(&self, command: win::Command) {
        if let Some(handle) = &self.handle {
            handle.send(command);
        }
    }

    /// Adds plugins at the end of the list, on (picked `.vst3` bundles, files, or the binaries in
    /// bundles). The ones that can't be used aren't added: the snapshot's notice says why.
    pub fn add(&self, paths: Vec<PathBuf>) {
        #[cfg(windows)]
        self.send(win::Command::Add(paths));
        #[cfg(not(windows))]
        let _ = paths;
    }

    /// Removes a plugin, and forgets its settings.
    pub fn remove(&self, id: u64) {
        #[cfg(windows)]
        self.send(win::Command::Remove(id));
        #[cfg(not(windows))]
        let _ = id;
    }

    /// Moves a plugin to `index` in the processing order.
    pub fn move_to(&self, id: u64, index: usize) {
        #[cfg(windows)]
        self.send(win::Command::Move(id, index));
        #[cfg(not(windows))]
        let _ = (id, index);
    }

    /// Turns a plugin on (it starts, and processes) or off (it stays out of the chain; its window
    /// and settings stay).
    pub fn set_enabled(&self, id: u64, enabled: bool) {
        #[cfg(windows)]
        self.send(win::Command::Enable(id, enabled));
        #[cfg(not(windows))]
        let _ = (id, enabled);
    }

    /// Opens the plugin's own window, centered over `near` (a window handle), or brings it to
    /// the front.
    pub fn open_editor(&self, id: u64, near: Option<isize>) {
        #[cfg(windows)]
        self.send(win::Command::Edit(id, near));
        #[cfg(not(windows))]
        let _ = (id, near);
    }

    /// The plugins' windows' title bars follow the app's theme.
    pub fn set_dark(&self, dark: bool) {
        #[cfg(windows)]
        self.send(win::Command::Dark(dark));
        #[cfg(not(windows))]
        let _ = dark;
    }

    /// Saves every plugin's settings and lets go of them (quitting). Waits up to 5 s.
    pub fn shutdown(&self) {
        #[cfg(windows)]
        if let Some(handle) = &self.handle {
            handle.shutdown();
        }
    }
}

/// The plugins installed in the standard VST3 folders, for the Add menu.
pub fn installed() -> Vec<PathBuf> {
    if AVAILABLE {
        bundle::installed()
    } else {
        Vec::new()
    }
}
