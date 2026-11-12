//! The capture and test harness (feature `capture`): a view rendered from a reference fixture in
//! a real GPUI window, written to PNG, compared with the reference render; and input dispatched
//! into it for behaviour tests. No live player: the fixture fills the [`Models`].
//!
//! **On the user's desktop.** Capture windows are `WindowKind::PopUp` (a tool window: no taskbar
//! button), never shown (`show: false`) and never activated (`focus: false`), placed far
//! off-screen anyway. GPUI doesn't draw a hidden window by itself, so the harness draws it
//! (`Window::draw`) and reads the pixels with `Window::render_to_image`, which renders the scene
//! to a texture without presenting it. Nothing appears on screen, nothing takes focus. Input is
//! dispatched with `Window::dispatch_event` (real hit-testing, drags, keys), never as OS input.
//! Native tooltips are turned off ([`qust_fluent::tooltip::set_native`]); the text that would
//! show is in `tooltip::last_shown`. Menus, pickers and other OS UI must be stubbed by the window
//! crate before a test reaches them.
//!
//! **Scale.** The window gets the scale factor of the monitor Windows assigns it (the primary
//! monitor's for an off-screen window): GPUI can't override it. Compare with the reference
//! render of that scale ([`Session::scale_factor`]); the 125 % / 150 % / 200 % references can only
//! be checked on a monitor at that scale.
//!
//! **Animations** are frozen as the reference froze them ([`qust_fluent::motion::Frozen`]): with
//! the fixture's `time` (or one of its `times`), every animation that far after it started;
//! without, infinite ones at 0 and finite ones at their end; transitions at their end unless the
//! fixture says `transitions`. An element whose animation starts at a known moment (the dialog
//! opening, "dropped") gets its start from the view as usual; frozen sampling ignores it.
//!
//! **Comparing**: [`compare`] runs `node reference/compare.mjs <reference> <capture> --json …`
//! and returns its verdict (exit 0 = match); the diff picture is written next to the capture.
//!
//! The example `cargo run -p qust-ui --example capture --features capture -- <fixture> [dark|light]`
//! shows the whole flow; window crates copy it with their own view.

use crate::fixture::{Fixture, TimeSpec};
use crate::models::Models;
use crate::window::WindowRoot;
use gpui::{
    point, px, size, AnyView, AnyWindowHandle, App, AppContext, Bounds, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PlatformInput, Point, ScrollDelta, ScrollEvent, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use qust_fluent::motion::{Frozen, Motion};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// What one capture is: a fixture variant in a theme.
#[derive(Clone, Debug)]
pub struct Job {
    pub fixture: Fixture,
    /// The render name (`stopped`, `playing-marquee-p50`).
    pub variant: String,
    pub time: Option<TimeSpec>,
    /// "dark" or "light".
    pub theme: String,
    pub out: PathBuf,
}

impl Job {
    /// Every variant of a fixture in `theme`, written to `dir/<variant>.<theme>.png`.
    pub fn all(fixture: &Fixture, theme: &str, dir: &Path) -> Vec<Job> {
        fixture
            .variants()
            .into_iter()
            .map(|(variant, time)| Job { out: dir.join(format!("{variant}.{theme}.png")), fixture: fixture.clone(), variant, time, theme: theme.to_string() })
            .collect()
    }
}

/// What the view under test gets.
pub struct Setup<'a> {
    pub job: &'a Job,
    pub models: &'a Models,
    /// The window root: for portal slots, live layers and overlays.
    pub root: WeakEntity<WindowRoot>,
}

/// The app-wide setup every harness run does: GPUI Kit, fluent, grayscale text, no native
/// tooltips.
pub fn init(cx: &mut App) {
    crate::init(cx);
    // Windows open and close one after another; the app ends when the harness says so.
    cx.set_quit_mode(gpui::QuitMode::Explicit);
    qust_fluent::tooltip::set_native(false, cx);
}

/// Prepares the globals for `job`: theme, accent, frozen animations.
pub fn prepare(job: &Job, cx: &mut App) {
    crate::theme::set_accent(Some(job.fixture.accent()), cx);
    crate::theme::apply(crate::theme::ThemeChoice::parse(&job.theme), cx);
    let time = job.fixture.frozen_time(&job.variant, job.time);
    cx.set_global(Motion { frozen: Some(Frozen { time, transitions: job.fixture.transitions }), reduced: false });
}

/// Options for an invisible, inactive window of `size` CSS px.
pub fn window_options(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(-32000.0), px(-32000.0)), size(px(width), px(height))))),
        titlebar: None::<TitlebarOptions>,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    }
}

/// A window under test.
#[derive(Clone)]
pub struct Session {
    pub handle: WindowHandle<WindowRoot>,
    pub models: Models,
}

