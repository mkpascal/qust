// The engine end to end on the Null output, with WAV files written here (no network).

use qust_audio::{Config, Engine, Event, OutputKind, Source};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn wav(name: &str, seconds: f64, rate: u32) -> PathBuf {
    write_wav(name, seconds, rate, |i, rate| (i as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 0.5)
}

// A ramp from -1 to 1 over the file: each sample says where in the file it is.
fn ramp(name: &str, seconds: f64, rate: u32) -> PathBuf {
    let frames = seconds * rate as f64;
    write_wav(name, seconds, rate, move |i, _| i as f64 / frames * 2.0 - 1.0)
}

fn write_wav(name: &str, seconds: f64, rate: u32, sample: impl Fn(u32, u32) -> f64) -> PathBuf {
    let frames = (seconds * rate as f64) as u32;
    let mut data = Vec::with_capacity(44 + frames as usize * 4);
    let bytes = frames * 4;
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&rate.to_le_bytes());
    data.extend_from_slice(&(rate * 4).to_le_bytes());
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for i in 0..frames {
        let sample = (sample(i, rate) * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }
    let path = std::env::temp_dir().join(format!("qust-audio-{}-{name}.wav", std::process::id()));
    std::fs::write(&path, data).expect("write a test file");
    path
}

fn engine() -> (Engine, mpsc::Receiver<(Instant, Event)>) {
    let (send, receive) = mpsc::channel();
    let engine = Engine::new(
        Config { codecs: symphonia::default::get_codecs(), output: OutputKind::Null, user_agent: "Qust-test".into() },
        move |event| {
            let _ = send.send((Instant::now(), event));
        },
    );
    (engine, receive)
}

fn next(events: &mpsc::Receiver<(Instant, Event)>) -> (Instant, Event) {
    events.recv_timeout(Duration::from_secs(5)).expect("an event")
}

#[test]
fn plays_seeks_and_ends() {
    let path = wav("seek", 2.0, 44100);
    let (engine, events) = engine();
    let start = Instant::now();
    let id = engine.load("file:a", Source::File(path.clone()), None);
    engine.set_level("file:a", 1.0);

    match next(&events).1 {
        Event::Duration { id: got, seconds: Some(seconds), .. } => {
            assert_eq!(got, id);
            assert!((seconds - 2.0).abs() < 0.01, "{seconds}");
        }
        other => panic!("{other:?}"),
    }
    let (at, event) = next(&events);
    assert!(matches!(event, Event::Playing { .. }), "{event:?}");
    assert!(at - start < Duration::from_millis(300), "started after {:?}", at - start);

    std::thread::sleep(Duration::from_millis(300));
    let status = engine.status("file:a").expect("a status");
    assert!(status.live && status.started && !status.ended);
    assert!((status.played - status.position).abs() < 0.05);
    let info = qust_audio::info("file:a");
    assert_eq!(info["format"], "WAV");
    assert_eq!(info["samplerate"], 44100);

    // A seek shows at once, plays on without a stall, and the end comes when it should.
    engine.seek("file:a", 1.5);
    assert_eq!(engine.status("file:a").map(|s| s.position), Some(1.5));
    let seeked = Instant::now();
    let (at, event) = next(&events);
    assert!(matches!(event, Event::Ended { .. }), "{event:?}");
    let left = (at - seeked).as_secs_f64();
    assert!((0.4..0.7).contains(&left), "ended {left} s after the seek");
    let status = engine.status("file:a").expect("a status");
    assert!(status.ended && !status.live && (status.position - 2.0).abs() < 0.01);

    // Seeking back into an ended file plays it again.
    engine.seek("file:a", 1.8);
    let (_, event) = next(&events);
    assert!(matches!(event, Event::Ended { .. }), "{event:?}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn muted_streams_keep_real_time_and_unloading_is_silent() {
    let path = wav("muted", 3.0, 48000);
    let (engine, events) = engine();
    engine.load("file:muted", Source::File(path.clone()), None);
    let started = loop {
        if let (at, Event::Playing { .. }) = next(&events) {
            break at;
        }
    };
    std::thread::sleep(Duration::from_millis(1000));
    let status = engine.status("file:muted").expect("a status");
    let elapsed = started.elapsed().as_secs_f64();
    assert!((status.played - elapsed).abs() < 0.06, "played {} in {elapsed}", status.played);

    // Replacing gives a new stream; the old one says nothing more.
    let first = status.id;
    let second = engine.load("file:muted", Source::File(path.clone()), Some(1.0));
    assert_ne!(first, second);
    loop {
        match next(&events).1 {
            Event::Playing { id, .. } => {
                assert_eq!(id, second);
                break;
            }
            Event::Duration { id, .. } => assert_eq!(id, second),
            other => panic!("{other:?}"),
        }
    }
    assert!(engine.status("file:muted").is_some_and(|s| s.position >= 1.0));
    engine.unload_all();
    assert!(engine.keys().is_empty());
    assert!(events.recv_timeout(Duration::from_millis(300)).is_err());

    // A file that isn't there fails.
    engine.load("file:missing", Source::File(path.with_extension("none")), None);
    assert!(matches!(next(&events).1, Event::Failed { .. }));
    let _ = std::fs::remove_file(path);
}

// What is heard (the tap at lead 0) matches the reported position, after a seek too, with and
// without resampling.
#[test]
fn position_matches_the_audio() {
    for rate in [48000, 44100] {
        let path = ramp(&format!("ramp{rate}"), 4.0, rate);
        let (engine, events) = engine();
        engine.load("file:ramp", Source::File(path.clone()), Some(1.0));
        engine.set_level("file:ramp", 1.0);
        while !matches!(next(&events).1, Event::Playing { .. }) {}
        let tap = engine.tap();
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        for seek in [None, Some(2.5)] {
            if let Some(at) = seek {
                engine.seek("file:ramp", at);
            }
            std::thread::sleep(Duration::from_millis(400));
            let position = engine.status("file:ramp").expect("a status").position;
            tap.read(0.0, &mut left, &mut right);
            let heard = (left[63] as f64 + 1.0) / 2.0 * 4.0;
            assert!((heard - position).abs() < 0.03, "{rate} Hz, seek {seek:?}: heard {heard:.4} s, position {position:.4} s");
        }
        drop(engine);
        let _ = std::fs::remove_file(path);
    }
}

// The output is open only while something is loaded, and closes 3 s after the last stream goes.
#[test]
fn output_opens_on_load_and_closes_when_idle() {
    let path = wav("idle", 2.0, 48000);
    let (engine, events) = engine();
    let tap = engine.tap();
    let (mut left, mut right) = (vec![0.0; 16], vec![0.0; 16]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(tap.read(0.0, &mut left, &mut right), None, "open before anything was loaded");

    let start = Instant::now();
    engine.load("file:idle", Source::File(path.clone()), None);
    while !matches!(next(&events).1, Event::Playing { .. }) {}
    assert!(start.elapsed() < Duration::from_millis(200), "first audio after {:?}", start.elapsed());
    assert_eq!(tap.read(0.0, &mut left, &mut right), Some(48000));

    engine.unload("file:idle");
    let gone = Instant::now();
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(tap.read(0.0, &mut left, &mut right), Some(48000), "closed before 3 s");
    while tap.read(0.0, &mut left, &mut right).is_some() {
        assert!(gone.elapsed() < Duration::from_millis(3600), "still open after {:?}", gone.elapsed());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(gone.elapsed() >= Duration::from_millis(2990), "closed after {:?}", gone.elapsed());

    // Loading again reopens at once.
    let start = Instant::now();
    engine.load("file:idle", Source::File(path.clone()), None);
    while !matches!(next(&events).1, Event::Playing { .. }) {}
    assert!(start.elapsed() < Duration::from_millis(200), "first audio after {:?}", start.elapsed());
    let _ = std::fs::remove_file(path);
}
