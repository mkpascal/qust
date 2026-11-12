//! The app's life: start-up, the shell's requests, the player window, the last window closing,
//! quitting, updates. What main.rs and background.rs did in the Tauri app.

use crate::compose::{self, Composed, HostEnv, PanelKind};
use crate::config::{Config, Hooks, WindowMode};
use crate::panels::{self, AppServices, PanelWindow};
use crate::win;
use gpui::{px, size, App, Bounds, GraphicalEnvironment, QuitMode, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowId, WindowKind, WindowOptions, WindowingRequest};
use qust_player::{OutputKind, Store};
use qust_shell::background::{self, Request};
use qust_shell::gpui_glue::{self, MainSender};
use qust_shell::shortcuts::Shortcuts;
use qust_shell::theme::{self as shell_theme, Theme};
use qust_shell::updates::UpdateService;
use qust_shell::{Event, Shell, Started};
use qust_ui::{Bridge, WindowRoot};
use qust_ui_player::MenuDone;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// The player window and what it is made of.
#[derive(Clone)]
pub struct PlayerWindow {
    pub handle: WindowHandle<WindowRoot>,
    pub id: WindowId,
    pub hwnd: isize,
    pub parts: Composed,
}

/// Everything the app owns, a GPUI global (`qust(cx)`).
pub struct Inner {
    pub bridge: Bridge,
    pub store: Store,
    pub shell: RefCell<Shell>,
    pub shortcuts: Arc<Shortcuts>,
    /// The shell thread's window (tests post shortcut presses to it).
    pub shell_hwnd: isize,
    pub updates: Option<Arc<UpdateService>>,
    pub main: MainSender,
    pub mode: WindowMode,
    pub keep_running: bool,
    pub hooks: Hooks,
    pub report: Option<PathBuf>,
    /// Where an off-screen window goes (physical, outer top-left).
    pub offscreen_at: (i32, i32),
    pub player: RefCell<Option<PlayerWindow>>,
    pub settings: RefCell<Option<PanelWindow>>,
    pub info: RefCell<Option<PanelWindow>>,
    /// The player window is being opened (switching to windowed first).
    opening: Cell<bool>,
    quitting: Cell<bool>,
    /// What the app did, in order (tests read it): "headless", "windowed", "player opened"...
    pub log: RefCell<Vec<String>>,
    pub env: Rc<AppEnv>,
    pub services: Rc<AppServices>,
}

struct Global(Rc<Inner>);

impl gpui::Global for Global {}

/// The app's state.
pub fn qust(cx: &App) -> Rc<Inner> {
    cx.global::<Global>().0.clone()
}

impl Inner {
    pub fn player_window(&self) -> Option<PlayerWindow> {
        self.player.borrow().clone()
    }

    pub fn player_hwnd(&self) -> Option<isize> {
        self.player.borrow().as_ref().map(|player| player.hwnd)
    }

    pub fn panel(&self, kind: PanelKind) -> Option<PanelWindow> {
        match kind {
            PanelKind::Settings => self.settings.borrow().clone(),
            PanelKind::Info => self.info.borrow().clone(),
        }
    }

    pub fn note(&self, what: impl Into<String>) {
        self.log.borrow_mut().push(what.into());
    }

    /// One line in the report file (measurements), or stdout.
    pub fn report(&self, line: &str) {
        match &self.report {
            Some(path) => {
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(file, "{line}");
                }
            }
            None => println!("{line}"),
        }
    }

    /// Whether closing the last window leaves Qust running (the background helper).
    pub fn keeps_running(&self) -> bool {
        self.keep_running || self.shell.borrow().keep_running_without_windows()
    }
}

/// Starts Qust and returns when it quits (the process's exit code).
pub fn run(config: Config) -> i32 {
    // The real settings file is read without being rewritten until this is the running Qust: a
    // second launch only hands its arguments over.
    let store = match &config.settings {
        Some(path) => Store::open(path.clone()),
        None => match qust_player::settings_path() {
            Some(path) if path.exists() => Store::open(path),
            _ => match Store::load() {
                Ok(store) => store,
                Err(error) => {
                    eprintln!("qust: settings: {error}");
                    return 1;
                }
            },
        },
    };
    let (shell, request) = match Shell::start(store.clone(), config.launch) {
        Ok(Started::Primary(shell, request)) => (shell, request),
        Ok(Started::Secondary) => return 0,
        Err(error) => {
            eprintln!("qust: {error}");
            return 1;
        }
    };
    // Tray only (the helper's launch at sign-in): no DirectX, no vsync thread until a window
    // opens.
    let windowing = if request == Request::Nothing { WindowingRequest::Headless } else { WindowingRequest::Windowed(GraphicalEnvironment::detect()) };
    let app = qust_ui::application().with_quit_mode(QuitMode::Explicit).with_windowing(windowing);
    app.run(move |cx: &mut App| start(cx, config, store, shell, request));
    0
}

