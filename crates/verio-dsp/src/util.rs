//! Small DSP utilities: gain conversion, RMS metering.

/// Convert decibels (dB) to a linear amplitude multiplier.
#[must_use]
pub fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Root-mean-square of `samples` as dBFS (full scale = 1.0). Digital silence
/// reports a -120 dBFS floor instead of -inf.
#[must_use]
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    if rms <= 0.0 {
        -120.0
    } else {
        20.0 * rms.log10()
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
        // Peak 1.0 sine has RMS of 1/sqrt(2) → ≈ -3.01 dBFS.
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
}
