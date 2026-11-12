//! The decoders Qust gives Symphonia on top of its own: Opus (pure Rust), and on Windows AAC through
//! the system's Media Foundation decoder, which also decodes HE-AAC v1 and v2. Symphonia's own AAC
//! decoder is LC only, so a HE-AAC stream through it plays its core at half the rate with no high
//! band; most AAC radio streams are HE-AAC.

use std::sync::OnceLock;

use symphonia::core::codecs::registry::CodecRegistry;

#[cfg(windows)]
mod mf;
mod opus;

#[cfg(windows)]
pub use mf::MfAacDecoder;
pub use opus::OpusDecoder;

/// Symphonia's own decoders (MP1/2/3, FLAC, Vorbis, PCM, ADPCM, ALAC, AAC-LC) plus Opus, and on
/// Windows AAC through Media Foundation in place of Symphonia's AAC-LC.
///
/// The Media Foundation decoder falls back to Symphonia's AAC-LC by itself when the system has no
/// AAC decoder (Windows N without the Media Feature Pack), so the registry needs no second lookup.
pub fn registry() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder::<OpusDecoder>();
        // Preferred outranks the standard tier Symphonia's AAC-LC is registered at.
        #[cfg(windows)]
        registry.register_audio_decoder_at_tier::<MfAacDecoder>(
            symphonia::core::common::Tier::Preferred,
        );
        registry
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::codecs::audio::well_known::*;

    #[test]
    fn registry_has_every_codec() {
        let registry = registry();
        for (id, name) in [
            (CODEC_ID_MP1, "mp1"),
            (CODEC_ID_MP2, "mp2"),
            (CODEC_ID_MP3, "mp3"),
            (CODEC_ID_FLAC, "flac"),
            (CODEC_ID_VORBIS, "vorbis"),
            (CODEC_ID_PCM_S16LE, "pcm"),
            (CODEC_ID_ADPCM_IMA_WAV, "adpcm"),
            (CODEC_ID_ALAC, "alac"),
            (CODEC_ID_AAC, "aac"),
            (CODEC_ID_OPUS, "opus"),
        ] {
            assert!(
                registry.get_audio_decoder(id).is_some(),
                "{name} is missing"
            );
        }
        let aac = registry.get_audio_decoder(CODEC_ID_AAC).unwrap();
        if cfg!(windows) {
            assert_eq!(aac.codec.info.short_name, "aac-mf");
        } else {
            assert_eq!(aac.codec.info.short_name, "aac");
        }
    }
}
