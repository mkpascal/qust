//! Qust on GPUI, the executable: the finished parts wired together the way the Tauri app's
//! main.rs, background.rs, shortcuts.rs, updater.rs, playlist.rs and menu.rs did it.
//!
//! # Threads
//!
//! - **GPUI's main thread** (the process's first thread): every window, every model, the bridge's
//!   UI task, and everything in this crate. Nothing here blocks it except quitting (the player's
//!   pending saves, at most 2 s).
//! - **The shell thread** (`qust_shell::NativeThread`, a plain Win32 message loop): global
//!   shortcuts, the tray icon and its menu, the single-instance window, system appearance
//!   changes. Its events reach the main thread through `gpui_glue::forward_events` and are
//!   turned into requests by `qust_shell::background::route`.
//! - **The player** (`qust_player::Player`'s thread, the audio engine's threads) and the bridge's
//!   command thread (`qust_ui::Player` queues every command there, so the UI never waits).
//! - Short-lived: the file picker (a thread of its own), update checks and installs, the drawer
//!   animation's timer (each step is applied on the main thread).
//!
//! # Start-up ([`run`])
//!
//! `Launch::from_env()` → the settings store → `Shell::start` (the single-instance lock first: a
//! second launch hands its arguments over and exits) → GPUI, headless when the request is
//! "nothing" (the helper's own launch at sign-in: tray only) → inside it: `qust_ui::init`
//! (GPUI Kit, the elements, grayscale text, reduced motion), the AppUserModelID, the bridge (the
//! player starts), the theme, the store watcher, the shortcut and updater listeners, the shell's
//! events, then the start-up request (usually: open the player window and attach).
//!
//! # What owns what
//!
//! [`app::Inner`] (a GPUI global) owns the shell, the bridge, the updater service and the open
//! windows. The player window's content is composed by [`compose::player_window`]: the
//! `PlayerView` is the `WindowRoot`'s content, the drawer is mounted as the root's live layer in
//! the view's slot, and one [`compose::WindowHost`] is both views' host. The panels are opened by
//! [`panels::open`] with [`panels::AppServices`].
//!
//!   qust-app [--null-output] [--settings <file>] [--background] [--toggle]
//!            [--play <file> | --play-tone] [--measure <s>] [--hidden | --offscreen]
//!            [--keep-running] [--scenario <name>] [--report <file>]
//!
//! `--null-output` plays silently and `--settings` uses another settings file (tests never touch
//! the real one). The rest is for tests and measurements: see [`config::Config`].

pub mod app;
pub mod compose;
pub mod config;
pub mod panels;
#[doc(hidden)]
pub mod test_stream;
mod win;

pub use app::{carry_out, qust, run, Inner};
pub use config::{Config, Hooks, WindowMode};

/// The release version (package.json), as the updater and Settings know it.
pub const VERSION: &str = env!("QUST_RELEASE_VERSION");
