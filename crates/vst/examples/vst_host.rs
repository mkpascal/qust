// Loads VST3 plugins into the host and runs them on a tone, on the silent output: what each one
// reports, and what it did to the level. With `--edit`, opens the first one's window too (close
// it to finish).
//
//   cargo run -p qust-vst --example vst_host -- [--edit] [--seconds N] [--dir D] <path.vst3>...
//
// `--dir` keeps the settings file and the plugins' states there (run twice: the second run
// starts the plugins already listed, from their saved states, before adding any more).
//   cargo run -p qust-vst --example vst_host -- --installed      (lists what's installed)

use qust_audio::{Config, EffectSlot, Engine, OutputKind, Source};
use qust_player::Store;
use qust_vst::{Plugins, Snapshot};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn tone(path: &std::path::Path, seconds: u32) {
    let rate = 48000u32;
    let frames = rate * seconds;
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + frames * 4).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&rate.to_le_bytes());
    data.extend_from_slice(&(rate * 4).to_le_bytes());
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&(frames * 4).to_le_bytes());
    for i in 0..frames {
        let s = ((i as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 0.25 * 32767.0) as i16;
        data.extend_from_slice(&s.to_le_bytes());
        data.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, data).unwrap();
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--installed") {
        for path in qust_vst::installed() {
            println!("{}", path.display());
        }
        return;
    }
    let edit = args.iter().position(|a| a == "--edit").map(|i| args.remove(i)).is_some();
    let seconds = match args.iter().position(|a| a == "--seconds") {
        Some(i) => {
            args.remove(i);
            args.remove(i).parse().unwrap_or(4)
        }
        None => 4u64,
    };
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => {
            args.remove(i);
            PathBuf::from(args.remove(i))
        }
        None => std::env::temp_dir().join(format!("qust-vst-host-{}", std::process::id())),
    };
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("settings.json"));
    let slot = EffectSlot::new();
    let last = Arc::new(Mutex::new(Snapshot::default()));
    let seen = last.clone();
    let start = Instant::now();
    let plugins = Plugins::start(store.clone(), slot.clone(), move |snapshot| {
        println!("{:6.2}s {:?}", start.elapsed().as_secs_f64(), snapshot);
        *seen.lock().unwrap() = snapshot;
    });
    let engine = Engine::new(Config { codecs: symphonia::default::get_codecs(), output: OutputKind::Null, user_agent: "Qust-test".into(), effects: Some(slot.clone()) }, |_| {});
    if !args.is_empty() {
        plugins.add(args.iter().map(PathBuf::from).collect());
    }
    let wav = dir.join("tone.wav");
    tone(&wav, seconds as u32 + 2);
    engine.load("a", Source::File(wav), None);
    engine.set_level("a", 1.0);
    if edit {
        std::thread::sleep(Duration::from_millis(1500));
        if let Some(row) = last.lock().unwrap().plugins.first() {
            plugins.open_editor(row.id, None);
        }
    }
    let tap = engine.tap();
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until || (edit && last.lock().unwrap().plugins.iter().any(|row| row.editor_open)) {
        std::thread::sleep(Duration::from_millis(500));
        let (mut left, mut right) = (vec![0.0; 4800], vec![0.0; 4800]);
        if tap.read(0.0, &mut left, &mut right).is_some() {
            let rms = (left.iter().chain(&right).map(|s| s * s).sum::<f32>() / 9600.0).sqrt();
            println!("{:6.2}s tap rms {:.4} (dry tone: 0.1768)", start.elapsed().as_secs_f64(), rms);
        }
    }
    drop(engine);
    std::thread::sleep(Duration::from_millis(300));
    plugins.shutdown();
    println!("saved list: {}", store.get("audio-plugins").map(|v| v.to_string()).unwrap_or_default());
    let states: Vec<_> = std::fs::read_dir(dir.join("audio-plugins")).map(|d| d.flatten().map(|e| (e.file_name(), e.metadata().map(|m| m.len()).unwrap_or(0))).collect()).unwrap_or_default();
    println!("states: {states:?}");
}
