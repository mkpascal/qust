// One thread per stream: open the source, demux and decode with Symphonia, turn the audio into
// stereo at the mixing rate and fill the stream's ring. A full ring blocks the thread, so the
// network backs up as it would in a browser.
//
// A live stream is allowed to be messy: joining mid-frame or a corrupt packet only skips that
// packet. It gives up on an I/O error, an unsupported format, or a long run of errors. A chained
// Ogg stream (a new logical stream per track) gets a new decoder at each change.

use crate::info;
use crate::source::{self, Opened};
use crate::stream::{lock, Segment, Stream};
use crate::Source;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::io;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use symphonia::core::audio::{Channels, GenericAudioBufferRef, Position};
use symphonia::core::codecs::audio::well_known::profiles::{CODEC_PROFILE_AAC_HE, CODEC_PROFILE_AAC_HE_V2, CODEC_PROFILE_AAC_LC};
use symphonia::core::codecs::audio::well_known::*;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::{Error, SeekErrorKind};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTag};
use symphonia::core::units::{Time, TimeBase};

// Consecutive packets that fail before the stream counts as undecodable.
const MAX_ERRORS: u32 = 100;
// How long the thread sleeps while the ring is full, or while a file waits at its end.
const IDLE_SLEEP: Duration = Duration::from_millis(15);

pub(crate) struct Job {
    pub stream: Arc<Stream>,
    pub source: Source,
    pub start_at: Option<f64>,
    pub codecs: &'static CodecRegistry,
    pub user_agent: String,
}

pub(crate) fn run(job: Job) {
    let stream = job.stream.clone();
    if let Err(message) = play(job) {
        if !stream.stopped() {
            *lock(&stream.error) = Some(message);
        }
    }
    stream.finished.store(true, Ordering::Release);
}

fn play(job: Job) -> Result<(), String> {
    let stream = job.stream;
    let (key, owner) = (stream.key.as_str(), stream.id);
    let opened = match &job.source {
        Source::Url(url) => source::open_url(url, key, owner, &job.user_agent)?,
        Source::File(path) => source::open_file(path, key, owner)?,
    };
    if stream.stopped() {
        return Ok(());
    }
    let Opened { source, hint, file, size } = opened;
    let local = matches!(job.source, Source::File(_));
    stream.file.store(file, Ordering::Relaxed);
    let format = open_format(source, &hint)?;
    let mut decoding = Decoding {
        stream: stream.clone(),
        codecs: job.codecs,
        format,
        decoder: None,
        track: 0,
        time_base: None,
        converter: Converter::new(stream.rate),
        skip_until: None,
        cursor: None,
        file,
        // A local file seeks exactly; over HTTP an exact MP3 seek would read the whole file up to
        // the target, so it seeks by the byte estimate.
        seek_mode: if local { SeekMode::Accurate } else { SeekMode::Coarse },
        described: false,
    };
    decoding.select_track()?;

    let duration = decoding.duration().or_else(|| {
        // A constant-bitrate MP3 or ADTS file without a header that gives its length.
        let bitrate = info::get(key, |info| info.bitrate).flatten()?;
        let size = size?;
        (file && bitrate > 0).then(|| size as f64 * 8.0 / (bitrate as f64 * 1000.0))
    });
    let duration = if file { duration } else { None };
    info::update(key, owner, |info| info.duration = duration);
    stream.set_duration(duration);
    if !file {
        decoding.record_title();
    }
    if let Some(at) = job.start_at.filter(|at| file && *at > 0.0) {
        decoding.seek(at)?;
    }
    decoding.run()
}

fn open_format(source: Box<dyn MediaSource>, hint: &Hint) -> Result<Box<dyn FormatReader>, String> {
    let stream = MediaSourceStream::new(source, Default::default());
    symphonia::default::get_probe()
        .probe(hint, stream, FormatOptions::default(), MetadataOptions::default())
        .map_err(|error| match error {
            Error::IoError(error) => connection_error(&error),
            _ => "Not a format Qust can play".to_string(),
        })
}

fn connection_error(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::UnexpectedEof => "The stream ended before any audio".to_string(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "The connection timed out".to_string(),
        _ => format!("The connection failed: {error}"),
    }
}

struct Decoding {
    stream: Arc<Stream>,
    codecs: &'static CodecRegistry,
    format: Box<dyn FormatReader>,
    decoder: Option<Box<dyn AudioDecoder>>,
    track: u32,
    time_base: Option<TimeBase>,
    converter: Converter,
    // after a seek: audio before this time (seconds) is dropped
    skip_until: Option<f64>,
    // meanwhile, the time of the next decoded frame
    cursor: Option<f64>,
    file: bool,
    seek_mode: SeekMode,
    // the decoder's details went into info()
    described: bool,
}

