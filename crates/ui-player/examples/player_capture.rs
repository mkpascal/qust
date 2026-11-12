// Captures the player window's part above the drawer for every reference fixture of the player
// window (or those named), in dark and light, and compares each with its reference render.
//
//   cargo run -p qust-ui-player --example player_capture --features capture -- [names or prefixes*] [--themes dark,light] [--out <dir>]
//
// Windows are hidden and never activated (qust_ui::capture). The comparison covers y < 267 (what
// this crate draws); with the dialog over an open drawer, the dialog card too. Menu fixtures also
// compare the menu the view built with the one the page built. A summary goes to
// <out>/summary.json.

use gpui::App;
use qust_ui::capture::{self, Job, Session};
use qust_ui::fixture::{self, element_box, layout, Fixture};
use qust_ui_player::testing::{self, Recorder};
use qust_ui_player::PlayerView;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

struct Done {
    job: Job,
    path: Result<PathBuf, String>,
    menus_ok: Option<bool>,
    menus: String,
}

fn wanted(name: &str, patterns: &[String]) -> bool {
    patterns.is_empty() || patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == p,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut patterns = Vec::new();
    let mut themes = vec!["dark".to_string(), "light".to_string()];
    let mut out = std::env::temp_dir().join("qust-ui-player-capture");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--themes" => {
                i += 1;
                themes = args.get(i).map(|t| t.split(',').map(str::to_string).collect()).unwrap_or(themes);
            }
            // Compare with reference/renders-gray (grayscale text, as the real window drew most).
            "--gray" => std::env::set_var("QUST_REFERENCE_SET", "renders-gray"),
            "--out" => {
                i += 1;
                if let Some(dir) = args.get(i) {
                    out = PathBuf::from(dir);
                }
            }
            name => patterns.push(name.to_string()),
        }
        i += 1;
    }
    let mut jobs = Vec::new();
    for name in fixture::all() {
        if !wanted(&name, &patterns) {
            continue;
        }
        let Ok(fixture) = Fixture::load(&name) else { continue };
        if fixture.window != "player" {
            continue;
        }
        for theme in &themes {
            if fixture.themes.contains(theme) {
                jobs.extend(Job::all(&fixture, theme, &out));
            }
        }
    }
    let done = Rc::new(RefCell::new(Vec::<Done>::new()));
    let results = done.clone();
    let count = jobs.len();
    capture::run(move |cx: &mut App| {
        cx.spawn(async move |cx| {
            for job in jobs {
                let recorder = Recorder::new();
                let player: Rc<RefCell<Option<gpui::Entity<PlayerView>>>> = Rc::default();
                let opened = cx.update(|cx| {
                    testing::install_forced(&job.fixture, cx);
                    testing::install_errors(&job.fixture, &recorder);
                    let slot = player.clone();
                    let rec = recorder.clone();
                    let session = Session::open(&job, cx, move |setup, window, cx| {
                        let (view, entity) = testing::build(setup, &rec, window, cx);
                        *slot.borrow_mut() = Some(entity);
                        view
                    })?;
                    testing::hide_caret(cx);
                    session.frames(3, cx);
                    Ok::<Session, String>(session)
                });
                let session = match opened {
                    Ok(session) => session,
                    Err(error) => {
                        results.borrow_mut().push(Done { job, path: Err(error), menus_ok: None, menus: String::new() });
                        continue;
                    }
                };
                let Some(view) = player.borrow().clone() else { continue };
                testing::wait_for_size(&session, job.fixture.window_size(), cx).await;
                let replay = cx.update(|cx| testing::run_actions(&job.fixture, &job.variant, &session, &view, &recorder, cx));
                // Icons load on first use, and answers from the stand-in resolve on the executor.
                cx.background_executor().timer(Duration::from_millis(80)).await;
                let (path, menus_ok, menus) = cx.update(|cx| {
                    session.frames(3, cx);
                    let saved = session.capture(&job.out, cx).map(|_| job.out.clone());
                    session.close(cx);
                    let reference = testing::reference_menus(&job.fixture, &job.variant);
                    let ours: Vec<_> = replay
                        .menus
                        .iter()
                        .map(|m| qust_ui_player::menus::rows(m).into_iter().map(|r| r.map(|(t, e, a)| (t, e, a.is_some()))).collect::<Vec<_>>())
                        .collect();
                    // The drawer's menus are the drawer's (qust-ui-playlist).
                    let drawer_menu = job.fixture.actions.iter().any(|a| a["contextmenu"].as_str().or(a["contextmenu"]["selector"].as_str()).is_some_and(|s| s.contains("playlist")));
                    let menus_ok = (!drawer_menu && (!reference.is_empty() || !ours.is_empty())).then(|| reference == ours);
                    (saved, menus_ok, format!("{ours:?}"))
                });
                results.borrow_mut().push(Done { job, path, menus_ok, menus });
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
                println!("{:<34} {:<5} ERROR {error}", job.variant, job.theme);
                continue;
            }
        };
        let reference = job.fixture.render_path(&job.variant, &job.theme, 1.0);
        let mut regions = vec!["0,0,860,267".to_string()];
        if job.fixture.drawer {
            if let Some(b) = layout(&job.fixture.window, &job.variant).and_then(|l| element_box(&l, "#dialogForm", 1.0)) {
                if b[2] > 0.0 {
                    regions = vec![format!("{},{},{},{}", b[0], b[1], b[2], b[3])];
                    regions.push("0,0,860,170".to_string());
                }
            }
        }
        let mut line = format!("{:<34} {:<5}", job.variant, job.theme);
        let mut ok = true;
        let mut reports = Vec::new();
        for (n, region) in regions.iter().enumerate() {
            let target = if n == 0 { path.clone() } else { path.with_extension(format!("r{n}.png")) };
            if n > 0 {
                let _ = std::fs::copy(path, &target);
            }
            match capture::compare(&reference, &target, &["--region", region]) {
                Ok(c) => {
                    ok &= c.ok;
                    line += &format!("  {} px {:.3}% text {}/{}", if c.ok { "MATCH" } else { "DIFF " }, c.ratio * 100.0, c.text_ok, c.text_runs);
                    let worst: Vec<String> = c.report["regions"].as_array().map(|r| r.iter().filter(|x| x["different"].as_u64().unwrap_or(0) > 0).take(4).map(|x| format!("{}={}", x["name"].as_str().unwrap_or(""), x["different"])).collect()).unwrap_or_default();
                    let bad_text: Vec<String> = c.report["text"]["items"].as_array().or(c.report["text"]["runs_detail"].as_array()).map(|r| r.iter().filter(|x| x["ok"] == false).take(4).map(|x| x["text"].as_str().unwrap_or("").chars().take(24).collect()).collect()).unwrap_or_default();
                    if !worst.is_empty() {
                        line += &format!(" [{}]", worst.join(" "));
                    }
                    if !bad_text.is_empty() {
                        line += &format!(" text✗ {:?}", bad_text);
                    }
                    reports.push(serde_json::json!({ "region": region, "ok": c.ok, "ratio": c.ratio, "text_ok": c.text_ok, "text_runs": c.text_runs }));
                }
                Err(error) => {
                    ok = false;
                    line += &format!("  compare: {}", error.trim());
                }
            }
        }
        if let Some(menus_ok) = done.menus_ok {
            ok &= menus_ok;
            line += if menus_ok { "  menus ok" } else { "  MENUS DIFFER" };
            if !menus_ok {
                line += &format!(" {}", done.menus);
            }
        }
        if ok {
            clean += 1;
        }
        println!("{line}");
        summary.push(serde_json::json!({ "variant": job.variant, "theme": job.theme, "ok": ok, "capture": path, "reports": reports, "menus_ok": done.menus_ok }));
    }
    println!("{clean} of {total} captures clean ({count} jobs)");
    let _ = std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap_or_default());
}
