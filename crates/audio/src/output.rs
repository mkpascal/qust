// Where the mix goes: the default output device through cpal, or nowhere (Null) at real-time
// pace. The output is open only while something is loaded. The device is watched from its own
// thread: when it errors, disappears, or the system default changes, a new stream is opened on
// the current default, once a second until one works. The mix keeps its rate whatever the
// device's; a device at another rate gets it resampled.

use crate::mixer::{MixShared, Mixer};
use crate::Inner;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, ErrorKind, FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

// The most frames mixed in one go.
const BLOCK: usize = 1024;

// The default device's rate, asked on a thread of its own so the caller's COM setup (Windows)
// isn't touched.
pub(crate) fn device_rate() -> Option<u32> {
    thread::spawn(|| {
        let device = cpal::default_host().default_output_device()?;
        device.default_output_config().ok().map(|config| config.sample_rate())
    })
    .join()
    .ok()
    .flatten()
    .filter(|rate| *rate >= 8000)
}

// The output closes this long after the last stream has gone, so a device isn't held (and the
// PC kept awake) while nothing plays.
const IDLE_CLOSE: Duration = Duration::from_secs(3);
// How often an open device is checked (moved default, lost device), and a failed open retried.
const POLL: Duration = Duration::from_secs(1);
// How long the thread sleeps while closed and nothing wakes it.
const ASLEEP: Duration = Duration::from_secs(60);

// Whether the output should be open now: something is loaded, or the last stream went less than
// IDLE_CLOSE ago. Also says how long until it closes, if it is counting down.
struct Idle {
    since: Option<Instant>,
}

impl Idle {
    fn wanted(&mut self, busy: bool, open: bool) -> (bool, Option<Duration>) {
        if busy {
            self.since = None;
            return (true, None);
        }
        if !open {
            self.since = None;
            return (false, None);
        }
        let since = *self.since.get_or_insert_with(Instant::now);
        let left = IDLE_CLOSE.saturating_sub(since.elapsed());
        if left.is_zero() {
            self.since = None;
            (false, None)
        } else {
            (true, Some(left))
        }
    }
}

fn busy(engine: &Weak<Inner>) -> Option<bool> {
    engine.upgrade().map(|inner| inner.busy())
}

// Null: the same mixer at real-time pace from this thread, open on the same rule as a device.
pub(crate) fn start_null(engine: Weak<Inner>, shared: Arc<MixShared>) -> Option<thread::Thread> {
    let spawned = thread::Builder::new().name("qust-null-output".into()).spawn(move || {
        let rate = shared.rate as u64;
        let mut buffer = vec![0f32; BLOCK * 2];
        let mut idle = Idle { since: None };
        // the mixer, when it started, and the frames it has made since
        let mut open: Option<(Mixer, Instant, u64)> = None;
        while let Some(busy) = busy(&engine) {
            let (wanted, _) = idle.wanted(busy, open.is_some());
            if !wanted && open.is_some() {
                open = None;
                shared.tap.set_running(None);
            }
            if wanted && open.is_none() {
                open = Some((Mixer::new(shared.clone()), Instant::now(), 0));
                shared.tap.set_running(Some(shared.rate));
                if let Some(inner) = engine.upgrade() {
                    inner.publish();
                }
            }
            let Some((mixer, start, produced)) = open.as_mut() else {
                thread::park_timeout(ASLEEP);
                continue;
            };
            let due = (start.elapsed().as_secs_f64() * rate as f64) as u64;
            // After a hiccup (a suspended PC), carry on from now rather than race to catch up.
            *produced = (*produced).max(due.saturating_sub(rate / 10));
            while *produced < due {
                let frames = ((due - *produced) as usize).min(BLOCK);
                mixer.render(&mut buffer[..frames * 2]);
                shared.tap.set_latency(frames as u64);
                *produced += frames as u64;
            }
            thread::park_timeout(Duration::from_millis(5));
        }
        shared.tap.set_running(None);
    });
    spawned.ok().map(|handle| handle.thread().clone())
}