impl Session {
    /// Opens a window for `job` with the view `build` makes, the fixture applied to fresh models.
    pub fn open(job: &Job, cx: &mut App, build: impl FnOnce(&Setup, &mut Window, &mut App) -> AnyView) -> Result<Session, String> {
        prepare(job, cx);
        let models = Models::new(cx);
        job.fixture.apply(&models, cx);
        let (width, height) = job.fixture.window_size();
        let models_for_window = models.clone();
        let handle = cx
            .open_window(window_options(width, height), move |window, cx| {
                let placeholder: AnyView = cx.new(|_| gpui::EmptyView).into();
                let root = cx.new(|_| WindowRoot::new(placeholder));
                let content = build(&Setup { job, models: &models_for_window, root: root.downgrade() }, window, cx);
                root.update(cx, |root, cx| root.set_content(content, cx));
                root
            })
            .map_err(|e| e.to_string())?;
        // A window that was never shown keeps the system's default size: set the client size.
        let _ = cx.update_window(handle.into(), |_, window, _| window.resize(size(px(width), px(height))));
        Ok(Session { handle, models })
    }

    pub fn any(&self) -> AnyWindowHandle {
        self.handle.into()
    }

    /// Waits (up to two seconds) until the window is laid out at `size` (CSS px). The client size
    /// comes from a resize that Windows applies a moment after the window opens; until then the
    /// first frames lay out at the system's default size, so early input would go to the wrong
    /// place. Three readings in a row, 10 ms apart: a late resize can still come after the first.
    pub async fn wait_for_size(&self, size: (f32, f32), cx: &mut gpui::AsyncApp) {
        let mut settled = 0;
        for _ in 0..200 {
            let now = cx.update(|cx| {
                self.frames(1, cx);
                cx.update_window(self.any(), |_, window, _| window.viewport_size()).ok()
            });
            let at_size = now.is_some_and(|s| (s.width.as_f32() - size.0).abs() < 0.5 && (s.height.as_f32() - size.1).abs() < 0.5);
            settled = if at_size { settled + 1 } else { 0 };
            if settled >= 3 {
                break;
            }
            cx.background_executor().timer(Duration::from_millis(10)).await;
        }
        cx.update(|cx| self.frames(2, cx));
    }

    /// Draws a frame now (the window is hidden: nothing draws it otherwise).
    pub fn frame(&self, cx: &mut App) {
        let _ = cx.update_window(self.handle.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }

    /// Draws `n` frames (portals take two: one to learn where their slot is).
    pub fn frames(&self, n: usize, cx: &mut App) {
        for _ in 0..n {
            self.frame(cx);
        }
    }

    pub fn scale_factor(&self, cx: &mut App) -> f32 {
        cx.update_window(self.handle.into(), |_, window, _| window.scale_factor()).unwrap_or(1.0)
    }

    /// The last frame's pixels, as PNG.
    pub fn capture(&self, path: &Path, cx: &mut App) -> Result<(u32, u32), String> {
        let image = cx.update_window(self.handle.into(), |_, window, _| window.render_to_image()).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        image.save(path).map_err(|e| e.to_string())?;
        Ok((image.width(), image.height()))
    }

    pub fn close(&self, cx: &mut App) {
        let _ = cx.update_window(self.handle.into(), |_, window, _| window.remove_window());
    }

    fn dispatch(&self, event: PlatformInput, cx: &mut App) {
        let _ = cx.update_window(self.handle.into(), |_, window, cx| {
            window.dispatch_event(event, cx);
        });
    }

    /// The pointer moves to `position` (CSS px in the window), with `button` held or not.
    pub fn mouse_move(&self, position: Point<f32>, button: Option<MouseButton>, modifiers: Modifiers, cx: &mut App) {
        self.dispatch(PlatformInput::MouseMove(MouseMoveEvent { position: point(px(position.x), px(position.y)), pressed_button: button, modifiers }), cx);
    }

    pub fn mouse_down(&self, position: Point<f32>, button: MouseButton, click_count: usize, modifiers: Modifiers, cx: &mut App) {
        self.dispatch(PlatformInput::MouseDown(MouseDownEvent { button, position: point(px(position.x), px(position.y)), modifiers, click_count, first_mouse: false }), cx);
    }

    pub fn mouse_up(&self, position: Point<f32>, button: MouseButton, click_count: usize, modifiers: Modifiers, cx: &mut App) {
        self.dispatch(PlatformInput::MouseUp(MouseUpEvent { button, position: point(px(position.x), px(position.y)), modifiers, click_count }), cx);
    }

    /// Move, press, release with the primary button (a frame first, so hitboxes exist).
    pub fn click(&self, position: Point<f32>, cx: &mut App) {
        self.frame(cx);
        self.mouse_move(position, None, Modifiers::default(), cx);
        self.mouse_down(position, MouseButton::Left, 1, Modifiers::default(), cx);
        self.mouse_up(position, MouseButton::Left, 1, Modifiers::default(), cx);
        self.frame(cx);
    }

    pub fn double_click(&self, position: Point<f32>, cx: &mut App) {
        self.click(position, cx);
        self.mouse_down(position, MouseButton::Left, 2, Modifiers::default(), cx);
        self.mouse_up(position, MouseButton::Left, 2, Modifiers::default(), cx);
        self.frame(cx);
    }

    /// A drag with the primary button, in `steps` moves, drawing between them.
    pub fn drag(&self, from: Point<f32>, to: Point<f32>, steps: usize, cx: &mut App) {
        self.frame(cx);
        self.mouse_move(from, None, Modifiers::default(), cx);
        self.mouse_down(from, MouseButton::Left, 1, Modifiers::default(), cx);
        for step in 1..=steps.max(1) {
            let t = step as f32 / steps.max(1) as f32;
            self.mouse_move(Point { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t }, Some(MouseButton::Left), Modifiers::default(), cx);
            self.frame(cx);
        }
        self.mouse_up(to, MouseButton::Left, 1, Modifiers::default(), cx);
        self.frame(cx);
    }

    /// A wheel step at `position` (`lines` > 0: away from the user, up).
    pub fn wheel(&self, position: Point<f32>, lines: f32, cx: &mut App) {
        self.frame(cx);
        self.mouse_move(position, None, Modifiers::default(), cx);
        self.dispatch(PlatformInput::Scroll(ScrollEvent { position: point(px(position.x), px(position.y)), delta: ScrollDelta::Lines(point(0.0, lines)), ..Default::default() }), cx);
        self.frame(cx);
    }

    /// A key pressed and released (`"tab"`, `"shift-tab"`, `"ctrl-a"`, `"right"`, `"enter"`).
    pub fn key(&self, keystroke: &'static str, cx: &mut App) {
        let Ok(stroke) = Keystroke::parse(keystroke) else { return };
        let _ = cx.update_window(self.handle.into(), |_, window, cx| {
            window.dispatch_keystroke(stroke.clone(), cx);
            window.dispatch_event(PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke: stroke }), cx);
        });
        self.frame(cx);
    }

    /// Text typed into the focused element.
    pub fn type_text(&self, text: &str, cx: &mut App) {
        for c in text.chars() {
            let s = c.to_string();
            let _ = cx.update_window(self.handle.into(), |_, window, cx| {
                window.dispatch_keystroke(Keystroke { modifiers: Modifiers::default(), key: s.clone(), key_char: Some(s.clone()) }, cx);
            });
        }
        self.frame(cx);
    }
}