enum Step {
    Continue,
    End,
}

impl Decoding {
    // The first audio track, and a decoder for it (again after a chained Ogg stream changes).
    fn select_track(&mut self) -> Result<(), String> {
        let track = self.format.default_track(TrackType::Audio).ok_or("No audio in the stream")?;
        let params = track.codec_params.as_ref().and_then(|params| params.audio()).ok_or("No audio in the stream")?;
        let decoder = self
            .codecs
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|_| format!("Qust can't decode {}", codec_name(params).unwrap_or("this format")))?;
        self.track = track.id;
        self.time_base = track.time_base;
        self.decoder = Some(decoder);
        self.described = false;
        Ok(())
    }

    fn duration(&self) -> Option<f64> {
        let track = self.format.tracks().iter().find(|track| track.id == self.track)?;
        let time_base = track.time_base?;
        let duration = track.duration.or_else(|| track.num_frames.map(Into::into))?;
        let seconds = time_base.calc_duration(duration)?.as_secs_f64();
        (seconds > 0.0).then_some(seconds)
    }

    // An Ogg stream carries its titles as Vorbis comments, some alongside empty ICY titles. ICY
    // titles win when there are any.
    fn record_title(&mut self) {
        let Some(title) = tag_title(self.format.as_mut()) else { return };
        info::update(&self.stream.key, self.stream.id, |info| {
            if !info.icy_titled {
                info.set_title(title);
            }
        });
    }

    fn run(&mut self) -> Result<(), String> {
        let mut errors = 0;
        loop {
            if self.stream.stopped() {
                return Ok(());
            }
            let request = lock(&self.stream.seek).take();
            if let Some(at) = request {
                self.seek(at)?;
                continue;
            }
            match self.step(&mut errors)? {
                Step::Continue => {}
                Step::End => {
                    self.finish();
                    self.stream.finished.store(true, Ordering::Release);
                    if !self.file {
                        return Ok(());
                    }
                    // A file stays open at its end: the player may seek back into it.
                    loop {
                        if self.stream.stopped() {
                            return Ok(());
                        }
                        if lock(&self.stream.seek).is_some() {
                            break;
                        }
                        thread::sleep(IDLE_SLEEP);
                    }
                }
            }
            if errors > MAX_ERRORS {
                return Err("Couldn't decode the stream".to_string());
            }
        }
    }

    fn step(&mut self, errors: &mut u32) -> Result<Step, String> {
        let packet = match self.format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => return Ok(Step::End),
            Err(Error::ResetRequired) => {
                // A new logical stream (the next track of a chained Ogg stream).
                self.converter.finish();
                self.flush_output();
                self.select_track()?;
                if !self.file {
                    self.record_title();
                }
                return Ok(Step::Continue);
            }
            Err(Error::IoError(error)) => {
                if self.stream.stopped() {
                    return Ok(Step::End);
                }
                // The server closing a live stream is its end.
                if !self.file && error.kind() == io::ErrorKind::UnexpectedEof {
                    return Ok(Step::End);
                }
                return Err(connection_error(&error));
            }
            Err(Error::Unsupported(_)) => return Err("Not a format Qust can play".to_string()),
            Err(_) => {
                *errors += 1;
                return Ok(Step::Continue);
            }
        };
        if packet.track_id != self.track {
            return Ok(Step::Continue);
        }
        // After a seek, the audio starts at the first packet's time and goes on by the frames
        // decoded: packet times can't be matched to buffers, since some decoders (AAC) give out
        // the previous packet's audio, and HE-AAC has twice the frames packet durations say.
        if self.skip_until.is_some() && self.cursor.is_none() {
            self.cursor = Some(self.time_base.and_then(|base| base.calc_time(packet.pts)).map_or(0.0, |time| time.as_secs_f64()));
        }
        let Some(decoder) = self.decoder.as_mut() else {
            return Err("No decoder".to_string());
        };
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(Error::ResetRequired) => {
                self.select_track()?;
                return Ok(Step::Continue);
            }
            Err(Error::Unsupported(_)) => return Err("Qust can't decode this stream".to_string()),
            Err(_) => {
                *errors += 1;
                return Ok(Step::Continue);
            }
        };
        *errors = 0;
        if buffer.frames() == 0 {
            return Ok(Step::Continue);
        }
        let rate = buffer.spec().rate();
        let channels = buffer.spec().channels().count();
        let mut drop = 0;
        if let (Some(until), Some(start)) = (self.skip_until, self.cursor) {
            drop = ((until - start).max(0.0) * rate as f64).round() as usize;
            self.cursor = Some(start + buffer.frames() as f64 / rate as f64);
            if drop < buffer.frames() {
                self.skip_until = None;
                self.cursor = None;
            }
        }
        if drop < buffer.frames() {
            self.converter.add(&buffer, drop);
        }
        if !self.described {
            self.described = true;
            let params = decoder.codec_params();
            let name = codec_name(params).unwrap_or(decoder.codec_info().short_name);
            let name = name.to_string();
            info::update(&self.stream.key, self.stream.id, |info| {
                if info.format.is_none() || info.format_guessed {
                    info.format = Some(name);
                    info.format_guessed = false;
                }
                info.samplerate.get_or_insert(rate);
                info.channels.get_or_insert(channels as u32);
            });
        }
        self.flush_output();
        Ok(Step::Continue)
    }

    // Moves the converted audio into the ring, waiting for room. Gives up on it when the stream
    // is stopped or a seek comes (which drops it anyway).
    fn flush_output(&mut self) {
        let mut offset = 0;
        let output = &self.converter.output;
        while offset < output.len() {
            offset += self.stream.ring.push(&output[offset..]) * 2;
            if offset >= output.len() || self.stream.stopped() || lock(&self.stream.seek).is_some() {
                break;
            }
            thread::sleep(IDLE_SLEEP);
        }
        self.converter.output.clear();
    }

    // The end of the audio: what the resampler still holds.
    fn finish(&mut self) {
        self.converter.finish();
        self.flush_output();
    }

    fn seek(&mut self, at: f64) -> Result<(), String> {
        let at = at.max(0.0);
        self.stream.epoch.fetch_add(1, Ordering::AcqRel);
        self.stream.finished.store(false, Ordering::Release);
        let result = match Time::try_from_secs_f64(at) {
            Some(time) => self.format.seek(self.seek_mode, SeekTo::Time { time, track_id: Some(self.track) }),
            None => Err(Error::SeekError(SeekErrorKind::OutOfRange)),
        };
        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }
        self.converter.reset();
        let frame = self.stream.ring.flush();
        self.stream.reprime.store(true, Ordering::Release);
        match result {
            Ok(_) => {
                self.skip_until = Some(at);
                self.cursor = None;
                *lock(&self.stream.segment) = Segment { frame, time: at, pending: None };
                Ok(())
            }
            Err(Error::IoError(error)) => Err(connection_error(&error)),
            Err(_) => {
                // Past the end, or the format can't seek there: the file ends here.
                let end = lock(&self.stream.duration).unwrap_or(at).min(at);
                *lock(&self.stream.segment) = Segment { frame, time: end, pending: None };
                self.skip_until = None;
                *lock(&self.stream.seek) = None;
                self.stream.finished.store(true, Ordering::Release);
                // Nothing more to decode until another seek.
                loop {
                    if self.stream.stopped() || lock(&self.stream.seek).is_some() {
                        return Ok(());
                    }
                    thread::sleep(IDLE_SLEEP);
                }
            }
        }
    }
}

