// Where the bytes come from: a local file, or an HTTP stream or file. Both record what they read
// in the stream's details. The HTTP source does what the app's proxy did: it asks for ICY
// metadata, strips it from the audio and records the titles, and for a plain file with a length
// it can seek, with a ranged request.

use crate::info::{self, Info};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSource;

pub(crate) struct Opened {
    pub source: Box<dyn MediaSource>,
    pub hint: Hint,
    // a file (local, or HTTP with a length and no ICY metadata), rather than a live stream
    pub file: bool,
    pub size: Option<u64>,
}

type Body = Box<dyn Read + Send + Sync>;

// Forward jumps up to this far read through the current connection instead of opening another.
const SKIP_LIMIT: u64 = 256 * 1024;
// The start of a file is kept: demuxers go back to it while probing, and a connection can't.
const HEAD_LIMIT: usize = 256 * 1024;

pub(crate) fn open_file(path: &Path, key: &str, owner: u64) -> Result<Opened, String> {
    let mut file = File::open(path).map_err(|error| format!("Couldn't open the file: {error}"))?;
    let size = file.metadata().map_err(|error| format!("Couldn't read the file: {error}"))?.len();
    let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("");
    let content_type = info::audio_type(extension).unwrap_or("application/octet-stream");
    info::begin(key, owner, info::describe_file(&mut file, content_type, size));
    let mut hint = Hint::new();
    if !extension.is_empty() {
        hint.with_extension(extension);
    }
    let source = FileSource { file, size, key: key.to_string(), owner };
    Ok(Opened { source: Box::new(source), hint, file: true, size: Some(size) })
}

struct FileSource {
    file: File,
    size: u64,
    key: String,
    owner: u64,
}

impl Read for FileSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buf)?;
        info::update(&self.key, self.owner, |info| info.add_bytes(read as u64));
        Ok(read)
    }
}

impl Seek for FileSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl MediaSource for FileSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.size)
    }
}

pub(crate) fn agent(user_agent: &str) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(user_agent)
        .build()
}

pub(crate) fn open_url(url: &str, key: &str, owner: u64, user_agent: &str) -> Result<Opened, String> {
    let agent = agent(user_agent);
    let response = agent.get(url).set("Icy-MetaData", "1").call().map_err(|error| match error {
        ureq::Error::Status(code, response) => format!("The server answered {code} {}", response.status_text()),
        ureq::Error::Transport(error) => format!("Couldn't connect: {error}"),
    })?;

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
    // The audio's length, unless ICY metadata is mixed in: then it's a stream.
    let size = header("Content-Length").and_then(|value| value.parse::<u64>().ok()).filter(|_| metaint.is_none());
    let ranges = header("Accept-Ranges").is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
    let content_type = text("Content-Type");
    let format = content_type.as_deref().and_then(info::container_format);
    info::begin(
        key,
        owner,
        Info {
            name: text("icy-name"),
            genre: text("icy-genre"),
            description: text("icy-description"),
            website: text("icy-url"),
            server: text("Server"),
            content_type: content_type.clone(),
            format: format.map(str::to_string),
            format_guessed: format.is_some(),
            bitrate: header("icy-br").and_then(number).or_else(|| from_audio_info("bitrate")),
            samplerate: from_audio_info("samplerate").or_else(|| header("icy-sr").and_then(number)),
            channels: from_audio_info("channels"),
            metaint,
            size,
            ..Default::default()
        },
    );

    let mut hint = Hint::new();
    if let Some(content_type) = &content_type {
        let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        hint.mime_type(&mime);
        let extension = match mime.as_str() {
            "audio/mpeg" | "audio/mp3" | "audio/mpeg3" => Some("mp3"),
            "audio/aac" | "audio/aacp" | "audio/x-aac" => Some("aac"),
            "audio/ogg" | "application/ogg" | "audio/vorbis" => Some("ogg"),
            "audio/opus" => Some("opus"),
            "audio/flac" | "audio/x-flac" => Some("flac"),
            "audio/mp4" | "audio/x-m4a" => Some("m4a"),
            "audio/webm" => Some("webm"),
            "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
            _ => None,
        };
        if let Some(extension) = extension {
            hint.with_extension(extension);
        }
    }
    if let Some(extension) = url::Url::parse(url)
        .ok()
        .and_then(|url| Path::new(url.path()).extension().and_then(|e| e.to_str()).map(str::to_string))
        .filter(|extension| info::audio_type(extension).is_some())
    {
        hint.with_extension(&extension);
    }

    let source = HttpSource {
        agent,
        url: url.to_string(),
        key: key.to_string(),
        owner,
        body: Some(response.into_reader()),
        body_pos: 0,
        parked: None,
        pos: 0,
        size,
        seekable: size.is_some() && ranges,
        metaint,
        until_meta: metaint.unwrap_or(usize::MAX),
        probe: Some(Vec::new()),
        head: Vec::new(),
    };
    Ok(Opened { source: Box::new(source), hint, file: size.is_some(), size })
}

struct HttpSource {
    agent: ureq::Agent,
    url: String,
    key: String,
    owner: u64,
    // the connection, and the position of the next byte it gives
    body: Option<Body>,
    body_pos: u64,
    // a connection left aside by a far seek (the demuxer looking at the end of the file), taken
    // up again when reading comes back to where it was
    parked: Option<(Body, u64)>,
    // the position the next read should start at
    pos: u64,
    size: Option<u64>,
    seekable: bool,
    metaint: Option<usize>,
    until_meta: usize,
    // the first bytes, until an MP3 or ADTS frame header is found in them
    probe: Option<Vec<u8>>,
    // the start of a file, as read
    head: Vec<u8>,
}

