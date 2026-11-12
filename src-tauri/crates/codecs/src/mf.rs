//! AAC through the Media Foundation AAC decoder (CLSID_MSAACDecMFT, msauddecmft.dll), which decodes
//! AAC-LC, HE-AAC v1 (SBR) and HE-AAC v2 (PS). It takes raw AAC frames plus an
//! AudioSpecificConfig, which suits both of Symphonia's AAC sources: MP4 gives the config as extra
//! data, and the ADTS reader strips each frame's header and gives none, so one is built from its
//! rate and channels.
//!
//! HE-AAC is mostly signalled implicitly: the config says LC at 22.05 or 24 kHz, and only the
//! frames carry SBR. The decoder finds it in the first frames and changes its output type to twice
//! the rate (and to stereo for PS) with MF_E_TRANSFORM_STREAM_CHANGE. Each decoded buffer carries
//! the output type in effect, so the rate and channels the engine reads from it are the real ones.
//!
//! Threads: the decoder holds the multithreaded apartment open (CoIncrementMTAUsage) instead of
//! initializing COM on the thread that creates it, so it can be created, used and dropped on any
//! thread that is not a single-threaded apartment, and moved between them. A plain std::thread
//! that never called CoInitialize is in the implicit MTA.

use std::mem::ManuallyDrop;

use symphonia::core::audio::{
    AsGenericAudioBufferRef, Audio, AudioBuffer, AudioMut, AudioSpec, Channels,
    GenericAudioBufferRef, Position,
};
use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia::core::codecs::audio::{
    AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::codecs::registry::{RegisterableAudioDecoder, SupportedAudioCodec};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::errors::{decode_error, unsupported_error, Result};
use symphonia::core::packet::PacketRef;
use symphonia::default::codecs::AacDecoder;

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoDecrementMTAUsage, CoIncrementMTAUsage, CLSCTX_INPROC_SERVER,
    CO_MTA_USAGE_COOKIE,
};

/// MPEG-4 audio sampling frequency indices (ISO 14496-3, 1.6.3.4).
const RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// The input type's view of the stream.
#[derive(Debug, PartialEq)]
pub(crate) struct Config {
    /// The AudioSpecificConfig handed to the decoder.
    pub asc: Vec<u8>,
    /// The core (AAC-LC) rate and channel count, as the input type wants them.
    pub rate: u32,
    pub channels: u32,
}

impl Config {
    pub(crate) fn from_params(params: &AudioCodecParameters) -> Result<Config> {
        match &params.extra_data {
            Some(asc) if asc.len() >= 2 => Config::from_asc(asc, params),
            _ => Config::from_adts(params),
        }
    }

    /// MP4 (and Matroska): the AudioSpecificConfig as extra data. Only the start is read, for the
    /// core rate and channels; the decoder parses the rest.
    fn from_asc(asc: &[u8], params: &AudioCodecParameters) -> Result<Config> {
        let mut bits = Bits { data: asc, pos: 0 };
        // The object type (31 escapes to six more bits).
        if bits.read(5)? == 31 {
            bits.read(6)?;
        }
        let rate = match bits.read(4)? {
            15 => bits.read(24)?,
            index => match RATES.get(index as usize) {
                Some(&rate) => rate,
                None => return decode_error("aac: invalid sampling frequency index"),
            },
        };
        let channels = match bits.read(4)? {
            // 0: a program config element in the payload decides; take the container's word.
            0 => params.channels.as_ref().map_or(2, |c| c.count() as u32),
            config @ 1..=6 => config,
            7 => 8,
            _ => return unsupported_error("aac: unsupported channel configuration"),
        };
        if rate == 0 {
            return decode_error("aac: invalid sample rate");
        }
        Ok(Config {
            asc: asc.to_vec(),
            rate,
            channels,
        })
    }

