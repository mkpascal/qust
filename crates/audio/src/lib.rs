//! Qust's audio engine: HTTP and file sources, demuxing and decoding with Symphonia, resampling to
//! one mixing rate, a lock-free mixer, and output to the default device (or nowhere, for tests).
//!
//! Each loaded stream runs on its own thread and fills a ring the mixer drains in real time,
//! muted or not, so a muted stream stays at the live point ("instant switch"). A separate thread
//! turns what the mixer and the decoders report into events, so the handler may call back in.

mod decode;
mod info;
mod mixer;
mod output;
mod ring;
mod source;
mod stream;

pub use info::{icecast_title, info};
pub use mixer::Tap;

use mixer::MixShared;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};
use stream::{lock, Stream, PLAYING};
use symphonia::core::codecs::registry::CodecRegistry;

// How much a stream buffers ahead: a live stream keeps the burst its server sends on connect,
// which is what makes later hiccups survivable; a local file needs little.
const STREAM_SECONDS: usize = 8;
const FILE_SECONDS: usize = 3;
// An unloaded stream fades out before the mixer lets go of it.
const CLOSE_DELAY: Duration = Duration::from_millis(40);
// How often the event thread looks for changes.
const EVENT_INTERVAL: Duration = Duration::from_millis(20);
// The mixing rate when there is no device to follow.
const DEFAULT_RATE: u32 = 48000;

pub struct Config {
    pub codecs: &'static CodecRegistry,
    pub output: OutputKind,
    pub user_agent: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputKind {
    /// The default output device, followed when it changes.
    Device,
    /// No sound device: a thread consumes the mix in real time at 48 kHz.
    Null,
}

#[derive(Clone, Debug)]
pub enum Source {
    Url(String),
    File(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StreamId(pub u64);

#[derive(Clone, Debug)]
pub enum Event {
    /// Audio started coming out, or came back after `Waiting`.
    Playing { key: String, id: StreamId },
    /// It was playing and ran out of buffered audio.
    Waiting { key: String, id: StreamId },
    /// End of file, or the server closed the stream; all audio played.
    Ended { key: String, id: StreamId },
    /// Couldn't connect, open or decode; or the connection failed (after the buffered audio played).
    Failed { key: String, id: StreamId, message: String },
    Duration { key: String, id: StreamId, seconds: Option<f64> },
}

#[derive(Clone, Debug)]
pub struct Status {
    pub id: StreamId,
    /// Has output audio at least once.
    pub started: bool,
    /// Decoding, buffered ahead and being consumed right now (not paused, not waiting).
    pub live: bool,
    pub paused: bool,
    pub ended: bool,
    pub failed: bool,
    /// Files: the timestamp at the play head. Streams: same as `played`.
    pub position: f64,
    /// Seconds consumed since this stream was loaded (muted time included).
    pub played: f64,
    /// Some = a file (local, or online with a known length); None = live.
    pub duration: Option<f64>,
    /// Seconds decoded ahead of the play head.
    pub buffered: f64,
}

type Handler = Box<dyn Fn(Event) + Send + Sync>;

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    codecs: &'static CodecRegistry,
    user_agent: String,
    rate: u32,
    streams: Mutex<HashMap<String, Arc<Stream>>>,
    // unloaded or replaced streams fading out, until they leave the mixer
    closing: Mutex<Vec<(Instant, Arc<Stream>)>>,
    mix: Arc<MixShared>,
    // the output thread, woken whenever the streams change
    output: OnceLock<thread::Thread>,
}

// Stream ids, unique across engines: info() is shared, and a stream's details belong to the
// newest id under its key.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

// The last Engine handle gone: the decode threads stop too.
impl Drop for Inner {
    fn drop(&mut self) {
        for stream in lock(&self.streams).values() {
            stream.stop.store(true, Ordering::Relaxed);
        }
        if let Some(output) = self.output.get() {
            output.unpark();
        }
    }
}

impl Inner {
    // Gives the mixer the current list of streams.
    pub(crate) fn publish(&self) {
        let mut list: Vec<Arc<Stream>> = lock(&self.streams).values().cloned().collect();
        list.extend(lock(&self.closing).iter().map(|(_, stream)| stream.clone()));
        self.mix.publish(list);
        // The output opens when the first stream comes, and starts counting down to closing
        // when the last one has gone.
        if let Some(output) = self.output.get() {
            output.unpark();
        }
    }

    // Anything loaded, or still fading out: the output should be open.
    pub(crate) fn busy(&self) -> bool {
        !lock(&self.streams).is_empty() || !lock(&self.closing).is_empty()
    }