fn start(cx: &mut App, mut config: Config, store: Store, shell: Shell, request: Request) {
    qust_ui::init(cx);
    cx.set_app_identity(qust_shell::instance::IDENTIFIER, "Qust");
    if config.windows == WindowMode::Hidden {
        // Nothing may appear on screen: tooltips would.
        qust_fluent::tooltip::set_native(false, cx);
    }
    let output = if config.null_output { OutputKind::Null } else { OutputKind::Device };
    let bridge = Bridge::start(store.clone(), output, cx);
    let choice = qust_ui::theme::ThemeChoice::parse(&bridge.models.settings.read(cx).theme);
    qust_ui::theme::apply(choice, cx);

    // Settings written by the Settings window take effect at once (main.rs's store.watch).
    let watcher = bridge.player.clone();
    store.watch(move |key, value| {
        let (key, value) = (key.to_string(), value.cloned());
        let _ = watcher.ask(move |player| player.setting_changed(&key, value));
    });

    let main = MainSender::new(cx);
    let shortcuts = shell.shortcuts.clone();
    let shell_hwnd = shell.native.hwnd();
    // `settings-changed` { key: "shortcuts" }: after set_shortcut and after resuming.
    let sender = main.clone();
    shortcuts.on_change(move |bindings| {
        let value = qust_shell::shortcuts::describe(bindings);
        sender.post(move |cx| apply_shortcuts(value, cx));
    });
    let sender = main.clone();
    let updates = match UpdateService::new(store.clone(), crate::VERSION, move |value| {
        sender.post(move |cx| apply_update(value, cx));
    }) {
        Ok(service) => Some(service),
        Err(error) => {
            eprintln!("qust: updater: {error}");
            None
        }
    };

    let env = Rc::new(AppEnv);
    let services = Rc::new(AppServices);
    let inner = Rc::new(Inner {
        bridge: bridge.clone(),
        store,
        shell: RefCell::new(shell),
        shortcuts: shortcuts.clone(),
        shell_hwnd,
        updates: updates.clone(),
        main: main.clone(),
        mode: config.windows,
        keep_running: config.keep_running,
        hooks: config.hooks.clone(),
        report: config.report.clone(),
        offscreen_at: config.offscreen_at,
        player: RefCell::new(None),
        settings: RefCell::new(None),
        info: RefCell::new(None),
        opening: Cell::new(false),
        quitting: Cell::new(false),
        log: RefCell::new(Vec::new()),
        env,
        services,
    });
    cx.set_global(Global(inner.clone()));

    apply_shortcuts(shortcuts.describe(), cx);
    let background = inner.shell.borrow().background();
    apply_background(background, cx);
    if let Some(updates) = &updates {
        apply_update(updates.describe(), cx);
        updates.schedule();
    }

    // The shell thread's events: shortcuts, the tray, second launches, appearance changes.
    {
        let shell = inner.shell.borrow();
        gpui_glue::forward_events(&shell.native, main, |event, cx| handle_event(event, cx));
    }
    cx.on_window_closed(window_closed).detach();

    carry_out(request, cx);

    let file = if config.play_tone {
        let path = std::env::temp_dir().join("qust-app-tone.wav");
        qust_ui::diag::write_tone_wav(&path, 120.0).ok().map(|_| path)
    } else {
        config.play.take()
    };
    if let Some(file) = file {
        let player = bridge.player.clone();
        cx.spawn(async move |_| {
            if let Some(ids) = player.add_paths(vec![file], None).get().await {
                if let Some(id) = ids.last() {
                    player.select_entry(*id);
                }
            }
        })
        .detach();
    }
    if let Some(name) = config.scenario.take() {
        scenario(name, cx);
    }
    if let Some(seconds) = config.measure {
        measure(seconds, cx);
    }
    if let Some(ready) = config.on_ready.take() {
        ready(cx);
    }
}

