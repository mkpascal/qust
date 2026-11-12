//! How a run of the app is set up: from the command line in the executable, by hand in tests.

use gpui::App;
use qust_shell::background::Launch;
use qust_shell::menu::Menu;
use std::path::PathBuf;
use std::rc::Rc;

/// How windows appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    /// As the user sees them.
    Normal,
    /// Never shown, never focused (tests draw them by hand and read their pixels).
    Hidden,
    /// Shown far off every screen, never activated, no taskbar button: they render as a real
    /// window does, nobody sees them (measurements).
    Offscreen,
}

/// What a test answers instead of the OS (no native menu, picker or clipboard is ever opened
/// when these are set).
#[derive(Clone, Default)]
pub struct Hooks {
    /// Answers a native menu (the chosen item's id, None to dismiss).
    pub menu: Option<Rc<dyn Fn(&Menu) -> Option<u32>>>,
    /// Answers the Add file picker.
    pub pick: Option<Rc<dyn Fn() -> Vec<PathBuf>>>,
    /// Takes what Copy would put on the clipboard.
    pub clipboard: Option<Rc<dyn Fn(String)>>,
}

pub struct Config {
    /// Another settings file instead of the real one.
    pub settings: Option<PathBuf>,
    /// Play on the null output (no sound).
    pub null_output: bool,
    pub launch: Launch,
    pub windows: WindowMode,
    /// Where `WindowMode::Offscreen` puts the player window (physical pixels, outer top-left;
    /// panels go 1000 px below). Far off every screen by default. Placed so that a sliver of it
    /// is on a monitor (`--offscreen-at x,y`), it renders at the display's refresh rate: GPUI
    /// paces animation frames by the compositor, which doesn't compose a window off every screen,
    /// and then draws them as fast as it can.
    pub offscreen_at: (i32, i32),
    /// Behave as if the background helper were on when the last window closes (go headless and
    /// keep running), whatever the settings say; for tests and measurements of the tray-only
    /// mode in builds that aren't installed. The tray icon itself is not shown for it.
    pub keep_running: bool,
    pub hooks: Hooks,
    /// Add this file to the playlist and play it once attached.
    pub play: Option<PathBuf>,
    /// The same with a generated tone.
    pub play_tone: bool,
    /// Print CPU, memory and render counts after this many seconds, then quit.
    pub measure: Option<f64>,
    /// A measurement scenario run after start-up (see `app::scenario`).
    pub scenario: Option<String>,
    /// Where measurements and the start-up time are written (one line each).
    pub report: Option<PathBuf>,
    /// Called once everything is up (tests drive the app from here).
    pub on_ready: Option<Box<dyn FnOnce(&mut App)>>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            settings: None,
            null_output: false,
            launch: Launch::default(),
            windows: WindowMode::Normal,
            offscreen_at: (-20000, -20000),
            keep_running: false,
            hooks: Hooks::default(),
            play: None,
            play_tone: false,
            measure: None,
            scenario: None,
            report: None,
            on_ready: None,
        }
    }
}

impl Config {
    /// From the command line (`std::env::args()` without the program).
    pub fn from_args(args: &[String]) -> Config {
        let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let has = |flag: &str| args.iter().any(|a| a == flag);
        Config {
            settings: value("--settings").map(PathBuf::from),
            null_output: has("--null-output"),
            launch: Launch::parse(args),
            windows: if has("--hidden") {
                WindowMode::Hidden
            } else if has("--offscreen") {
                WindowMode::Offscreen
            } else {
                WindowMode::Normal
            },
            offscreen_at: value("--offscreen-at")
                .and_then(|at| {
                    let (x, y) = at.split_once(',')?;
                    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                })
                .unwrap_or((-20000, -20000)),
            keep_running: has("--keep-running"),
            hooks: Hooks::default(),
            play: value("--play").map(PathBuf::from),
            play_tone: has("--play-tone"),
            measure: value("--measure").and_then(|s| s.parse().ok()),
            scenario: value("--scenario"),
            report: value("--report").map(PathBuf::from),
            on_ready: None,
        }
    }
}
