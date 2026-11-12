// The details of each stream or file, for the player and the Stream information window: what the
// HTTP headers, the ICY metadata, the first frame header and then the demuxer and decoder say.
// Moved here from the app's HTTP proxy, with the same JSON. Kept by key across reconnects, so the
// reconnect count survives; each connection starts the rest afresh.

use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Info {
    // the stream that owns these details; a replaced stream's thread may still be winding down
    pub owner: u64,
    pub name: Option<String>,
    pub title: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub website: Option<String>,
    pub server: Option<String>,
    pub content_type: Option<String>,
    pub format: Option<String>,
    // the format is only the container's, from the content type: the codec's name replaces it
    pub format_guessed: bool,
    pub bitrate: Option<u32>,
    pub samplerate: Option<u32>,
    pub channels: Option<u32>,
    pub metaint: Option<usize>,
    // the length of a file (local, or a plain HTTP file rather than a stream)
    pub size: Option<u64>,
    pub duration: Option<f64>,
    pub started: Option<Instant>,
    // audio bytes read, and ICY metadata bytes stripped from the stream
    pub bytes: u64,
    pub meta_bytes: u64,
    pub title_changes: u32,
    // a title came in the ICY metadata
    pub icy_titled: bool,
    // connections under this key since the app started; more than one means it reconnected
    pub connections: u32,
    // (time, bytes) a few seconds in, past the burst servers send on connect
    pub baseline: Option<(Instant, u64)>,
}

impl Info {
    pub fn set_title(&mut self, title: String) {
        if self.title.as_ref().is_some_and(|current| *current != title) {
            self.title_changes += 1;
        }
        self.title = Some(title);
    }

    pub fn add_bytes(&mut self, bytes: u64) {
        self.bytes += bytes;
        if self.baseline.is_none() && self.started.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
            self.baseline = Some((Instant::now(), self.bytes));
        }
    }

    pub fn apply_frame(&mut self, frame: &Frame) {
        if self.format.is_none() || self.format_guessed {
            self.format = Some(frame.format.to_string());
            self.format_guessed = false;
        }
        self.samplerate.get_or_insert(frame.samplerate);
        self.channels.get_or_insert(frame.channels);
        if let Some(bitrate) = frame.bitrate {
            self.bitrate.get_or_insert(bitrate);
        }
    }
}

pub(crate) struct Frame {
    pub format: &'static str,
    pub samplerate: u32,
    pub channels: u32,
    pub bitrate: Option<u32>,
}

fn infos() -> MutexGuard<'static, HashMap<String, Info>> {
    static INFOS: OnceLock<Mutex<HashMap<String, Info>>> = OnceLock::new();
    // A panic elsewhere while holding the lock leaves the map usable.
    INFOS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// Starts the details of a new connection under `key`, keeping the connection count.
pub(crate) fn begin(key: &str, owner: u64, info: Info) {
    let mut infos = infos();
    // A replaced stream's thread connecting late doesn't overwrite its successor's details.
    if infos.get(key).is_some_and(|info| info.owner > owner) {
        return;
    }
    let connections = infos.get(key).map_or(0, |info| info.connections) + 1;
    infos.insert(key.to_string(), Info { owner, connections, started: Some(Instant::now()), ..info });
}

// Changes the details of `key`, if they still belong to the stream `owner`.
pub(crate) fn update(key: &str, owner: u64, change: impl FnOnce(&mut Info)) {
    if let Some(info) = infos().get_mut(key).filter(|info| info.owner == owner) {
        change(info);
    }
}

pub(crate) fn get<T>(key: &str, read: impl FnOnce(&Info) -> T) -> Option<T> {
    infos().get(key).map(read)
}

