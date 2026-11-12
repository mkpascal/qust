// Drives the engine from the command line, on the Null output unless --device is given (then at
// volume 0, so nothing is heard), and prints events, statuses, the tap and info() as it goes.
//
//   cargo run -p qust-audio --example probe -- [--device] [--quiet] SECONDS ACTION...
//
// Each ACTION is TIME=VERB,KEY[,ARG] (TIME in seconds from the start):
//   0=load,a,https://ice1.somafm.com/groovesalad-128-mp3   (or a path; ARG may be URL@START for a start position)
//   0=level,a,1   3=pause,a   5=resume,a   6=seek,a,30   9=unload,a   9=unloadall,-   2=volume,-,0.5
// Statuses print every second for every loaded key, and info() for every key at the end.

use qust_audio::{Config, Engine, Event, OutputKind, Source};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let device = take_flag(&mut args, "--device");
    let quiet = take_flag(&mut args, "--quiet");
    let Some(seconds) = args.first().and_then(|s| s.parse::<f64>().ok()) else {
        eprintln!("usage: probe [--device] [--quiet] SECONDS TIME=VERB,KEY[,ARG]...");
        std::process::exit(2);
    };
    let mut actions: Vec<(f64, String, String, String)> = args[1..]
        .iter()
        .filter_map(|action| {
            let (time, rest) = action.split_once('=')?;
            let mut parts = rest.splitn(3, ',');
            let verb = parts.next()?.to_string();
            let key = parts.next().unwrap_or("-").to_string();
            let arg = parts.next().unwrap_or("").to_string();
            Some((time.parse().ok()?, verb, key, arg))
        })
        .collect();
    actions.sort_by(|a, b| a.0.total_cmp(&b.0));

    let start = Instant::now();
    let first_audio = Arc::new(AtomicU64::new(0));
    let first = first_audio.clone();
    let engine = Engine::new(
        Config {
            codecs: symphonia::default::get_codecs(),
            output: if device { OutputKind::Device } else { OutputKind::Null },
            user_agent: "Qust-test/0.1".to_string(),
        },
        move |event| {
            let at = start.elapsed().as_secs_f64();
            if matches!(event, Event::Playing { .. }) {
                let _ = first.compare_exchange(0, (at * 1000.0) as u64, Ordering::Relaxed, Ordering::Relaxed);
            }
            println!("{at:7.3}  EVENT {event:?}");
        },
    );
    if device {
        engine.set_volume(0.0);
    }

    let tap = engine.tap();
    let (mut left, mut right) = (vec![0f32; 8192], vec![0f32; 8192]);
    let mut keys: Vec<String> = Vec::new();
    let mut next_status = 1.0;
    let mut index = 0;
    while start.elapsed().as_secs_f64() < seconds {
        let now = start.elapsed().as_secs_f64();
        while index < actions.len() && actions[index].0 <= now {
            let (_, verb, key, arg) = &actions[index];
            index += 1;
            println!("{now:7.3}  DO {verb} {key} {arg}");
            match verb.as_str() {
                "load" => {
                    let (target, at) = match arg.rsplit_once('@') {
                        Some((target, at)) if at.parse::<f64>().is_ok() => (target, at.parse().ok()),
                        _ => (arg.as_str(), None),
                    };
                    let source = if target.starts_with("http://") || target.starts_with("https://") {
                        Source::Url(target.to_string())
                    } else {
                        Source::File(target.into())
                    };
                    let id = engine.load(key, source, at);
                    println!("{now:7.3}  loaded {key} as {id:?}");
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
                "level" => engine.set_level(key, arg.parse().unwrap_or(1.0)),
                "pause" => engine.set_paused(key, true),
                "resume" => engine.set_paused(key, false),
                "seek" => engine.seek(key, arg.parse().unwrap_or(0.0)),
                "unload" => engine.unload(key),
                "unloadall" => engine.unload_all(),
                "volume" => engine.set_volume(arg.parse().unwrap_or(0.0)),
                "balance" => engine.set_balance(arg.parse().unwrap_or(0.0)),
                "duration" => {
                    let begun = Instant::now();
                    let result = qust_audio::probe_duration(std::path::Path::new(arg), symphonia::default::get_codecs());
                    println!("{now:7.3}  DURATION {arg}: {result:?} in {:.1} ms", begun.elapsed().as_secs_f64() * 1000.0);
                }
                _ => println!("unknown action {verb}"),
            }
        }
        if now >= next_status {
            next_status += 1.0;
            if !quiet {
                for key in engine.keys() {
                    if let Some(s) = engine.status(&key) {
                        println!(
                            "{now:7.3}  STATUS {key} id={} started={} live={} paused={} ended={} failed={} position={:.3} played={:.3} duration={:?} buffered={:.2} bytes={}",
                            s.id.0, s.started, s.live, s.paused, s.ended, s.failed, s.position, s.played, s.duration.map(|d| (d * 1000.0).round() / 1000.0), s.buffered,
                            qust_audio::info(&key)["bytes"]
                        );
                    }
                }
                match tap.read(576.0 / 44100.0, &mut left, &mut right) {
                    Some(rate) => {
                        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
                        let peak = left.iter().chain(&right).fold(0f32, |p, x| p.max(x.abs()));
                        println!("{now:7.3}  TAP rate={rate} rms={:.4}/{:.4} peak={peak:.4}", rms(&left), rms(&right));
                    }
                    None => println!("{now:7.3}  TAP none"),
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let first = first_audio.load(Ordering::Relaxed);
    if first > 0 {
        println!("first Playing at {:.3} s", first as f64 / 1000.0);
    }
    for key in &keys {
        println!("INFO {key} {}", qust_audio::info(key));
    }
    if let Some(cpu) = cpu_seconds() {
        let wall = start.elapsed().as_secs_f64();
        println!("CPU {cpu:.2} s in {wall:.1} s: {:.2}% of one core", cpu / wall * 100.0);
    }
}

// The process's user + kernel time, to see what the streams cost.
#[cfg(windows)]
fn cpu_seconds() -> Option<f64> {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime(u32, u32);
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(process: isize, creation: *mut FileTime, exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }
    let (mut creation, mut exit, mut kernel, mut user) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
    // SAFETY: plain out-parameters on the current process.
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) };
    let ticks = |t: &FileTime| ((t.1 as u64) << 32 | t.0 as u64) as f64 / 1e7;
    (ok != 0).then(|| ticks(&kernel) + ticks(&user))
}

#[cfg(not(windows))]
fn cpu_seconds() -> Option<f64> {
    None
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|arg| arg != flag);
    args.len() != before
}
