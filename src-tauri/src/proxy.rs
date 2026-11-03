// Local HTTP proxy for streams and their status pages. The webview enforces CORS, unlike the
// Electron window that ran with webSecurity off: the visualizer needs CORS headers on the audio,
// and Icecast's status-json.xsl rarely sends them. The proxy adds them, and the random token in
// the path keeps other local programs from using it.
//
// It also asks for ICY metadata, strips it from the audio and keeps the stream's details (title,
// format, ICY headers, traffic) for the player and the Stream information window to poll on /info.

use serde_json::json;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Info {
    name: Option<String>,
    title: Option<String>,
    genre: Option<String>,
    description: Option<String>,
    website: Option<String>,
    server: Option<String>,
    content_type: Option<String>,
    format: Option<String>,
    bitrate: Option<u32>,
    samplerate: Option<u32>,
    channels: Option<u32>,
    metaint: Option<usize>,
    started: Option<Instant>,
    // audio bytes passed to the player, and ICY metadata bytes stripped from the stream
    bytes: u64,
    meta_bytes: u64,
    title_changes: u32,
    // connections to this URL since Qust started; more than one means it reconnected
    connections: u32,
    // (time, bytes) a few seconds in, past the burst servers send on connect
    baseline: Option<(Instant, u64)>,
}

struct Frame {
    format: &'static str,
    samplerate: u32,
    channels: u32,
    bitrate: Option<u32>,
}

fn infos() -> &'static Mutex<HashMap<String, Info>> {
    static INFOS: OnceLock<Mutex<HashMap<String, Info>>> = OnceLock::new();
    INFOS.get_or_init(Default::default)
}

fn update(url: &str, change: impl FnOnce(&mut Info)) {
    if let Some(info) = infos().lock().unwrap().get_mut(url) {
        change(info);
    }
}

pub fn start() -> io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let prefix = format!("/{:016x}", RandomState::new().build_hasher().finish());
    let base = format!("http://127.0.0.1:{port}{prefix}");
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let prefix = prefix.clone();
            thread::spawn(move || {
                let _ = serve(client, &prefix);
            });
        }
    });
    Ok(base)
}

fn serve(mut client: TcpStream, prefix: &str) -> io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? <= 2 {
            break;
        }
    }

    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let Some(rest) = parts.next().and_then(|target| target.strip_prefix(prefix)) else {
        return not_found(&mut client);
    };
    let (route, query) = rest.split_once('?').unwrap_or((rest, ""));
    let url = query
        .strip_prefix("url=")
        .and_then(decode)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"));
    match (route, url) {
        ("", Some(url)) => relay(client, method, &url),
        ("/info", Some(url)) => info(client, &url),
        _ => not_found(&mut client),
    }
}

fn not_found(client: &mut TcpStream) -> io::Result<()> {
    client.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

fn info(mut client: TcpStream, url: &str) -> io::Result<()> {
    let body = match infos().lock().unwrap().get(url) {
        Some(info) => {
            let measured = info
                .baseline
                .filter(|(at, _)| at.elapsed() >= Duration::from_secs(3))
                .map(|(at, bytes)| ((info.bytes - bytes) as f64 * 8.0 / at.elapsed().as_secs_f64() / 1000.0).round() as u32);
            json!({
                "name": info.name,
                "title": info.title,
                "genre": info.genre,
                "description": info.description,
                "website": info.website,
                "server": info.server,
                "contentType": info.content_type,
                "format": info.format,
                "bitrate": info.bitrate.or(measured),
                "rate": measured,
                "samplerate": info.samplerate,
                "channels": info.channels,
                "metaint": info.metaint,
                "bytes": info.bytes,
                "metaBytes": info.meta_bytes,
                "titleChanges": info.title_changes,
                "reconnects": info.connections.saturating_sub(1),
                "connectedSeconds": info.started.map(|at| at.elapsed().as_secs()),
            })
        }
        None => json!({}),
    }
    .to_string();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn relay(mut client: TcpStream, method: &str, url: &str) -> io::Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("Qust/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = match agent.get(url).set("Icy-MetaData", "1").call() {
        Ok(response) => response,
        Err(_) => {
            return client.write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
        }
    };

    let header = |name: &str| response.header(name).map(str::trim).filter(|value| !value.is_empty());
    let number = |value: &str| value.split(|c: char| !c.is_ascii_digit()).next()?.parse::<u32>().ok();
    let audio_info: HashMap<String, String> = header("ice-audio-info")
        .unwrap_or("")
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().trim_start_matches("ice-").to_lowercase(), value.trim().to_string()))
        .collect();
    let from_audio_info = |key: &str| audio_info.get(key).and_then(|value| number(value));
    let text = |name: &str| header(name).map(str::to_string);
    let metaint = header("icy-metaint").and_then(number).map(|n| n as usize).filter(|n| *n > 0);
    {
        let mut infos = infos().lock().unwrap();
        let connections = infos.get(url).map_or(0, |info| info.connections) + 1;
        infos.insert(
            url.to_string(),
            Info {
                name: text("icy-name"),
                genre: text("icy-genre"),
                description: text("icy-description"),
                website: text("icy-url"),
                server: text("Server"),
                content_type: text("Content-Type"),
                bitrate: header("icy-br").and_then(number).or_else(|| from_audio_info("bitrate")),
                samplerate: from_audio_info("samplerate").or_else(|| header("icy-sr").and_then(number)),
                channels: from_audio_info("channels"),
                metaint,
                started: Some(Instant::now()),
                connections,
                ..Default::default()
            },
        );
    }

    let content_type = header("Content-Type").unwrap_or("application/octet-stream").to_string();
    write!(
        client,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    if method == "HEAD" {
        return Ok(());
    }

    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut until_meta = metaint.unwrap_or(usize::MAX);
    let mut probe = Some(Vec::new());
    loop {
        if until_meta == 0 {
            let mut length = [0u8; 1];
            reader.read_exact(&mut length)?;
            let mut meta = vec![0u8; length[0] as usize * 16];
            reader.read_exact(&mut meta)?;
            let title = stream_title(&meta);
            update(url, |info| {
                info.meta_bytes += 1 + meta.len() as u64;
                if let Some(title) = title {
                    if info.title.as_ref().is_some_and(|current| *current != title) {
                        info.title_changes += 1;
                    }
                    info.title = Some(title);
                }
            });
            until_meta = metaint.unwrap_or(usize::MAX);
            continue;
        }

        let wanted = buffer.len().min(until_meta);
        let read = reader.read(&mut buffer[..wanted])?;
        if read == 0 {
            return Ok(());
        }
        if metaint.is_some() {
            until_meta -= read;
        }
        client.write_all(&buffer[..read])?;

        let frame = probe.as_mut().and_then(|bytes| {
            bytes.extend_from_slice(&buffer[..read]);
            parse_frame(bytes)
        });
        if frame.is_some() || probe.as_ref().is_some_and(|bytes| bytes.len() > 64 * 1024) {
            probe = None;
        }
        update(url, |info| {
            if let Some(frame) = frame {
                info.format.get_or_insert_with(|| frame.format.to_string());
                info.samplerate.get_or_insert(frame.samplerate);
                info.channels.get_or_insert(frame.channels);
                if let Some(bitrate) = frame.bitrate {
                    info.bitrate.get_or_insert(bitrate);
                }
            }
            info.bytes += read as u64;
            if info.baseline.is_none() && info.started.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
                info.baseline = Some((Instant::now(), info.bytes));
            }
        });
    }
}

