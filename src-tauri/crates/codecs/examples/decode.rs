//! Decodes a file or URL through `qust_codecs::registry()` into memory (never to a device) and
//! reports what came out: codec, output rate and channels, frames decoded against the frames the
//! packets account for, peak and RMS, and the share of energy above 11 kHz (near zero for HE-AAC
//! decoded as plain LC, which also reports half the rate).
//!
//! cargo run -p qust-codecs --example decode -- <file or URL> [seconds] [--repeat N] [--raw out.f32]
//!
//! `--repeat N` opens and decodes the source N times, each with a fresh decoder, for checking that
//! memory stays flat. `--raw` writes the decoded audio as interleaved f32 for comparing against a
//! reference decoder.

use std::io::Write;
use std::time::Instant;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

const FFT: usize = 4096;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(source) = args.first().cloned() else {
        eprintln!("usage: decode <file or URL> [seconds] [--repeat N] [--raw out.f32]");
        std::process::exit(2);
    };
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let repeat: usize = flag("--repeat").and_then(|s| s.parse().ok()).unwrap_or(1);
    let raw = flag("--raw");

    println!("pid {}", std::process::id());
    let started = Instant::now();
    for round in 0..repeat {
        let mut stats = Stats::default();
        let mut raw_out = raw
            .as_ref()
            .map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap()));
        if let Err(err) = decode(&source, seconds, &mut stats, raw_out.as_mut()) {
            println!("error: {err}");
            std::process::exit(1);
        }
        if repeat == 1 || round == 0 || round + 1 == repeat || round % 10 == 9 {
            println!(
                "--- round {} ({:.0} s in)",
                round + 1,
                started.elapsed().as_secs_f64()
            );
            stats.report();
        }
    }
}

fn open(source: &str) -> Result<(Box<dyn MediaSource>, Hint), String> {
    let mut hint = Hint::new();
    if source.starts_with("http://") || source.starts_with("https://") {
        // No Icy-MetaData header: the body is then pure audio.
        let response = ureq::get(source)
            .set("User-Agent", "qust-codecs-test")
            .call()
            .map_err(|e| e.to_string())?;
        let content_type = response.content_type().to_string();
        println!("content-type {content_type}");
        hint.mime_type(&content_type);
        if let Some(ext) = extension(source) {
            hint.with_extension(&ext);
        }
        Ok((Box::new(ReadOnlySource::new(response.into_reader())), hint))
    } else {
        if let Some(ext) = extension(source) {
            hint.with_extension(&ext);
        }
        let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
        Ok((Box::new(file), hint))
    }
}

fn extension(source: &str) -> Option<String> {
    let path = source.split(['?', '#']).next()?;
    let name = path.rsplit(['/', '\\']).next()?;
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
}