    /// ADTS: Symphonia's reader strips the header and gives only its rate and channels (not even
    /// the profile). ADTS can't signal SBR or PS anyway, so HE-AAC in ADTS is always LC with
    /// implicit SBR, and an LC config is what the stream says. Main and LTP in ADTS are too rare to
    /// matter.
    fn from_adts(params: &AudioCodecParameters) -> Result<Config> {
        let rate = match params.sample_rate {
            Some(rate) => rate,
            None => return unsupported_error("aac: sample rate is required"),
        };
        let index = match RATES.iter().position(|&r| r == rate) {
            Some(index) => index as u8,
            None => return unsupported_error("aac: unsupported sample rate"),
        };
        let channels = params.channels.as_ref().map_or(2, |c| c.count() as u32);
        let config = match channels {
            1..=6 => channels as u8,
            8 => 7,
            _ => return unsupported_error("aac: unsupported channel count"),
        };
        // 5 bits object type (2: LC), 4 bits frequency index, 4 bits channel configuration, then
        // a GASpecificConfig of three zero bits (1024-sample frames, no core, no extension).
        let asc = vec![(2 << 3) | (index >> 1), ((index & 1) << 7) | (config << 3)];
        Ok(Config {
            asc,
            rate,
            channels,
        })
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, count: usize) -> Result<u32> {
        let mut value = 0;
        for _ in 0..count {
            let Some(byte) = self.data.get(self.pos / 8) else {
                return decode_error("aac: AudioSpecificConfig too short");
            };
            value = (value << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(value)
    }
}

/// Keeps COM's multithreaded apartment and Media Foundation running while a decoder exists. Both
/// are reference counted by the system, so every decoder takes its own reference and gives it back
/// when dropped, from whatever thread that happens on.
struct Runtime {
    cookie: CO_MTA_USAGE_COOKIE,
}

#[cfg(test)]
thread_local! {
    /// Tests set this to act like a system without Media Foundation.
    static NO_MEDIA_FOUNDATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Runtime {
    fn start() -> windows::core::Result<Runtime> {
        #[cfg(test)]
        if NO_MEDIA_FOUNDATION.get() {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_NOTIMPL,
            ));
        }
        // SAFETY: no preconditions; the cookie is given back exactly once, in Drop or below.
        let cookie = unsafe { CoIncrementMTAUsage() }?;
        // SAFETY: no preconditions; balanced by MFShutdown in Drop.
        if let Err(err) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) } {
            // SAFETY: the cookie came from CoIncrementMTAUsage above and isn't used again.
            let _ = unsafe { CoDecrementMTAUsage(cookie) };
            return Err(err);
        }
        Ok(Runtime { cookie })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: MFStartup succeeded in start(); the cookie came from CoIncrementMTAUsage and is
        // given back once. The decoder's COM objects are fields declared before this one, so they
        // are already released.
        unsafe {
            let _ = MFShutdown();
            let _ = CoDecrementMTAUsage(self.cookie);
        }
    }
}

/// The output type in effect.
struct Output {
    rate: u32,
    channels: Channels,
    float: bool,
    /// Bytes the decoder wants in an output buffer (0: no preference).
    size: u32,
    /// The decoder allocates its own output samples.
    provides: bool,
}

/// The decoder holds each frame back until the next one arrives, so the first call returns no
/// audio and every later call returns the previous frame's (1024 frames for LC, 2048 for HE-AAC).
/// The last frame of a file stays inside: 23 or 46 ms, not worth a drain call Symphonia's decoder
/// interface has no place for.
pub struct MfAacDecoder {
    params: AudioCodecParameters,
    transform: IMFTransform,
    output: Output,
    /// The sample (and its buffer) handed to ProcessOutput, reused for every call.
    out_sample: Option<(IMFSample, IMFMediaBuffer, u32)>,
    pcm: Vec<f32>,
    buf: AudioBuffer<f32>,
    /// Kept last: fields drop in order, so the COM objects above are released before it.
    _runtime: Runtime,
}

