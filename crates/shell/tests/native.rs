// The shell thread against the real OS, isolated from the user's Qust: throwaway shortcuts (never
// Ctrl+Alt+Insert or Ctrl+Alt+H, which the installed Qust owns), the tray icon for a moment, the
// single-instance channel between two processes under a test name, and the startup entry under a
// test value name (the real "Qust" value is checked unchanged before and after).
#![cfg(windows)]

use qust_shell::accelerator::Accelerator;
use qust_shell::hotkeys::{HotkeyBackend, HotkeyError, Hotkeys};
use qust_shell::instance::{self, Instance};
use qust_shell::shortcuts::{Action, Shortcuts};
use qust_shell::startup::StartupEntry;
use qust_shell::tray::{self, Tray, TrayCommand};
use qust_shell::{Event, NativeThread};
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SendMessageW, WM_APP, WM_DWMCOLORIZATIONCOLORCHANGED, WM_HOTKEY, WM_LBUTTONUP, WM_SETTINGCHANGE};

fn post(window: isize, msg: u32, wparam: usize, lparam: isize) {
    unsafe {
        PostMessageW(Some(HWND(window as *mut _)), msg, WPARAM(wparam), LPARAM(lparam)).expect("post");
    }
}

fn events(native: &NativeThread) -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    native.on_event(move |event| {
        let _ = tx.lock().unwrap().send(event);
    });
    rx
}

// Unusual enough that nothing else on this PC uses them.
const THROWAWAY: &str = "Ctrl+Alt+Shift+F23";
const THROWAWAY_2: &str = "Ctrl+Alt+Shift+F22";
// Tests run in parallel: each its own keys.
const THROWAWAY_3: &str = "Ctrl+Alt+Shift+F21";

#[test]
fn hotkey_registers_and_delivers() {
    let native = NativeThread::start().expect("shell thread");
    let rx = events(&native);
    let hotkeys = Hotkeys::new(&native);
    let accelerator = Accelerator::parse(THROWAWAY_3).unwrap();
    hotkeys.register(7, &accelerator).expect("register a throwaway shortcut");
    // The same keys again (another id) are taken: what another app's shortcut looks like.
    assert_eq!(hotkeys.register(8, &accelerator), Err(HotkeyError::AlreadyRegistered));
    // The press arrives as WM_HOTKEY on the shell window; posted here, not typed.
    post(native.hwnd(), WM_HOTKEY, 7, 0);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Event::Hotkey(7)));
    hotkeys.unregister(7);
    // Free again once unregistered.
    hotkeys.register(8, &accelerator).expect("free after unregister");
    hotkeys.unregister(8);
    native.stop();
}

#[test]
fn shortcuts_with_the_real_backend() {
    let native = NativeThread::start().expect("shell thread");
    let path = std::env::temp_dir().join(format!("qust-shell-native-shortcuts-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = qust_player::Store::open(path.clone());
    // Never the defaults: the installed Qust owns them.
    store.set("shortcut-play", serde_json::json!(THROWAWAY));
    store.set("shortcut-stop", serde_json::json!(""));
    store.set("shortcut-toggle", serde_json::json!(""));
    let shortcuts = Shortcuts::new(std::sync::Arc::new(Hotkeys::new(&native)), store);
    shortcuts.register_saved();
    assert!(shortcuts.binding(Action::Play).unwrap().registered);
    shortcuts.set_paused(true);
    let set = shortcuts.set_shortcut("toggle", THROWAWAY_2).expect("set a throwaway shortcut");
    assert_eq!(set[2].accelerator, THROWAWAY_2);
    let resumed = shortcuts.set_paused(false);
    assert!(resumed[0].registered && resumed[2].registered);
    // Now held by us: probing it for another action is refused before the OS is asked.
    shortcuts.set_paused(true);
    assert_eq!(shortcuts.set_shortcut("stop", THROWAWAY), Err("Already used for another action".into()));
    shortcuts.set_paused(false);
    shortcuts.unregister_all();
    native.stop();
    let _ = std::fs::remove_file(path);
}

#[test]
fn tray_icon_appears_and_goes() {
    let native = NativeThread::start().expect("shell thread");
    let rx = events(&native);
    let tray = Tray::new(&native);
    assert!(tray.show(), "Shell_NotifyIcon(NIM_ADD)");
    assert!(tray.is_shown());
    assert!(tray::icon_rect(&native).is_some(), "the shell knows the icon");
    // A left click (on release), as the shell reports it: the callback message.
    post(native.hwnd(), WM_APP + 2, 0, WM_LBUTTONUP as isize);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Event::TrayClick));
    tray.hide();
    assert!(!tray.is_shown());
    assert!(tray::icon_rect(&native).is_none(), "removed");
    let _ = TrayCommand::Open;
    native.stop();
}