fn parse<T: serde::de::DeserializeOwned + Default>(value: Value) -> T {
    serde_json::from_value(value).unwrap_or_default()
}

fn apply_shortcuts(value: Value, cx: &mut App) {
    let models = Bridge::global(cx).models.clone();
    models.apply_shortcuts(parse(value), cx);
}

fn apply_update(value: Value, cx: &mut App) {
    let models = Bridge::global(cx).models.clone();
    models.apply_update(parse(value), cx);
}

pub(crate) fn apply_background(value: Value, cx: &mut App) {
    let models = Bridge::global(cx).models.clone();
    models.apply_background(parse(value), cx);
}

// A shell event, on the main thread.
fn handle_event(event: Event, cx: &mut App) {
    let inner = qust(cx);
    if event == Event::AppearanceChanged {
        // The accent or the system's light / dark mode: the theme global, the menus and the
        // title bars.
        qust_ui::theme::refresh_accent(cx);
        let theme = Theme::from_store(&inner.store);
        outside(cx, move || shell_theme::set_app_theme(theme));
        return;
    }
    inner.note(format!("event {event:?}"));
    let open = inner.player.borrow().is_some();
    carry_out(background::route(&event, open), cx);
}

/// Does what the shell asks (also what the app asks itself at start-up).
pub fn carry_out(request: Request, cx: &mut App) {
    let inner = qust(cx);
    match request {
        Request::ShowPlayer { play } => show_player(play, cx),
        Request::PlayerAction(action) => inner.bridge.player.action(action.as_str()),
        Request::Quit => quit(cx),
        Request::Nothing => {}
    }
}

// Shows the player, opening it when closed. With `play`, it also starts the last cartridge
// (once attached, when it had to open).
fn show_player(play: bool, cx: &mut App) {
    let inner = qust(cx);
    if let Some(player) = inner.player_window() {
        if inner.mode == WindowMode::Normal {
            let hwnd = player.hwnd;
            outside(cx, move || qust_shell::window::bring_to_front(hwnd));
        }
        if play {
            inner.bridge.player.action("play");
        }
        return;
    }
    // It plays once the window has attached.
    inner.bridge.player.request_play(play);
    if inner.opening.replace(true) {
        return;
    }
    let task = gpui_glue::windowed(cx);
    cx.spawn(async move |cx| {
        let windowed = task.await;
        cx.update(|cx| {
            let inner = qust(cx);
            inner.opening.set(false);
            match windowed {
                Ok(()) => inner.note("windowed"),
                Err(error) => inner.note(format!("windowed failed: {error}")),
            }
            if inner.quitting.get() {
                return;
            }
            if let Err(error) = open_player(cx) {
                eprintln!("qust: could not open the player: {error}");
                inner.note(format!("player failed: {error}"));
            }
        });
    })
    .detach();
}

fn player_options(mode: WindowMode, height: f32, cx: &App) -> WindowOptions {
    let content = size(px(qust_ui_player::WIDTH), px(height));
    // Windows: created hidden at the system's default place and size (tao let Windows choose
    // where the window opens, CW_USEDEFAULT), then sized and shown. Elsewhere: centered.
    let show = cfg!(not(windows)) && mode == WindowMode::Normal;
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, content, cx))),
        titlebar: Some(TitlebarOptions { title: Some("Qust".into()), appears_transparent: false, traffic_light_position: None }),
        focus: mode == WindowMode::Normal,
        show,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: true,
        app_id: Some(qust_shell::instance::IDENTIFIER.into()),
        // The page drew the analyzer at 60 updates a second whether the window was active or
        // not; GPUI would halve an inactive window's frames.
        inactive_frame_interval: None,
        ..Default::default()
    }
}