// SAFETY: the decoder MFT is registered with ThreadingModel "Both" and is created in the
// multithreaded apartment (see the module comment), where any thread may call it without
// marshalling, so the COM pointers can move between threads. Sync: no method taking &self touches
// them (codec_info, codec_params and last_decoded read only Rust data); every call into Media
// Foundation goes through &mut self.
unsafe impl Send for MfAacDecoder {}
unsafe impl Sync for MfAacDecoder {}

impl MfAacDecoder {
    pub fn try_new(params: &AudioCodecParameters, _opts: &AudioDecoderOptions) -> Result<Self> {
        if params.codec != CODEC_ID_AAC {
            return unsupported_error("aac: invalid codec");
        }
        let config = Config::from_params(params)?;
        let runtime = match Runtime::start() {
            Ok(runtime) => runtime,
            Err(_) => return unsupported_error("aac: Media Foundation is not available"),
        };
        let transform = match create(&config) {
            Ok(transform) => transform,
            Err(_) => return unsupported_error("aac: the system AAC decoder refused the stream"),
        };
        let output = match set_output(&transform) {
            Ok(output) => output,
            Err(_) => return unsupported_error("aac: the system AAC decoder has no usable output"),
        };

        let mut params = params.clone();
        params
            .with_sample_rate(output.rate)
            .with_channels(output.channels.clone());
        let buf = AudioBuffer::new(AudioSpec::new(output.rate, output.channels.clone()), 2048);

        Ok(MfAacDecoder {
            params,
            transform,
            output,
            out_sample: None,
            pcm: Vec::with_capacity(2048 * 2),
            buf,
            _runtime: runtime,
        })
    }

