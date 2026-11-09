// Local HTTP proxy for streams. The webview enforces CORS, unlike the Electron window that ran
// with webSecurity off, and the visualizer needs CORS headers on the audio. The proxy adds them,
// and the random token in the path keeps other local programs from using it.
//
// A second listener serves the playlist's local files where they are (`?path=`), with range
// requests so they can seek. It has its own port: the webview's six connections per host can all
// be taken by instant-switch streams, and a file must never wait behind them.
//
// It also asks for ICY metadata, strips it from the audio and keeps the stream's details (title,
// format, ICY headers, traffic), which the player and the Stream information window read with the
// `stream_info` command. That and `icecast_title` go through IPC, not HTTP: with instant switch on,
// six streams fill the webview's six connections per host, and a request to the proxy would wait
// behind them.

use serde_json::json;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
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
    // the length of a file (local, or a plain HTTP file rather than a stream)
    size: Option<u64>,
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

// The base URLs of the stream proxy and of the file server.
pub fn start() -> io::Result<(String, String)> {
    let prefix = format!("/{:016x}", RandomState::new().build_hasher().finish());
    Ok((listen(&prefix, false)?, listen(&prefix, true)?))
}

fn listen(prefix: &str, files: bool) -> io::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let base = format!("http://127.0.0.1:{port}{prefix}");
    let prefix = prefix.to_string();
    thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let prefix = prefix.clone();
            thread::spawn(move || {
                let _ = serve(client, &prefix, files);
            });
        }
    });
    Ok(base)
}

fn serve(mut client: TcpStream, prefix: &str, files: bool) -> io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? <= 2 {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }

    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let Some(rest) = parts.next().and_then(|target| target.strip_prefix(prefix)) else {
        return not_found(&mut client);
    };
    let (route, query) = rest.split_once('?').unwrap_or((rest, ""));
    if files {
        return match (route, query.strip_prefix("path=").and_then(decode)) {
            ("", Some(path)) => send_file(client, method, &path, range.as_deref()),
            _ => not_found(&mut client),
        };
    }
    let url = query
        .strip_prefix("url=")
        .and_then(decode)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"));
    match (route, url) {
        ("", Some(url)) => relay(client, method, &url, range.as_deref()),
        _ => not_found(&mut client),
    }
}

// The first byte a "bytes=N-M" range asks for (0 without one).
fn range_start(range: Option<&str>) -> u64 {
    range
        .and_then(|range| range.strip_prefix("bytes="))
        .and_then(|span| span.split('-').next())
        .and_then(|start| start.trim().parse().ok())
        .unwrap_or(0)
}

// The inclusive span a range asks for in a file of `total` bytes. Single ranges only: that's all
// media players ask for.
fn parse_range(range: &str, total: u64) -> Option<(u64, u64)> {
    let (start, end) = range.strip_prefix("bytes=")?.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());
    let last = total.checked_sub(1)?;
    let span = if start.is_empty() {
        (total.saturating_sub(end.parse().ok()?), last)
    } else {
        let end = if end.is_empty() { last } else { end.parse::<u64>().ok()?.min(last) };
        (start.parse().ok()?, end)
    };
    (span.0 <= span.1).then_some(span)
}

// The MIME type of a file the player can play, by its extension.
pub fn audio_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "webm" | "weba" => "audio/webm",
        _ => return None,
    })
}

