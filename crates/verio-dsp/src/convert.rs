//! Device-native → 48 kHz mono conversion: channel mixdown + linear resampler.

use crate::SAMPLE_RATE;

/// Mix interleaved multi-channel audio down to mono by averaging channels.
/// Mono input passes through unchanged (still copied into `out`).
#[must_use]
pub fn mixdown_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    match channels {
        0 | 1 => interleaved.to_vec(),
        n => interleaved
            .chunks_exact(n as usize)
            .map(|frame| frame.iter().sum::<f32>() / n as f32)
            .collect(),
    }
}

/// Naive linear-interpolation resampler with carry-over state between calls.
/// Only used for device-fallback conversion (non-48 kHz native devices); the
/// preferred path opens devices at exactly 48 kHz and bypasses this entirely.
pub struct LinearResampler {
    /// Samples to advance in the input stream per output sample.
    step: f64,
    pos: f64,
    prev: f32,
    primed: bool,
}

impl LinearResampler {
    /// `from_rate` = device native rate, `to_rate` = 48 kHz.
    #[must_use]
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            step: f64::from(from_rate) / f64::from(to_rate),
            pos: 0.0,
            prev: 0.0,
            primed: false,
        }
    }

    /// Push converted samples onto `out` (reused buffer — never allocates a
    /// new one internally).
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if (self.step - 1.0).abs() < 1e-9 || input.is_empty() {
            out.extend_from_slice(input);
            return;
        }
        for &s in input {
            if !self.primed {
                self.prev = s;
                self.primed = true;
                // pos stays at 0; first output sample is emitted with the next
                // input sample available for interpolation.
            }
            let base = self.prev;
            let delta = s - base;
            while self.pos < 1.0 {
                out.push(base + delta * self.pos as f32);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = s;
        }
    }
}

/// Convenience: full device-native → 48 kHz mono conversion.
#[must_use]
pub fn to_pipeline_format(interleaved: &[f32], channels: u16, native_rate: u32) -> Vec<f32> {
    let mono = mixdown_to_mono(interleaved, channels);
    if native_rate == SAMPLE_RATE {
        mono
    } else {
        let mut resampler = LinearResampler::new(native_rate, SAMPLE_RATE);
        let mut out = Vec::new();
        resampler.process(&mono, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_mixdown_averages() {
        let stereo = [0.0_f32, 1.0, 0.5, -0.5, 1.0, 1.0];
        let mono = mixdown_to_mono(&stereo, 2);
        assert_eq!(mono, vec![0.5, 0.0, 1.0]);
    }

    #[test]
    fn mono_passthrough() {
        let mono = [0.1_f32, 0.2];
        assert_eq!(mixdown_to_mono(&mono, 1), vec![0.1, 0.2]);
    }

    #[test]
    fn resampler_identity_at_48k() {
        let mut r = LinearResampler::new(48_000, 48_000);
        let input: Vec<f32> = (0..1000).map(|i| i as f32 * 0.001).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn resampler_96k_halves_sample_count() {
        let mut r = LinearResampler::new(96_000, 48_000);
        let input = vec![0.5_f32; 2000];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert!((out.len() as i64 - 1000).abs() <= 2, "len={}", out.len());
        assert!(out.iter().all(|s| *s == 0.5));
    }

    #[test]
    fn resampler_44k_upsamples_close_to_ratio() {
        let mut r = LinearResampler::new(44_100, 48_000);
        let input = vec![0.5_f32; 44_100];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let expected = 48_000.0;
        assert!(
            (out.len() as f64 - expected).abs() < 10.0,
            "len={}",
            out.len()
        );
    }
}
