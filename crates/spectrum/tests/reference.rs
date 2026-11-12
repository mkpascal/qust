// Compares qust-spectrum with the page's analyzer it was ported from: tests/reference/frames.txt
// holds what tests/reference/visualizer.js drew for synthetic audio (made by tests/reference.mjs,
// which explains the format). This rebuilds the same audio, sample for sample, and expects every
// frame byte for byte.

use qust_spectrum::{Spectrum, FRAME_LEN, WAVE_LEN};

const FIXTURE: &str = include_str!("reference/frames.txt");

// The same signals as tests/reference.mjs, with the same operations in the same order.

fn wave(p: f64) -> f64 {
    let u = 2.0 * p - 1.0;
    let y = 4.0 * u * (1.0 - u.abs());
    0.225 * (y * y.abs() - y) + y
}

fn noise(n: i64, seed: u32) -> f64 {
    let mut h = n as u32 ^ seed;
    h = (h ^ (h >> 16)).wrapping_mul(0x45d9f3b);
    h = (h ^ (h >> 16)).wrapping_mul(0x45d9f3b);
    h ^= h >> 16;
    h as f64 / 4294967296.0 * 2.0 - 1.0
}

enum Component {
    Sine(f64, f64),
    Sweep(f64, f64, f64, f64),
    Noise(u32, f64),
}

fn parse_signal(spec: &str) -> Vec<Component> {
    if spec == "zero" {
        return Vec::new();
    }
    spec.split('+')
        .map(|part| {
            let mut words = part.split_whitespace();
            let kind = words.next().unwrap();
            let args: Vec<f64> = words.map(|w| w.parse().unwrap()).collect();
            match kind {
                "sine" => Component::Sine(args[0], args[1]),
                "sweep" => Component::Sweep(args[0], args[1], args[2], args[3]),
                "noise" => Component::Noise(args[0] as u32, args[1]),
                _ => panic!("unknown signal {kind}"),
            }
        })
        .collect()
}

fn sample(components: &[Component], n: i64, rate: f64) -> f32 {
    let mut sum = 0.0;
    let x = n as f64;
    for component in components {
        match *component {
            Component::Sine(f, a) => sum += wave((x * f / rate) % 1.0) * a,
            Component::Sweep(f0, f1, dur, a) => {
                let tt = (x / rate) % dur;
                sum += wave((f0 * tt + (f1 - f0) * tt * tt / (2.0 * dur)) % 1.0) * a;
            }
            Component::Noise(seed, a) => sum += noise(n, seed) * a,
        }
    }
    sum as f32
}

fn unhex(text: &str) -> [u8; FRAME_LEN] {
    assert_eq!(text.len(), FRAME_LEN * 2, "frame length");
    let mut frame = [0; FRAME_LEN];
    for (i, byte) in frame.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap();
    }
    frame
}