/// Opens the player window (it must be closed) and attaches the player.
pub fn open_player(cx: &mut App) -> Result<(), String> {
    let inner = qust(cx);
    let height = qust_player::playlist::player_height(&inner.store) as f32;
    // The window opens as the store has it: the view's Playlist button reads the model.
    let open = inner.store.get("playlist-open") == Some(Value::Bool(true));
    set_playlist_model(open, cx);
    let models = inner.bridge.models.clone();
    let player = inner.bridge.player.clone();
    let env: Rc<dyn HostEnv> = inner.env.clone();
    let parts: Rc<RefCell<Option<Composed>>> = Rc::default();
    let out = parts.clone();
    let options = player_options(inner.mode, height, cx);
    let handle = cx
        .open_window(options, move |window, cx| {
            let composed = compose::player_window(&models, Rc::new(player.clone()), Rc::new(player), env, window, cx);
            let root = composed.root.clone();
            *out.borrow_mut() = Some(composed);
            root
        })
        .map_err(|error| error.to_string())?;
    let parts = parts.borrow_mut().take().ok_or("no window content")?;
    let mode = inner.mode;
    let report = inner.report.is_some();
    let hwnd = handle
        .update(cx, |_, window, cx| {
            for subscription in qust_ui::theme::follow(window, cx) {
                subscription.detach();
            }
            // Closing the player window (its close button, Alt+F4, the taskbar): it goes; what
            // that means for the app is decided once it is gone (`window_closed`).
            window.on_window_should_close(cx, |_, _| true);
            if report {
                window.on_next_frame(|_, cx| {
                    let inner = qust(cx);
                    inner.report(&format!("STARTUP first frame {:.0} ms after the process started", win::process_age_ms()));
                });
            }
            let hwnd = gpui_glue::hwnd(window).unwrap_or(0);
            if cfg!(windows) || mode != WindowMode::Normal {
                window.resize(size(px(qust_ui_player::WIDTH), px(height)));
            }
            hwnd
        })
        .map_err(|error| error.to_string())?;
    let id = handle.window_id();
    *inner.player.borrow_mut() = Some(PlayerWindow { handle, id, hwnd, parts });
    inner.note("player opened");
    // Placed and shown once GPUI's resize (queued on its executor) has landed, and outside GPUI's
    // handling: Win32 sends these windows' messages straight to GPUI, which then needs the app
    // (a call from inside an update would find it borrowed).
    let offscreen_at = inner.offscreen_at;
    cx.spawn(async move |cx| {
        if !cx.update(|cx| qust(cx).player_hwnd() == Some(hwnd)) {
            return;
        }
        qust_shell::window::follow_theme(hwnd);
        if cfg!(windows) {
            match mode {
                WindowMode::Normal => qust_shell::window::bring_to_front(hwnd),
                WindowMode::Offscreen => win::show_offscreen(hwnd, offscreen_at.0, offscreen_at.1),
                WindowMode::Hidden => {}
            }
        }
    })
    .detach();
    inner.bridge.player.attach();
    Ok(())
}

// The player model's `playlist_open` (the Playlist button of the next window built).
fn set_playlist_model(open: bool, cx: &mut App) {
    let models = Bridge::global(cx).models.clone();
    let mut settings = models.settings.read(cx).clone();
    if settings.playlist_open != open {
        settings.playlist_open = open;
        models.apply_settings(settings, cx);
    }
}

/// The Playlist button, as a click would do it (scenarios and tests).
pub fn set_drawer(open: bool, cx: &mut App) {
    let inner = qust(cx);
    let Some(player) = inner.player_window() else { return };
    let host = player.parts.host.clone();
    let view = player.parts.view.clone();
    let _ = player.handle.update(cx, |_, window, cx| {
        view.update(cx, |view, cx| view.set_drawer_open(open, cx));
        qust_ui_player::PlayerHost::set_drawer(&*host, open, window, cx);
    });
}

fn window_closed(cx: &mut App, id: WindowId) {
    let inner = qust(cx);
    let player = inner.player.borrow().as_ref().is_some_and(|player| player.id == id);
    if player {
        inner.player.borrow_mut().take();
        inner.note("player closed");
        // Playback doesn't outlive the player window (the pending saves are written).
        inner.bridge.player.detach();
    }
    if inner.settings.borrow().as_ref().is_some_and(|panel| panel.id == id) {
        inner.settings.borrow_mut().take();
        inner.note("settings closed");
        // Closing Settings mid-recording must not leave the shortcuts paused.
        inner.shortcuts.set_paused(false);
    }
    if inner.info.borrow().as_ref().is_some_and(|panel| panel.id == id) {
        inner.info.borrow_mut().take();
        inner.note("info closed");
    }
    if inner.player.borrow().is_none() && !inner.opening.get() && !inner.quitting.get() && cx.windows().is_empty() {
        last_window_closed(cx);
    }
}