// The second process: started by `single_instance_hands_over` with the test id in the environment.
#[test]
fn second_instance_child() {
    let Ok(id) = std::env::var("QUST_SHELL_INSTANCE_ID") else { return };
    let args = vec!["qust.exe".to_string(), "--toggle".to_string(), "with space".to_string()];
    match instance::acquire_with(&id, &args) {
        Instance::Secondary { delivered: true } => println!("CHILD: delivered"),
        Instance::Secondary { delivered: false } => println!("CHILD: not delivered"),
        Instance::Primary(_) => println!("CHILD: primary"),
    }
}

fn run_child(id: &str) -> String {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["second_instance_child", "--exact", "--nocapture", "--test-threads=1"])
        .env("QUST_SHELL_INSTANCE_ID", id)
        .output()
        .expect("child process");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn single_instance_hands_over() {
    let id = format!("qust-shell-test-{}", std::process::id());
    let Instance::Primary(mut lock) = instance::acquire_with(&id, &[]) else { panic!("first instance") };
    let native = NativeThread::start().expect("shell thread");
    let rx = events(&native);
    assert!(lock.listen(&native));

    let child = run_child(&id);
    assert!(child.contains("CHILD: delivered"), "{child}");
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Event::Launched { args, cwd }) => {
            assert_eq!(args, ["qust.exe", "--toggle", "with space"]);
            assert!(!cwd.is_empty());
            assert_eq!(qust_shell::background::second_launch(&args, false), qust_shell::background::Request::ShowPlayer { play: true });
        }
        other => panic!("{other:?}"),
    }

    // Released (as before the updater's installer runs): the next launch is a first instance.
    lock.release();
    let child = run_child(&id);
    assert!(child.contains("CHILD: primary"), "{child}");
    native.stop();
}

#[test]
fn startup_entry_round_trip() {
    let real = StartupEntry::new(qust_shell::startup::NAME, "", &[]);
    let real_before = (real.current_command(), real.current_approval());

    let name = format!("QustShellTest{}", std::process::id());
    let entry = StartupEntry::new(&name, r"C:\Qust Test\qust.exe", &["--background"]);
    assert_eq!(entry.is_enabled(), Ok(false));
    entry.enable().expect("enable");
    assert_eq!(entry.current_command().as_deref(), Some(r"C:\Qust Test\qust.exe --background"));
    assert_eq!(entry.is_enabled(), Ok(true));
    if let Some(approval) = entry.current_approval() {
        assert_eq!(approval, [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    entry.disable().expect("disable");
    assert_eq!(entry.current_command(), None);
    assert_eq!(entry.is_enabled(), Ok(false));
    entry.forget_approval();
    assert_eq!(entry.current_approval(), None);

    assert_eq!((real.current_command(), real.current_approval()), real_before, "the real Qust entry is untouched");
}

#[test]
fn system_appearance_changes_are_reported() {
    let native = NativeThread::start().expect("shell thread");
    let rx = events(&native);
    let window = HWND(native.hwnd() as *mut _);
    // What Windows broadcasts when the theme or accent changes, sent to our window only.
    let area = windows::core::w!("ImmersiveColorSet");
    unsafe {
        SendMessageW(window, WM_SETTINGCHANGE, Some(WPARAM(0)), Some(LPARAM(area.as_ptr() as isize)));
    }
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Event::AppearanceChanged));
    // Other areas are not.
    let other = windows::core::w!("Environment");
    unsafe {
        SendMessageW(window, WM_SETTINGCHANGE, Some(WPARAM(0)), Some(LPARAM(other.as_ptr() as isize)));
    }
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    post(native.hwnd(), WM_DWMCOLORIZATIONCOLORCHANGED, 0, 0);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Event::AppearanceChanged));
    native.stop();
}