// The device: opened when the first stream is loaded (the engine wakes this thread), watched
// while open, and closed IDLE_CLOSE after the last one has gone.
pub(crate) fn start_device(engine: Weak<Inner>, shared: Arc<MixShared>) -> Option<thread::Thread> {
    let spawned = thread::Builder::new().name("qust-output".into()).spawn(move || {
        let host = cpal::default_host();
        let broken = Arc::new(AtomicBool::new(false));
        let mut idle = Idle { since: None };
        // The stream playing, and the device it was opened on.
        let mut current: Option<(cpal::Stream, Option<DeviceId>)> = None;
        while let Some(busy) = busy(&engine) {
            let (wanted, closing_in) = idle.wanted(busy, current.is_some());
            if !wanted && current.is_some() {
                current = None;
                shared.tap.set_running(None);
            }
            if current.is_some() {
                let id = host.default_output_device().and_then(|device| device.id().ok());
                let moved = current.as_ref().is_some_and(|(_, opened)| *opened != id);
                if broken.swap(false, Ordering::AcqRel) || moved {
                    current = None;
                    shared.tap.set_running(None);
                }
            }
            if current.is_none() && wanted {
                broken.store(false, Ordering::Release);
                if let Some(device) = host.default_output_device() {
                    let id = device.id().ok();
                    if let Ok(stream) = open(&device, &shared, &broken) {
                        current = Some((stream, id));
                        shared.tap.set_running(Some(shared.rate));
                        if let Some(inner) = engine.upgrade() {
                            inner.publish();
                        }
                    }
                }
            }
            let wait = match (&current, wanted) {
                (Some(_), _) => closing_in.map_or(POLL, |left| left.min(POLL)),
                // Wanted but no device yet: try again soon.
                (None, true) => POLL,
                (None, false) => ASLEEP,
            };
            thread::park_timeout(wait);
        }
        drop(current);
        shared.tap.set_running(None);
    });
    spawned.ok().map(|handle| handle.thread().clone())
}

fn open(device: &cpal::Device, shared: &Arc<MixShared>, broken: &Arc<AtomicBool>) -> Result<cpal::Stream, cpal::Error> {
    let supported = device.default_output_config()?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(device, config, shared, broken),
        SampleFormat::F64 => build::<f64>(device, config, shared, broken),
        SampleFormat::I16 => build::<i16>(device, config, shared, broken),
        SampleFormat::I24 => build::<cpal::I24>(device, config, shared, broken),
        SampleFormat::I32 => build::<i32>(device, config, shared, broken),
        SampleFormat::I8 => build::<i8>(device, config, shared, broken),
        SampleFormat::U8 => build::<u8>(device, config, shared, broken),
        SampleFormat::U16 => build::<u16>(device, config, shared, broken),
        SampleFormat::U32 => build::<u32>(device, config, shared, broken),
        _ => Err(ErrorKind::UnsupportedConfig.into()),
    }?;
    stream.play()?;
    Ok(stream)
}

fn build<T>(device: &cpal::Device, config: StreamConfig, shared: &Arc<MixShared>, broken: &Arc<AtomicBool>) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels.max(1) as usize;
    let mut output = DeviceOutput::new(shared.clone(), config.sample_rate);
    let supervisor = thread::current();
    let broken = broken.clone();
    device.build_output_stream(
        config,
        move |data: &mut [T], info: &OutputCallbackInfo| output.fill(data, channels, info),
        move |error: cpal::Error| match error.kind() {
            ErrorKind::Xrun | ErrorKind::RealtimeDenied => {}
            _ => {
                broken.store(true, Ordering::Release);
                supervisor.unpark();
            }
        },
        None,
    )
}

// The callback's state: the mixer, and a resampler when the device's rate isn't the mix's.
struct DeviceOutput {
    mixer: Mixer,
    shared: Arc<MixShared>,
    ratio: f64,
    resampler: Option<Fft<f32>>,
    mixed: Vec<f32>,
    resampled: Vec<f32>,
    fifo: Vec<f32>,
    out: Vec<f32>,
}

impl DeviceOutput {
    fn new(shared: Arc<MixShared>, device_rate: u32) -> DeviceOutput {
        let resampler = (device_rate != shared.rate)
            .then(|| Fft::<f32>::new(shared.rate as usize, device_rate as usize, 256, 2, FixedSync::Input).ok())
            .flatten();
        let input_max = resampler.as_ref().map_or(0, |r| r.input_frames_max());
        let output_max = resampler.as_ref().map_or(0, |r| r.output_frames_max());
        DeviceOutput {
            mixer: Mixer::new(shared.clone()),
            ratio: shared.rate as f64 / device_rate.max(1) as f64,
            shared,
            resampler,
            mixed: vec![0.0; BLOCK.max(input_max) * 2],
            resampled: vec![0.0; output_max * 2],
            fifo: Vec::with_capacity((BLOCK + output_max) * 4),
            out: vec![0.0; BLOCK * 2],
        }
    }