// With the helper on, Qust stays in the tray (GPUI headless: no DirectX, no vsync thread);
// otherwise closing the player quits, as closing the last window did.
fn last_window_closed(cx: &mut App) {
    let inner = qust(cx);
    if !inner.keeps_running() {
        quit(cx);
        return;
    }
    let task = gpui_glue::headless(cx);
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(()) => qust(cx).note("headless"),
            Err(error) => qust(cx).note(format!("headless failed: {error}")),
        });
    })
    .detach();
}

/// "Quit Qust", or the last window closed without the helper: the pending saves, the shell's
/// shortcuts, tray icon and instance lock let go, then GPUI quits.
pub fn quit(cx: &mut App) {
    let inner = qust(cx);
    if inner.quitting.replace(true) {
        return;
    }
    inner.note("quit");
    inner.bridge.player.detach_and_wait(Duration::from_secs(2));
    inner.shell.borrow_mut().stop();
    cx.quit();
}

// The update was installed (the installer runs): the next Qust must be the first instance, then
// this one goes. On macOS it starts the new copy itself.
pub(crate) fn installed_update(cx: &mut App) {
    let inner = qust(cx);
    inner.quitting.set(true);
    inner.bridge.player.detach_and_wait(Duration::from_secs(2));
    {
        let mut shell = inner.shell.borrow_mut();
        shell.release_instance();
        #[cfg(target_os = "macos")]
        if let Err(error) = qust_shell::updates::relaunch() {
            eprintln!("qust: relaunch: {error}");
        }
        shell.stop();
    }
    cx.quit();
}

/// Runs `f` soon, outside GPUI's handling of anything: for Win32 calls that send messages to GPUI
/// windows synchronously (showing, activating, moving, restyling, the title bar theme). GPUI's
/// window procedure needs the app for some of them; called from inside an update or an event
/// handler, it would find the app borrowed and panic.
pub(crate) fn outside(cx: &mut App, f: impl FnOnce() + 'static) {
    cx.spawn(async move |_| f()).detach();
}

/// The app's side of the player window.
pub struct AppEnv;

impl HostEnv for AppEnv {
    fn open_panel(&self, panel: PanelKind, cx: &mut App) {
        panels::open(panel, cx);
    }

    fn drawer_moved(&self, open: bool, window: &mut Window, cx: &mut App) {
        let inner = qust(cx);
        // set_playlist_open: saved, then the window grows or shrinks over ~200 ms.
        inner.bridge.player.write_setting("playlist-open", Some(json!(open)));
        set_playlist_model(open, cx);
        let height = qust_ui_player::HEIGHT + if open { qust_ui_player::DRAWER_HEIGHT } else { 0.0 };
        #[cfg(windows)]
        if let Some(hwnd) = gpui_glue::hwnd(window) {
            qust_shell::drawer::animate(hwnd, height as f64, || {});
        }
        #[cfg(not(windows))]
        window.resize(size(px(qust_ui_player::WIDTH), px(height)));
    }

    fn show_menu(&self, menu: qust_shell::menu::Menu, window: &mut Window, done: MenuDone, cx: &mut App) {
        show_menu(menu, qust_shell::menu::Position::Cursor, window, done, cx);
    }

    fn pick_files(&self, window: &mut Window, done: Box<dyn FnOnce(Vec<PathBuf>, &mut App)>, cx: &mut App) {
        let inner = qust(cx);
        if let Some(pick) = inner.hooks.pick.clone() {
            cx.defer(move |cx| done(pick(), cx));
            return;
        }
        let (tx, rx) = futures::channel::oneshot::channel();
        cx.spawn(async move |cx| {
            if let Ok(paths) = rx.await {
                cx.update(|cx| done(paths, cx));
            }
        })
        .detach();
        #[cfg(windows)]
        {
            // Blocks until closed, on a thread of its own: the windows keep running.
            let owner = gpui_glue::hwnd(window);
            std::thread::spawn(move || {
                let _ = tx.send(qust_shell::picker::pick_files(owner));
            });
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            let prompt = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: true, multiple: true, prompt: None });
            cx.spawn(async move |_| {
                let paths = prompt.await.ok().and_then(Result::ok).flatten().unwrap_or_default();
                let _ = tx.send(paths);
            })
            .detach();
        }
    }
}

