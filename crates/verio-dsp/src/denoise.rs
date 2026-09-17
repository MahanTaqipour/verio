//! RNNoise wrapper (nnnoiseless — pure-Rust port of Xiph's RNNoise).
//!
//! Operates on exactly 10 ms chunks (480 samples). nnnoiseless expects input
//! in the 16-bit PCM range [-32768, 32767], so samples are scaled up on the
//! way in and back down on the way out. `process_frame` returns the voice
//! probability in [0, 1], which drives the VAD gate.

use nnnoiseless::DenoiseState;

use crate::RNNOISE_FRAME;

pub struct RnNoise {
    state: Box<DenoiseState<'static>>,
    scaled: Vec<f32>,
}

impl Default for RnNoise {
    fn default() -> Self {
        Self::new()
    }
}

impl RnNoise {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            scaled: vec![0.0; RNNOISE_FRAME],
        }
    }

    /// Denoise one 480-sample chunk (input/output in [-1, 1] f32 scale).
    /// Returns `(denoised, voice_probability)`.
    ///
    /// # Panics
    /// Panics if `chunk.len() != 480` (a programming error in the pipeline).
    pub fn process(&mut self, chunk: &[f32]) -> (Vec<f32>, f32) {
        assert_eq!(chunk.len(), RNNOISE_FRAME);
        for (dst, src) in self.scaled.iter_mut().zip(chunk.iter()) {
            *dst = src * 32768.0;
        }
        let mut denoised_scaled = vec![0.0_f32; RNNOISE_FRAME];
        let vad = self.state.process_frame(&mut denoised_scaled, &self.scaled);
        let denoised: Vec<f32> = denoised_scaled.iter().map(|s| s / 32768.0).collect();
        (denoised, vad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_stays_silence() {
        let mut rn = RnNoise::new();
        let (denoised, vad) = rn.process(&[0.0; RNNOISE_FRAME]);
        assert!((0.0..=1.0).contains(&vad));
        // Silence in → near-silence out.
        let max = denoised.iter().cloned().fold(0.0_f32, f32::max);
        assert!(max < 1e-3, "max {max}");
    }

    #[test]
    fn loud_tone_survives() {
        let mut rn = RnNoise::new();
        // Loud tone, first frame output is discarded internally by design
        // (fade-in); process two frames and check the second.
        let tone: Vec<f32> = (0..RNNOISE_FRAME)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI / 96.0).sin() * 0.5)
            .collect();
        let _ = rn.process(&tone);
        let (d2, vad) = rn.process(&tone);
        let peak = d2.iter().cloned().fold(0.0_f32, f32::max);
        assert!(peak > 0.05, "peak {peak}");
        assert!((0.0..=1.0).contains(&vad));
    }
}