    // Leaves the next `frames` frames at the device's rate in `self.out`.
    fn produce(&mut self, frames: usize) {
        let Some(resampler) = self.resampler.as_mut() else {
            self.mixer.render(&mut self.out[..frames * 2]);
            return;
        };
        while self.fifo.len() < frames * 2 {
            let needed = resampler.input_frames_next();
            self.mixer.render(&mut self.mixed[..needed * 2]);
            let capacity = self.resampled.len() / 2;
            let (Ok(input), Ok(mut output)) = (
                InterleavedSlice::new(&self.mixed[..needed * 2], 2, needed),
                InterleavedSlice::new_mut(&mut self.resampled[..], 2, capacity),
            ) else {
                break;
            };
            match resampler.process_into_buffer(&input, &mut output, None) {
                Ok((_, written)) => self.fifo.extend_from_slice(&self.resampled[..written * 2]),
                Err(_) => break,
            }
        }
        let ready = self.fifo.len().min(frames * 2);
        self.out[..ready].copy_from_slice(&self.fifo[..ready]);
        self.out[ready..frames * 2].fill(0.0);
        self.fifo.drain(..ready);
    }

    fn fill<T: SizedSample + FromSample<f32>>(&mut self, data: &mut [T], channels: usize, info: &OutputCallbackInfo) {
        let frames = data.len() / channels;
        let mut done = 0;
        while done < frames {
            let count = (frames - done).min(BLOCK);
            self.produce(count);
            for (i, frame) in data[done * channels..(done + count) * channels].chunks_exact_mut(channels).enumerate() {
                // The resampler can ring a hair past full scale.
                let (left, right) = (self.out[i * 2].clamp(-1.0, 1.0), self.out[i * 2 + 1].clamp(-1.0, 1.0));
                if channels == 1 {
                    frame[0] = T::from_sample((left + right) * 0.5);
                } else {
                    frame[0] = T::from_sample(left);
                    frame[1] = T::from_sample(right);
                    for sample in &mut frame[2..] {
                        *sample = T::EQUILIBRIUM;
                    }
                }
            }
            done += count;
        }
        // What separates the end of the mix from what is heard: the device's latency, this
        // buffer, and what waits in the resampler.
        let timestamp = info.timestamp();
        let latency = timestamp.playback.saturating_duration_since(timestamp.callback).as_secs_f64();
        let queued = frames as f64 + self.fifo.len() as f64 / 2.0 + self.resampler.as_ref().map_or(0.0, |r| r.output_delay() as f64);
        let rate = self.shared.rate as f64;
        self.shared.tap.set_latency((latency * rate + queued * self.ratio) as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::Stream;

    #[test]
    fn resamples_the_mix_for_a_device_at_another_rate() {
        let shared = Arc::new(MixShared::new(48000));
        let stream = Arc::new(Stream::new(1, "k", 96000, 48000));
        let sine: Vec<f32> = (0..96000).flat_map(|i| {
            let s = (i as f32 * 1000.0 * std::f32::consts::TAU / 48000.0).sin() * 0.5;
            [s, s]
        }).collect();
        stream.ring.push(&sine);
        stream.level.store(1f32.to_bits(), Ordering::Relaxed);
        stream.gain.store(1f32.to_bits(), Ordering::Relaxed);
        shared.publish(vec![stream.clone()]);
        let mut output = DeviceOutput::new(shared, 44100);
        assert!(output.resampler.is_some());
        let mut peak = 0f32;
        for _ in 0..100 {
            output.produce(441);
            peak = output.out[..882].iter().fold(peak, |peak, s| peak.max(s.abs()));
        }
        // 1 s at the device's rate takes 1 s of the mix, give or take the resampler's queue.
        let consumed = stream.consumed.load(Ordering::Relaxed) as i64;
        assert!((consumed - 48000).abs() < 1500, "{consumed}");
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
    }
}
