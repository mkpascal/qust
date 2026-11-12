// The window and menu helpers against real GPUI windows, all hidden or off every screen, none
// focused, nothing injected (messages are posted to our own windows only):
//
//   ownership and centering, the title bar theme (also after WM_SETTINGCHANGE), Esc closing,
//   recording keys from window messages, the drawer animation, switching GPUI between headless and
//   windowed, and whether a GPUI window keeps painting while a native menu is open (a simulated
//   modal loop, then real menus that open off-screen and close themselves within a second, with
//   and without TPM_NONOTIFY).
//
//   cargo run -p qust-shell --features gpui-examples --example gpui_windows
//
// Prints one line per check and exits with 1 if any failed.

use gpui::*;
use qust_shell::gpui_glue;
use qust_shell::recorder::Recorded;
use qust_shell::theme::{self, Theme};
use qust_shell::window as shell_window;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static FAILED: AtomicBool = AtomicBool::new(false);

fn check(name: &str, ok: bool, detail: String) {
    println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        FAILED.store(true, Ordering::SeqCst);
    }
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x202020))
    }
}

// Counts its frames and asks for the next one: a window that animates all the time.
struct Animated {
    frames: Arc<AtomicU64>,
}

impl Render for Animated {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let frames = self.frames.fetch_add(1, Ordering::Relaxed);
        window.request_animation_frame();
        div().size_full().bg(rgb(0x101010 + (frames % 200) as u32))
    }
}

fn hidden_options(kind: WindowKind) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(100.), px(100.)), size(px(860.), px(267.))))),
        titlebar: Some(TitlebarOptions { title: Some("qust-shell test".into()), appears_transparent: false, traffic_light_position: None }),
        focus: false,
        show: false,
        kind,
        is_resizable: false,
        // Full rate while inactive, so the frame counts show any stall clearly.
        inactive_frame_interval: None,
        ..Default::default()
    }
}

async fn sleep(cx: &AsyncApp, ms: u64) {
    cx.background_executor().timer(Duration::from_millis(ms)).await;
}

#[cfg(windows)]
mod win32 {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn h(window: isize) -> HWND {
        HWND(window as *mut _)
    }

    pub fn post(window: isize, msg: u32, wparam: usize, lparam: isize) {
        unsafe {
            let _ = PostMessageW(Some(h(window)), msg, WPARAM(wparam), LPARAM(lparam));
        }
    }