fn fnv(bits: &[u64]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bits {
        h ^= b;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[test]
fn tables_match_v8() {
    let line = FIXTURE.lines().find(|l| l.starts_with("tables ")).expect("tables line");
    let words: Vec<&str> = line.split_whitespace().skip(1).collect();
    let bits = Spectrum::new().table_bits();
    // Same order and sizes as table_bits().
    let sizes = [1152, 288, 288, 288, 288, 16, 16, 288, 288, 81, 81, 1];
    let mut at = 0;
    let mut wrong = Vec::new();
    for (pair, size) in words.chunks(2).zip(sizes) {
        if fnv(&bits[at..at + size]) != pair[1] {
            wrong.push(pair[0]);
        }
        at += size;
    }
    assert_eq!(at, bits.len());
    // A libm that differs from V8's in the last bit would show here first. The frames can still
    // match (a last-bit difference rarely moves a rounding), but it's worth knowing.
    assert!(wrong.is_empty(), "tables differ from V8's: {wrong:?}");
}

#[test]
fn frames_match_the_page() {
    let mut spectrum = Spectrum::new();
    let mut rate = 0.0;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut wired = true;
    let mut scenario = "";
    let mut previous = [0; FRAME_LEN];
    let mut frames = 0;
    let mut mismatches = Vec::new();
    for line in FIXTURE.lines() {
        let mut words = line.splitn(2, ' ');
        let head = words.next().unwrap();
        let rest = words.next().unwrap_or("");
        match head {
            "scenario" => {
                let mut parts = rest.split_whitespace();
                scenario = parts.next().unwrap();
                rate = parts.next().unwrap().parse().unwrap();
                spectrum = Spectrum::new();
                wired = true;
            }
            "left" => left = parse_signal(rest),
            "right" => right = parse_signal(rest),
            "step" => {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let dt: f64 = parts[0].parse().unwrap();
                let pos: i64 = parts[1].parse().unwrap();
                if parts[2] != "-" {
                    for event in parts[2].split(',') {
                        match event {
                            "L1" => spectrum.set_loading(true),
                            "L0" => spectrum.set_loading(false),
                            "O1" => spectrum.set_off(true),
                            "O0" => spectrum.set_off(false),
                            "R" => spectrum.reset(),
                            "W1" => wired = true,
                            "W0" => wired = false,
                            _ => panic!("unknown event {event}"),
                        }
                    }
                }
                let expected = if parts[3] == "=" { previous } else { unhex(parts[3]) };
                previous = expected;
                assert_eq!(dt as f32 as f64, dt, "dt must be an f32");
                let mut reads = 0;
                let frame = spectrum.tick(dt as f32, &mut |l: &mut [f32], r: &mut [f32]| {
                    reads += 1;
                    assert_eq!((l.len(), r.len()), (WAVE_LEN, WAVE_LEN));
                    if !wired {
                        return None;
                    }
                    let first = pos - WAVE_LEN as i64;
                    for k in 0..WAVE_LEN {
                        l[k] = sample(&left, first + k as i64, rate);
                        r[k] = sample(&right, first + k as i64, rate);
                    }
                    Some(rate as f32)
                });
                assert!(reads <= 1, "read more than once in a tick");
                frames += 1;
                if frame != expected {
                    mismatches.push(format!("{scenario} frame {frames}\n  js:   {expected:?}\n  rust: {frame:?}"));
                }
            }
            _ => {}
        }
    }
    assert!(frames > 2000, "only {frames} frames");
    assert!(mismatches.is_empty(), "{} of {frames} frames differ:\n{}", mismatches.len(), mismatches[..mismatches.len().min(5)].join("\n"));
    println!("{frames} frames identical");
}

#[test]
fn idle_after_stop_and_after_silence() {
    let mut spectrum = Spectrum::new();
    assert!(spectrum.idle());
    let mut loud = |l: &mut [f32], r: &mut [f32]| {
        for (k, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            *a = sample(&[Component::Sine(440.0, 0.8), Component::Noise(1, 0.3)], k as i64, 48000.0);
            *b = *a;
        }
        Some(48000.0)
    };
    for _ in 0..30 {
        spectrum.tick(1.0 / 60.0, &mut loud);
    }
    assert!(!spectrum.idle());
    // Silence: the bars fall and the peaks drop, then nothing is lit.
    let mut silence = |l: &mut [f32], r: &mut [f32]| {
        l.fill(0.0);
        r.fill(0.0);
        Some(48000.0)
    };
    let mut ticks = 0;
    while !spectrum.idle() {
        let frame = spectrum.tick(1.0 / 60.0, &mut silence);
        ticks += 1;
        assert!(ticks < 600, "never idle");
        if spectrum.idle() {
            assert_eq!(frame, [0; FRAME_LEN]);
        }
    }
    assert_eq!(spectrum.tick(1.0 / 60.0, &mut silence), [0; FRAME_LEN]);
    // Loading isn't idle until the loader has faded out.
    spectrum.set_loading(true);
    assert!(!spectrum.idle());
    spectrum.tick(0.05, &mut silence);
    spectrum.set_loading(false);
    assert!(!spectrum.idle());
    for _ in 0..40 {
        spectrum.tick(1.0 / 60.0, &mut silence);
    }
    assert!(spectrum.idle());
    for _ in 0..30 {
        spectrum.tick(1.0 / 60.0, &mut loud);
    }
    spectrum.reset();
    assert!(spectrum.idle());
}

#[test]
fn no_audio_holds_the_bars() {
    let mut spectrum = Spectrum::new();
    let mut loud = |l: &mut [f32], r: &mut [f32]| {
        for (k, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            *a = sample(&[Component::Sine(1000.0, 0.8)], k as i64, 44100.0);
            *b = *a;
        }
        Some(44100.0)
    };
    let mut frame = [0; FRAME_LEN];
    for _ in 0..30 {
        frame = spectrum.tick(1.0 / 60.0, &mut loud);
    }
    for _ in 0..30 {
        assert_eq!(spectrum.tick(1.0 / 60.0, &mut |_: &mut [f32], _: &mut [f32]| None), frame);
    }
    // "Off" never reads the audio.
    spectrum.set_off(true);
    let frame = spectrum.tick(1.0 / 60.0, &mut |_: &mut [f32], _: &mut [f32]| panic!("read while off"));
    assert_eq!(frame, [0; FRAME_LEN]);
}