fn codec_name(params: &AudioCodecParameters) -> Option<&'static str> {
    let codec = params.codec;
    let name = if codec == CODEC_ID_MP3 {
        "MP3"
    } else if codec == CODEC_ID_MP2 {
        "MP2"
    } else if codec == CODEC_ID_MP1 {
        "MP1"
    } else if codec == CODEC_ID_AAC {
        match params.profile {
            Some(profile) if profile == CODEC_PROFILE_AAC_HE => "HE-AAC",
            Some(profile) if profile == CODEC_PROFILE_AAC_HE_V2 => "HE-AACv2",
            Some(profile) if profile == CODEC_PROFILE_AAC_LC => "AAC-LC",
            _ => "AAC",
        }
    } else if codec == CODEC_ID_VORBIS {
        "Vorbis"
    } else if codec == CODEC_ID_OPUS {
        "Opus"
    } else if codec == CODEC_ID_FLAC {
        "FLAC"
    } else if codec == CODEC_ID_ALAC {
        "ALAC"
    } else {
        return None;
    };
    Some(name)
}

// "Artist - Title" from the newest tags that have one (Vorbis comments in a chained Ogg stream).
// Every revision is looked at: a picture block (Ogg FLAC) comes as a revision of its own after
// the comments.
fn tag_title(format: &mut dyn FormatReader) -> Option<String> {
    let mut metadata = format.metadata();
    let mut found = None;
    loop {
        if let Some(title) = metadata.current().and_then(revision_title) {
            found = Some(title);
        }
        if metadata.pop().is_none() {
            return found;
        }
    }
}

