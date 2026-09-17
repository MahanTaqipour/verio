//! Verio audio I/O layer.
//!
//! Phase 1: device enumeration (cpal), capture stream copying samples into a
//! lock-free ring buffer, playback stream draining one. Architecture rule:
//! cpal callbacks NEVER block, allocate, lock a mutex, or log — they only
//! convert sample formats (pure arithmetic) and move samples through the
//! lock-free ring buffers.
//!
//! Devices are opened at 48 kHz mono when possible; otherwise the native
//! config is used and the caller converts in the processing path
//! (stereo → mono = average channels; linear resample to 48 kHz).
//!
//! Design note: an AEC stage will later be inserted into the pipeline between
//! capture and gain behind a trait (verio_dsp::aec::AecStage) — this layer
//! stays a dumb transport of samples.

pub mod devices;
pub mod streams;

/// Preferred stream sample rate (the pipeline format).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;

pub use ringbuf::traits::{Consumer, Observer, Producer};
pub use ringbuf::{HeapCons, HeapProd};