// A playlist file, read where it is. Audio files only, so the token alone can't read anything else.
fn send_file(mut client: TcpStream, method: &str, path: &str, range: Option<&str>) -> io::Result<()> {
    let Some(content_type) = audio_type(Path::new(path)) else {
        return not_found(&mut client);
    };
    let Ok(mut file) = File::open(path) else {
        return not_found(&mut client);
    };
    let total = file.metadata()?.len();
    let (start, end, partial) = match range.map(|range| parse_range(range, total)) {
        Some(Some((start, end))) => (start, end, true),
        Some(None) => {
            return write!(
                client,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        }
        None => (0, total.saturating_sub(1), false),
    };
    if start == 0 {
        describe_file(path, &mut file, content_type, total);
    }
    let length = if total == 0 { 0 } else { end - start + 1 };
    let status = if partial { "206 Partial Content" } else { "200 OK" };
    let content_range = if partial { format!("Content-Range: bytes {start}-{end}/{total}\r\n") } else { String::new() };
    write!(
        client,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n{content_range}Accept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    if method == "HEAD" {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut reader = file.take(length);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        client.write_all(&buffer[..read])?;
        update(path, |info| info.bytes += read as u64);
    }
}

// The details of a local file for `stream_info`: from its first MP3 / AAC frame, or its FLAC or
// WAV header, or else just the container from the extension.
fn describe_file(path: &str, file: &mut File, content_type: &str, size: u64) {
    let mut head = [0u8; 10];
    let read = file.read(&mut head).unwrap_or(0);
    let skip = id3_length(&head[..read]);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(skip as u64)).is_ok() {
        let _ = Read::by_ref(file).take(64 * 1024).read_to_end(&mut bytes);
    }
    let mut info = Info {
        content_type: Some(content_type.to_string()),
        size: Some(size),
        started: Some(Instant::now()),
        connections: 1,
        ..Default::default()
    };
    if let Some(frame) = parse_frame(&bytes) {
        info.format = Some(frame.format.to_string());
        info.samplerate = Some(frame.samplerate);
        info.channels = Some(frame.channels);
        info.bitrate = frame.bitrate;
    } else if bytes.len() >= 26 && &bytes[..4] == b"fLaC" {
        // STREAMINFO, the first metadata block: a 20-bit rate, then 3 bits of channels - 1.
        let b = &bytes[8..];
        info.format = Some("FLAC".into());
        info.samplerate = Some(((b[10] as u32) << 12) | ((b[11] as u32) << 4) | ((b[12] as u32) >> 4));
        info.channels = Some((((b[12] >> 1) & 7) + 1) as u32);
    } else if bytes.len() >= 36 && &bytes[..4] == b"RIFF" && &bytes[8..16] == b"WAVEfmt " {
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as u32;
        let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        info.format = Some("WAV".into());
        info.channels = Some(u16_at(22));
        info.samplerate = Some(u32_at(24));
        info.bitrate = Some(u32_at(28) * 8 / 1000);
    } else {
        info.format = match content_type {
            "audio/mp4" | "audio/aac" => Some("AAC".into()),
            "audio/ogg" => Some("Ogg".into()),
            "audio/opus" => Some("Opus".into()),
            "audio/flac" => Some("FLAC".into()),
            "audio/webm" => Some("WebM".into()),
            _ => None,
        };
    }
    infos().lock().unwrap().insert(path.to_string(), info);
}

// The length of an ID3v2 tag at the start of an MP3 (0 without one). Cover art can make it large,
// so frames are looked for after it.
fn id3_length(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let size = bytes[6..10].iter().fold(0usize, |size, &b| (size << 7) | (b & 0x7F) as usize);
    10 + size + if bytes[5] & 0x10 != 0 { 10 } else { 0 }
}

fn not_found(client: &mut TcpStream) -> io::Result<()> {
    client.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

// The details of a stream the proxy is relaying ({} when it isn't).
#[tauri::command]
pub fn stream_info(url: String) -> serde_json::Value {
    let infos = infos().lock().unwrap();
    let Some(info) = infos.get(&url) else {
        return json!({});
    };
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
        "size": info.size,
        "bytes": info.bytes,
        "metaBytes": info.meta_bytes,
        "titleChanges": info.title_changes,
        "reconnects": info.connections.saturating_sub(1),
        "connectedSeconds": info.started.map(|at| at.elapsed().as_secs()),
    })
}

// The title from the server's Icecast status page (status-json.xsl), for streams without ICY
// titles: the source whose listen URL has the stream's path, or the first one for a stream at "/".
#[tauri::command]
pub async fn icecast_title(url: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let stream = tauri::Url::parse(&url).ok()?;
        let mut status = stream.clone();
        status.set_path("/status-json.xsl");
        status.set_query(None);
        let body = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("Qust/", env!("CARGO_PKG_VERSION")))
            .build()
            .get(status.as_str())
            .call()
            .ok()?
            .into_string()
            .ok()?;
        let data: serde_json::Value = serde_json::from_str(&body).ok()?;
        let sources = match &data["icestats"]["source"] {
            serde_json::Value::Array(sources) => sources.clone(),
            serde_json::Value::Null => Vec::new(),
            source => vec![source.clone()],
        };
        let source = if url.ends_with('/') {
            sources.first()
        } else {
            sources.iter().find(|source| {
                source["listenurl"]
                    .as_str()
                    .and_then(|listen| tauri::Url::parse(listen).ok())
                    .is_some_and(|listen| listen.path() == stream.path())
            })
        }?;
        source["title"].as_str().map(str::trim).filter(|title| !title.is_empty()).map(str::to_string)
    })
    .await
    .ok()
    .flatten()
}