fn revision_title(revision: &MetadataRevision) -> Option<String> {
    let (mut title, mut artist) = (None, None);
    let tags = revision.media.tags.iter().chain(revision.per_track.iter().flat_map(|track| track.metadata.tags.iter()));
    for tag in tags {
        match &tag.std {
            Some(StandardTag::TrackTitle(value)) if !value.trim().is_empty() => title = Some(value.trim().to_string()),
            Some(StandardTag::Artist(value)) if !value.trim().is_empty() => artist = Some(value.trim().to_string()),
            _ => {}
        }
    }
    match (artist, title) {
        (Some(artist), Some(title)) => Some(format!("{artist} - {title}")),
        (artist, title) => title.or(artist),
    }
}

// Decoded audio to interleaved stereo at the mixing rate. The rate and channel count come from
// each decoded buffer (an HE-AAC decoder may switch to twice the announced rate after the first
// packets), and the resampler is rebuilt whenever the rate changes.
pub(crate) struct Converter {
    to: u32,
    from: u32,
    planes: Vec<Vec<f32>>,
    // stereo at the source rate, waiting for a full resampler chunk
    input: Vec<f32>,
    // stereo at the mixing rate, ready for the ring
    pub output: Vec<f32>,
    resampler: Option<Fft<f32>>,
    scratch: Vec<f32>,
    // output frames still to drop: the resampler's delay
    delay: usize,
    fed: u64,
    made: u64,
}

impl Converter {
    pub fn new(to: u32) -> Converter {
        Converter {
            to,
            from: to,
            planes: Vec::new(),
            input: Vec::new(),
            output: Vec::new(),
            resampler: None,
            scratch: Vec::new(),
            delay: 0,
            fed: 0,
            made: 0,
        }
    }

    fn restart(&mut self, from: u32) {
        self.from = from;
        self.input.clear();
        self.resampler = None;
        if from != self.to && from > 0 {
            // FFT resampling: as good as the best sinc, and cheap for a fixed ratio.
            if let Ok(resampler) = Fft::<f32>::new(from as usize, self.to as usize, 1024, 2, FixedSync::Input) {
                self.scratch.resize(resampler.output_frames_max() * 2, 0.0);
                self.delay = resampler.output_delay();
                self.resampler = Some(resampler);
            }
        }
        self.fed = 0;
        self.made = 0;
    }

    // Drops what is buffered (a seek).
    pub fn reset(&mut self) {
        let from = self.from;
        self.restart(from);
        self.output.clear();
    }

    pub fn add(&mut self, buffer: &GenericAudioBufferRef<'_>, skip: usize) {
        let rate = buffer.spec().rate();
        if rate != self.from || (self.resampler.is_none() && rate != self.to) {
            self.restart(rate);
        }
        buffer.copy_to_vecs_planar::<f32>(&mut self.planes);
        let frames = buffer.frames();
        let target = if self.resampler.is_some() { &mut self.input } else { &mut self.output };
        stereo(&self.planes, buffer.spec().channels(), skip.min(frames), frames, target);
        if self.resampler.is_some() {
            self.resample();
        } else if rate != self.to {
            // No resampler could be made for this rate: nothing sensible to play.
            self.output.clear();
        }
    }

    fn resample(&mut self) {
        let Some(resampler) = self.resampler.as_mut() else { return };
        loop {
            let needed = resampler.input_frames_next();
            if self.input.len() / 2 < needed {
                return;
            }
            let frames_out = self.scratch.len() / 2;
            let (Ok(input), Ok(mut output)) = (
                InterleavedSlice::new(&self.input[..needed * 2], 2, needed),
                InterleavedSlice::new_mut(&mut self.scratch[..], 2, frames_out),
            ) else {
                return;
            };
            let Ok((read, written)) = resampler.process_into_buffer(&input, &mut output, None) else {
                self.input.clear();
                return;
            };
            self.input.drain(..read * 2);
            self.fed += read as u64;
            let dropped = self.delay.min(written);
            self.delay -= dropped;
            self.output.extend_from_slice(&self.scratch[dropped * 2..written * 2]);
            self.made += (written - dropped) as u64;
        }
    }

