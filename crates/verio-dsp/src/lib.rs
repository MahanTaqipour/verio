//! Verio DSP pipeline.
//!
//! Phase 1 scope:
//! - TX stages, in order: AEC slot (future) → input gain → RNNoise (10 ms / 480
//!   samples, voice probability) → VAD gate (hangover 250 ms) → rechunk to 20 ms
//!   (960 samples) → Opus encode (32 kbps, complexity 5, in-band FEC, DTX,
//!   fullband, VOIP application).
//! - RX stages: Opus decode (FEC/PLC on loss) → fixed 40 ms pre-buffer.
//! - Debug WAV writer/reader (48 kHz mono 16-bit PCM), hand-rolled so no extra
//!   dependency is needed.
//!
//! Pipeline spec: 48 kHz mono f32 everywhere. Device-native input is converted
//! (mixdown + linear resample) in the processing path, never in cpal callbacks.
//!
//! The future AEC stage sits between capture and gain behind the [`aec::AecStage`]
//! trait — that slot stays clean, but AEC itself is NOT built in this phase.

pub mod aec;
pub mod codec;
pub mod convert;
pub mod denoise;
pub mod earcon;
pub mod music;
pub mod util;
pub mod vad;
pub mod wav;

/// Pipeline sample rate (Hz).
pub const SAMPLE_RATE: u32 = 48_000;
/// One RNNoise frame: 10 ms at 48 kHz.
pub const RNNOISE_FRAME: usize = 480;
/// One Opus frame: 10 ms at 48 kHz (aligned with RNNoise 10 ms frames, no rechunking).
pub const OPUS_FRAME: usize = 480;