    pub fn send(window: isize, msg: u32, wparam: usize, lparam: isize) -> isize {
        unsafe { SendMessageW(h(window), msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
    }

    pub fn is_window(window: isize) -> bool {
        unsafe { IsWindow(Some(h(window))).as_bool() }
    }

    // A key message's lParam: repeat 1, the scan code, the extended flag.
    pub fn key_lparam(scan: u32, extended: bool) -> isize {
        (1 | (scan << 16) | if extended { 1 << 24 } else { 0 }) as isize
    }

    // Client area of `width`×`height` physical pixels at (x, y) (outer), shown or not.
    pub fn place(window: isize, x: i32, y: i32, width: i32, height: i32, show: bool) {
        unsafe {
            let (mut outer, mut client) = (Default::default(), Default::default());
            let _ = GetWindowRect(h(window), &mut outer);
            let _ = GetClientRect(h(window), &mut client);
            let frame_w = (outer.right - outer.left) - (client.right - client.left);
            let frame_h = (outer.bottom - outer.top) - (client.bottom - client.top);
            let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
            if show {
                flags |= SWP_SHOWWINDOW;
            }
            let _ = SetWindowPos(h(window), None, x, y, width + frame_w, height + frame_h, flags);
        }
    }

    pub fn work_area_bottom(window: isize) -> i32 {
        use windows::Win32::Graphics::Gdi::*;
        unsafe {
            let monitor = MonitorFromWindow(h(window), MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            info.rcWork.bottom
        }
    }

    pub fn outer_bottom(window: isize) -> i32 {
        let mut rect = Default::default();
        unsafe {
            let _ = GetWindowRect(h(window), &mut rect);
        }
        rect.bottom
    }

    pub fn set_dark_attribute(window: isize, dark: bool) {
        use windows::Win32::Graphics::Dwm::*;
        let value: windows::core::BOOL = dark.into();
        unsafe {
            let _ = DwmSetWindowAttribute(h(window), DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const _ as *const _, 4);
        }
    }

    // A modal loop like a menu's, for `ms`, announced to `owner` as a menu loop.
    pub fn simulated_menu_loop(owner: isize, ms: u64) {
        unsafe {
            SendMessageW(h(owner), WM_ENTERMENULOOP, Some(WPARAM(1)), Some(LPARAM(0)));
            let start = std::time::Instant::now();
            let mut msg = MSG::default();
            while start.elapsed().as_millis() < ms as u128 {
                if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                } else {
                    let _ = MsgWaitForMultipleObjects(None, false, 5, QS_ALLINPUT);
                }
            }
            SendMessageW(h(owner), WM_EXITMENULOOP, Some(WPARAM(1)), Some(LPARAM(0)));
        }
    }

    // Runs `f` from the message loop itself (a zero timer), like menu::popup.
    pub fn from_message_loop(f: Box<dyn FnOnce()>) {
        thread_local! {
            static JOBS: std::cell::RefCell<Vec<Box<dyn FnOnce()>>> = std::cell::RefCell::new(Vec::new());
        }
        unsafe extern "system" fn run(_: HWND, _: u32, timer: usize, _: u32) {
            let _ = KillTimer(None, timer);
            let jobs = JOBS.with(|jobs| std::mem::take(&mut *jobs.borrow_mut()));
            for job in jobs {
                job();
            }
        }
        JOBS.with(|jobs| jobs.borrow_mut().push(f));
        unsafe {
            SetTimer(None, 0, 0, Some(run));
        }
    }
}

fn main() {
    let app = gpui_platform::application().with_quit_mode(QuitMode::Explicit).with_windowing(WindowingRequest::Headless);
    app.run(|cx: &mut App| {
        cx.spawn(async move |cx| {
            run(cx).await;
            let failed = FAILED.load(Ordering::SeqCst);
            println!("{}", if failed { "SOME CHECKS FAILED" } else { "ALL CHECKS PASSED" });
            std::process::exit(if failed { 1 } else { 0 });
        })
        .detach();
    });
}

#[cfg(not(windows))]
async fn run(_cx: &mut AsyncApp) {
    println!("Windows only");
}

#[cfg(windows)]
async fn run(cx: &mut AsyncApp) {
    use windows::Win32::UI::WindowsAndMessaging::*;

    // --- Shell events reach GPUI's main thread ----------------------------------------------
    let native = qust_shell::NativeThread::start().expect("shell thread");
    let sender = cx.update(gpui_glue::MainSender::new);
    let main_thread = std::thread::current().id();
    let received: Arc<std::sync::Mutex<Vec<(qust_shell::Event, bool)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = received.clone();
    gpui_glue::forward_events(&native, sender.clone(), move |event, _cx| {
        log.lock().unwrap().push((event, std::thread::current().id() == main_thread));
    });
    win32::post(native.hwnd(), WM_HOTKEY, 2, 0);
    sleep(cx, 200).await;
    let got = received.lock().unwrap().clone();
    let route = got.first().map(|(event, _)| qust_shell::background::route(event, false));
    check(
        "a shortcut reaches the app on the main thread",
        got == vec![(qust_shell::Event::Hotkey(2), true)] && route == Some(qust_shell::background::Request::Nothing),
        format!("{got:?} → {route:?} (Stop with the player closed does nothing)"),
    );
    native.stop();

    // --- Headless ↔ windowed, twice -------------------------------------------------------
    let open_without_gpu = cx.update(|cx| cx.open_window(hidden_options(WindowKind::Normal), |_, cx| cx.new(|_| Blank)).is_err());
    check("headless refuses windows", open_without_gpu, "open_window fails while headless".into());
    for round in 1..=2 {
        let windowed = cx.update(gpui_glue::windowed).await;
        let window = cx.update(|cx| cx.open_window(hidden_options(WindowKind::Normal), |_, cx| cx.new(|_| Blank)));
        let opened = window.is_ok();
        if let Ok(window) = window {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
        sleep(cx, 200).await;
        let headless = cx.update(gpui_glue::headless).await;
        check(&format!("windowed → window → headless, round {round}"), windowed.is_ok() && opened && headless.is_ok(), format!("{windowed:?} opened={opened} {headless:?}"));
    }
    let _ = cx.update(gpui_glue::windowed).await;

    // --- Ownership and centering ------------------------------------------------------------
    let owner = cx.update(|cx| cx.open_window(hidden_options(WindowKind::Normal), |_, cx| cx.new(|_| Blank))).expect("owner window");
    let panel = cx.update(|cx| cx.open_window(hidden_options(WindowKind::Normal), |_, cx| cx.new(|_| Blank))).expect("panel window");
    let owner_hwnd = owner.update(cx, |_, window, _| gpui_glue::hwnd(window)).ok().flatten().unwrap_or(0);
    let panel_hwnd = panel.update(cx, |_, window, _| gpui_glue::hwnd(window)).ok().flatten().unwrap_or(0);
    check("window handles", owner_hwnd != 0 && panel_hwnd != 0, format!("{owner_hwnd:#x} {panel_hwnd:#x}"));
    let had_taskbar = shell_window::has_taskbar_style(panel_hwnd);
    shell_window::set_owner(panel_hwnd, owner_hwnd);
    check(
        "owned panel",
        shell_window::owner_of(panel_hwnd) == owner_hwnd && !shell_window::has_taskbar_style(panel_hwnd),
        format!("owner {:#x}, WS_EX_APPWINDOW before {had_taskbar}, after {}", shell_window::owner_of(panel_hwnd), shell_window::has_taskbar_style(panel_hwnd)),
    );
    let scale = shell_window::scale(owner_hwnd);
    win32::place(owner_hwnd, (300.0 * scale) as i32, (200.0 * scale) as i32, (860.0 * scale) as i32, (267.0 * scale) as i32, false);
    let outer = shell_window::outer_rect(owner_hwnd).expect("owner rect");
    let expected = shell_window::centered_over(outer, (580.0, 590.0), shell_window::work_area(owner_hwnd));
    let position = shell_window::panel_position(owner_hwnd, (580.0, 590.0));
    let ok = position.is_some_and(|(logical, _)| (logical.0 - expected.0).abs() < 0.01 && (logical.1 - expected.1).abs() < 0.01);
    if let Some((_, physical)) = position {
        shell_window::set_outer_position(panel_hwnd, physical.0, physical.1);
    }
    let placed = shell_window::outer_rect(panel_hwnd);
    check(
        "panel centered over the player",
        ok && placed.is_some_and(|rect| (rect.x - expected.0).abs() <= 1.0 && (rect.y - expected.1).abs() <= 1.0),
        format!("owner {outer:?} scale {scale}, panel at {placed:?}, expected {expected:?}"),
    );
    shell_window::set_minimizable(panel_hwnd, false);
    let style = unsafe { GetWindowLongPtrW(win32::h(panel_hwnd), GWL_STYLE) };
    check("panel not minimizable", style & WS_MINIMIZEBOX.0 as isize == 0, format!("style {style:#x}"));

    // --- Title bar theme ---------------------------------------------------------------------
    theme::set_app_theme(Theme::Dark);
    shell_window::follow_theme(panel_hwnd);
    let dark = shell_window::dark_title_bar(panel_hwnd);
    theme::set_app_theme(Theme::Light);
    let light = !shell_window::dark_title_bar(panel_hwnd);
    theme::set_app_theme(Theme::Dark);
    // What GPUI does on a system theme change: the system's value on the title bar. Then the
    // WM_SETTINGCHANGE it came with; the forced theme must win.
    win32::set_dark_attribute(panel_hwnd, false);
    let setting = windows::core::w!("ImmersiveColorSet");
    win32::send(panel_hwnd, WM_SETTINGCHANGE, 0, setting.as_ptr() as isize);
    let after_change = shell_window::dark_title_bar(panel_hwnd);
    theme::set_app_theme(Theme::System);
    let system = shell_window::dark_title_bar(panel_hwnd) == theme::system_dark();
    check("title bar follows the app theme", dark && light && after_change && system, format!("dark {dark}, light {light}, forced dark after WM_SETTINGCHANGE {after_change}, system {system}"));

    // --- Recording keys from window messages -------------------------------------------------
    let seen: Rc<RefCell<Vec<Recorded>>> = Rc::new(RefCell::new(Vec::new()));
    shell_window::close_on_escape(panel_hwnd);
    let capture = {
        let seen = seen.clone();
        shell_window::KeyCapture::start(panel_hwnd, move |recorded| seen.borrow_mut().push(recorded))
    };
    win32::post(panel_hwnd, WM_KEYDOWN, 0x48, win32::key_lparam(0x23, false)); // H, no modifiers: ignored
    win32::post(panel_hwnd, WM_KEYDOWN, 0x78, win32::key_lparam(0x43, false)); // F9
    sleep(cx, 100).await;
    let first = seen.borrow().clone();
    let capture_done = !capture.is_active();
    drop(capture);
    let capture = {
        let seen = seen.clone();
        shell_window::KeyCapture::start(panel_hwnd, move |recorded| seen.borrow_mut().push(recorded))
    };
    win32::post(panel_hwnd, WM_KEYDOWN, 0x1B, win32::key_lparam(0x01, false)); // Esc: cancels, doesn't close
    sleep(cx, 100).await;
    let still_open = win32::is_window(panel_hwnd);
    drop(capture);
    let capture = {
        let seen = seen.clone();
        shell_window::KeyCapture::start(panel_hwnd, move |recorded| seen.borrow_mut().push(recorded))
    };
    win32::post(panel_hwnd, WM_KEYDOWN, 0x08, win32::key_lparam(0x0E, false)); // Backspace
    sleep(cx, 100).await;
    drop(capture);
    let capture = {
        let seen = seen.clone();
        shell_window::KeyCapture::start(panel_hwnd, move |recorded| seen.borrow_mut().push(recorded))
    };
    win32::post(panel_hwnd, WM_KEYDOWN, 0xB3, win32::key_lparam(0x22, true)); // Media Play/Pause
    sleep(cx, 100).await;
    drop(capture);
    let capture = {
        let seen = seen.clone();
        shell_window::KeyCapture::start(panel_hwnd, move |recorded| seen.borrow_mut().push(recorded))
    };
    win32::post(panel_hwnd, WM_ACTIVATE, WA_INACTIVE as usize, 0); // losing the focus
    sleep(cx, 100).await;
    drop(capture);
    let all = seen.borrow().clone();
    let expected = vec![Recorded::Set("F9".into()), Recorded::Cancel, Recorded::Reset, Recorded::Set("MediaPlayPause".into()), Recorded::Cancel];
    check("key capture", first == vec![Recorded::Set("F9".into())] && capture_done && still_open && all == expected, format!("{all:?}, stopped after F9 {capture_done}, open after Esc {still_open}"));

    // --- Esc closes ---------------------------------------------------------------------------
    win32::post(panel_hwnd, WM_KEYDOWN, 0x1B, win32::key_lparam(0x01, false));
    sleep(cx, 200).await;
    check("Esc closes the panel", !win32::is_window(panel_hwnd), format!("window exists: {}", win32::is_window(panel_hwnd)));

    // --- Drawer animation (hidden window) -------------------------------------------------------
    let scale = shell_window::scale(owner_hwnd);
    win32::place(owner_hwnd, (300.0 * scale) as i32, (200.0 * scale) as i32, (860.0 * scale) as i32, (267.0 * scale) as i32, false);
    let started = Instant::now();
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    qust_shell::drawer::animate(owner_hwnd, 587.0, move || flag.store(true, Ordering::SeqCst));
    sleep(cx, 80).await;
    let (_, mid) = shell_window::client_size(owner_hwnd).unwrap_or_default();
    sleep(cx, 400).await;
    let (width, height) = shell_window::client_size(owner_hwnd).unwrap_or_default();
    check(
        "drawer opens",
        done.load(Ordering::SeqCst) && height == (587.0 * scale).round() as i32 && width == (860.0 * scale).round() as i32 && mid > (267.0 * scale) as i32 && mid < height,
        format!("client {width}×{height} (mid-way {mid}) at scale {scale}, {} ms", started.elapsed().as_millis()),
    );
    // A second click mid-way: the newer animation wins.
    let first_done = Arc::new(AtomicBool::new(false));
    let flag = first_done.clone();
    qust_shell::drawer::animate(owner_hwnd, 267.0, move || flag.store(true, Ordering::SeqCst));
    sleep(cx, 60).await;
    qust_shell::drawer::animate(owner_hwnd, 587.0, || {});
    sleep(cx, 400).await;
    let (_, height) = shell_window::client_size(owner_hwnd).unwrap_or_default();
    check("newer animation cancels", !first_done.load(Ordering::SeqCst) && height == (587.0 * scale).round() as i32, format!("first finished {}, client height {height}", first_done.load(Ordering::SeqCst)));
    // Near the bottom of the work area: it moves up to stay on screen.
    qust_shell::drawer::animate(owner_hwnd, 267.0, || {});
    sleep(cx, 400).await;
    let bottom = win32::work_area_bottom(owner_hwnd);
    win32::place(owner_hwnd, (300.0 * scale) as i32, bottom - (300.0 * scale) as i32, (860.0 * scale) as i32, (267.0 * scale) as i32, false);
    qust_shell::drawer::animate(owner_hwnd, 587.0, || {});
    sleep(cx, 400).await;
    let outer_bottom = win32::outer_bottom(owner_hwnd);
    check("drawer stays on screen", outer_bottom <= bottom, format!("window bottom {outer_bottom}, work area bottom {bottom}"));
    let _ = owner.update(cx, |_, window, _| window.remove_window());

    // --- Painting during a menu's modal loop ---------------------------------------------------
    let frames = Arc::new(AtomicU64::new(0));
    let other_frames = Arc::new(AtomicU64::new(0));
    let counter = frames.clone();
    let animated = cx.update(|cx| cx.open_window(hidden_options(WindowKind::PopUp), move |_, cx| cx.new(|_| Animated { frames: counter }))).expect("animated window");
    let counter = other_frames.clone();
    let other = cx.update(|cx| cx.open_window(hidden_options(WindowKind::PopUp), move |_, cx| cx.new(|_| Animated { frames: counter }))).expect("second window");
    let animated_hwnd = animated.update(cx, |_, window, _| gpui_glue::hwnd(window)).ok().flatten().unwrap_or(0);
    let other_hwnd = other.update(cx, |_, window, _| gpui_glue::hwnd(window)).ok().flatten().unwrap_or(0);
    // Shown far off every screen, without activation: they paint, nobody sees them.
    win32::place(animated_hwnd, -20000, -20000, 200, 100, true);
    win32::place(other_hwnd, -20000, -19000, 200, 100, true);
    sleep(cx, 500).await;

    let measure = |frames: &Arc<AtomicU64>| frames.load(Ordering::Relaxed);
    let before = (measure(&frames), measure(&other_frames));
    sleep(cx, 1000).await;
    let baseline = (measure(&frames) - before.0, measure(&other_frames) - before.1);
    println!("INFO baseline: {} frames/s (second window {})", baseline.0, baseline.1);

    // A phase runs `start` from the message loop and waits until it reports its end.
    async fn phase(cx: &AsyncApp, frames: &Arc<AtomicU64>, other: &Arc<AtomicU64>, start: impl FnOnce(Box<dyn FnOnce()>) + 'static) -> (u64, u64, u128) {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let begin = (frames.load(Ordering::Relaxed), other.load(Ordering::Relaxed));
        let clock = Instant::now();
        let started_at = Rc::new(RefCell::new(None::<(u64, u64, Instant)>));
        let stamp = started_at.clone();
        let (f1, f2) = (frames.clone(), other.clone());
        win32::from_message_loop(Box::new(move || {
            *stamp.borrow_mut() = Some((f1.load(Ordering::Relaxed), f2.load(Ordering::Relaxed), Instant::now()));
            start(Box::new(move || flag.store(true, Ordering::SeqCst)));
        }));
        while !finished.load(Ordering::SeqCst) && clock.elapsed() < Duration::from_secs(5) {
            sleep(cx, 20).await;
        }
        let (start_frames, start_other, at) = started_at.borrow().unwrap_or((begin.0, begin.1, clock));
        (frames.load(Ordering::Relaxed) - start_frames, other.load(Ordering::Relaxed) - start_other, at.elapsed().as_millis())
    }

    let owner_for_loop = animated_hwnd;
    let simulated = phase(cx, &frames, &other_frames, move |done| {
        win32::simulated_menu_loop(owner_for_loop, 1000);
        done();
    })
    .await;
    println!("INFO simulated menu loop: {} frames in {} ms (second window {})", simulated.0, simulated.2, simulated.1);

    let menu = || qust_shell::menu::Menu::new().item(1, "Test item");
    qust_shell::menu::test_hide_menus(true, Duration::from_millis(1000));
    let real = phase(cx, &frames, &other_frames, move |done| {
        qust_shell::menu::popup_for_test(owner_for_loop, menu(), qust_shell::menu::Position::Client { x: 0.0, y: 0.0 }, false, move |_| done());
    })
    .await;
    println!("INFO real menu (no TPM_NONOTIFY): {} frames in {} ms (second window {})", real.0, real.2, real.1);
    qust_shell::menu::test_hide_menus(true, Duration::from_millis(1000));
    let nonotify = phase(cx, &frames, &other_frames, move |done| {
        qust_shell::menu::popup_for_test(owner_for_loop, menu(), qust_shell::menu::Position::Client { x: 0.0, y: 0.0 }, true, move |_| done());
    })
    .await;
    println!("INFO real menu with TPM_NONOTIFY: {} frames in {} ms (second window {})", nonotify.0, nonotify.2, nonotify.1);

    // What `popup` avoids: the menu shown from inside a GPUI callback (the app borrowed).
    let mut inside = Vec::new();
    for no_notify in [false, true] {
        qust_shell::menu::test_hide_menus(true, Duration::from_millis(1000));
        let begin = (measure(&frames), measure(&other_frames));
        let clock = Instant::now();
        cx.update(|_| {
            qust_shell::menu::show_now_for_test(owner_for_loop, menu(), qust_shell::menu::Position::Client { x: 0.0, y: 0.0 }, no_notify);
        });
        let elapsed = clock.elapsed().as_millis();
        let counts = (measure(&frames) - begin.0, measure(&other_frames) - begin.1, elapsed);
        println!("INFO menu shown inside a GPUI callback (TPM_NONOTIFY {no_notify}): {} frames in {} ms (second window {})", counts.0, counts.2, counts.1);
        inside.push(counts);
        sleep(cx, 300).await;
    }
    qust_shell::menu::test_hide_menus(false, Duration::ZERO);

    let rate = |(frames, _, ms): (u64, u64, u128)| frames as f64 * 1000.0 / ms.max(1) as f64;
    let base_rate = baseline.0 as f64;
    check(
        "a GPUI window keeps painting during a menu",
        real.2 >= 800 && real.2 < 2000 && rate(real) > base_rate * 0.5,
        format!("{:.0} frames/s during the menu vs {base_rate:.0} before (simulated loop {:.0}, with TPM_NONOTIFY {:.0}); menu open {} ms", rate(real), rate(simulated), rate(nonotify), real.2),
    );
    let _ = animated.update(cx, |_, window, _| window.remove_window());
    let _ = other.update(cx, |_, window, _| window.remove_window());
}
