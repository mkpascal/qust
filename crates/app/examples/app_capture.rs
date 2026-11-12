// Full-window captures of the player window as the app composes it (`qust_app::compose`: the
// player view as the root's content, the drawer mounted in its slot), for player fixtures,
// compared with the reference renders: the whole window, then the player's part (y < 267) and
// the drawer's (y 267 to 587) on their own, so the numbers line up with the window crates' own
// captures (qust-ui-player's player_capture, qust-ui-playlist's capture).
//
//   cargo run -p qust-app --example app_capture -- [fixture ...] [--themes dark,light] [--out <dir>]
//
// The fixture fills the models (no player); the player's commands and the drawer's go to the
// window crates' recording stand-ins, menus are dismissed. Windows are hidden and never
// activated (qust_ui::capture). A summary goes to <out>/summary.json.

use gpui::{App, AppContext, FileDropEvent, Window};
use qust_app::compose::{self, HostEnv, PanelKind};
use qust_ui::capture::{self, Job, Session};
use qust_ui::fixture::Fixture;
use qust_ui_player::MenuDone;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const FIXTURES: &[&str] = &[
    "stopped",
    "playing",
    "empty",
    "dialog-load",
    "drawer-entries",
    "drawer-playing-entry",
    "drawer-select-single",
    "drawer-empty",
    "accent-orange-drawer",
    "dialog-edit-track-file",
    "dialog-edit-track-url",
];

/// Nothing leaves the window: menus are dismissed, the picker picks nothing.
struct CaptureEnv;

impl HostEnv for CaptureEnv {
    fn open_panel(&self, _: PanelKind, _: &mut App) {}
    fn drawer_moved(&self, _: bool, _: &mut Window, _: &mut App) {}
    fn show_menu(&self, _: qust_shell::menu::Menu, _: &mut Window, done: MenuDone, cx: &mut App) {
        cx.defer(move |cx| done(None, cx));
    }
    fn pick_files(&self, _: &mut Window, _: Box<dyn FnOnce(Vec<PathBuf>, &mut App)>, _: &mut App) {}
}

