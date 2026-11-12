// The background helper: with "background" on, Qust starts with Windows in the tray with no window
// (--background), keeps running in the tray when the player is closed, and opens the player when a
// Play or Play / Stop / Start shortcut is pressed. What background.rs and main.rs decided in the
// Tauri app, as plain functions over the store, the arguments and whether the player is open, so
// the app only carries out the `Request`s.

use crate::shortcuts::Action;
use crate::startup::StartupEntry;
use crate::tray::{Tray, TrayCommand};
use crate::Event;
use qust_player::Store;
use serde_json::{json, Value};

pub const LAUNCH_ARG: &str = "--background";
pub const TOGGLE_ARG: &str = "--toggle";

// Only the installed version can be the helper. Test and portable builds share the installed
// copy's settings, so they ignore the setting rather than act on (or rewrite) its startup entry.
pub fn available() -> bool {
    qust_updater::installed()
}

pub fn enabled(store: &Store) -> bool {
    available() && store.get("background") == Some(Value::Bool(true))
}

// The flags Qust is launched with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Launch {
    pub background: bool,
    pub toggle: bool,
}

impl Launch {
    // Any argument equal to the flag, as `launched_with` checked.
    pub fn parse<S: AsRef<str>>(args: &[S]) -> Launch {
        Launch { background: args.iter().any(|arg| arg.as_ref() == LAUNCH_ARG), toggle: args.iter().any(|arg| arg.as_ref() == TOGGLE_ARG) }
    }

    pub fn from_env() -> Launch {
        Launch::parse(&std::env::args().collect::<Vec<_>>())
    }
}

// What the app does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    // Show the player (unminimize, show, focus), opening it when closed; with `play`, also start
    // the last cartridge (once the player is up: `Player::request_play(true)` before opening).
    ShowPlayer { play: bool },
    // Pass an action to the player (`Player::action`), which is open.
    PlayerAction(Action),
    // Leave everything: "Quit Qust".
    Quit,
    Nothing,
}

// At startup: the tray when the helper is on; then the player, unless this is the helper's own
// launch at sign-in. `--toggle` (a Stream Deck "Open" action) opens and plays.
pub fn at_startup(launch: Launch, helper_enabled: bool) -> (bool, Request) {
    let tray = helper_enabled;
    let request = if launch.toggle {
        Request::ShowPlayer { play: true }
    } else if helper_enabled && launch.background {
        Request::Nothing
    } else {
        Request::ShowPlayer { play: false }
    };
    (tray, request)
}

// A shortcut or tray action. With the player closed, Play and Play / Stop / Start open it and
// play; Stop has nothing to do.
pub fn player_action(action: Action, player_open: bool) -> Request {
    if player_open {
        Request::PlayerAction(action)
    } else if action == Action::Play || action == Action::Toggle {
        Request::ShowPlayer { play: true }
    } else {
        Request::Nothing
    }
}

// A second launch's arguments: `--toggle` acts like Play / Stop / Start, a plain launch shows the
// player, and a second `--background` (sign-in while running) does nothing.
pub fn second_launch<S: AsRef<str>>(args: &[S], player_open: bool) -> Request {
    let launch = Launch::parse(args);
    if launch.toggle {
        player_action(Action::Toggle, player_open)
    } else if !launch.background {
        Request::ShowPlayer { play: false }
    } else {
        Request::Nothing
    }
}

// Everything the shell thread reports, as requests. `action_for` maps a hotkey id to its action
// (`Shortcuts::action_for`).
pub fn route(event: &Event, player_open: bool) -> Request {
    match event {
        Event::Hotkey(id) => match Action::from_hotkey_id(*id) {
            Some(action) => player_action(action, player_open),
            None => Request::Nothing,
        },
        Event::TrayClick => Request::ShowPlayer { play: false },
        Event::TrayMenu(TrayCommand::Open) => Request::ShowPlayer { play: false },
        Event::TrayMenu(TrayCommand::Toggle) => player_action(Action::Toggle, player_open),
        Event::TrayMenu(TrayCommand::Quit) => Request::Quit,
        Event::Launched { args, .. } => second_launch(args, player_open),
        // Not a request: the app re-reads the theme and accent.
        Event::AppearanceChanged => Request::Nothing,
    }
}

// When the player window has closed: whether Qust keeps running (in the tray). Otherwise the app
// quits, as it did when its last window closed.
pub fn keep_running_without_windows(store: &Store) -> bool {
    enabled(store)
}

// What Settings shows: get_background.
pub fn describe(store: &Store, entry: &StartupEntry) -> Value {
    json!({
        "available": available(),
        "enabled": enabled(store),
        "startsWithWindows": available() && entry.is_enabled().unwrap_or(false),
    })
}