    fn close(&self, stream: Arc<Stream>) {
        stream.stop.store(true, Ordering::Relaxed);
        stream.closing.store(true, Ordering::Relaxed);
        lock(&self.closing).push((Instant::now(), stream));
    }

    fn get(&self, key: &str) -> Option<Arc<Stream>> {
        lock(&self.streams).get(key).cloned()
    }
}

impl Engine {
    pub fn new(config: Config, on_event: impl Fn(Event) + Send + Sync + 'static) -> Engine {
        let rate = match config.output {
            OutputKind::Device => output::device_rate().unwrap_or(DEFAULT_RATE),
            OutputKind::Null => DEFAULT_RATE,
        };
        let mix = Arc::new(MixShared::new(rate));
        let inner = Arc::new(Inner {
            codecs: config.codecs,
            user_agent: config.user_agent,
            rate,
            streams: Mutex::new(HashMap::new()),
            closing: Mutex::new(Vec::new()),
            mix: mix.clone(),
            output: OnceLock::new(),
        });
        let weak = Arc::downgrade(&inner);
        // No device stream is opened here: only once something is loaded.
        let output = match config.output {
            OutputKind::Device => output::start_device(weak.clone(), mix),
            OutputKind::Null => output::start_null(weak.clone(), mix),
        };
        if let Some(output) = output {
            let _ = inner.output.set(output);
        }
        let handler: Handler = Box::new(on_event);
        let _ = thread::Builder::new().name("qust-events".into()).spawn(move || events(weak, handler));
        Engine { inner }
    }

    /// Connects or opens and starts decoding. Replaces a stream already loaded under `key` with a
    /// fresh connection. A new stream has level 0 and is not paused: it is consumed in real time
    /// as soon as it has audio (muted until its level is raised). `start_at` seeks a file first.
    pub fn load(&self, key: &str, source: Source, start_at: Option<f64>) -> StreamId {
        let inner = &self.inner;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let seconds = if matches!(source, Source::File(_)) { FILE_SECONDS } else { STREAM_SECONDS };
        let stream = Arc::new(Stream::new(id, key, seconds * inner.rate as usize, inner.rate));
        if let Some(at) = start_at.filter(|at| *at > 0.0) {
            lock(&stream.segment).pending = Some(at);
        }
        let old = lock(&inner.streams).insert(key.to_string(), stream.clone());
        if let Some(old) = old {
            inner.close(old);
        }
        inner.publish();
        let job = decode::Job { stream: stream.clone(), source, start_at, codecs: inner.codecs, user_agent: inner.user_agent.clone() };
        let spawned = thread::Builder::new().name("qust-stream".into()).spawn(move || decode::run(job));
        if spawned.is_err() {
            *lock(&stream.error) = Some("Couldn't start the stream".to_string());
            stream.finished.store(true, Ordering::Release);
        }
        StreamId(id)
    }

    pub fn unload(&self, key: &str) {
        let old = lock(&self.inner.streams).remove(key);
        if let Some(old) = old {
            self.inner.close(old);
            self.inner.publish();
        }
    }

    pub fn unload_all(&self) {
        let old: Vec<_> = lock(&self.inner.streams).drain().map(|(_, stream)| stream).collect();
        if old.is_empty() {
            return;
        }
        for stream in old {
            self.inner.close(stream);
        }
        self.inner.publish();
    }

    pub fn keys(&self) -> Vec<String> {
        lock(&self.inner.streams).keys().cloned().collect()
    }

    pub fn status(&self, key: &str) -> Option<Status> {
        let stream = self.inner.get(key)?;
        let rate = stream.rate as f64;
        let played = stream.consumed.load(Ordering::Relaxed) as f64 / rate;
        let paused = stream.paused.load(Ordering::Relaxed);
        let done = stream.is_done();
        let failed = done && lock(&stream.error).is_some();
        let duration = *lock(&stream.duration);
        let file = stream.file.load(Ordering::Relaxed) && duration.is_some();
        Some(Status {
            id: StreamId(stream.id),
            started: stream.transitions.load(Ordering::Acquire) > 0,
            live: !paused && !done && stream.phase.load(Ordering::Relaxed) == PLAYING,
            paused,
            ended: done && !failed,
            failed,
            // Decoder padding can run a hair past the stated length.
            position: if file { stream.position().min(duration.unwrap_or(f64::MAX)) } else { played },
            played,
            duration: if file { duration } else { None },
            buffered: stream.ring.available() as f64 / rate,
        })
    }