    /// Feeds one AAC frame and collects everything the decoder outputs for it into `pcm`.
    fn feed(&mut self, data: &[u8]) -> windows::core::Result<()> {
        let sample = input_sample(data)?;
        // SAFETY: the transform and sample are valid COM objects; stream 0 is the only input.
        match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            Err(err) if err.code() == MF_E_NOTACCEPTING => {
                // Output is pending (it shouldn't be, as every call drains): drain and retry once.
                self.drain()?;
                // SAFETY: as above.
                unsafe { self.transform.ProcessInput(0, &sample, 0) }?;
            }
            result => result?,
        }
        self.drain()
    }

    /// Pulls output until the decoder wants more input, following output type changes.
    fn drain(&mut self) -> windows::core::Result<()> {
        loop {
            let ours = if self.output.provides {
                None
            } else {
                Some(self.output_sample()?)
            };
            let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(ours),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            }];
            let mut status = 0;
            // SAFETY: one output buffer for the one output stream, with our own sample in it
            // unless the decoder provides its own.
            let result = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
            // SAFETY: ProcessOutput has returned, so these fields are ours again: the sample we
            // passed (or the one the decoder allocated) and any events it attached. Each is taken
            // exactly once and the array isn't used afterwards.
            let (sample, _events) = unsafe {
                (
                    ManuallyDrop::take(&mut buffers[0].pSample),
                    ManuallyDrop::take(&mut buffers[0].pEvents),
                )
            };
            match result {
                Ok(()) => {
                    if let Some(sample) = sample {
                        self.collect(&sample)?;
                    }
                }
                Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // Frames decoded under the old type can't share a buffer with the new ones.
                    // The change comes on the first frames of a stream, before anything was
                    // output, so nothing audible is lost.
                    self.pcm.clear();
                    self.output = set_output(&self.transform)?;
                    self.out_sample = None;
                    self.params
                        .with_sample_rate(self.output.rate)
                        .with_channels(self.output.channels.clone());
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Our output sample, sized for what the decoder asks, reset to empty.
    fn output_sample(&mut self) -> windows::core::Result<IMFSample> {
        // A HE-AAC frame is 2048 frames; 8 channels of float leave room for anything.
        let size = self.output.size.max(2048 * 8 * 4);
        if self
            .out_sample
            .as_ref()
            .is_none_or(|(_, _, have)| *have < size)
        {
            // SAFETY: plain constructors; the buffer is attached to the sample it was made for.
            let (sample, buffer) = unsafe {
                let sample = MFCreateSample()?;
                let buffer = MFCreateMemoryBuffer(size)?;
                sample.AddBuffer(&buffer)?;
                (sample, buffer)
            };
            self.out_sample = Some((sample, buffer, size));
        }
        let (sample, buffer, _) = self.out_sample.as_ref().expect("set above");
        // SAFETY: a valid buffer; 0 is within its length.
        unsafe { buffer.SetCurrentLength(0) }?;
        Ok(sample.clone())
    }

    /// Appends a decoded sample's audio to `pcm` as interleaved f32.
    fn collect(&mut self, sample: &IMFSample) -> windows::core::Result<()> {
        // SAFETY: a valid sample; the contiguous buffer is the sample's own when it has one.
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
        let mut ptr = std::ptr::null_mut();
        let mut len = 0;
        // SAFETY: Lock gives a pointer to `len` valid bytes until Unlock, which follows the copy
        // below; the slice doesn't outlive it, and isn't made from a null pointer when empty.
        unsafe {
            buffer.Lock(&mut ptr, None, Some(&mut len))?;
            if !ptr.is_null() && len > 0 {
                let bytes = std::slice::from_raw_parts(ptr, len as usize);
                if self.output.float {
                    let (samples, _) = bytes.as_chunks::<4>();
                    self.pcm
                        .extend(samples.iter().map(|&b| f32::from_le_bytes(b)));
                } else {
                    let (samples, _) = bytes.as_chunks::<2>();
                    self.pcm.extend(
                        samples
                            .iter()
                            .map(|&b| f32::from(i16::from_le_bytes(b)) / 32768.0),
                    );
                }
            }
            buffer.Unlock()?;
        }
        Ok(())
    }

    fn decode_inner(&mut self, packet: &PacketRef<'_>) -> Result<()> {
        self.pcm.clear();
        self.buf.clear();
        if self.feed(packet.data).is_err() {
            // A corrupt frame. Flushing makes sure nothing half-processed is left inside, and the
            // decoder carries on with the next frame.
            self.reset();
            return decode_error("aac: the system AAC decoder rejected a frame");
        }

        let channels = self.output.channels.count().max(1);
        let frames = self.pcm.len() / channels;
        let spec = AudioSpec::new(self.output.rate, self.output.channels.clone());
        if *self.buf.spec() != spec || self.buf.capacity() < frames {
            self.buf = AudioBuffer::new(spec, frames.max(2048));
        }
        self.buf.render_uninit(Some(frames));
        // The packet's trims aren't applied: the decoder outputs each frame one call late (see
        // MfAacDecoder), so they would cut the wrong frame. Symphonia's ADTS and MP4 readers give
        // none anyway.
        self.buf
            .copy_from_slice_interleaved(&&self.pcm[..frames * channels]);
        Ok(())
    }
}

impl AudioDecoder for MfAacDecoder {
    fn reset(&mut self) {
        // SAFETY: a valid transform; flushing drops queued input and output and keeps the types.
        let _ = unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) };
        self.pcm.clear();
        self.buf.clear();
    }

    fn codec_info(&self) -> &CodecInfo {
        &Self::supported_codecs()[0].info
    }

    fn codec_params(&self) -> &AudioCodecParameters {
        &self.params
    }

    fn decode_ref(&mut self, packet: &PacketRef<'_>) -> Result<GenericAudioBufferRef<'_>> {
        if let Err(err) = self.decode_inner(packet) {
            self.buf.clear();
            return Err(err);
        }
        Ok(self.buf.as_generic_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
        self.buf.as_generic_audio_buffer_ref()
    }
}

impl RegisterableAudioDecoder for MfAacDecoder {
    /// Falls back to Symphonia's AAC-LC decoder when Media Foundation has no AAC decoder (Windows N
    /// without the Media Feature Pack) or won't take the stream.
    fn try_registry_new(
        params: &AudioCodecParameters,
        opts: &AudioDecoderOptions,
    ) -> Result<Box<dyn AudioDecoder>> {
        match MfAacDecoder::try_new(params, opts) {
            Ok(decoder) => Ok(Box::new(decoder)),
            Err(_) => AacDecoder::try_registry_new(params, opts),
        }
    }

    fn supported_codecs() -> &'static [SupportedAudioCodec] {
        const CODECS: &[SupportedAudioCodec] = &[SupportedAudioCodec {
            id: CODEC_ID_AAC,
            info: CodecInfo {
                short_name: "aac-mf",
                long_name: "Advanced Audio Coding (Media Foundation: LC, HE-AAC v1 and v2)",
                profiles: &[],
            },
        }];
        CODECS
    }
}

