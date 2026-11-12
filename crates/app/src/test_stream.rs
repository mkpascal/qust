// A live stream on 127.0.0.1 for tests and measurements: a WAV header for ten hours (the engine
// takes it for a stream), one second at once, then real time, a 440 Hz tone with a slow wobble
// so the analyzer has moving bars. With a title, it speaks ICY: `icy-metaint` and a
// StreamTitle block every 16000 bytes, when the client asks (`Icy-MetaData: 1`).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;
const METAINT: usize = 16_000;

fn wav_header(frames: u32) -> Vec<u8> {
    let bytes = frames.saturating_mul(4);
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&36u32.saturating_add(bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&RATE.to_le_bytes());
    data.extend_from_slice(&(RATE * 4).to_le_bytes());
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    data
}

fn pcm(from: u64, frames: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(frames * 4);
    for i in 0..frames as u64 {
        let t = (from + i) as f64 / RATE as f64;
        let wobble = 0.5 + 0.5 * (t * 1.3).sin();
        let s = 0.25 * (t * 440.0 * std::f64::consts::TAU).sin() + 0.15 * wobble * (t * 2500.0 * std::f64::consts::TAU).sin() + 0.1 * (1.0 - wobble) * (t * 90.0 * std::f64::consts::TAU).sin();
        let sample = (s * 32767.0) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }
    data
}

fn metadata(title: &str) -> Vec<u8> {
    let text = format!("StreamTitle='{}';", title.replace('\'', "’"));
    let blocks = text.len().div_ceil(16).min(255);
    let mut out = vec![blocks as u8];
    out.extend_from_slice(text.as_bytes());
    out.resize(1 + blocks * 16, 0);
    out
}

/// Starts the server; returns its port. `name` is the station (icy-name); `title`, if any, is
/// sent as ICY metadata.
pub fn serve(name: &str, title: Option<&str>) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let name = name.to_string();
    let title = title.map(str::to_string);
    std::thread::Builder::new().name("qust-test-stream".into()).spawn(move || {
        for socket in listener.incoming().flatten() {
            let name = name.clone();
            let title = title.clone();
            std::thread::spawn(move || {
                let Ok(clone) = socket.try_clone() else { return };
                let mut reader = BufReader::new(clone);
                let mut wants_icy = false;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) <= 2 {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("icy-metadata:") && lower.contains('1') {
                        wants_icy = true;
                    }
                }
                let icy = wants_icy && title.is_some();
                let mut socket = socket;
                let mut head = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nicy-name: {name}\r\nConnection: close\r\n");
                if icy {
                    head.push_str(&format!("icy-metaint: {METAINT}\r\n"));
                }
                head.push_str("\r\n");
                if socket.write_all(head.as_bytes()).is_err() {
                    return;
                }
                // The audio as one byte stream, metadata blocks inserted every METAINT bytes.
                let block = title.as_deref().map(metadata).unwrap_or_default();
                let mut until_meta = METAINT;
                let mut send = |socket: &mut std::net::TcpStream, mut bytes: &[u8]| -> bool {
                    if !icy {
                        return socket.write_all(bytes).is_ok();
                    }
                    while !bytes.is_empty() {
                        let n = until_meta.min(bytes.len());
                        if socket.write_all(&bytes[..n]).is_err() {
                            return false;
                        }
                        bytes = &bytes[n..];
                        until_meta -= n;
                        if until_meta == 0 {
                            if socket.write_all(&block).is_err() {
                                return false;
                            }
                            until_meta = METAINT;
                        }
                    }
                    true
                };
                if !send(&mut socket, &wav_header(RATE * 36_000)) {
                    return;
                }
                let start = Instant::now();
                let mut sent = 0u64;
                loop {
                    let due = ((1.0 + start.elapsed().as_secs_f64()) * RATE as f64) as u64;
                    if due > sent {
                        if !send(&mut socket, &pcm(sent, (due - sent) as usize)) {
                            return;
                        }
                        sent = due;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
        }
    })?;
    Ok(port)
}
