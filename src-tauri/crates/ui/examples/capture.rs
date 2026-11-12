// Captures a reference fixture with a view and compares it with the reference render.
//
//   cargo run -p qust-ui --example capture --features capture -- <fixture> [dark|light] [out-dir]
//
// The view here is the analyzer alone, laid out where the player window has it (its slot at CSS
// 45, 58 over the screen's color), the rest of the window left at --bg: a window crate copies
// this file and returns its own view from `build`.

use gpui::{div, prelude::*, px, AnyView, App, Window};
use qust_ui::capture::{self, Job, Setup};
use qust_ui::fixture::Fixture;
use qust_ui::{analyzer_slot, AnalyzerView};
use std::path::PathBuf;

struct Demo {
    root: gpui::WeakEntity<qust_ui::WindowRoot>,
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = qust_fluent::theme(cx);
        // The screen's background where the canvas sits (#screen is 33,17 99×69 at 100 %).
        div().size_full().child(
            div().absolute().left(px(33.0)).top(px(17.0)).w(px(99.0)).h(px(69.0)).rounded(px(6.0)).bg(theme.display).child(
                div().absolute().left(px(12.0)).top(px(41.0)).child(analyzer_slot(self.root.clone(), window)),
            ),
        )
    }
}

fn build(setup: &Setup, _: &mut Window, cx: &mut App) -> AnyView {
    let root = setup.root.clone();
    let analyzer = cx.new(|cx| AnalyzerView::new(&setup.models.analyzer, cx));
    if let Some(root_entity) = root.upgrade() {
        root_entity.update(cx, |root, cx| root.set_live("analyzer", analyzer, cx));
    }
    cx.new(|_| Demo { root }).into()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().cloned().unwrap_or_else(|| "colors-classic".into());
    let theme = args.get(1).cloned().unwrap_or_else(|| "dark".into());
    let out = args.get(2).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("qust-capture"));
    let fixture = match Fixture::load(&name) {
        Ok(fixture) => fixture,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let jobs = Job::all(&fixture, &theme, &out);
    let results = capture::capture_all(jobs.clone(), build);
    for (job, result) in jobs.iter().zip(results) {
        match result {
            Ok((path, scale)) => {
                let reference = fixture.render_path(&job.variant, &job.theme, scale);
                println!("{} at {scale}: {}", job.variant, path.display());
                match capture::compare(&reference, &path, &["--region", "#canvas"]) {
                    Ok(c) => println!("  {} pixels {:.3} % different, text {}/{}", if c.ok { "MATCH" } else { "DIFFERENT" }, c.ratio * 100.0, c.text_ok, c.text_runs),
                    Err(e) => println!("  compare: {e}"),
                }
            }
            Err(error) => println!("{}: {error}", job.variant),
        }
    }
}