    // The end of the audio: pads with silence until the resampler has given out all of it.
    pub fn finish(&mut self) {
        let Some(resampler) = self.resampler.as_ref() else { return };
        let pending = (self.input.len() / 2) as u64;
        let expected = (self.fed + pending) * self.to as u64 / self.from as u64;
        let chunk = resampler.input_frames_max();
        let before = self.output.len();
        for _ in 0..8 {
            if self.made >= expected {
                break;
            }
            self.input.resize(self.input.len() + chunk * 2, 0.0);
            self.resample();
        }
        let excess = (self.made.saturating_sub(expected) as usize * 2).min(self.output.len() - before);
        self.output.truncate(self.output.len() - excess);
        self.input.clear();
        let from = self.from;
        self.restart(from);
    }
}

// Planar audio to interleaved stereo, from frame `start`. Mono plays on both sides; surround gets
// Web Audio's speaker downmix (centre and surrounds at -3 dB, LFE dropped).
fn stereo(planes: &[Vec<f32>], channels: &Channels, start: usize, frames: usize, out: &mut Vec<f32>) {
    let range = start..frames;
    out.reserve(range.len() * 2);
    match planes {
        [] => out.extend(range.flat_map(|_| [0.0, 0.0])),
        [mono] => out.extend(mono[range].iter().flat_map(|&s| [s, s])),
        [left, right] => out.extend(left[range.clone()].iter().zip(&right[range]).flat_map(|(&l, &r)| [l, r])),
        _ => {
            let plane = |position: Position| match channels {
                Channels::Positioned(present) if present.contains(position) => {
                    channels.get_canonical_index_for_positioned_channel(position).and_then(|index| planes.get(index))
                }
                _ => None,
            };
            let (left, right) = match (plane(Position::FRONT_LEFT), plane(Position::FRONT_RIGHT)) {
                (Some(left), Some(right)) => (left, right),
                _ => (&planes[0], &planes[1]),
            };
            let centre = plane(Position::FRONT_CENTER);
            let surround_left = plane(Position::SIDE_LEFT).or_else(|| plane(Position::REAR_LEFT));
            let surround_right = plane(Position::SIDE_RIGHT).or_else(|| plane(Position::REAR_RIGHT));
            let at = |plane: Option<&Vec<f32>>, i: usize| plane.map_or(0.0, |plane| plane[i]);
            const HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;
            for i in range {
                let c = at(centre, i);
                out.push(left[i] + HALF * (c + at(surround_left, i)));
                out.push(right[i] + HALF * (c + at(surround_right, i)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::audio::{AudioBuffer, AudioMut, AudioSpec};

    fn sine(rate: u32, frames: usize) -> AudioBuffer<f32> {
        let mut buffer = AudioBuffer::<f32>::new(AudioSpec::new(rate, Channels::Positioned(Position::FRONT_CENTER)), frames);
        buffer.render_uninit(Some(frames));
        if let Some(plane) = buffer.plane_mut(0) {
            for (i, sample) in plane.iter_mut().enumerate() {
                *sample = (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5;
            }
        }
        buffer
    }

    #[test]
    fn resamples_to_the_mixing_rate() {
        let mut converter = Converter::new(48000);
        let buffer = sine(44100, 44100);
        converter.add(&GenericAudioBufferRef::F32(&buffer), 0);
        converter.finish();
        let frames = converter.output.len() / 2;
        assert!((frames as i64 - 48000).abs() <= 2, "{frames} frames");
        // Mono on both sides, and the level kept.
        let peak = converter.output.iter().fold(0f32, |peak, s| peak.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "peak {peak}");
        assert_eq!(converter.output[2000], converter.output[2001]);
    }

    #[test]
    fn follows_a_rate_change() {
        let mut converter = Converter::new(48000);
        converter.add(&GenericAudioBufferRef::F32(&sine(48000, 4800)), 0);
        assert_eq!(converter.output.len(), 4800 * 2);
        converter.output.clear();
        converter.add(&GenericAudioBufferRef::F32(&sine(24000, 24000)), 0);
        converter.finish();
        let frames = converter.output.len() / 2;
        assert!((frames as i64 - 48000).abs() <= 2, "{frames} frames");
    }

    #[test]
    fn skips_frames_after_a_seek() {
        let mut converter = Converter::new(48000);
        converter.add(&GenericAudioBufferRef::F32(&sine(48000, 1000)), 600);
        assert_eq!(converter.output.len(), 400 * 2);
    }
}
