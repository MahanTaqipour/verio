//! Debug WAV I/O: 48 kHz mono 16-bit PCM writer + a minimal reader for the
//! latency measurement tool. Hand-rolled (no `hound`) to keep the dependency
//! list exactly at the Phase 1 allowance.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Cap for debug WAV captures: 60 seconds.
pub const MAX_WAV_SECONDS: u32 = 60;

fn i16_le_bytes(sample: f32) -> [u8; 2] {
    let clamped = sample.clamp(-1.0, 1.0);
    // Symmetric full-scale with rounding: error ≤ 0.5 LSB after /32 768.
    let v = (clamped * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
    v.to_le_bytes()
}

/// Streaming WAV writer (16-bit PCM, mono, 48 kHz). Header sizes are patched
/// on finalize/drop, so an app crash mid-write still leaves a playable file.
pub struct WavWriter {
    file: BufWriter<File>,
    samples_written: u32,
    finalized: bool,
}

impl WavWriter {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = BufWriter::new(File::create(path)?);
        // 44-byte canonical WAV header; RIFF/data sizes patched at finalize.
        let mut header = [0_u8; 44];
        header[0..4].copy_from_slice(b"RIFF");
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes()); // fmt chunk size
        header[20..22].copy_from_slice(&1_u16.to_le_bytes()); // PCM
        header[22..24].copy_from_slice(&1_u16.to_le_bytes()); // mono
        header[24..28].copy_from_slice(&48_000_u32.to_le_bytes()); // sample rate
        header[28..32].copy_from_slice(&96_000_u32.to_le_bytes()); // byte rate
        header[32..34].copy_from_slice(&2_u16.to_le_bytes()); // block align
        header[34..36].copy_from_slice(&16_u16.to_le_bytes()); // bits
        header[36..40].copy_from_slice(b"data");
        file.write_all(&header)?;
        Ok(Self {
            file,
            samples_written: 0,
            finalized: false,
        })
    }

    pub fn write_f32(&mut self, samples: &[f32]) -> std::io::Result<()> {
        let mut buf = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            buf.extend_from_slice(&i16_le_bytes(s));
        }
        self.file.write_all(&buf)?;
        self.samples_written += samples.len() as u32;
        Ok(())
    }

    #[must_use]
    pub fn samples_written(&self) -> u32 {
        self.samples_written
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.samples_written >= MAX_WAV_SECONDS * 48_000
    }

    /// Patch the RIFF/data size fields. Idempotent.
    pub fn finalize(&mut self) -> std::io::Result<()> {
        if self.finalized {
            return Ok(());
        }
        self.finalized = true;
        self.file.flush()?;
        let data_bytes = self.samples_written * 2;
        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&(36 + data_bytes).to_le_bytes())?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&data_bytes.to_le_bytes())?;
        file.flush()
    }
}

impl Drop for WavWriter {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}

/// Minimal WAV reader for 16-bit PCM files (any rate/channels); samples are
/// returned normalized to [-1, 1], interleaved.
#[derive(Debug)]
pub struct WavData {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

pub fn read_wav(path: &Path) -> Result<WavData, String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{}: not a RIFF/WAVE file", path.display()));
    }
    let mut fmt = None;
    let mut data = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(
            bytes[pos + 4..pos + 8]
                .try_into()
                .map_err(|_| "bad chunk header")?,
        ) as usize;
        let body_end = (pos + 8 + size).min(bytes.len());
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err("fmt chunk too small".into());
                }
                let audio_format = u16::from_le_bytes(
                    bytes[pos + 8..pos + 10].try_into().map_err(|_| "fmt")?,
                );
                let channels = u16::from_le_bytes(
                    bytes[pos + 10..pos + 12].try_into().map_err(|_| "fmt")?,
                );
                let sample_rate = u32::from_le_bytes(
                    bytes[pos + 12..pos + 16].try_into().map_err(|_| "fmt")?,
                );
                let bits = u16::from_le_bytes(
                    bytes[pos + 22..pos + 24].try_into().map_err(|_| "fmt")?,
                );
                if audio_format != 1 || bits != 16 {
                    return Err(format!(
                        "unsupported WAV format (fmt={audio_format}, bits={bits}); want PCM 16-bit"
                    ));
                }
                fmt = Some((channels, sample_rate));
            }
            b"data" => data = Some(bytes[pos + 8..body_end].to_vec()),
            _ => {}
        }
        pos = pos + 8 + size + (size % 2); // chunks are word-aligned
    }
    let (channels, sample_rate) = fmt.ok_or("missing fmt chunk")?;
    let raw = data.ok_or("missing data chunk")?;
    let samples: Vec<f32> = raw
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0)
        .collect();
    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}

/// Index of the first sample exceeding `threshold_dbfs`. Requires a short
/// sustain (3 consecutive samples above threshold) so single-sample noise
/// blips don't count — a clap is a sharp transient well above the floor.
#[must_use]
pub fn first_onset(samples: &[f32], threshold_dbfs: f32) -> Option<usize> {
    let threshold = 10.0_f32.powf(threshold_dbfs / 20.0);
    const SUSTAIN: usize = 3;
    samples
        .windows(SUSTAIN)
        .position(|w| w.iter().all(|s| s.abs() >= threshold))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = std::env::temp_dir().join("verio-dsp-wav-test");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("test.wav");
        let samples: Vec<f32> = (0..960)
            .map(|i| (i as f32 / 960.0) * 2.0 - 1.0)
            .collect();
        {
            let mut w = WavWriter::create(&path).expect("create");
            w.write_f32(&samples).expect("write");
        } // drop → finalize patches sizes
        let data = read_wav(&path).expect("read");
        assert_eq!(data.sample_rate, 48_000);
        assert_eq!(data.channels, 1);
        assert_eq!(data.samples.len(), 960);
        for (orig, back) in samples.iter().zip(data.samples.iter()) {
            assert!((orig - back).abs() < 3.1e-5, "{orig} vs {back}");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn onset_found_at_expected_index() {
        let mut samples = vec![0.0_f32; 1000];
        for s in &mut samples[480..] {
            *s = 0.5; // -6 dBFS, above -20 dBFS threshold
        }
        assert_eq!(first_onset(&samples, -20.0), Some(480));
    }

    #[test]
    fn onset_ignores_subthreshold_noise() {
        let mut samples = vec![0.05_f32; 2000]; // ~-26 dBFS, below threshold
        for s in &mut samples[960..] {
            *s = 0.9;
        }
        assert_eq!(first_onset(&samples, -20.0), Some(960));
    }

    #[test]
    fn onset_none_when_all_silent() {
        assert_eq!(first_onset(&[0.0_f32; 4800], -20.0), None);
    }
}