/// A native menu for `window` (`qust_shell::menu::popup`, which shows it once GPUI's event
/// handling is over), then `done` with the choice, on the main thread, in a task of its own.
pub(crate) fn show_menu(menu: qust_shell::menu::Menu, position: qust_shell::menu::Position, window: &Window, done: MenuDone, cx: &mut App) {
    let inner = qust(cx);
    if let Some(answer) = inner.hooks.menu.clone() {
        let choice = answer(&menu);
        cx.defer(move |cx| done(choice, cx));
        return;
    }
    let Some(owner) = gpui_glue::hwnd(window) else {
        cx.defer(move |cx| done(None, cx));
        return;
    };
    let (tx, rx) = futures::channel::oneshot::channel();
    cx.spawn(async move |cx| {
        if let Ok(choice) = rx.await {
            cx.update(|cx| done(choice, cx));
        }
    })
    .detach();
    qust_shell::menu::popup(owner, menu, position, move |choice| {
        let _ = tx.send(choice);
    });
}

// --- Measurements -----------------------------------------------------------------------------

// Steps run on a timer after start-up, so a measurement finds the app in a known state without
// any input: "drawer" (open it), "drawer-panels" (open and close the drawer, Settings, Stream
// information), "tray" (close the player window, as with the helper on).
fn scenario(name: String, cx: &mut App) {
    cx.spawn(async move |cx| {
        let wait = |seconds: f64| cx.background_executor().timer(Duration::from_secs_f64(seconds));
        wait(2.0).await;
        match name.as_str() {
            "drawer" => cx.update(|cx| set_drawer(true, cx)),
            "drawer-panels" => {
                cx.update(|cx| set_drawer(true, cx));
                wait(1.5).await;
                cx.update(|cx| set_drawer(false, cx));
                wait(1.5).await;
                cx.update(|cx| panels::open(PanelKind::Settings, cx));
                wait(1.5).await;
                cx.update(|cx| panels::close(PanelKind::Settings, cx));
                wait(1.0).await;
                cx.update(|cx| panels::open(PanelKind::Info, cx));
                wait(1.5).await;
                cx.update(|cx| panels::close(PanelKind::Info, cx));
            }
            "tray" => cx.update(|cx| {
                if let Some(player) = qust(cx).player_window() {
                    let _ = player.handle.update(cx, |_, window, _| window.remove_window());
                }
            }),
            other => cx.update(|cx| qust(cx).report(&format!("SCENARIO unknown {other}"))),
        }
        cx.update(|cx| qust(cx).report(&format!("SCENARIO {name} done after {:.0} ms", win::process_age_ms())));
    })
    .detach();
}

// CPU, memory and render counts over `seconds` (after a 3 s warm-up), then quit.
fn measure(seconds: f64, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let sample = |cx: &mut gpui::AsyncApp| {
            cx.update(|cx| {
                let inner = qust(cx);
                let frames = inner.bridge.models.analyzer.read(cx).received;
                let (root, view, badges) = match inner.player_window() {
                    Some(player) => {
                        let view = player.parts.view.read(cx);
                        let badges = view.live().map(|live| live.badges.iter().map(|badge| badge.read(cx).renders).sum::<u64>()).unwrap_or(0);
                        (player.parts.root.read(cx).renders, view.renders, badges)
                    }
                    None => (0, 0, 0),
                };
                (frames, root, view, badges, inner.bridge.player.frames_sent())
            })
        };
        let start = qust_ui::diag::counters();
        let t0 = std::time::Instant::now();
        let before = sample(cx);
        cx.background_executor().timer(Duration::from_secs_f64(seconds)).await;
        let end = qust_ui::diag::counters();
        let after = sample(cx);
        let secs = t0.elapsed().as_secs_f64();
        let per = |a: u64, b: u64| b.saturating_sub(a) as f64 / secs;
        cx.update(|cx| {
            let inner = qust(cx);
            inner.report(&format!(
                "MEASURE {secs:.1} s: CPU {:.2} % of one core; per second: analyzer frames sent {:.1}, received {:.1}, root renders {:.1}, player view renders {:.1}, cartridge badge renders {:.1}; private {:.1} MB, working set {:.1} MB",
                (end.cpu_ms - start.cpu_ms) / (secs * 10.0),
                per(before.4, after.4),
                per(before.0, after.0),
                per(before.1, after.1),
                per(before.2, after.2),
                per(before.3, after.3),
                end.private as f64 / 1e6,
                end.working_set as f64 / 1e6,
            ));
            quit(cx);
        });
    })
    .detach();
}