/// The details of a stream or file, by key, as the app's `stream_info` command returns them
/// (`{}` when unknown).
pub fn info(key: &str) -> serde_json::Value {
    let infos = infos();
    let Some(info) = infos.get(key) else {
        return json!({});
    };
    let measured = info
        .baseline
        .filter(|(at, _)| at.elapsed() >= Duration::from_secs(3))
        .map(|(at, bytes)| ((info.bytes - bytes) as f64 * 8.0 / at.elapsed().as_secs_f64() / 1000.0).round() as u32);
    // A file whose header doesn't give its bitrate (Ogg, M4A, FLAC): its average.
    let average = match (info.size, info.duration) {
        (Some(size), Some(duration)) if duration > 0.0 => Some((size as f64 * 8.0 / duration / 1000.0).round() as u32),
        _ => None,
    };
    json!({
        "name": info.name,
        "title": info.title,
        "genre": info.genre,
        "description": info.description,
        "website": info.website,
        "server": info.server,
        "contentType": info.content_type,
        "format": info.format,
        "bitrate": info.bitrate.or(average).or(measured),
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

// The MIME type of a file the player can play, by its extension.
pub(crate) fn audio_type(extension: &str) -> Option<&'static str> {
    Some(match extension.to_ascii_lowercase().as_str() {
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

// The container's name when nothing better is known, from the content type.
pub(crate) fn container_format(content_type: &str) -> Option<&'static str> {
    let content_type = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    Some(match content_type.as_str() {
        "audio/mp4" | "audio/aac" | "audio/aacp" | "audio/x-m4a" => "AAC",
        "audio/ogg" | "application/ogg" => "Ogg",
        "audio/opus" => "Opus",
        "audio/flac" | "audio/x-flac" => "FLAC",
        "audio/webm" => "WebM",
        _ => return None,
    })
}

// The details of a local file: from its first MP3 / AAC frame, or its FLAC or WAV header, or else
// just the container from the extension.
pub(crate) fn describe_file(file: &mut std::fs::File, content_type: &str, size: u64) -> Info {
    let mut head = [0u8; 10];
    let read = file.read(&mut head).unwrap_or(0);
    let skip = id3_length(&head[..read]);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(skip as u64)).is_ok() {
        let _ = Read::by_ref(file).take(64 * 1024).read_to_end(&mut bytes);
    }
    let _ = file.seek(SeekFrom::Start(0));
    let mut info = Info { content_type: Some(content_type.to_string()), size: Some(size), ..Default::default() };
    if let Some(frame) = parse_frame(&bytes) {
        info.apply_frame(&frame);
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
        info.format = container_format(content_type).map(str::to_string);
        info.format_guessed = info.format.is_some();
    }
    info
}

// The length of an ID3v2 tag at the start of an MP3 (0 without one). Cover art can make it large,
// so frames are looked for after it.
pub(crate) fn id3_length(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let size = bytes[6..10].iter().fold(0usize, |size, &b| (size << 7) | (b & 0x7F) as usize);
    10 + size + if bytes[5] & 0x10 != 0 { 10 } else { 0 }
}

pub(crate) fn stream_title(meta: &[u8]) -> Option<String> {
    let meta = &meta[..meta.iter().position(|&b| b == 0).unwrap_or(meta.len())];
    let text = String::from_utf8(meta.to_vec()).unwrap_or_else(|_| meta.iter().map(|&b| b as char).collect());
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let end = text[start..].find("';").map_or(text.len(), |end| start + end);
    let title = text[start..end].trim();
    (!title.is_empty()).then(|| title.to_string())
}

// The first MP3 or ADTS (AAC) frame whose next frame also checks out, so a stray sync word in
// the data isn't taken for a header.
pub(crate) fn parse_frame(bytes: &[u8]) -> Option<Frame> {
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

/// The title from the server's Icecast status page (status-json.xsl), for streams without ICY
/// titles: the source whose listen URL has the stream's path, or the first one for a stream at
/// "/". Blocking (10 s timeout).
pub fn icecast_title(url: &str, user_agent: &str) -> Option<String> {
    let stream = url::Url::parse(url).ok()?;
    let mut status = stream.clone();
    status.set_path("/status-json.xsl");
    status.set_query(None);
    let body = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .user_agent(user_agent)
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
                .and_then(|listen| url::Url::parse(listen).ok())
                .is_some_and(|listen| listen.path() == stream.path())
        })
    }?;
    source["title"].as_str().map(str::trim).filter(|title| !title.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icy_title() {
        assert_eq!(stream_title(b"StreamTitle='Artist - Song';StreamUrl='';\0\0").as_deref(), Some("Artist - Song"));
        assert_eq!(stream_title(b"StreamTitle='';\0"), None);
    }

    #[test]
    fn mp3_and_adts_frames() {
        // Two MPEG-1 layer III frames, 128 kbps, 44.1 kHz, stereo: 417 bytes each.
        let mut bytes = vec![0u8; 417 * 2 + 8];
        for at in [0, 417] {
            bytes[at..at + 4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        }
        bytes[834..838].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        let frame = parse_frame(&bytes).expect("an MP3 frame");
        assert_eq!((frame.format, frame.samplerate, frame.channels, frame.bitrate), ("MP3", 44100, 2, Some(128)));

        // ADTS AAC-LC at 24 kHz, stereo, 100-byte frames: taken for HE-AAC at 48 kHz.
        let header = |b: &mut [u8]| {
            let length = 100usize;
            b[..7].copy_from_slice(&[
                0xFF,
                0xF1,
                (1 << 6) | (6 << 2),
                (2 << 6) | ((length >> 11) & 3) as u8,
                ((length >> 3) & 0xFF) as u8,
                (((length & 7) << 5) as u8) | 0x1F,
                0xFC,
            ]);
        };
        let mut bytes = vec![0u8; 220];
        header(&mut bytes[0..]);
        header(&mut bytes[100..]);
        let frame = parse_frame(&bytes).expect("an ADTS frame");
        assert_eq!((frame.format, frame.samplerate, frame.channels), ("HE-AAC", 48000, 2));
    }

    #[test]
    fn id3() {
        let mut tag = vec![b'I', b'D', b'3', 4, 0, 0, 0, 0, 1, 0];
        tag.resize(20, 0);
        assert_eq!(id3_length(&tag), 10 + 128);
        assert_eq!(id3_length(b"RIFF....WAVE"), 0);
    }
}
