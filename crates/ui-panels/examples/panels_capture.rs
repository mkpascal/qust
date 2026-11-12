// Captures the Settings and Stream information fixtures and compares each with its reference
// render (hidden windows: nothing appears on screen).
//
//   cargo run -p qust-ui-panels --features capture --example panels_capture -- [name-or-prefix ...] [--themes dark,light] [--out dir] [--verbose]
//
// With no names: every `settings-*` and `info-*` fixture, in both themes. Prints one line per
// render (MATCH / DIFFERENT, non-text pixels different, text runs), the worst regions and the text
// runs that differ; the diff pictures are written next to the captures.

use qust_ui::capture::{self, Job};
use qust_ui::fixture::{self, Fixture};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names = Vec::new();
    let mut themes = vec!["dark".to_string(), "light".to_string()];
    let mut out = std::env::temp_dir().join("qust-panels-capture");
    let mut verbose = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--themes" => {
                i += 1;
                themes = args.get(i).map(|t| t.split(',').map(str::to_string).collect()).unwrap_or_default();
            }
            // Compare with reference/renders-gray (grayscale text, as the real window drew most).
            "--gray" => std::env::set_var("QUST_REFERENCE_SET", "renders-gray"),
            "--out" => {
                i += 1;
                if let Some(dir) = args.get(i) {
                    out = PathBuf::from(dir);
                }
            }
            "--verbose" => verbose = true,
            name => names.push(name.to_string()),
        }
        i += 1;
    }
    // Every fixture of the two windows (`settings-*`, `info-*`, and others such as `text-rtl-info`).
    let all: Vec<String> = fixture::all().into_iter().filter(|n| n.starts_with("settings-") || n.starts_with("info-") || Fixture::load(n).is_ok_and(|f| f.window == "settings" || f.window == "info")).collect();
    let chosen: Vec<String> = if names.is_empty() { all } else { all.into_iter().filter(|n| names.iter().any(|p| n == p || n.starts_with(p.as_str()))).collect() };
    let mut jobs = Vec::new();
    for name in &chosen {
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
    let results = qust_ui_panels::harness::capture_all(jobs.clone());
    let (mut clean, mut total) = (0, 0);
    for (job, result) in jobs.iter().zip(results) {
        total += 1;
        let label = format!("{}.{}", job.variant, job.theme);
        let (path, scale) = match result {
            Ok(done) => done,
            Err(error) => {
                println!("{label:<48} ERROR {error}");
                continue;
            }
        };
        let reference = job.fixture.render_path(&job.variant, &job.theme, scale);
        match capture::compare(&reference, &path, &[]) {
            Ok(c) => {
                if c.ok {
                    clean += 1;
                }
                println!("{label:<48} {:<9} pixels {:.3} %  text {}/{}", if c.ok { "MATCH" } else { "DIFFERENT" }, c.ratio * 100.0, c.text_ok, c.text_runs);
                if verbose || !c.ok {
                    let regions = c.report["regions"].as_array().cloned().unwrap_or_default();
                    for region in regions.iter().filter(|r| r["different"].as_u64().unwrap_or(0) > 0).take(if verbose { 12 } else { 5 }) {
                        println!("    region {:>7.2} % ({} px) {}", region["ratio"].as_f64().unwrap_or(0.0) * 100.0, region["different"], region["name"].as_str().unwrap_or(""));
                    }
                    let runs = c.report["textRuns"].as_array().cloned().unwrap_or_default();
                    for run in runs.iter().filter(|r| r["ok"] != true) {
                        let problems: Vec<String> = run["problems"].as_array().map(|p| p.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        println!("    text {} {:?} ({}): {}", run["status"].as_str().unwrap_or(""), run["text"].as_str().unwrap_or(""), run["element"].as_str().unwrap_or(""), problems.join("; "));
                    }
                }
            }
            Err(error) => println!("{label:<48} compare: {error}"),
        }
    }
    println!("\n{clean} of {total} renders match. Captures and diffs in {}", out.display());
}