fn stream_title(meta: &[u8]) -> Option<String> {
    let meta = &meta[..meta.iter().position(|&b| b == 0).unwrap_or(meta.len())];
    let text = String::from_utf8(meta.to_vec()).unwrap_or_else(|_| meta.iter().map(|&b| b as char).collect());
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let end = text[start..].find("';").map_or(text.len(), |end| start + end);
    let title = text[start..end].trim();
    (!title.is_empty()).then(|| title.to_string())
}

// The first MP3 or ADTS (AAC) frame whose next frame also checks out, so a stray sync word in
// the data isn't taken for a header.
fn parse_frame(bytes: &[u8]) -> Option<Frame> {
    for start in 0..bytes.len().saturating_sub(7) {
        let Some((frame, length)) = frame_header(&bytes[start..]) else {
            continue;
        };
        let next = start + length;
        if next + 7 > bytes.len() {
            return None;
        }
        if frame_header(&bytes[next..]).is_some_and(|(other, _)| other.samplerate == frame.samplerate) {
            return Some(frame);
        }
    }
    None
}

fn frame_header(b: &[u8]) -> Option<(Frame, usize)> {
    if b.len() < 7 || b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let layer = (b[1] >> 1) & 3;
    if layer == 0 {
        if b[1] & 0xF6 != 0xF0 {
            return None;
        }
        const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
        let samplerate = *RATES.get(((b[2] >> 2) & 0xF) as usize)?;
        let channels = (((b[2] & 1) << 2) | (b[3] >> 6)) as u32;
        let length = (((b[3] & 3) as usize) << 11) | ((b[4] as usize) << 3) | ((b[5] >> 5) as usize);
        if channels == 0 || length < 7 {
            return None;
        }
        let format = ["AAC Main", "AAC-LC", "AAC SSR", "AAC LTP"][(b[2] >> 6) as usize];
        return Some((Frame { format, samplerate, channels, bitrate: None }, length));
    }

    let version = (b[1] >> 3) & 3;
    let rate_index = ((b[2] >> 2) & 3) as usize;
    let bitrate_index = (b[2] >> 4) as usize;
    if version == 1 || rate_index == 3 || bitrate_index == 0 || bitrate_index == 15 {
        return None;
    }
    let base = [44100, 48000, 32000][rate_index];
    let samplerate = match version {
        3 => base,
        2 => base / 2,
        _ => base / 4,
    };
    let kbps: usize = match (version, layer) {
        (3, 1) => [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320][bitrate_index],
        (3, 2) => [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384][bitrate_index],
        (3, _) => [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448][bitrate_index],
        (_, 3) => [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256][bitrate_index],
        _ => [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bitrate_index],
    };
    let padding = ((b[2] >> 1) & 1) as usize;
    let rate = samplerate as usize;
    let length = match layer {
        3 => (12 * kbps * 1000 / rate + padding) * 4,
        1 if version != 3 => 72 * kbps * 1000 / rate + padding,
        _ => 144 * kbps * 1000 / rate + padding,
    };
    let channels = if b[3] >> 6 == 3 { 1 } else { 2 };
    let format = ["", "MP3", "MP2", "MP1"][layer as usize];
    Some((Frame { format, samplerate, channels, bitrate: Some(kbps as u32) }, length))
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
