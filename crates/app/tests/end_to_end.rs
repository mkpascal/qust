// The whole app as `qust` runs it (`qust_app::run`), on the null output (no sound), with a
// temporary settings file, a live WAV stream served on 127.0.0.1 and generated files, and its
// windows hidden (never shown or focused, drawn here by hand). Through dispatched input and
// messages posted to its own windows only:
//
//   launch and attach; play a cartridge (its ICY title shows); open and close the drawer (the
//   window's height animates and `playlist-open` is saved); drop files on the drawer; open
//   Settings and Stream information (owned by the player, centered over it, not minimizable)
//   and close them with Esc; change the analyzer colors in Settings and see the player follow;
//   record a throwaway shortcut through the services; a shortcut press reaches the player; close
//   the player with the helper on (GPUI goes headless, the shell keeps running); a shortcut
//   brings the window back and plays; again with a second launch (`qust --toggle`); quit.
//
// Never touches the user's Qust: the debug build's instance id is its own, the shortcuts are
// throwaway ones (Ctrl+Alt+Shift+F18 / F19 / F20), the tray icon isn't shown (the helper is
// simulated with `keep_running`), menus and the picker are answered here, and the registry's
// "Qust" startup value and the real settings file are checked unchanged.
//
//   cargo test -p qust-app --test end_to_end

use gpui::{point, px, App, AppContext, AsyncApp, FileDropEvent, Modifiers, MouseButton, PlatformInput, Point};
use qust_app::{qust, Config, Hooks, WindowMode};
use qust_shell::background::Request;
use qust_shell::menu::{Item, Menu};
use qust_ui::capture::Session;
use qust_ui::fixture::{element_box, layout};
use qust_ui::Playback;
use qust_ui_panels::SettingsView;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

const THROWAWAY_PLAY: &str = "Ctrl+Alt+Shift+F19";
const THROWAWAY_STOP: &str = "Ctrl+Alt+Shift+F18";
const RECORDED: &str = "Ctrl+Alt+Shift+F20";

#[derive(Default)]
struct Outcome {
    ran: bool,
    lines: Vec<String>,
    failures: Vec<String>,
}

impl Outcome {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Debug) {
        let line = format!("{} {name}: {detail:?}", if ok { "PASS" } else { "FAIL" });
        println!("{line}");
        self.lines.push(line);
        if !ok {
            self.failures.push(name.to_string());
        }
    }
}

#[cfg(windows)]
mod win32 {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn post(window: isize, msg: u32, wparam: usize, lparam: isize) {
        unsafe {
            let _ = PostMessageW(Some(HWND(window as *mut _)), msg, WPARAM(wparam), LPARAM(lparam));
        }
    }

    pub fn escape(window: isize) {
        post(window, WM_KEYDOWN, 0x1B, 1 | (0x01 << 16));
    }

    pub fn hotkey(shell: isize, id: usize) {
        post(shell, WM_HOTKEY, id, 0);
    }

    pub fn close(window: isize) {
        post(window, WM_CLOSE, 0, 0);
    }

    pub fn style(window: isize) -> isize {
        unsafe { GetWindowLongPtrW(HWND(window as *mut _), GWL_STYLE) }
    }

    pub fn minimize_box(window: isize) -> bool {
        style(window) & WS_MINIMIZEBOX.0 as isize != 0
    }

    pub fn visible(window: isize) -> bool {
        unsafe { IsWindowVisible(HWND(window as *mut _)).as_bool() }
    }
}

fn draw_all(cx: &mut App) {
    for handle in cx.windows() {
        let _ = cx.update_window(handle, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }
}

async fn until(cx: &mut AsyncApp, seconds: f64, mut done: impl FnMut(&mut App) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        let ok = cx.update(|cx| {
            draw_all(cx);
            done(cx)
        });
        if ok {
            return true;
        }
        cx.background_executor().timer(Duration::from_millis(16)).await;
    }
    false
}

async fn pause(cx: &mut AsyncApp, ms: u64) {
    until(cx, ms as f64 / 1000.0, |_| false).await;
}

fn player_session(cx: &App) -> Option<Session> {
    let inner = qust(cx);
    let player = inner.player_window()?;
    Some(Session { handle: player.handle, models: inner.bridge.models.clone() })
}

