// What an idle process costs, for choosing the background (tray-only) mode. One mode per run:
//
//   win32        a hidden window and a plain message loop: what a tray-only process without GPUI costs
//   windowed     a GPUI app started the default way (windowed), with no window open
//   headless     a GPUI app started headless, with no window open
//   cycle        headless, then windowed with one hidden window for 3 s, then headless again
//   shell        the qust-shell tray thread (hidden windows, hotkey, no tray icon) inside a headless GPUI app
//
// After a settle time, it samples the process for `--seconds` (default 60): CPU time, context
// switches (all threads, i.e. wakeups), private bytes and working set, and prints one JSON line.
//
//   cargo run -p qust-shell --features gpui-examples --example idle --release -- headless --seconds 60

use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_else(|| "headless".into());
    let seconds = args
        .iter()
        .position(|arg| arg == "--seconds")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);
    let settle = Duration::from_secs(5);

    // The sampler runs on its own thread and ends the process when done.
    let sample_mode = mode.clone();
    let start_sampler = move || {
        std::thread::spawn(move || {
            std::thread::sleep(settle);
            let before = qust_shell::diag::sample();
            let started = Instant::now();
            std::thread::sleep(Duration::from_secs(seconds));
            let after = qust_shell::diag::sample();
            let elapsed = started.elapsed().as_secs_f64();
            // Everything but this sampling thread.
            let (wakeups, cpu_ms) = after.activity_since(&before, qust_shell::diag::current_thread());
            let cpu = cpu_ms / 1000.0;
            println!(
                "{}",
                serde_json::json!({
                    "mode": sample_mode,
                    "seconds": elapsed,
                    "cpu_seconds": cpu,
                    "cpu_percent_of_one_core": cpu / elapsed * 100.0,
                    "process_cpu_seconds": (after.cpu_ms - before.cpu_ms) / 1000.0,
                    "context_switches": wakeups,
                    "wakeups_per_second": wakeups as f64 / elapsed,
                    "private_mb": after.private_bytes as f64 / 1e6,
                    "working_set_mb": after.working_set as f64 / 1e6,
                    "threads": after.threads,
                    "busiest_threads": after.busiest(&before, 4),
                })
            );
            std::process::exit(0);
        });
    };

    match mode.as_str() {
        "win32" => {
            start_sampler();
            qust_shell::diag::plain_message_loop();
        }
        "windowed" | "headless" | "cycle" | "shell" => run_gpui(&mode, start_sampler),
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    }
}

fn run_gpui(mode: &str, start_sampler: impl FnOnce() + 'static) {
    use gpui::*;

    let mut app = gpui_platform::application().with_quit_mode(QuitMode::Explicit);
    if mode != "windowed" {
        app = app.with_windowing(WindowingRequest::Headless);
    }
    let mode = mode.to_string();
    app.run(move |cx: &mut App| match mode.as_str() {
        "cycle" => {
            cx.spawn(async move |cx| {
                let windowed = cx.update(|cx| cx.request_windowing(WindowingRequest::Windowed(GraphicalEnvironment::detect())));
                if let Err(error) = windowed.await {
                    eprintln!("windowed: {error}");
                }
                let window = cx.update(|cx| {
                    let options = WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(-20000.), px(-20000.)), size(px(400.), px(200.))))),
                        focus: false,
                        show: false,
                        ..Default::default()
                    };
                    cx.open_window(options, |_, cx| cx.new(|_| Blank))
                });
                cx.background_executor().timer(Duration::from_secs(3)).await;
                if let Ok(window) = window {
                    let _ = window.update(cx, |_, window, _| window.remove_window());
                }
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let headless = cx.update(|cx| cx.request_windowing(WindowingRequest::Headless));
                if let Err(error) = headless.await {
                    eprintln!("headless: {error}");
                }
                cx.update(|_| start_sampler());
            })
            .detach();
        }
        "shell" => {
            let shell = qust_shell::NativeThread::start().expect("shell thread");
            std::mem::forget(shell);
            start_sampler();
        }
        _ => start_sampler(),
    });
}

struct Blank;

impl gpui::Render for Blank {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, rgb, ParentElement, Styled};
        div().size_full().bg(rgb(0x202020)).text_color(rgb(0xffffff)).child("idle")
    }
}
