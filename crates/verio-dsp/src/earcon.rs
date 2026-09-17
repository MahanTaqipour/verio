//! Audio cues (earcons) synthesized as subtle sine wave tones.
//!
//! Hotkey press feedback:
//! - Mute ON: 400 Hz tone (20 ms).
//! - Mute OFF: 800 Hz tone (20 ms).
//! - Deafen: two short 300 Hz blips.

use std::f32::consts::PI;
use crate::SAMPLE_RATE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarconKind {
    MuteOn,
    MuteOff,
    Deafen,
    /// Pleasant rising two-note chime played when a peer joins the room.
    PeerJoin,
    /// Descending two-note chime played when a peer leaves the room.
    PeerLeave,
}

const DEFAULT_VOLUME: f32 = 0.12; // -18 dBFS, subtle but clearly audible

/// Synthesize samples for the given earcon at 48 kHz mono f32.
#[must_use]
pub fn synthesize_earcon(kind: EarconKind) -> Vec<f32> {
    match kind {
        EarconKind::MuteOn => generate_tone(400.0, 20, DEFAULT_VOLUME),
        EarconKind::MuteOff => generate_tone(800.0, 20, DEFAULT_VOLUME),
        EarconKind::Deafen => {
            // Two short 300 Hz blips: 15 ms tone, 15 ms silence, 15 ms tone
            let blip = generate_tone(300.0, 15, DEFAULT_VOLUME);
            let silence_len = (15 * SAMPLE_RATE / 1000) as usize;
            let mut out = Vec::with_capacity(blip.len() * 2 + silence_len);
            out.extend_from_slice(&blip);
            out.extend(std::iter::repeat_n(0.0_f32, silence_len));
            out.extend_from_slice(&blip);
            out
        }
        EarconKind::PeerJoin => {
            // Rising perfect-fifth chime: 660 Hz then 990 Hz.
            let mut out = generate_tone(660.0, 70, DEFAULT_VOLUME);
            out.extend(generate_tone(990.0, 95, DEFAULT_VOLUME));
            out
        }
        EarconKind::PeerLeave => {
            // Falling mirror of the join cue: 990 Hz then 660 Hz.
            let mut out = generate_tone(990.0, 70, DEFAULT_VOLUME);
            out.extend(generate_tone(660.0, 95, DEFAULT_VOLUME));
            out
        }
    }
}

/// Generate a sine wave tone with smooth 2 ms fade-in and fade-out to prevent clicks.
fn generate_tone(freq_hz: f32, duration_ms: u32, volume: f32) -> Vec<f32> {
    let total_samples = (duration_ms * SAMPLE_RATE / 1000) as usize;
    let fade_samples = (2 * SAMPLE_RATE / 1000) as usize; // 2 ms ramp
    let mut out = Vec::with_capacity(total_samples);

    for i in 0..total_samples {
        let env = if i < fade_samples {
            i as f32 / fade_samples as f32
        } else if i >= total_samples.saturating_sub(fade_samples) {
            (total_samples - 1 - i) as f32 / fade_samples as f32
        } else {
            1.0
        };

        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (2.0 * PI * freq_hz * t).sin() * volume * env;
        out.push(s);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mute_on_earcon() {
        let samples = synthesize_earcon(EarconKind::MuteOn);
        assert_eq!(samples.len(), (20 * SAMPLE_RATE / 1000) as usize);
        assert!(samples.iter().any(|&s| s.abs() > 0.05));
    }

    #[test]
    fn test_mute_off_earcon() {
        let samples = synthesize_earcon(EarconKind::MuteOff);
        assert_eq!(samples.len(), (20 * SAMPLE_RATE / 1000) as usize);
        assert!(samples.iter().any(|&s| s.abs() > 0.05));
    }

    #[test]
    fn test_deafen_earcon() {
        let samples = synthesize_earcon(EarconKind::Deafen);
        let expected_len = (45 * SAMPLE_RATE / 1000) as usize;
        assert_eq!(samples.len(), expected_len);
    }
}