fn client_height(cx: &App) -> i32 {
    qust(cx).player_hwnd().and_then(qust_shell::window::client_size).map_or(0, |(_, h)| h)
}

fn state(cx: &App) -> Playback {
    qust(cx).bridge.models.clock.read(cx).state
}

fn logged(cx: &App, what: &str) -> usize {
    qust(cx).log.borrow().iter().filter(|line| *line == what).count()
}

// The menu item with this text.
fn item(menu: &Menu, text: &str) -> Option<u32> {
    menu.items.iter().find_map(|item| match item {
        Item::Action { id, text: t, .. } if t == text => Some(*id),
        _ => None,
    })
}

fn real_settings() -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(qust_player::settings_path()?).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn main() {
    if !cfg!(windows) {
        println!("Windows only");
        return;
    }
    let dir = std::env::temp_dir().join(format!("qust-app-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("drop")).expect("temp dir");
    let a = dir.join("drop").join("a.wav");
    let b = dir.join("drop").join("b.wav");
    qust_ui::diag::write_tone_wav(&a, 3.0).expect("wav");
    qust_ui::diag::write_tone_wav(&b, 4.0).expect("wav");
    let port = qust_app::test_stream::serve("Test Radio", Some("Test title")).expect("stream server");
    let settings = dir.join("settings.json");
    let initial = json!({
        "player1-title": "Test Radio",
        "player1-url": format!("http://127.0.0.1:{port}/live"),
        "volume": 0,
        "theme": "dark",
        "visualizer-colors": "classic",
        "shortcut-play": THROWAWAY_PLAY,
        "shortcut-stop": THROWAWAY_STOP,
        "shortcut-toggle": "",
    });
    std::fs::write(&settings, serde_json::to_string_pretty(&initial).unwrap_or_default()).expect("settings");
    // Untouched by this test, checked afterwards.
    let startup = qust_shell::startup::StartupEntry::new(qust_shell::startup::NAME, "", &[]);
    let registry_before = (startup.current_command(), startup.current_approval());
    let real_before = real_settings();

    let outcome = Rc::new(RefCell::new(Outcome::default()));
    let menus: Rc<RefCell<Vec<Menu>>> = Rc::default();
    // The text of the item the next menu answers with (None: dismissed).
    let answer: Rc<RefCell<Option<&'static str>>> = Rc::default();
    let hooks = {
        let (menus, answer) = (menus.clone(), answer.clone());
        Hooks {
            menu: Some(Rc::new(move |menu: &Menu| {
                menus.borrow_mut().push(menu.clone());
                answer.borrow_mut().take().and_then(|text| item(menu, text))
            })),
            pick: Some(Rc::new(|| Vec::new())),
            clipboard: Some(Rc::new(|_| {})),
        }
    };
    let result = outcome.clone();
    let child_settings = dir.join("child-settings.json");
    let _ = std::fs::write(&child_settings, serde_json::to_string(&json!({ "shortcut-play": "", "shortcut-stop": "", "shortcut-toggle": "" })).unwrap_or_default());
    let paths = (a.clone(), b.clone());
    let store_file = settings.clone();
    let config = Config {
        settings: Some(settings.clone()),
        null_output: true,
        windows: WindowMode::Hidden,
        keep_running: true,
        hooks,
        on_ready: Some(Box::new(move |cx: &mut App| {
            cx.spawn(async move |cx| {
                script(cx, &result, &menus, &answer, &paths, &child_settings, &store_file).await;
                cx.update(|cx| qust_app::carry_out(Request::Quit, cx));
            })
            .detach();
        })),
        ..Default::default()
    };
    let code = qust_app::run(config);

    let mut out = outcome.borrow_mut();
    let ran = out.ran;
    out.check("the app ran as the first instance and quit", code == 0 && ran, code);
    let stored: Value = std::fs::read_to_string(&settings).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
    out.check("saved on quit: the colors, the drawer, the recorded shortcut", stored["visualizer-colors"] == "white" && stored["playlist-open"] == true && stored["shortcut-toggle"] == RECORDED, (&stored["visualizer-colors"], &stored["playlist-open"], &stored["shortcut-toggle"]));
    out.check("the real Qust startup entry is unchanged", (startup.current_command(), startup.current_approval()) == registry_before, &registry_before.0);
    out.check("the real settings file is unchanged", real_settings() == real_before, real_before.map(|(len, _)| len));
    let failures = out.failures.clone();
    drop(out);
    let _ = std::fs::remove_dir_all(&dir);
    if !failures.is_empty() {
        println!("FAILED: {failures:?}");
        std::process::exit(1);
    }
    println!("end_to_end: OK");
}

#[cfg(windows)]
async fn script(cx: &mut AsyncApp, out: &Rc<RefCell<Outcome>>, menus: &Rc<RefCell<Vec<Menu>>>, answer: &Rc<RefCell<Option<&'static str>>>, paths: &(PathBuf, PathBuf), child_settings: &PathBuf, store_file: &PathBuf) {
    out.borrow_mut().ran = true;
    let reference = layout("player", "stopped").unwrap_or_default();
    let at = |name: &str| -> Point<f32> {
        let b = element_box(&reference, name, 1.0).unwrap_or([0.0; 4]);
        point(b[0] + b[2] / 2.0, b[1] + b[3] / 2.0)
    };
    let check = |name: &str, ok: bool, detail: String| out.borrow_mut().check(name, ok, detail);
    let click = |cx: &mut AsyncApp, p: Point<f32>| {
        cx.update(|cx| {
            if let Some(session) = player_session(cx) {
                session.click(p, cx);
            }
        })
    };
    let store = cx.update(|cx| qust(cx).store.clone());

    // --- Launch -----------------------------------------------------------------------------
    let attached = until(cx, 10.0, |cx| qust(cx).player_window().is_some() && qust(cx).bridge.models.snapshot.read(cx).attached).await;
    let sized = until(cx, 5.0, |cx| client_height(cx) == 267 && player_session(cx).is_some_and(|s| cx.update_window(s.any(), |_, w, _| w.viewport_size().height == px(267.0)).unwrap_or(false))).await;
    let hidden = cx.update(|cx| qust(cx).player_hwnd().is_some_and(|hwnd| !win32::visible(hwnd)));
    check("launch: the player window opens (hidden here) at 860 × 267 and attaches", attached && sized && hidden, format!("attached {attached}, sized {sized}, never shown {hidden}"));
    cx.update(|cx| {
        if let Some(session) = player_session(cx) {
            session.mouse_move(point(2.0, 264.0), None, Modifiers::default(), cx);
        }
    });

    // --- Play a cartridge -------------------------------------------------------------------
    click(cx, at("#cartridges>.cartridge[0]>.cartridge-body"));
    let playing = until(cx, 15.0, |cx| state(cx) == Playback::Playing).await;
    let titled = until(cx, 10.0, |cx| qust(cx).bridge.models.now.read(cx).title == "Test title").await;
    let frames = cx.update(|cx| qust(cx).bridge.models.analyzer.read(cx).received);
    check("a click on the cartridge plays it; its ICY title shows; analyzer frames arrive", playing && titled && frames > 10, format!("playing {playing}, title {titled}, frames {frames}"));

    // --- The drawer -------------------------------------------------------------------------
    click(cx, at("#playlistButton"));
    let opened = until(cx, 3.0, |cx| client_height(cx) == 587).await;
    let open_state = cx.update(|cx| qust(cx).player_window().map(|p| (p.parts.drawer.read(cx).is_open(), p.parts.view.read(cx).drawer_open())));
    let saved_open = until(cx, 2.0, |_| store.get("playlist-open") == Some(json!(true))).await;
    check("Playlist: the drawer opens, the window grows to 587, playlist-open saved", opened && open_state == Some((true, true)) && saved_open, format!("height {opened}, drawer/button {open_state:?}, saved {saved_open}"));
    click(cx, at("#playlistButton"));
    let closed = until(cx, 3.0, |cx| client_height(cx) == 267).await;
    let saved_closed = until(cx, 2.0, |_| store.get("playlist-open") == Some(json!(false))).await;
    check("Playlist again: back to 267, saved closed", closed && saved_closed, format!("height {closed}, saved {saved_closed}"));

    // --- Dropping files ---------------------------------------------------------------------
    click(cx, at("#playlistButton"));
    until(cx, 3.0, |cx| client_height(cx) == 587).await;
    let drop_at = point(px(430.0), px(400.0));
    let files = vec![paths.0.clone(), paths.1.clone()];
    cx.update(|cx| {
        if let Some(session) = player_session(cx) {
            let _ = cx.update_window(session.any(), |_, window, cx| {
                window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position: drop_at, paths: gpui::ExternalPaths(files.into_iter().collect()) }), cx);
                window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position: drop_at }), cx);
            });
        }
    });
    let added = until(cx, 5.0, |cx| qust(cx).bridge.models.playlist.read(cx).entries.len() == 2).await;
    let (titles, selected) = cx.update(|cx| {
        let inner = qust(cx);
        let titles: Vec<String> = inner.bridge.models.playlist.read(cx).entries.iter().map(|e| e.title.clone()).collect();
        let selected = inner.player_window().map(|p| p.parts.drawer.read(cx).selection().len()).unwrap_or(0);
        (titles, selected)
    });
    check("files dropped on the drawer become entries, selected", added && titles == ["a", "b"] && selected == 2, format!("{titles:?}, {selected} selected"));
    cx.update(|cx| {
        if let Some(session) = player_session(cx) {
            let _ = cx.update_window(session.any(), |_, window, cx| window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Exited), cx));
            session.mouse_move(point(2.0, 264.0), None, Modifiers::default(), cx);
        }
    });

    // --- Settings: owned, centered, Esc ----------------------------------------------------
    click(cx, at("#settingsButton"));
    let placed = until(cx, 3.0, |cx| {
        let inner = qust(cx);
        match (inner.panel(qust_app::compose::PanelKind::Settings), inner.player_hwnd()) {
            (Some(panel), Some(player)) => qust_shell::window::owner_of(panel.hwnd) == player,
            _ => false,
        }
    })
    .await;
    let geometry = cx.update(|cx| {
        let inner = qust(cx);
        let panel = inner.panel(qust_app::compose::PanelKind::Settings)?;
        let player = inner.player_hwnd()?;
        let expected = qust_shell::window::centered_over(qust_shell::window::outer_rect(player)?, (520.0, 850.0), qust_shell::window::work_area(player));
        let rect = qust_shell::window::outer_rect(panel.hwnd)?;
        let (w, h) = qust_shell::window::client_size(panel.hwnd)?;
        Some((rect.x - expected.0, rect.y - expected.1, w, h, win32::minimize_box(panel.hwnd), qust_shell::window::has_taskbar_style(panel.hwnd), win32::visible(panel.hwnd)))
    });
    let ok = geometry.is_some_and(|(dx, dy, w, h, minimizable, taskbar, visible)| dx.abs() <= 1.0 && dy.abs() <= 1.0 && w == 520 && h == 850 && !minimizable && !taskbar && !visible);
    check("the cog opens Settings: owned by the player, 520 × 850, centered over it, not minimizable, no taskbar button", placed && ok, format!("owned {placed}, (dx, dy, w, h, minimize box, taskbar, shown) {geometry:?}"));
    let settings_hwnd = cx.update(|cx| qust(cx).panel(qust_app::compose::PanelKind::Settings).map(|p| p.hwnd)).unwrap_or(0);
    click(cx, at("#settingsButton"));
    let single = cx.update(|cx| cx.windows().len());
    check("the cog again: no second Settings", single == 2, format!("{single} windows"));
    win32::escape(settings_hwnd);
    let closed = until(cx, 3.0, |cx| qust(cx).panel(qust_app::compose::PanelKind::Settings).is_none() && cx.windows().len() == 1).await;
    check("Esc closes Settings (the player stays)", closed, String::new());

    // --- Stream information from the player's right-click menu ------------------------------
    *answer.borrow_mut() = Some("Stream information");
    cx.update(|cx| {
        if let Some(session) = player_session(cx) {
            let p = point(10.0, 150.0);
            session.mouse_move(p, None, Modifiers::default(), cx);
            session.mouse_down(p, MouseButton::Right, 1, Modifiers::default(), cx);
            session.mouse_up(p, MouseButton::Right, 1, Modifiers::default(), cx);
        }
    });
    let info = until(cx, 3.0, |cx| {
        let inner = qust(cx);
        match (inner.panel(qust_app::compose::PanelKind::Info), inner.player_hwnd()) {
            (Some(panel), Some(player)) => qust_shell::window::owner_of(panel.hwnd) == player,
            _ => false,
        }
    })
    .await;
    let menu = menus.borrow().last().map(qust_ui_player::menus::rows);
    let info_geometry = cx.update(|cx| {
        let inner = qust(cx);
        let panel = inner.panel(qust_app::compose::PanelKind::Info)?;
        let player = inner.player_hwnd()?;
        let expected = qust_shell::window::centered_over(qust_shell::window::outer_rect(player)?, (580.0, 590.0), qust_shell::window::work_area(player));
        let rect = qust_shell::window::outer_rect(panel.hwnd)?;
        Some((rect.x - expected.0, rect.y - expected.1, qust_shell::window::client_size(panel.hwnd)?))
    });
    let centered = info_geometry.is_some_and(|(dx, dy, size)| dx.abs() <= 1.0 && dy.abs() <= 1.0 && size == (580, 590));
    check("right-click on the player: Pause / Stop / Stream information / Settings; Stream information opens owned and centered", info && centered, format!("menu {menu:?}, geometry {info_geometry:?}"));
    // The info window follows the player.
    let info_state = until(cx, 4.0, |cx| {
        let inner = qust(cx);
        let Some(panel) = inner.panel(qust_app::compose::PanelKind::Info) else { return false };
        let Ok(root) = panel.handle.read(cx) else { return false };
        root.content().clone().downcast::<qust_ui_panels::InfoView>().ok().is_some_and(|view| view.read(cx).texts().state == "Playing" && view.read(cx).texts().title == "Test title")
    })
    .await;
    check("Stream information shows the playing stream", info_state, String::new());
    let info_hwnd = cx.update(|cx| qust(cx).panel(qust_app::compose::PanelKind::Info).map(|p| p.hwnd)).unwrap_or(0);
    win32::escape(info_hwnd);
    let closed = until(cx, 3.0, |cx| qust(cx).panel(qust_app::compose::PanelKind::Info).is_none()).await;
    check("Esc closes Stream information", closed, String::new());

    // --- A setting changed in Settings reaches the player -----------------------------------
    click(cx, at("#settingsButton"));
    let view = {
        let mut found = None;
        until(cx, 3.0, |cx| {
            let panel = qust(cx).panel(qust_app::compose::PanelKind::Settings);
            found = panel.and_then(|panel| panel.handle.read(cx).ok().and_then(|root| root.content().clone().downcast::<SettingsView>().ok()).map(|view| (panel.handle, view)));
            found.as_ref().is_some_and(|(_, view)| view.read(cx).scroll_handle().bounds().size.height == px(850.0))
        })
        .await;
        found
    };
    let Some((settings_handle, settings_view)) = view else {
        check("Settings for the setting change", false, "no Settings window".into());
        return;
    };
    let settings = cx.update(|cx| Session { handle: settings_handle, models: qust(cx).bridge.models.clone() });
    *answer.borrow_mut() = Some("White");
    cx.update(|cx| settings.click(point(390.0, 136.0), cx));
    let followed = until(cx, 3.0, |cx| qust(cx).bridge.models.snapshot.read(cx).state.colors == "white").await;
    let saved = until(cx, 2.0, |_| store.get("visualizer-colors") == Some(json!("white"))).await;
    check("Visualizer colors → White in Settings: saved, and the player follows", followed && saved, format!("player {followed}, saved {saved}"));

    // --- Recording a shortcut through the services ------------------------------------------
    let _ = cx.update(|cx| settings_handle.update(cx, |_, window, cx| settings_view.update(cx, |view, cx| view.start_recording(2, window, cx))));
    cx.update(|cx| settings.key("ctrl-alt-shift-f20", cx));
    let recorded = until(cx, 3.0, |cx| {
        let shortcuts = &qust(cx).bridge.models.shortcuts.read(cx).shortcuts;
        shortcuts.toggle.accelerator == RECORDED && shortcuts.toggle.registered
    })
    .await;
    let bindings = cx.update(|cx| qust(cx).shortcuts.describe());
    check("recording Play / Stop / Start: Ctrl+Alt+Shift+F20 set, registered, shown", recorded && store.get("shortcut-toggle") == Some(json!(RECORDED)), bindings.to_string());
    win32::escape(settings_handle.update(cx, |_, window, _| qust_shell::gpui_glue::hwnd(window).unwrap_or(0)).unwrap_or(0));
    let closed = until(cx, 3.0, |cx| qust(cx).panel(qust_app::compose::PanelKind::Settings).is_none()).await;
    let resumed = cx.update(|cx| qust(cx).shortcuts.bindings().iter().filter(|b| b.assigned()).all(|b| b.registered));
    check("Esc closes Settings; the shortcuts are all registered", closed && resumed, String::new());

    // --- A global shortcut ------------------------------------------------------------------
    let shell = cx.update(|cx| qust(cx).shell_hwnd);
    win32::hotkey(shell, 2);
    let stopped = until(cx, 5.0, |cx| state(cx) == Playback::Stopped).await;
    check("the Stop shortcut stops", stopped, String::new());

    // --- Tray only: close the player with the helper on --------------------------------------
    let headless_before = cx.update(|cx| logged(cx, "headless"));
    let player_hwnd = cx.update(|cx| qust(cx).player_hwnd()).unwrap_or(0);
    win32::close(player_hwnd);
    let gone = until(cx, 5.0, |cx| qust(cx).player_window().is_none() && cx.windows().is_empty() && logged(cx, "headless") > headless_before).await;
    check("closing the player with the helper on: no window, GPUI headless, still running", gone, cx.update(|cx| qust(cx).log.borrow().join(", ")));
    pause(cx, 500).await;
    win32::hotkey(shell, 1);
    let back = until(cx, 10.0, |cx| qust(cx).player_window().is_some() && qust(cx).bridge.models.snapshot.read(cx).attached).await;
    let plays = until(cx, 15.0, |cx| state(cx) == Playback::Playing).await;
    let reopened = until(cx, 3.0, |cx| client_height(cx) == 587).await;
    let button = cx.update(|cx| qust(cx).player_window().map(|p| (p.parts.view.read(cx).drawer_open(), p.parts.drawer.read(cx).is_open())));
    check("the Play shortcut opens the player again (drawer as saved) and plays the last cartridge", back && plays && reopened && button == Some((true, true)), format!("back {back}, playing {plays}, 587 {reopened}, drawer {button:?}"));

    // --- Tray only again, then a second launch with --toggle --------------------------------
    let headless_before = cx.update(|cx| logged(cx, "headless"));
    let player_hwnd = cx.update(|cx| qust(cx).player_hwnd()).unwrap_or(0);
    win32::close(player_hwnd);
    let gone = until(cx, 5.0, |cx| qust(cx).player_window().is_none() && logged(cx, "headless") > headless_before).await;
    let exe = env!("CARGO_BIN_EXE_qust");
    let child = std::process::Command::new(exe).args(["--toggle", "--null-output", "--hidden", "--settings"]).arg(child_settings).spawn();
    let mut exited = None;
    if let Ok(mut child) = child {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if let Ok(Some(status)) = child.try_wait() {
                exited = Some(status.code());
                break;
            }
            pause(cx, 50).await;
        }
        if exited.is_none() {
            let _ = child.kill();
        }
    }
    let back = until(cx, 10.0, |cx| qust(cx).player_window().is_some()).await;
    let plays = until(cx, 15.0, |cx| state(cx) == Playback::Playing).await;
    let launched = cx.update(|cx| qust(cx).log.borrow().iter().any(|line| line.starts_with("event Launched") && line.contains("--toggle")));
    check("closed again, then `qust --toggle`: it hands over and exits; the player opens and plays", gone && exited == Some(Some(0)) && launched && back && plays, format!("headless {gone}, child exit {exited:?}, handed over {launched}, back {back}, playing {plays}"));
    let _ = store_file;
}

#[cfg(not(windows))]
async fn script(_: &mut AsyncApp, _: &Rc<RefCell<Outcome>>, _: &Rc<RefCell<Vec<Menu>>>, _: &Rc<RefCell<Option<&'static str>>>, _: &(PathBuf, PathBuf), _: &PathBuf, _: &PathBuf) {}
