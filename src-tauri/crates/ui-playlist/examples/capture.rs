// Captures every player fixture that shows the drawer, with the drawer mounted as the app
// should mount it, and compares the drawer's region (y 267 to 587) with the reference renders.
//
//   cargo run -p qust-ui-playlist --example capture --features capture -- [fixture ...] [--out <dir>] [--themes dark,light] [--tolerance n]
//
// Hidden, unfocused, off-screen windows (qust-ui's harness); nothing shows on screen.

use qust_ui::capture::{self, Job};
use qust_ui::fixture::Fixture;
use std::path::PathBuf;

/// Every fixture that opens the drawer and whose drawer the reference shows (the dialog
/// fixtures cover it with their backdrop and belong to the dialog's owner).
const FIXTURES: &[&str] = &[
    "drawer-empty",
    "drawer-empty-no-cartridges",
    "drawer-entries",
    "drawer-playing-entry",
    "drawer-active-stopped",
    "drawer-select-single",
    "drawer-select-multi",
    "drawer-row-hover",
    "drawer-many",
    "drawer-many-scrolled",
    "drop-line-before",
    "drop-line-after",
    "drop-empty",
    "drop-over-rows",
    "reorder-drag",
    "drawer-long-durations",
    "accent-orange-drawer",
    "menu-drawer-empty",
    "menu-row-cartridge",
    "menu-row-track",
    "menu-row-tracks",
];

const REGION: &str = "0,267,860,320";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names: Vec<String> = Vec::new();
    let mut out = std::env::temp_dir().join("qust-playlist-capture");
    let mut themes = vec!["dark".to_string(), "light".to_string()];
    let mut tolerance: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = PathBuf::from(&args[i]);
            }
            "--tolerance" => {
                i += 1;
                tolerance = Some(args[i].clone());
            }
            "--themes" => {
                i += 1;
                themes = args[i].split(',').map(str::to_string).collect();
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
    let results = qust_ui_playlist::harness::capture_all(jobs.clone());
    let mut clean = 0;
    for (job, result) in jobs.iter().zip(results) {
        match result {
            Ok((path, scale, applied)) => {
                let reference = job.fixture.render_path(&job.variant, &job.theme, scale);
                let skipped = if applied.skipped.is_empty() { String::new() } else { format!("  (skipped: {:?})", applied.skipped) };
                let mut extra = vec!["--region", REGION];
                if let Some(tolerance) = &tolerance {
                    extra.extend(["--tolerance", tolerance.as_str()]);
                }
                match capture::compare(&reference, &path, &extra) {
                    Ok(c) => {
                        if c.ok {
                            clean += 1;
                        }
                        println!("{:<32} {:<5} {} pixels {:.3} %, text {}/{}{skipped}", job.variant, job.theme, if c.ok { "MATCH    " } else { "DIFFERENT" }, c.ratio * 100.0, c.text_ok, c.text_runs);
                    }
                    Err(e) => println!("{:<32} {:<5} compare: {e}", job.variant, job.theme),
                }
            }
            Err(error) => println!("{:<32} {:<5} {error}", job.variant, job.theme),
        }
    }
    println!("{clean}/{} clean; captures and diffs in {}", jobs.len(), out.display());
}