struct Done {
    job: Job,
    path: Result<PathBuf, String>,
    skipped: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names = Vec::new();
    let mut themes = vec!["dark".to_string(), "light".to_string()];
    let mut out = std::env::temp_dir().join("qust-app-capture");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--themes" => {
                i += 1;
                themes = args.get(i).map(|t| t.split(',').map(str::to_string).collect()).unwrap_or(themes);
            }
            "--out" => {
                i += 1;
                if let Some(dir) = args.get(i) {
                    out = PathBuf::from(dir);
                }
            }
            name => names.push(name.to_string()),
        }
        i += 1;
    }
    if names.is_empty() {
        names = FIXTURES.iter().map(|s| s.to_string()).collect();
    }
    let mut jobs = Vec::new();
    for name in &names {
        let fixture = match Fixture::load(name) {
            Ok(fixture) => fixture,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
        for theme in &themes {
            if fixture.themes.contains(theme) {
                jobs.extend(Job::all(&fixture, theme, &out));
            }
        }
    }
    let done = Rc::new(RefCell::new(Vec::<Done>::new()));
    let results = done.clone();
    capture::run(move |cx: &mut App| {
        cx.spawn(async move |cx| {
            for job in jobs {
                let recorder = qust_ui_player::testing::Recorder::new();
                let drawer_recorder = qust_ui_playlist::harness::Recorder::new();
                let parts: Rc<RefCell<Option<compose::Composed>>> = Rc::default();
                let opened = cx.update(|cx| {
                    qust_ui_player::testing::install_forced(&job.fixture, cx);
                    qust_ui_player::testing::install_errors(&job.fixture, &recorder);
                    let (slot, rec, drec) = (parts.clone(), recorder.clone(), drawer_recorder.clone());
                    // Session::open, with the app's composition as the window's root (the
                    // harness would wrap the content in a root of its own).
                    capture::prepare(&job, cx);
                    let models = qust_ui::Models::new(cx);
                    job.fixture.apply(&models, cx);
                    // The fixture says whether the drawer is open; the app reads `playlist-open`.
                    let mut settings = models.settings.read(cx).clone();
                    settings.playlist_open = job.fixture.drawer;
                    models.apply_settings(settings, cx);
                    let (width, height) = job.fixture.window_size();
                    let window_models = models.clone();
                    let handle = cx
                        .open_window(capture::window_options(width, height), move |window, cx| {
                            let commands: Rc<dyn qust_ui_player::Commands> = Rc::new(rec);
                            let playlist: Rc<dyn qust_ui_playlist::PlaylistCommands> = Rc::new(qust_ui_playlist::harness::RecordingCommands(drec));
                            let composed = compose::player_window(&window_models, commands, playlist, Rc::new(CaptureEnv), window, cx);
                            let root = composed.root.clone();
                            *slot.borrow_mut() = Some(composed);
                            root
                        })
                        .map_err(|error| error.to_string())?;
                    let _ = cx.update_window(handle.into(), |_, window, _| window.resize(gpui::size(gpui::px(width), gpui::px(height))));
                    let session = Session { handle, models };
                    qust_ui_player::testing::hide_caret(cx);
                    session.frames(3, cx);
                    Ok::<Session, String>(session)
                });
                let session = match opened {
                    Ok(session) => session,
                    Err(error) => {
                        results.borrow_mut().push(Done { job, path: Err(error), skipped: 0 });
                        continue;
                    }
                };
                let Some(parts) = parts.borrow().clone() else { continue };
                qust_ui_player::testing::wait_for_size(&session, job.fixture.window_size(), cx).await;
                let skipped = cx.update(|cx| {
                    let replay = qust_ui_player::testing::run_actions(&job.fixture, &job.variant, &session, &parts.view, &recorder, cx);
                    let applied = qust_ui_playlist::harness::apply_actions(&job.fixture, &session, &parts.drawer, cx);
                    // What neither interpreter knew.
                    replay.skipped.iter().filter(|action| applied.skipped.iter().any(|s| action.to_string().contains(s.as_str()) || s.contains(&action.to_string()))).count()
                });
                cx.background_executor().timer(Duration::from_millis(80)).await;
                let path = cx.update(|cx| {
                    session.frames(3, cx);
                    let saved = session.capture(&job.out, cx).map(|_| job.out.clone());
                    qust_ui_playlist::harness::file_drop(&session, FileDropEvent::Exited, cx);
                    session.close(cx);
                    saved
                });
                results.borrow_mut().push(Done { job, path, skipped });
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });

    let mut summary = Vec::new();
    let (mut clean, mut total) = (0, 0);
    for done in done.borrow().iter() {
        total += 1;
        let job = &done.job;
        let path = match &done.path {
            Ok(path) => path,
            Err(error) => {
                println!("{:<26} {:<5} ERROR {error}", job.variant, job.theme);
                continue;
            }
        };
        let reference = job.fixture.render_path(&job.variant, &job.theme, 1.0);
        let mut regions = vec![("window", None), ("player", Some("0,0,860,267"))];
        if job.fixture.drawer {
            regions.push(("drawer", Some("0,267,860,320")));
        }
        let mut line = format!("{:<26} {:<5}", job.variant, job.theme);
        let mut ok = true;
        let mut reports = Vec::new();
        for (name, region) in regions {
            // compare.mjs writes <capture>.diff.png: one copy per region.
            let target = if name == "window" { path.clone() } else { path.with_extension(format!("{name}.png")) };
            if name != "window" {
                let _ = std::fs::copy(path, &target);
            }
            let extra: Vec<&str> = match region {
                Some(region) => vec!["--region", region],
                None => Vec::new(),
            };
            match capture::compare(&reference, &target, &extra) {
                Ok(c) => {
                    ok &= c.ok;
                    line += &format!("  {name} {} {:.3}% text {}/{}", if c.ok { "MATCH" } else { "DIFF " }, c.ratio * 100.0, c.text_ok, c.text_runs);
                    reports.push(serde_json::json!({ "region": name, "ok": c.ok, "ratio": c.ratio, "text_ok": c.text_ok, "text_runs": c.text_runs }));
                }
                Err(error) => {
                    ok = false;
                    line += &format!("  {name} compare: {}", error.trim());
                }
            }
        }
        if done.skipped > 0 {
            line += &format!("  ({} actions skipped)", done.skipped);
        }
        if ok {
            clean += 1;
        }
        println!("{line}");
        summary.push(serde_json::json!({ "variant": job.variant, "theme": job.theme, "ok": ok, "capture": path, "reports": reports }));
    }
    println!("{clean} of {total} captures clean");
    let _ = std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap_or_default());
}