// Settings turns the helper on or off: set_background.
pub fn set_background(store: &Store, entry: &StartupEntry, tray: &Tray, on: bool) -> Result<Value, String> {
    if !available() {
        return Err("The background helper works in the installed version of Qust".into());
    }
    let result = if on { entry.enable() } else { entry.disable() };
    result.map_err(|error| format!("Couldn't change the Windows startup entry: {error}"))?;
    store.set("background", Value::Bool(on));
    if on {
        tray.show();
    } else {
        tray.hide();
    }
    Ok(describe(store, entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_flags() {
        assert_eq!(Launch::parse(&["qust.exe"]), Launch::default());
        assert_eq!(Launch::parse(&["qust.exe", "--background"]), Launch { background: true, toggle: false });
        assert_eq!(Launch::parse(&["qust.exe", "x", "--toggle"]), Launch { background: false, toggle: true });
        assert_eq!(Launch::parse(&["qust.exe", "--Toggle"]), Launch::default());
    }

    #[test]
    fn startup_like_main_rs() {
        let plain = Launch::default();
        let background = Launch { background: true, toggle: false };
        let toggle = Launch { background: false, toggle: true };
        assert_eq!(at_startup(plain, false), (false, Request::ShowPlayer { play: false }));
        assert_eq!(at_startup(plain, true), (true, Request::ShowPlayer { play: false }));
        // Helper's own launch: tray only.
        assert_eq!(at_startup(background, true), (true, Request::Nothing));
        // --background with the helper off (or not installed) opens the player.
        assert_eq!(at_startup(background, false), (false, Request::ShowPlayer { play: false }));
        assert_eq!(at_startup(toggle, false), (false, Request::ShowPlayer { play: true }));
        assert_eq!(at_startup(toggle, true), (true, Request::ShowPlayer { play: true }));
    }

    #[test]
    fn actions_and_second_launches() {
        assert_eq!(player_action(Action::Stop, true), Request::PlayerAction(Action::Stop));
        assert_eq!(player_action(Action::Stop, false), Request::Nothing);
        assert_eq!(player_action(Action::Play, false), Request::ShowPlayer { play: true });
        assert_eq!(player_action(Action::Toggle, false), Request::ShowPlayer { play: true });
        assert_eq!(player_action(Action::Toggle, true), Request::PlayerAction(Action::Toggle));
        assert_eq!(second_launch(&["qust.exe"], true), Request::ShowPlayer { play: false });
        assert_eq!(second_launch(&["qust.exe"], false), Request::ShowPlayer { play: false });
        assert_eq!(second_launch(&["qust.exe", "--toggle"], false), Request::ShowPlayer { play: true });
        assert_eq!(second_launch(&["qust.exe", "--toggle"], true), Request::PlayerAction(Action::Toggle));
        assert_eq!(second_launch(&["qust.exe", "--background"], true), Request::Nothing);
    }

    #[test]
    fn routing() {
        assert_eq!(route(&Event::Hotkey(1), false), Request::ShowPlayer { play: true });
        assert_eq!(route(&Event::Hotkey(2), true), Request::PlayerAction(Action::Stop));
        assert_eq!(route(&Event::Hotkey(99), true), Request::Nothing);
        assert_eq!(route(&Event::TrayClick, true), Request::ShowPlayer { play: false });
        assert_eq!(route(&Event::TrayMenu(TrayCommand::Open), false), Request::ShowPlayer { play: false });
        assert_eq!(route(&Event::TrayMenu(TrayCommand::Toggle), true), Request::PlayerAction(Action::Toggle));
        assert_eq!(route(&Event::TrayMenu(TrayCommand::Quit), true), Request::Quit);
        let launched = Event::Launched { args: vec!["qust.exe".into(), "--toggle".into()], cwd: String::new() };
        assert_eq!(route(&launched, true), Request::PlayerAction(Action::Toggle));
    }

    #[test]
    fn not_installed_means_no_helper() {
        // Debug test builds are never "installed".
        let path = std::env::temp_dir().join(format!("qust-shell-background-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(path);
        store.set("background", Value::Bool(true));
        assert!(!available());
        assert!(!enabled(&store));
        let entry = StartupEntry::new("QustShellNeverWritten", "x", &[]);
        assert_eq!(set_background(&store, &entry, &Tray::new(&crate::NativeThread::start().unwrap()), true), Err("The background helper works in the installed version of Qust".into()));
        assert_eq!(describe(&store, &entry), json!({ "available": false, "enabled": false, "startsWithWindows": false }));
    }
}