/// Runs a GPUI app with the harness set up, calls `f` once it runs, and returns when `f`'s
/// work called `cx.quit()`. One app per process.
pub fn run(f: impl FnOnce(&mut App) + 'static) {
    crate::application().run(move |cx| {
        init(cx);
        f(cx);
    });
}

/// Captures every job with the view `build` makes and returns where each went. Waits `settle`
/// between opening and capturing (for images and fonts; frozen animations need none).
pub fn capture_all(jobs: Vec<Job>, build: impl Fn(&Setup, &mut Window, &mut App) -> AnyView + 'static) -> Vec<Result<(PathBuf, f32), String>> {
    let results = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let out = results.clone();
    run(move |cx| {
        cx.spawn(async move |cx| {
            for job in jobs {
                let result = cx.update(|cx| {
                    let session = Session::open(&job, cx, |setup, window, cx| build(setup, window, cx))?;
                    session.frames(3, cx);
                    Ok::<_, String>(session)
                });
                let result = match result {
                    Ok(session) => {
                        // A short wait for asset loads (SVG icons are loaded on first use).
                        cx.background_executor().timer(Duration::from_millis(60)).await;
                        cx.update(|cx| {
                            session.frames(2, cx);
                            let scale = session.scale_factor(cx);
                            let saved = session.capture(&job.out, cx).map(|_| (job.out.clone(), scale));
                            session.close(cx);
                            saved
                        })
                    }
                    Err(error) => Err(error),
                };
                out.borrow_mut().push(result);
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let results = results.borrow_mut().drain(..).collect();
    results
}

/// `compare.mjs`'s verdict.
#[derive(Clone, Debug)]
pub struct Comparison {
    pub ok: bool,
    /// Non-text pixels that differ (ratio, 0–1).
    pub ratio: f64,
    pub text_ok: u64,
    pub text_runs: u64,
    pub report: serde_json::Value,
}

/// Runs `node reference/compare.mjs` on a capture (`extra`: more options, like
/// `["--region", "#playButton"]`).
pub fn compare(reference: &Path, capture: &Path, extra: &[&str]) -> Result<Comparison, String> {
    let dir = crate::fixture::reference_dir();
    let json = capture.with_extension("report.json");
    let output = Command::new("node")
        .arg(dir.join("compare.mjs"))
        .arg(reference)
        .arg(capture)
        .arg("--json")
        .arg(&json)
        .arg("--quiet")
        .args(extra)
        .output()
        .map_err(|e| format!("node: {e}"))?;
    if output.status.code() == Some(2) {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(Comparison {
        ok: report["ok"].as_bool().unwrap_or(false),
        ratio: report["pixels"]["ratio"].as_f64().unwrap_or(1.0),
        text_ok: report["text"]["ok"].as_u64().unwrap_or(0),
        text_runs: report["text"]["runs"].as_u64().unwrap_or(0),
        report,
    })
}