fn decode(
    source: &str,
    seconds: f64,
    stats: &mut Stats,
    mut raw: Option<&mut std::io::BufWriter<std::fs::File>>,
) -> Result<(), String> {
    let (media, hint) = open(source)?;
    let mss = MediaSourceStream::new(media, Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    stats.format = format.format_info().short_name.to_string();

    let (mut track_id, mut time_base, mut decoder) = make_decoder(format.as_mut(), stats)?;
    let mut interleaved: Vec<f32> = Vec::new();

    while stats.packet_seconds < seconds {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(Error::ResetRequired) => {
                // Chained Ogg: a new logical stream, so a new decoder.
                stats.resets += 1;
                (track_id, time_base, decoder) = make_decoder(format.as_mut(), stats)?;
                continue;
            }
            Err(err) if stats.packets > 0 => {
                // A truncated capture ends mid-page.
                println!("demux stopped: {err}");
                break;
            }
            Err(err) => return Err(format!("demux: {err}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        if stats.packets < 3 {
            println!(
                "packet {}: {} bytes, pts {:?}, dur {:?}, trim {:?}/{:?}",
                stats.packets,
                packet.data.len(),
                packet.pts,
                packet.dur,
                packet.trim_start,
                packet.trim_end
            );
        }
        stats.packets += 1;
        stats.packet_seconds += time_base * packet.dur.get() as f64;

        let timer = Instant::now();
        let result = decoder.decode(&packet);
        stats.decode_time += timer.elapsed().as_secs_f64();
        match result {
            Ok(buf) => {
                stats.add(&buf, &mut interleaved);
                if let Some(raw) = raw.as_mut() {
                    for s in &interleaved {
                        raw.write_all(&s.to_le_bytes()).unwrap();
                    }
                }
            }
            Err(Error::DecodeError(e)) | Err(Error::Unsupported(e))
                if stats.frames > 0 || stats.errors < 50 =>
            {
                stats.errors += 1;
                if stats.errors <= 3 {
                    println!("decode error (skipped): {e}");
                }
            }
            Err(err) => return Err(format!("decode: {err}")),
        }
    }
    Ok(())
}

fn make_decoder(
    format: &mut dyn FormatReader,
    stats: &mut Stats,
) -> Result<(u32, f64, Box<dyn AudioDecoder>), String> {
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("no audio parameters")?;
    // `--stock`: Symphonia's own registry, for comparison.
    let registry = if std::env::args().any(|a| a == "--stock") {
        symphonia::default::get_codecs()
    } else {
        qust_codecs::registry()
    };
    let codec = registry
        .get_audio_decoder(params.codec)
        .map(|r| r.codec.info.short_name)
        .unwrap_or("?");
    println!(
        "track {}: codec {} ({}), params rate {:?}, channels {:?}, extra {:02x?}, delay {:?}",
        track.id,
        codec,
        params.codec,
        params.sample_rate,
        params.channels,
        params.extra_data.as_deref().map(|d| &d[..d.len().min(24)]),
        track.delay,
    );
    let time_base = track
        .time_base
        .map(|tb| tb.numer.get() as f64 / tb.denom.get() as f64)
        .or(params.sample_rate.map(|r| 1.0 / r as f64))
        .ok_or("no time base")?;
    let decoder = registry
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("make decoder: {e}"))?;
    stats.codec = decoder.codec_info().short_name.to_string();
    println!(
        "decoder params at creation: rate {:?}, channels {:?}",
        decoder.codec_params().sample_rate,
        decoder.codec_params().channels
    );
    Ok((track.id, time_base, decoder))
}

#[derive(Default)]
struct Stats {
    format: String,
    codec: String,
    packets: u64,
    errors: u64,
    resets: u64,
    packet_seconds: f64,
    /// Seconds spent inside decoder.decode().
    decode_time: f64,
    /// (frame index, rate, channels) at every spec change.
    specs: Vec<(u64, u32, usize)>,
    frames: u64,
    /// Output frames weighted by rate, for comparing against packet time across a rate change.
    seconds: f64,
    peak: f32,
    sum_sq: f64,
    samples: u64,
    clipped: u64,
    /// Energy of (L+R)/2 and (L-R)/2, to tell real stereo (or parametric stereo) from dual mono.
    mid_side: (f64, f64),
    /// Mean absolute step between consecutive samples, inside buffers and across buffer joins.
    step_inside: (f64, u64),
    step_join: (f64, u64),
    last: Vec<f32>,
    mono: Vec<f32>,
    spectrum: Vec<f64>,
    spectrum_rate: u32,
}

impl Stats {
    fn add(&mut self, buf: &GenericAudioBufferRef<'_>, interleaved: &mut Vec<f32>) {
        let rate = buf.spec().rate();
        let channels = buf.spec().channels().count();
        if buf.frames() == 0 {
            interleaved.clear();
            return;
        }
        if self.specs.last().map(|&(_, r, c)| (r, c)) != Some((rate, channels)) {
            self.specs.push((self.frames, rate, channels));
            self.last.clear();
        }
        buf.copy_to_vec_interleaved(interleaved);
        let frames = buf.frames();
        self.frames += frames as u64;
        self.seconds += frames as f64 / rate as f64;

        for frame in interleaved.chunks_exact(channels) {
            let mut sum = 0.0;
            for &s in frame {
                self.peak = self.peak.max(s.abs());
                self.sum_sq += f64::from(s) * f64::from(s);
                if s.abs() >= 0.999 {
                    self.clipped += 1;
                }
                sum += s;
            }
            self.mono.push(sum / channels as f32);
            if channels == 2 {
                let (m, s) = ((frame[0] + frame[1]) / 2.0, (frame[0] - frame[1]) / 2.0);
                self.mid_side.0 += f64::from(m * m);
                self.mid_side.1 += f64::from(s * s);
            }
        }
        self.samples += interleaved.len() as u64;

        // Steps on channel 0.
        let ch0: Vec<f32> = interleaved.iter().step_by(channels).copied().collect();
        if let (Some(&prev), Some(&first)) = (self.last.first(), ch0.first()) {
            self.step_join.0 += f64::from((first - prev).abs());
            self.step_join.1 += 1;
        }
        for w in ch0.windows(2) {
            self.step_inside.0 += f64::from((w[1] - w[0]).abs());
            self.step_inside.1 += 1;
        }
        self.last = vec![*ch0.last().unwrap()];

        if self.spectrum_rate != rate {
            self.spectrum = vec![0.0; FFT / 2];
            self.spectrum_rate = rate;
            self.mono.clear();
        }
        while self.mono.len() >= FFT {
            let block: Vec<f32> = self.mono.drain(..FFT).collect();
            power_spectrum(&block, &mut self.spectrum);
        }
    }

    fn report(&self) {
        println!("format {}, decoder {}", self.format, self.codec);
        for (at, rate, channels) in &self.specs {
            println!("  output from frame {at}: {rate} Hz, {channels} ch");
        }
        let rms = (self.sum_sq / self.samples.max(1) as f64).sqrt();
        println!(
            "packets {} ({:.2} s of packet time), decode errors {}, decoder resets (chained) {}",
            self.packets, self.packet_seconds, self.errors, self.resets
        );
        println!(
            "time in decode() {:.3} s for {:.1} s of audio ({:.0}x real time)",
            self.decode_time,
            self.seconds,
            self.seconds / self.decode_time.max(1e-9)
        );
        let rate = self.specs.last().map_or(0, |s| s.1);
        println!(
            "frames {} = {:.3} s of output; packet time x {} Hz = {:.0} frames (ratio {:.4})",
            self.frames,
            self.seconds,
            rate,
            self.packet_seconds * rate as f64,
            self.seconds / self.packet_seconds.max(1e-9)
        );
        println!(
            "peak {:.4} ({:.1} dBFS), rms {:.4} ({:.1} dBFS), samples at full scale {}",
            self.peak,
            20.0 * f64::from(self.peak.max(1e-9)).log10(),
            rms,
            20.0 * rms.max(1e-9).log10(),
            self.clipped
        );
        if self.mid_side.0 > 0.0 {
            println!("side/mid energy {:.4}", self.mid_side.1 / self.mid_side.0);
        }
        let inside = self.step_inside.0 / self.step_inside.1.max(1) as f64;
        let join = self.step_join.0 / self.step_join.1.max(1) as f64;
        println!(
            "mean |step| inside buffers {inside:.5}, across buffer joins {join:.5} ({} joins)",
            self.step_join.1
        );
        if self.spectrum_rate > 0 {
            let bin_hz = self.spectrum_rate as f64 / FFT as f64;
            let total: f64 = self.spectrum.iter().sum();
            let share = |from: f64| -> f64 {
                self.spectrum
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i as f64 * bin_hz >= from)
                    .map(|(_, p)| p)
                    .sum::<f64>()
                    / total.max(1e-30)
            };
            let top = self
                .spectrum
                .iter()
                .enumerate()
                .rev()
                .find(|(_, &p)| p > total * 1e-7)
                .map_or(0.0, |(i, _)| i as f64 * bin_hz);
            println!(
                "energy above 11 kHz {:.3}%, above 16 kHz {:.4}%; highest bin above -70 dB of total: {:.0} Hz",
                100.0 * share(11_000.0),
                100.0 * share(16_000.0),
                top
            );
        }
    }
}

/// Adds the Hann-windowed power spectrum of `block` (FFT samples) to `acc`.
fn power_spectrum(block: &[f32], acc: &mut [f64]) {
    let n = block.len();
    let mut re: Vec<f64> = block
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            f64::from(s) * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos())
        })
        .collect();
    let mut im = vec![0.0; n];
    // Iterative radix-2 FFT.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((angle * k as f64).cos(), (angle * k as f64).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len *= 2;
    }
    for (k, p) in acc.iter_mut().enumerate() {
        *p += re[k] * re[k] + im[k] * im[k];
    }
}
