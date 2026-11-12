//! Opus through `opus1`, a pure-Rust port of libopus (no C), as a Symphonia decoder. It decodes at
//! libopus speed (600-1300x real time for stereo music here) and gives float output straight from
//! the codec, not bounded by +-1.0, as libopus's `opus_decode_float` does. Output is always 48 kHz,
//! the rate Opus is coded at, so the engine never resamples inside the codec.

use symphonia::core::audio::{
    layouts, AsGenericAudioBufferRef, AudioBuffer, AudioMut, AudioSpec, Channels,
    GenericAudioBufferRef,
};
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{
    AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::codecs::registry::{RegisterableAudioDecoder, SupportedAudioCodec};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::errors::{decode_error, unsupported_error, Result};
use symphonia::core::packet::PacketRef;

const RATE: u32 = 48_000;
/// The longest Opus packet: 120 ms at 48 kHz.
const MAX_FRAMES: usize = 5760;

/// What the decoder needs from the identification header (OpusHead, RFC 7845 section 5.1).
#[derive(Debug, PartialEq)]
pub(crate) struct OpusHead {
    pub channels: usize,
    pub pre_skip: usize,
    /// Output gain in dB.
    pub gain_db: f32,
}

impl OpusHead {
    /// Ogg and Matroska carry the header as is (little-endian, version 1). MP4's dOps box has the
    /// same fields big-endian with version 0, and Symphonia's MP4 reader only puts the magic in
    /// front, so the version byte tells which one this is.
    pub(crate) fn parse(data: &[u8]) -> Result<OpusHead> {
        if data.len() < 19 || &data[..8] != b"OpusHead" {
            return decode_error("opus: missing or short OpusHead");
        }
        let version = data[8];
        if version > 15 {
            return unsupported_error("opus: unsupported OpusHead version");
        }
        let u16_at = |at: usize| {
            let bytes = [data[at], data[at + 1]];
            if version == 0 {
                u16::from_be_bytes(bytes)
            } else {
                u16::from_le_bytes(bytes)
            }
        };
        let channels = usize::from(data[9]);
        let pre_skip = usize::from(u16_at(10));
        let gain = u16_at(16) as i16;
        let family = data[18];

        // Family 0 is mono or stereo. Family 1 may still describe one mono or one coupled stereo
        // stream in the plain order, which decodes the same; anything else needs the multistream
        // decoder and a downmix, which Qust doesn't do.
        let plain = match family {
            0 => true,
            1 if channels <= 2 && data.len() >= 21 + channels => {
                let (streams, coupled) = (data[19], data[20]);
                let mapping = &data[21..21 + channels];
                streams == 1
                    && usize::from(coupled) == channels - 1
                    && mapping
                        .iter()
                        .enumerate()
                        .all(|(i, &m)| usize::from(m) == i)
            }
            _ => false,
        };
        if !plain || !(1..=2).contains(&channels) {
            return unsupported_error("opus: only mono and stereo are supported");
        }

        Ok(OpusHead {
            channels,
            pre_skip,
            gain_db: f32::from(gain) / 256.0,
        })
    }
}

pub struct OpusDecoder {
    params: AudioCodecParameters,
    /// Boxed: the decoder state is tens of kilobytes.
    decoder: Box<opus1::OpusDecoder>,
    channels: usize,
    pre_skip: usize,
    /// Frames still to drop at the start: the pre-skip, which can span several packets.
    skip: usize,
    /// Linear output gain from the header (1.0 almost always).
    gain: f32,
    /// The last packet's length, for concealing an empty packet.
    last_frames: usize,
    pcm: Vec<f32>,
    buf: AudioBuffer<f32>,
}

impl OpusDecoder {
    pub fn try_new(params: &AudioCodecParameters, _opts: &AudioDecoderOptions) -> Result<Self> {
        if params.codec != CODEC_ID_OPUS {
            return unsupported_error("opus: invalid codec");
        }
        // Every container Symphonia reads gives the OpusHead as extra data. Without one, fall
        // back to the channel count with no pre-skip or gain.
        let head = match &params.extra_data {
            Some(data) => OpusHead::parse(data)?,
            None => match params.channels.as_ref().map(Channels::count) {
                Some(channels @ 1..=2) => OpusHead {
                    channels,
                    pre_skip: 0,
                    gain_db: 0.0,
                },
                _ => return unsupported_error("opus: only mono and stereo are supported"),
            },
        };

        let channels = if head.channels == 1 {
            opus1::Channels::Mono
        } else {
            opus1::Channels::Stereo
        };
        let decoder = match opus1::OpusDecoder::new(opus1::SampleRate::Hz48000, channels) {
            Ok(decoder) => Box::new(decoder),
            Err(_) => return unsupported_error("opus: decoder could not be created"),
        };
        let layout = if head.channels == 1 {
            layouts::CHANNEL_LAYOUT_MONO
        } else {
            layouts::CHANNEL_LAYOUT_STEREO
        };

        // The engine reads rate and channels from the decoded buffers, but keep the parameters
        // honest too: Matroska reports discrete channels and the track's nominal rate.
        let mut params = params.clone();
        params.with_sample_rate(RATE).with_channels(layout.clone());

        Ok(OpusDecoder {
            params,
            decoder,
            channels: head.channels,
            pre_skip: head.pre_skip,
            skip: head.pre_skip,
            gain: 10f32.powf(head.gain_db / 20.0),
            last_frames: 960,
            pcm: vec![0.0; MAX_FRAMES * head.channels],
            buf: AudioBuffer::new(AudioSpec::new(RATE, layout), MAX_FRAMES),
        })
    }

    fn decode_inner(&mut self, packet: &PacketRef<'_>) -> Result<()> {
        // An empty packet asks for concealment of `room` frames, as in libopus: cover one packet
        // of the length the last one had, not the 120 ms of room there is.
        let room = if packet.data.is_empty() {
            self.last_frames
        } else {
            MAX_FRAMES
        };
        let frames = match self.decoder.decode(packet.data, room, &mut self.pcm) {
            Ok(frames) => frames.min(MAX_FRAMES),
            Err(_) => return decode_error("opus: invalid packet"),
        };
        if frames > 0 {
            self.last_frames = frames;
        }

        self.buf.clear();
        self.buf.render_uninit(Some(frames));
        self.buf
            .copy_from_slice_interleaved(&&self.pcm[..frames * self.channels]);
        if self.gain != 1.0 {
            let gain = self.gain;
            self.buf.apply(|s| s * gain);
        }

        // Neither Symphonia's Ogg nor its Matroska reader trims the pre-skip (they give the track
        // a delay but leave the packets untrimmed), so it's done here. Should a reader start
        // trimming it, its trim takes over.
        let mut start = packet.trim_start.get() as usize;
        if start > 0 {
            self.skip = 0;
        } else {
            start = self.skip.min(frames);
            self.skip -= start;
        }
        self.buf.trim(start, packet.trim_end.get() as usize);
        Ok(())
    }
}

impl AudioDecoder for OpusDecoder {
    /// After a seek the decoder starts cold, and the first frames are its warm-up. Dropping the
    /// pre-skip again (usually 6.5 ms) is what RFC 7845 asks for at the stream start and is harmless
    /// elsewhere.
    fn reset(&mut self) {
        self.decoder.reset_state();
        self.skip = self.pre_skip;
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

impl RegisterableAudioDecoder for OpusDecoder {
    fn try_registry_new(
        params: &AudioCodecParameters,
        opts: &AudioDecoderOptions,
    ) -> Result<Box<dyn AudioDecoder>> {
        Ok(Box::new(OpusDecoder::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [SupportedAudioCodec] {
        const CODECS: &[SupportedAudioCodec] = &[SupportedAudioCodec {
            id: CODEC_ID_OPUS,
            info: CodecInfo {
                short_name: "opus",
                long_name: "Opus",
                profiles: &[],
            },
        }];
        CODECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::packet::Packet;
    use symphonia::core::units::{Duration, Timestamp};

    fn head(channels: u8, pre_skip: u16, gain: i16, family: u8, tail: &[u8]) -> Vec<u8> {
        let mut data = b"OpusHead".to_vec();
        data.push(1);
        data.push(channels);
        data.extend_from_slice(&pre_skip.to_le_bytes());
        data.extend_from_slice(&48_000u32.to_le_bytes());
        data.extend_from_slice(&gain.to_le_bytes());
        data.push(family);
        data.extend_from_slice(tail);
        data
    }

    fn params(extra: Vec<u8>) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_OPUS)
            .with_sample_rate(48_000)
            .with_extra_data(extra.into());
        params
    }

    #[test]
    fn parses_opus_head() {
        let parsed = OpusHead::parse(&head(2, 312, -256, 0, &[])).unwrap();
        assert_eq!(
            parsed,
            OpusHead {
                channels: 2,
                pre_skip: 312,
                gain_db: -1.0
            }
        );

        // MP4's dOps: version 0, big-endian.
        let mut dops = b"OpusHead".to_vec();
        dops.extend_from_slice(&[0, 1, 0x01, 0x38, 0, 0, 0xbb, 0x80, 0x01, 0x00, 0]);
        let parsed = OpusHead::parse(&dops).unwrap();
        assert_eq!(
            parsed,
            OpusHead {
                channels: 1,
                pre_skip: 312,
                gain_db: 1.0
            }
        );

        // Family 1 describing plain stereo is accepted, surround is refused.
        assert!(OpusHead::parse(&head(2, 0, 0, 1, &[1, 1, 0, 1])).is_ok());
        assert!(OpusHead::parse(&head(6, 0, 0, 1, &[4, 2, 0, 4, 1, 2, 3, 5])).is_err());
        assert!(OpusHead::parse(b"OpusTags").is_err());
    }

    #[test]
    fn refuses_surround_cleanly() {
        let result = OpusDecoder::try_new(
            &params(head(6, 0, 0, 1, &[4, 2, 0, 4, 1, 2, 3, 5])),
            &Default::default(),
        );
        assert!(matches!(
            result,
            Err(symphonia::core::errors::Error::Unsupported(_))
        ));
    }

    /// A packet of just a TOC byte (CELT fullband, 20 ms, mono, one frame) has an empty frame,
    /// which decodes as concealment: 960 frames of near silence. Enough to check the pre-skip is
    /// carried across packets and the buffer has the right spec.
    #[test]
    fn pre_skip_spans_packets() {
        let mut decoder =
            OpusDecoder::try_new(&params(head(1, 1500, 0, 0, &[])), &Default::default()).unwrap();
        let packet = Packet::new(0, Timestamp::new(0), Duration::new(960), vec![0xf8u8]);

        let mut lens = Vec::new();
        for _ in 0..3 {
            let buf = decoder.decode(&packet).unwrap();
            assert_eq!(buf.spec().rate(), 48_000);
            assert_eq!(buf.spec().channels().count(), 1);
            lens.push(buf.frames());
        }
        assert_eq!(lens, [0, 420, 960]);

        decoder.reset();
        assert_eq!(decoder.decode(&packet).unwrap().frames(), 0);
    }

    #[test]
    fn bad_packet_is_a_decode_error() {
        let mut decoder =
            OpusDecoder::try_new(&params(head(2, 0, 0, 0, &[])), &Default::default()).unwrap();
        // Code 3 packet claiming 0 frames is invalid.
        let packet = Packet::new(0, Timestamp::new(0), Duration::new(960), vec![0xfbu8, 0x00]);
        assert!(matches!(
            decoder.decode(&packet),
            Err(symphonia::core::errors::Error::DecodeError(_))
        ));
        assert_eq!(decoder.last_decoded().frames(), 0);
    }

    /// Encodes `input` (stereo, 20 ms packets) with opus1's own encoder, into packets an OpusHead
    /// with the encoder's look-ahead as pre-skip describes.
    fn encode(input: &[f32]) -> (Vec<Vec<u8>>, u16) {
        let mut encoder = opus1::OpusEncoder::new(
            opus1::SampleRate::Hz48000,
            opus1::Channels::Stereo,
            opus1::Application::Audio,
        )
        .unwrap();
        let pre_skip = encoder.lookahead() as u16;
        let mut packets = Vec::new();
        let mut out = vec![0u8; 4000];
        for frame in input.as_chunks::<{ 960 * 2 }>().0 {
            let n = encoder.encode(frame, 960, &mut out).unwrap();
            packets.push(out[..n].to_vec());
        }
        (packets, pre_skip)
    }

    fn decode_all(packets: &[Vec<u8>], pre_skip: u16) -> (Vec<f32>, Vec<usize>) {
        let mut decoder =
            OpusDecoder::try_new(&params(head(2, pre_skip, 0, 0, &[])), &Default::default())
                .unwrap();
        let (mut out, mut lens) = (Vec::new(), Vec::new());
        let mut interleaved = Vec::new();
        for (i, data) in packets.iter().enumerate() {
            let packet = Packet::new(
                0,
                Timestamp::new(960 * i as i64),
                Duration::new(960),
                data.clone(),
            );
            let buf = decoder.decode(&packet).unwrap();
            buf.copy_to_vec_interleaved(&mut interleaved);
            lens.push(buf.frames());
            out.extend_from_slice(&interleaved);
        }
        (out, lens)
    }

    /// A round trip through the encoder: after the pre-skip, the output lines up with the input
    /// sample for sample and is continuous across packets.
    #[test]
    fn round_trip_lines_up_with_the_input() {
        let input: Vec<f32> = (0..48_000 * 2)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 997.0 * (i / 2) as f32 / 48_000.0).sin())
            .collect();
        let (packets, pre_skip) = encode(&input);
        let (out, lens) = decode_all(&packets, pre_skip);
        assert_eq!(lens[0], 960 - usize::from(pre_skip));
        assert!(lens[1..].iter().all(|&n| n == 960));

        // Skip the encoder's warm-up, then compare.
        let (start, end) = (4800 * 2, out.len());
        let signal: f64 = input[start..end].iter().map(|&s| f64::from(s * s)).sum();
        let error: f64 = input[start..end]
            .iter()
            .zip(&out[start..end])
            .map(|(&a, &b)| f64::from((a - b) * (a - b)))
            .sum();
        let snr = 10.0 * (signal / error).log10();
        assert!(snr > 25.0, "SNR {snr:.1} dB");

        // No step at packet joins larger than the sine's own steepest step plus coding noise.
        let max_step = 0.5 * 2.0 * std::f32::consts::PI * 997.0 / 48_000.0;
        for frame in start / 2..out.len() / 2 - 1 {
            let step = (out[(frame + 1) * 2] - out[frame * 2]).abs();
            assert!(step < max_step * 1.5, "step {step} at frame {frame}");
        }
    }

    /// A full-scale square wave rings past full scale when decoded. The output keeps those peaks
    /// (libopus float behaviour) instead of clipping them at 1.0.
    #[test]
    fn output_is_not_clipped() {
        let input: Vec<f32> = (0..48_000 * 2)
            .map(|i| if (i / 2) % 96 < 48 { 1.0 } else { -1.0 })
            .collect();
        let (packets, pre_skip) = encode(&input);
        let (out, _) = decode_all(&packets, pre_skip);
        let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak > 1.0, "peak {peak}");
    }
}
