pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    let mean_sq = sum_sq / samples.len() as f32;
    if mean_sq <= 1e-12 {
        -120.0
    } else {
        10.0 * mean_sq.log10()
    }
}

/// Peak envelope limiter with instant attack and smooth exponential release.
/// Used pre-RNNoise to guarantee that input samples never exceed the ceiling.
#[derive(Debug, Clone)]
pub struct PeakLimiter {
    gain: f32,
    ceiling: f32,
    release_coeff: f32,
}

impl Default for PeakLimiter {
    fn default() -> Self {
        Self::new(0.944_060_9, 50.0, 48_000.0)
    }
}

impl PeakLimiter {
    #[must_use]
    pub fn new(ceiling: f32, release_ms: f32, sample_rate: f32) -> Self {
        let release_samples = (release_ms * 0.001 * sample_rate).max(1.0);
        let release_coeff = (-1.0 / release_samples).exp();
        Self {
            gain: 1.0,
            ceiling,
            release_coeff,
        }
    }

    #[inline]
    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            let abs_x = s.abs();
            let target_gain = if abs_x > self.ceiling {
                self.ceiling / abs_x
            } else {
                1.0
            };
            if target_gain < self.gain {
                // Instant attack: never let a sample exceed ceiling
                self.gain = target_gain;
            } else {
                // Smooth exponential recovery toward unity (1.0)
                self.gain = self.release_coeff * self.gain + (1.0 - self.release_coeff) * target_gain;
            }
            *s = (*s * self.gain).clamp(-self.ceiling, self.ceiling);
        }
    }
}

/// Piecewise C1-continuous cubic soft-knee limiter.
/// Linear in [-0.707, 0.707] (-3 dBFS); smoothly saturates to +/- 0.9885 (-0.1 dBFS).
#[inline]
#[must_use]
pub fn soft_limit_sample(x: f32) -> f32 {
    const T: f32 = 0.707_106_77; // -3.01 dBFS knee threshold
    const C: f32 = 0.988_553_1;  // -0.10 dBFS hard ceiling

    if x.is_nan() {
        return 0.0;
    }

    let abs_x = x.abs();
    if abs_x <= T {
        // Fully linear region
        x
    } else {
        let delta = C - T;
        let u = ((abs_x - T) / delta).min(1.0);
        // Hermite smooth saturation: f(u) = u - u^3 / 3
        let saturated = T + delta * (u - (u * u * u) / 3.0);
        if x < 0.0 {
            -saturated
        } else {
            saturated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_unity() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn twenty_db_is_ten() {
        assert!((db_to_linear(20.0) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn full_scale_sine_rms_is_minus_3db() {
        // Peak 1.0 sine has RMS of 1/sqrt(2) -> ~ -3.01 dBFS.
        let samples: Vec<f32> = (0..480)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI / 48.0).sin())
            .collect();
        let db = rms_dbfs(&samples);
        assert!((db - (-3.01)).abs() < 0.1, "got {db}");
    }

    #[test]
    fn silence_floors_at_minus_120() {
        assert_eq!(rms_dbfs(&[0.0; 480]), -120.0);
        assert_eq!(rms_dbfs(&[]), -120.0);
    }

    #[test]
    fn peak_limiter_clamps_overshoot() {
        let mut limiter = PeakLimiter::default();
        let mut samples = vec![0.5, 1.5, -2.0, 0.2];
        limiter.process_in_place(&mut samples);
        for &s in &samples {
            assert!(s.abs() <= 0.944_061, "sample {s} exceeded ceiling");
        }
    }

    #[test]
    fn soft_limiter_is_linear_below_threshold() {
        assert_eq!(soft_limit_sample(0.0), 0.0);
        assert_eq!(soft_limit_sample(0.5), 0.5);
        assert_eq!(soft_limit_sample(-0.5), -0.5);
        assert_eq!(soft_limit_sample(0.707), 0.707);
    }

    #[test]
    fn soft_limiter_compresses_above_threshold() {
        let limited = soft_limit_sample(1.5);
        assert!(limited <= 0.988_553_1);
        assert!(limited > 0.707);
        let neg_limited = soft_limit_sample(-2.0);
        assert!(neg_limited >= -0.988_553_1);
        assert!(neg_limited < -0.707);
        assert_eq!(soft_limit_sample(f32::NAN), 0.0);
    }
}