    /// 0.0 or 1.0, reached with a ~15 ms fade.
    pub fn set_level(&self, key: &str, level: f32) {
        if let Some(stream) = self.inner.get(key) {
            stream.level.store(level.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    /// Stops consuming (after a short fade); the buffer fills and the source waits.
    pub fn set_paused(&self, key: &str, paused: bool) {
        if let Some(stream) = self.inner.get(key) {
            stream.paused.store(paused, Ordering::Relaxed);
        }
    }

    /// Files only.
    pub fn seek(&self, key: &str, seconds: f64) {
        let Some(stream) = self.inner.get(key) else { return };
        if !stream.file.load(Ordering::Relaxed) || !seconds.is_finite() {
            return;
        }
        let seconds = seconds.max(0.0);
        lock(&stream.segment).pending = Some(seconds);
        *lock(&stream.seek) = Some(seconds);
    }

    /// 0.0..=1.0, after the analyzer tap.
    pub fn set_volume(&self, volume: f32) {
        self.inner.mix.set_volume(volume);
    }

    /// -1.0..=1.0, Web Audio StereoPannerNode law (stereo input).
    pub fn set_balance(&self, balance: f32) {
        self.inner.mix.set_balance(balance);
    }

    pub fn tap(&self) -> Arc<Tap> {
        self.inner.mix.tap.clone()
    }
}

// The event thread: what the mixer and the decoders noted, as events, in order per stream. It
// also lets go of streams that have faded out, and drops the lists the mixer handed back.
fn events(engine: Weak<Inner>, handler: Handler) {
    let mut pending = Vec::new();
    loop {
        thread::sleep(EVENT_INTERVAL);
        let Some(inner) = engine.upgrade() else { return };
        let streams: Vec<Arc<Stream>> = lock(&inner.streams).values().cloned().collect();
        let faded = {
            let mut closing = lock(&inner.closing);
            let before = closing.len();
            closing.retain(|(since, _)| since.elapsed() < CLOSE_DELAY);
            closing.len() != before
        };
        if faded {
            inner.publish();
        }
        inner.mix.collect();
        drop(inner);

        for stream in &streams {
            let (key, id) = (stream.key.clone(), StreamId(stream.id));
            if stream.duration_known.load(Ordering::Acquire) && !stream.duration_sent.swap(true, Ordering::AcqRel) {
                let seconds = *lock(&stream.duration);
                pending.push(Event::Duration { key: key.clone(), id, seconds });
            }
            let transitions = stream.transitions.load(Ordering::Acquire);
            let announced = stream.announced.swap(transitions, Ordering::AcqRel);
            for n in announced + 1..=transitions {
                pending.push(if n % 2 == 1 { Event::Playing { key: key.clone(), id } } else { Event::Waiting { key: key.clone(), id } });
            }
            let epoch = stream.epoch.load(Ordering::Acquire);
            if stream.finished.load(Ordering::Acquire) && stream.ring.available() == 0 && stream.done.load(Ordering::Acquire) != epoch + 1 {
                stream.done.store(epoch + 1, Ordering::Release);
                pending.push(match lock(&stream.error).clone() {
                    Some(message) => Event::Failed { key, id, message },
                    None => Event::Ended { key, id },
                });
            }
        }
        drop(streams);
        for event in pending.drain(..) {
            // An unloaded stream has nothing more to say.
            let id = match &event {
                Event::Playing { id, .. } | Event::Waiting { id, .. } | Event::Ended { id, .. } | Event::Failed { id, .. } | Event::Duration { id, .. } => *id,
            };
            let key = match &event {
                Event::Playing { key, .. } | Event::Waiting { key, .. } | Event::Ended { key, .. } | Event::Failed { key, .. } | Event::Duration { key, .. } => key,
            };
            let current = engine.upgrade().and_then(|inner| inner.get(key)).is_some_and(|stream| stream.id == id.0);
            if current {
                handler(event);
            }
        }
    }
}

/// A local file's duration, without decoding it. Ok(None): playable but unknown. Err: can't be
/// opened or isn't audio.
pub fn probe_duration(path: &Path, codecs: &'static CodecRegistry) -> Result<Option<f64>, String> {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).map_err(|error| format!("Couldn't open the file: {error}"))?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let format = symphonia::default::get_probe()
        .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
        .map_err(|_| "Not a format Qust can play".to_string())?;
    let track = format.default_track(TrackType::Audio).ok_or("No audio in the file")?;
    let params = track.codec_params.as_ref().and_then(|params| params.audio()).ok_or("No audio in the file")?;
    codecs.get_audio_decoder(params.codec).ok_or("Qust can't decode this file")?;
    let seconds = track.time_base.and_then(|base| {
        let duration = track.duration.or_else(|| track.num_frames.map(Into::into))?;
        base.calc_duration(duration).map(|time| time.as_secs_f64())
    });
    Ok(seconds.filter(|seconds| *seconds > 0.0))
}
