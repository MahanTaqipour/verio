//! Audio cues (earcons), played from WAV assets compiled into the binary.
//!
//! Six files in `assets/sounds/` are embedded with `include_bytes!` and decoded
//! once into resident 48 kHz mono f32 buffers. `verio-app`'s RX mixer adds them
//! to the output stream, which is what keeps them audible while deafened:
//! deafen silences *remote* audio, not the local cues.
//!
//! Session 27 replaced a sine-wave synthesizer with these files, and there is
//! deliberately no fallback to synthesis. If an asset fails to decode the cue is
//! silent and the failure is logged. A cue that quietly reverts to a different
//! sound would be worse than no cue at all — the entire point of the pairs is
//! that mute/unmute and deafen/undeafen are distinguishable by ear.
//!
//! The assets are 44.1 kHz stereo 16-bit PCM. Downmix and resample happen once,
//! at load, so the audio thread only ever copies f32 samples.
//!
//! **Gain is unity, and that is a measurement, not a default.** Measured peak is
//! -10.9 to -17.5 dBFS and RMS -24.6 to -31.0 dBFS, against -18.4 dBFS peak /
//! -21.4 dBFS RMS for the 0.12-amplitude sine this replaced: comparable peak,
//! lower RMS. Trimming to match the old tone would make them inaudible.

use std::sync::{Arc, OnceLock};

use crate::convert::to_pipeline_format;
use crate::wav::read_wav_bytes;

/// A cue the app can play.
///
/// The variant order is load-bearing: it indexes [`CUE_BYTES`]. Reordering the
/// enum without reordering that array silently swaps two sounds, so
/// `cue_labels_match_enum_order` pins the pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarconKind {
    /// Local microphone muted.
    MuteOn,
    /// Local microphone unmuted.
    Unmute,
    /// Local deafen engaged.
    Deafen,
    /// Local deafen released.
    Undeafen,
    /// A peer joined the room.
    PeerJoin,
    /// A peer left the room.
    PeerLeave,
}

/// Number of cues. Named so the array type and the index bound cannot drift.
const CUE_COUNT: usize = 6;

/// `(label, bytes)` per cue, in [`EarconKind`] order.
///
/// The label is only used in error messages — it names the asset that failed to
/// decode rather than a path that does not exist at runtime.
const CUE_BYTES: [(&str, &[u8]); CUE_COUNT] = [
    ("mute.wav", include_bytes!("../assets/sounds/mute.wav")),
    ("unmute.wav", include_bytes!("../assets/sounds/unmute.wav")),
    ("deafen.wav", include_bytes!("../assets/sounds/deafen.wav")),
    (
        "undeafen.wav",
        include_bytes!("../assets/sounds/undeafen.wav"),
    ),
    ("join.wav", include_bytes!("../assets/sounds/join.wav")),
    ("leave.wav", include_bytes!("../assets/sounds/leave.wav")),
];

/// Decoded cues, resident for the process lifetime.
static CUES: OnceLock<Result<[Arc<[f32]>; CUE_COUNT], String>> = OnceLock::new();

fn decode_all() -> Result<[Arc<[f32]>; CUE_COUNT], String> {
    let mut out = Vec::with_capacity(CUE_COUNT);
    for (label, bytes) in CUE_BYTES {
        let wav = read_wav_bytes(bytes, label)?;
        // 44.1 kHz stereo on disk -> 48 kHz mono here, so playback is a copy.
        let mono = to_pipeline_format(&wav.samples, wav.channels, wav.sample_rate);
        out.push(Arc::from(mono.into_boxed_slice()));
    }
    out.try_into()
        .map_err(|_| format!("expected {CUE_COUNT} earcon assets"))
}

fn cues() -> &'static Result<[Arc<[f32]>; CUE_COUNT], String> {
    CUES.get_or_init(decode_all)
}

/// Decode every cue and keep it resident.
///
/// Call once when the audio pipeline starts. Without it the first cue pays the
/// decode cost on the audio thread; nothing breaks, but it shows up as a late
/// cue. A decode failure is logged here so it is visible before the first press
/// rather than only when a cue is asked for.
pub fn warm() {
    if let Err(e) = cues() {
        tracing::error!(error = %e, "earcon assets failed to decode; cues will be silent");
    }
}

/// Samples for `kind`: 48 kHz mono f32, ready to mix.
///
/// The `Arc` is a pointer copy — the samples themselves are decoded once. An
/// empty buffer means the assets failed to decode (already logged by [`warm`]);
/// the caller mixes silence rather than panicking on the audio thread.
#[must_use]
pub fn cue(kind: EarconKind) -> Arc<[f32]> {
    match cues() {
        Ok(bank) => Arc::clone(&bank[kind as usize]),
        Err(_) => Arc::from(Vec::<f32>::new().into_boxed_slice()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [EarconKind; CUE_COUNT] = [
        EarconKind::MuteOn,
        EarconKind::Unmute,
        EarconKind::Deafen,
        EarconKind::Undeafen,
        EarconKind::PeerJoin,
        EarconKind::PeerLeave,
    ];

    #[test]
    fn cue_labels_match_enum_order() {
        // The enum is used as an index into CUE_BYTES, so a reorder on one side
        // and not the other silently swaps two sounds. This pins the pairing.
        let expected = [
            "mute.wav",
            "unmute.wav",
            "deafen.wav",
            "undeafen.wav",
            "join.wav",
            "leave.wav",
        ];
        for (i, (label, _)) in CUE_BYTES.iter().enumerate() {
            assert_eq!(*label, expected[i], "CUE_BYTES index {i}");
        }
        assert_eq!(EarconKind::MuteOn as usize, 0);
        assert_eq!(EarconKind::Unmute as usize, 1);
        assert_eq!(EarconKind::Deafen as usize, 2);
        assert_eq!(EarconKind::Undeafen as usize, 3);
        assert_eq!(EarconKind::PeerJoin as usize, 4);
        assert_eq!(EarconKind::PeerLeave as usize, 5);
    }

    #[test]
    fn every_cue_decodes_to_audio_at_48k() {
        for kind in ALL {
            let c = cue(kind);
            assert!(!c.is_empty(), "{kind:?} decoded to nothing");
            // Source is ~0.43-0.45 s; 44.1 k -> 48 k grows the count by ~8.8 %.
            let secs = c.len() as f32 / 48_000.0;
            assert!((0.40..0.50).contains(&secs), "{kind:?} is {secs:.3}s");
            let peak = c.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05, "{kind:?} is effectively silent (peak {peak})");
        }
    }

    #[test]
    fn the_six_cues_are_pairwise_distinct() {
        // Catches a copy-paste in CUE_BYTES that would make two events sound
        // identical — the one failure the mute/unmute and deafen/undeafen pairs
        // exist to prevent. `Arc<[f32]>`'s PartialEq compares contents, so this
        // is a real audio comparison and not pointer identity. `assert!` rather
        // than `assert_ne!` so a failure does not dump 20 000 floats.
        for (i, a) in ALL.iter().enumerate() {
            for b in ALL.iter().skip(i + 1) {
                assert!(cue(*a) != cue(*b), "{a:?} and {b:?} are the same audio");
            }
        }
    }

    #[test]
    fn cues_are_cached_not_redecoded() {
        // Same buffer, not an equal copy: proves the OnceLock is actually
        // caching rather than decoding per call.
        assert!(Arc::ptr_eq(&cue(EarconKind::MuteOn), &cue(EarconKind::MuteOn)));
    }
}