/// Creates the decoder and sets its input type.
fn create(config: &Config) -> windows::core::Result<IMFTransform> {
    // SAFETY: COM's MTA is held by the caller's Runtime; plain object creation.
    let transform: IMFTransform =
        unsafe { CoCreateInstance(&CLSID_MSAACDecMFT, None, CLSCTX_INPROC_SERVER) }?;

    // MF_MT_USER_DATA is the part of HEAACWAVEINFO after its WAVEFORMATEX: payload type (0: raw
    // frames), audio profile level indication (0xFE: not specified), structure type and three
    // reserved fields, then the AudioSpecificConfig.
    let mut user_data = vec![0u8; 12];
    user_data[2..4].copy_from_slice(&0xFEu16.to_le_bytes());
    user_data.extend_from_slice(&config.asc);

    // SAFETY: a fresh media type set up with valid keys, then handed to the transform's only
    // input stream.
    unsafe {
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, config.rate)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, config.channels)?;
        input.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
        input.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0xFE)?;
        input.SetBlob(&MF_MT_USER_DATA, &user_data)?;
        transform.SetInputType(0, &input, 0)?;
    }
    Ok(transform)
}

/// Picks the decoder's float output (16-bit PCM if it offers no float), sets it, and describes it.
fn set_output(transform: &IMFTransform) -> windows::core::Result<Output> {
    let mut chosen: Option<(IMFMediaType, bool)> = None;
    for index in 0.. {
        // SAFETY: a valid transform; enumeration ends with MF_E_NO_MORE_TYPES.
        let Ok(candidate) = (unsafe { transform.GetOutputAvailableType(0, index) }) else {
            break;
        };
        // SAFETY: a valid media type.
        let subtype = unsafe { candidate.GetGUID(&MF_MT_SUBTYPE) }.unwrap_or(GUID::zeroed());
        if subtype == MFAudioFormat_Float {
            chosen = Some((candidate, true));
            break;
        }
        if subtype == MFAudioFormat_PCM
            && chosen.is_none()
            // SAFETY: a valid media type.
            && unsafe { candidate.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE) }.ok() == Some(16)
        {
            chosen = Some((candidate, false));
        }
    }
    let Some((output, float)) = chosen else {
        return Err(MF_E_TRANSFORM_TYPE_NOT_SET.into());
    };

    // SAFETY: a valid transform and media type; reading attributes the decoder sets on every audio
    // output type (the mask may be missing, then the count decides).
    unsafe {
        transform.SetOutputType(0, &output, 0)?;
        let rate = output.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;
        let count = output.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?;
        let mask = output.GetUINT32(&MF_MT_AUDIO_CHANNEL_MASK).ok();
        let info = transform.GetOutputStreamInfo(0)?;
        Ok(Output {
            rate,
            channels: channels(count, mask),
            float,
            size: info.cbSize,
            provides: info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0,
        })
    }
}

/// Media Foundation interleaves in channel mask order, which is Symphonia's canonical order (its
/// first 18 positions are WAVEFORMATEXTENSIBLE's).
fn channels(count: u32, mask: Option<u32>) -> Channels {
    let positions = mask.map(|m| Position::from_bits_truncate(u64::from(m)));
    match positions {
        Some(p) if p.bits().count_ones() == count => Channels::Positioned(p),
        _ => match count {
            1 => Channels::Positioned(Position::FRONT_CENTER),
            2 => Channels::Positioned(Position::FRONT_LEFT | Position::FRONT_RIGHT),
            n => Channels::Discrete(n as u16),
        },
    }
}