// Streams, and plain HTTP files too: a range request (seeking in a file) is passed on, and the
// length is kept so the player knows the file's duration while it downloads.
fn relay(mut client: TcpStream, method: &str, url: &str, range: Option<&str>) -> io::Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("Qust/", env!("CARGO_PKG_VERSION")))
        .build();
    // "bytes=0-" (the whole file, which the webview asks for first) isn't passed on: some stream
    // servers don't expect ranges.
    let start = range_start(range);
    let mut request = agent.get(url).set("Icy-MetaData", "1");
    if let (Some(range), true) = (range, start > 0) {
        request = request.set("Range", range);
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(_) => {
            return client.write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
        }
    };

    let header = |name: &str| response.header(name).map(str::trim).filter(|value| !value.is_empty());
    let number = |value: &str| value.split(|c: char| !c.is_ascii_digit()).next()?.parse::<u32>().ok();
    // Icecast sends ice-audio-info, some other servers (AIS) icy-audio-info.
    let audio_info: HashMap<String, String> = header("ice-audio-info")
        .or_else(|| header("icy-audio-info"))
        .unwrap_or("")
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().trim_start_matches("ice-").to_lowercase(), value.trim().to_string()))
        .collect();
    let from_audio_info = |key: &str| audio_info.get(key).and_then(|value| number(value));
    let text = |name: &str| header(name).map(str::to_string);
    let metaint = header("icy-metaint").and_then(number).map(|n| n as usize).filter(|n| *n > 0);
    let partial = response.status() == 206;
    let content_range = header("Content-Range").filter(|_| partial).map(str::to_string);
    // The audio's length, unless ICY metadata (stripped here) is mixed in.
    let length = header("Content-Length").and_then(|value| value.parse::<u64>().ok()).filter(|_| metaint.is_none());
    let size = match &content_range {
        Some(range) => range.rsplit('/').next().and_then(|total| total.parse().ok()),
        None => length,
    };
    let seeking = partial && start > 0;
    {
        let mut infos = infos().lock().unwrap();
        // Seeking in a file keeps the details of the connection that started it.
        let connections = infos.get(url).map_or(0, |info| info.connections) + 1;
        if !(seeking && infos.contains_key(url)) {
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
                    size,
                    started: Some(Instant::now()),
                    connections,
                    ..Default::default()
                },
            );
        }
    }

    let content_type = header("Content-Type").unwrap_or("application/octet-stream").to_string();
    let status = if partial { "206 Partial Content" } else { "200 OK" };
    let mut extra = String::new();
    if let Some(range) = &content_range {
        extra += &format!("Content-Range: {range}\r\n");
    }
    if let Some(length) = length {
        extra += &format!("Content-Length: {length}\r\n");
    }
    if header("Accept-Ranges").is_some_and(|value| value.eq_ignore_ascii_case("bytes")) {
        extra += "Accept-Ranges: bytes\r\n";
    }
    write!(
        client,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra}Access-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    if method == "HEAD" {
        return Ok(());
    }

    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut until_meta = metaint.unwrap_or(usize::MAX);
    let mut probe = (!seeking).then(Vec::new);
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
            parse_frame(bytes.get(id3_length(bytes)..).unwrap_or_default())
        });
        if frame.is_some() || probe.as_ref().is_some_and(|bytes| bytes.len() > id3_length(bytes) + 64 * 1024) {
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
        let profile = b[2] >> 6;
        // HE-AAC (AAC+) signals SBR implicitly: the header has the AAC-LC core at half the rate
        // (22.05 / 24 kHz), and the decoder plays it at twice that. A plain AAC-LC stream at 24 kHz
        // or less is rare enough to take it for HE-AAC.
        if profile == 1 && samplerate <= 24000 {
            return Some((Frame { format: "HE-AAC", samplerate: samplerate * 2, channels, bitrate: None }, length));
        }
        let format = ["AAC Main", "AAC-LC", "AAC SSR", "AAC LTP"][profile as usize];
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

pub fn decode(text: &str) -> Option<String> {
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