impl HttpSource {
    // Gets a connection that gives the byte at `pos` next.
    fn reposition(&mut self) -> io::Result<()> {
        let pos = self.pos;
        if let Some(body) = self.body.as_mut() {
            if pos >= self.body_pos && pos - self.body_pos <= SKIP_LIMIT {
                skip(body, pos - self.body_pos)?;
                self.body_pos = pos;
                return Ok(());
            }
        }
        let current = self.body.take().map(|body| (body, self.body_pos));
        if let Some((mut body, at)) = self.parked.take() {
            if pos >= at && pos - at <= SKIP_LIMIT {
                skip(&mut body, pos - at)?;
                self.parked = current;
                self.body = Some(body);
                self.body_pos = pos;
                return Ok(());
            }
        }
        // Keep the connection nearest the start: the one reading the file through, while the
        // demuxer looks at the end and comes back.
        if let Some((body, at)) = current {
            if self.parked.as_ref().is_none_or(|(_, parked)| at < *parked) {
                self.parked = Some((body, at));
            }
        }
        let response = self
            .agent
            .get(&self.url)
            .set("Range", &format!("bytes={pos}-"))
            .call()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let partial = response.status() == 206;
        let mut body = response.into_reader();
        // A server that ignores the range sends the file from the start: read up to the
        // position, which is only bearable for a short way.
        if !partial && pos > 0 {
            if pos > 4 * SKIP_LIMIT {
                self.seekable = false;
                return Err(io::Error::other("the server doesn't take ranges"));
            }
            skip(&mut body, pos)?;
        }
        self.body = Some(body);
        self.body_pos = pos;
        Ok(())
    }
}

fn skip(body: &mut Body, bytes: u64) -> io::Result<()> {
    let skipped = io::copy(&mut body.take(bytes), &mut io::sink())?;
    if skipped < bytes {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(())
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // A file cut short is an error, but the demuxers take an unexpected end for the end of
        // the audio: give it another kind, so the player reconnects rather than moves on.
        self.read_audio(buf).map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof if self.size.is_some() => io::Error::new(io::ErrorKind::ConnectionAborted, error),
            _ => error,
        })
    }
}

impl HttpSource {
    fn read_audio(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.size.is_some_and(|size| self.pos >= size) {
            return Ok(0);
        }
        if let Some(cached) = self.head.get(self.pos as usize..).filter(|cached| !cached.is_empty()) {
            let count = cached.len().min(buf.len());
            buf[..count].copy_from_slice(&cached[..count]);
            self.pos += count as u64;
            return Ok(count);
        }
        if self.body.is_none() || self.body_pos != self.pos {
            if !self.seekable {
                return Err(io::Error::other("the stream can't seek"));
            }
            self.reposition()?;
        }
        let Some(body) = self.body.as_mut() else {
            return Err(io::Error::other("no connection"));
        };
        if self.until_meta == 0 {
            let mut length = [0u8; 1];
            body.read_exact(&mut length)?;
            let mut meta = vec![0u8; length[0] as usize * 16];
            body.read_exact(&mut meta)?;
            let title = info::stream_title(&meta);
            info::update(&self.key, self.owner, |info| {
                info.meta_bytes += 1 + meta.len() as u64;
                if let Some(title) = title {
                    info.icy_titled = true;
                    info.set_title(title);
                }
            });
            self.until_meta = self.metaint.unwrap_or(usize::MAX);
        }
        let wanted = buf.len().min(self.until_meta);
        let read = body.read(&mut buf[..wanted])?;
        if read == 0 {
            // A file must arrive whole; a stream may end whenever the server likes.
            if self.size.is_some_and(|size| self.pos < size) {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "the connection closed early"));
            }
            return Ok(0);
        }
        if self.metaint.is_some() {
            self.until_meta -= read;
        }
        if self.seekable && self.pos == self.head.len() as u64 && self.head.len() < HEAD_LIMIT {
            self.head.extend_from_slice(&buf[..read.min(HEAD_LIMIT - self.head.len())]);
        }
        self.pos += read as u64;
        self.body_pos = self.pos;

        let frame = self.probe.as_mut().and_then(|bytes| {
            bytes.extend_from_slice(&buf[..read]);
            info::parse_frame(bytes.get(info::id3_length(bytes)..).unwrap_or_default())
        });
        if frame.is_some() || self.probe.as_ref().is_some_and(|bytes| bytes.len() > info::id3_length(bytes) + 64 * 1024) {
            self.probe = None;
        }
        info::update(&self.key, self.owner, |info| {
            if let Some(frame) = frame {
                info.apply_frame(&frame);
            }
            info.add_bytes(read as u64);
        });
        Ok(read)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
            SeekFrom::End(delta) => self.size.and_then(|size| size.checked_add_signed(delta)),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        if target != self.pos && !self.seekable {
            return Err(io::Error::other("the stream can't seek"));
        }
        // Connecting waits for the next read: the demuxer often seeks several times in a row.
        self.pos = target;
        Ok(target)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        self.seekable
    }

    fn byte_len(&self) -> Option<u64> {
        self.size
    }
}