/// Wraps one AAC frame in a Media Foundation sample.
fn input_sample(data: &[u8]) -> windows::core::Result<IMFSample> {
    let len = data.len() as u32;
    // SAFETY: the buffer is created with room for `len` bytes, and Lock gives a pointer to at
    // least that many until Unlock; the copy stays within it.
    unsafe {
        let buffer = MFCreateMemoryBuffer(len.max(1))?;
        let mut ptr = std::ptr::null_mut();
        buffer.Lock(&mut ptr, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(len)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        Ok(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::audio::layouts;
    use symphonia::core::packet::Packet;
    use symphonia::core::units::{Duration, Timestamp};

    fn adts(rate: u32, channels: Channels) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_AAC)
            .with_sample_rate(rate)
            .with_channels(channels);
        params
    }

    #[test]
    fn builds_config_for_adts() {
        // 44.1 kHz stereo LC is the well-known 0x12 0x10.
        let config = Config::from_params(&adts(44100, layouts::CHANNEL_LAYOUT_STEREO)).unwrap();
        assert_eq!(
            config,
            Config {
                asc: vec![0x12, 0x10],
                rate: 44100,
                channels: 2
            }
        );
        // 24 kHz mono (the core of a HE-AAC v2 stream): 0x13 0x08.
        let config = Config::from_params(&adts(24000, layouts::CHANNEL_LAYOUT_MONO)).unwrap();
        assert_eq!(
            config,
            Config {
                asc: vec![0x13, 0x08],
                rate: 24000,
                channels: 1
            }
        );
    }

    #[test]
    fn reads_core_from_mp4_config() {
        // Explicit HE-AAC v1: object type 5, core 24 kHz stereo, extension 48 kHz, then LC.
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_AAC)
            .with_extra_data(vec![0x2b, 0x11, 0x88, 0x00].into());
        let config = Config::from_params(&params).unwrap();
        assert_eq!((config.rate, config.channels), (24000, 2));
    }

    /// The decoder is created and dropped from several threads at once, many times, which would
    /// trip over unbalanced COM or Media Foundation start-up. Garbage input must give a decode
    /// error, not a crash, and leave the decoder usable.
    #[test]
    fn creates_and_drops_on_any_thread() {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..25 {
                        let params = adts(24000, layouts::CHANNEL_LAYOUT_STEREO);
                        let mut decoder = MfAacDecoder::try_new(&params, &Default::default())
                            .expect("system AAC decoder");
                        let packet = Packet::new(
                            0,
                            Timestamp::new(0),
                            Duration::new(1024),
                            vec![0xa5u8; 300],
                        );
                        let _ = decoder.decode(&packet);
                        decoder.reset();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn falls_back_to_symphonia_without_media_foundation() {
        NO_MEDIA_FOUNDATION.set(true);
        let params = adts(44100, layouts::CHANNEL_LAYOUT_STEREO);
        assert!(MfAacDecoder::try_new(&params, &Default::default()).is_err());
        let decoder = crate::registry()
            .make_audio_decoder(&params, &Default::default())
            .unwrap();
        assert_eq!(decoder.codec_info().short_name, "aac");
        NO_MEDIA_FOUNDATION.set(false);
        let decoder = crate::registry()
            .make_audio_decoder(&params, &Default::default())
            .unwrap();
        assert_eq!(decoder.codec_info().short_name, "aac-mf");
    }

    #[test]
    fn decoder_moves_between_threads() {
        let params = adts(44100, layouts::CHANNEL_LAYOUT_STEREO);
        let decoder =
            MfAacDecoder::try_new(&params, &Default::default()).expect("system AAC decoder");
        assert_eq!(decoder.codec_params().sample_rate, Some(44100));
        let mut decoder = std::thread::spawn(move || decoder).join().unwrap();
        decoder.reset();
        drop(std::thread::spawn(move || drop(decoder)).join());
    }
}
