//! The panels in the capture harness (feature `capture`): a reference fixture's window opened
//! hidden with the recording fake, its `actions` replayed the way the reference renderer did them
//! (forced pseudo-states, a scroll, clicks at the element's box, keys, waits), then captured.

use crate::fake::FakeServices;
use crate::info::InfoView;
use crate::services::PanelServices;
use crate::settings::SettingsView;
use gpui::{point, px, AnyView, App, AppContext, AsyncApp, Entity, Point};
use qust_fluent::Forced;
use qust_shell::recorder::record;
use qust_ui::capture::{Job, Session};
use qust_ui::fixture::layout;
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone)]
pub enum Panel {
    Settings(Entity<SettingsView>),
    Info(Entity<InfoView>),
}

#[derive(Clone)]
pub struct Opened {
    pub session: Session,
    pub panel: Panel,
    pub fake: Rc<FakeServices>,
}

/// Opens `job`'s window (hidden) with its panel and a fake answering like the fixture.
pub fn open(job: &Job, cx: &mut App) -> Result<Opened, String> {
    type Built = Option<(Panel, Rc<FakeServices>)>;
    let slot: Rc<RefCell<Built>> = Rc::new(RefCell::new(None));
    let out = slot.clone();
    let session = Session::open(job, cx, move |setup, window, cx| {
        let fake = FakeServices::for_fixture(setup.models, &setup.job.fixture);
        let services: Rc<dyn PanelServices> = fake.clone();
        if setup.job.fixture.window == "info" {
            let view = cx.new(|cx| InfoView::new(setup.models, services, window, cx));
            *out.borrow_mut() = Some((Panel::Info(view.clone()), fake));
            AnyView::from(view)
        } else {
            let view = cx.new(|cx| SettingsView::new(setup.models, services, window, cx));
            *out.borrow_mut() = Some((Panel::Settings(view.clone()), fake));
            AnyView::from(view)
        }
    })?;
    let (panel, fake) = slot.borrow_mut().take().ok_or("no panel")?;
    Ok(Opened { session, panel, fake })
}

/// The element a fixture's selector names, as the layout files name it.
fn element_name(selector: &str) -> Option<&'static str> {
    Some(match selector {
        "[data-action=\"play\"]" => "body>.settings-list>.setting[5]>.shortcut",
        "[data-action=\"stop\"]" => "body>.settings-list>.setting[6]>.shortcut",
        "[data-action=\"toggle\"]" => "body>.settings-list>.setting[7]>.shortcut",
        "#autoplayToggle + .toggle-track" => "body>.settings-list>.setting[2]>.toggle>.toggle-track",
        "#backgroundToggle + .toggle-track" => "body>.settings-list>.setting[4]>.toggle>.toggle-track",
        "#autoUpdateToggle + .toggle-track" => "body>.settings-list>.setting[9]>.toggle>.toggle-track",
        "#themeCombo" => "#themeCombo",
        "#colorsCombo" => "#colorsCombo",
        "#reconnectCombo" => "#reconnectCombo",
        "#updateButton" => "#updateButton",
        _ => return None,
    })
}

/// The centre of an element in the job's layout file (CSS px), per theme when they differ.
pub fn center(job: &Job, name: &str) -> Option<Point<f32>> {
    let layout = layout(&job.fixture.window, &job.variant).or_else(|| layout(&job.fixture.window, &job.fixture.name))?;
    let element = layout["elements"].as_array()?.iter().find(|e| e["name"] == name)?;
    let values = element["box"].as_array().or_else(|| element["box"][job.theme.as_str()].as_array())?;
    let n = |i: usize| values.get(i).and_then(Value::as_f64).map(|v| v as f32);
    Some(point(n(0)? + n(2)? / 2.0, n(1)? + n(3)? / 2.0))
}

pub async fn settle(opened: &Opened, cx: &mut AsyncApp) {
    // Let spawned work (the key capture's task, a details request) run, then draw.
    cx.background_executor().timer(Duration::from_millis(5)).await;
    cx.update(|cx| opened.session.frames(2, cx));
}

/// The hidden window gets its client size from a resize that Windows applies a moment after it
/// opens; until then it lays out at the system's default size. Waits (up to a second) until the
/// window has the fixture's size, so clicks land where the layout file says.
pub async fn wait_for_size(opened: &Opened, expected: (f32, f32), cx: &mut AsyncApp) {
    opened.session.wait_for_size(expected, cx).await;
}

/// Replays the fixture's actions.
pub async fn act(job: &Job, opened: &Opened, cx: &mut AsyncApp) -> Result<(), String> {
    for action in &job.fixture.actions {
        if let Some(force) = action.get("force").and_then(Value::as_object) {
            cx.update(|cx| {
                for (selector, names) in force {
                    let names: Vec<&str> = names.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    if let Panel::Settings(view) = &opened.panel {
                        view.update(cx, |view, cx| view.force(selector, Forced::from_names(names), cx));
                    }
                }
            });
        } else if let Some(top) = action.get("top").and_then(Value::as_f64) {
            cx.update(|cx| opened.session.frames(2, cx));
            cx.update(|cx| {
                if let Panel::Settings(view) = &opened.panel {
                    let handle = view.read(cx).scroll_handle().clone();
                    let max = handle.max_offset().y.as_f32();
                    handle.set_offset(point(px(0.0), px(-(top as f32).min(max))));
                }
            });
        } else if let Some(selector) = action.get("click").and_then(Value::as_str) {
            let name = element_name(selector).ok_or_else(|| format!("unknown selector {selector}"))?;
            let at = center(job, name).ok_or_else(|| format!("no box for {name}"))?;
            cx.update(|cx| opened.session.click(at, cx));
        } else if let Some(key) = action.get("key") {
            let code = key["code"].as_str().unwrap_or("");
            let modifiers = key["modifiers"].as_u64().unwrap_or(0);
            // DevTools' modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8.
            let recorded = record(code, modifiers & 2 != 0, modifiers & 1 != 0, modifiers & 8 != 0, modifiers & 4 != 0);
            opened.fake.press(recorded);
        } else if let Some(ms) = action.get("wait").and_then(Value::as_u64) {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
        } else {
            return Err(format!("unsupported action {action}"));
        }
        settle(opened, cx).await;
    }
    Ok(())
}

/// Captures every job; each result is the PNG written and the window's scale.
pub fn capture_all(jobs: Vec<Job>) -> Vec<Result<(PathBuf, f32), String>> {
    let results = Rc::new(RefCell::new(Vec::new()));
    let out = results.clone();
    qust_ui::capture::run(move |cx| {
        cx.spawn(async move |cx| {
            for job in jobs {
                let result = capture_one(&job, cx).await;
                out.borrow_mut().push(result);
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let results = results.borrow_mut().drain(..).collect();
    results
}

/// One job: open, act, capture, close.
pub async fn capture_one(job: &Job, cx: &mut AsyncApp) -> Result<(PathBuf, f32), String> {
    let opened = cx.update(|cx| {
        let opened = open(job, cx)?;
        opened.session.frames(3, cx);
        Ok::<_, String>(opened)
    })?;
    wait_for_size(&opened, job.fixture.window_size(), cx).await;
    let acted = act(job, &opened, cx).await;
    // A short wait for asset loads (SVG icons are loaded on first use).
    cx.background_executor().timer(Duration::from_millis(60)).await;
    cx.update(|cx| {
        opened.session.frames(3, cx);
        let scale = opened.session.scale_factor(cx);
        let saved = acted.and_then(|_| opened.session.capture(&job.out, cx).map(|_| (job.out.clone(), scale)));
        opened.session.close(cx);
        saved
    })
}
