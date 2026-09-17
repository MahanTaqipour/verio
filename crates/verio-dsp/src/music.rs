//! Music playback decoding: mp3 / flac / wav / ogg-vorbis  ->  48 kHz mono f32.
//!
//! Session 13. Music is a separate audio bus: it never passes through RNNoise or
//! the VAD gate, and it is mixed straight into the outgoing Opus frame by the
//! pipeline. This module only turns a file into 10 ms chunks of 48 kHz mono f32.
//!
//! Decoding uses symphonia (pure Rust, no C/C++ toolchain). Only the four formats
//! in scope are enabled - AAC and ALAC are deliberately excluded for v1.

use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time as SymphoniaTime;

use crate::convert::{mixdown_to_mono, LinearResampler};
use crate::SAMPLE_RATE;

/// Anything that can go wrong opening or decoding a track.
#[derive(Debug)]
pub struct MusicError(pub String);

impl std::fmt::Display for MusicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "music: {}", self.0)
    }
}

impl std::error::Error for MusicError {}

impl From<std::io::Error> for MusicError {
    fn from(e: std::io::Error) -> Self {
        MusicError(e.to_string())
    }
}

/// Metadata returned when a track is opened.
#[derive(Debug, Clone)]
pub struct MusicTrackInfo {
    pub path: PathBuf,
    pub title: String,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// A decoder with a small FIFO so callers can always pull exactly 480 samples
/// (10 ms) while the underlying codec hands out whatever frame size it likes.
pub struct MusicDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    /// Resampler from the file's rate to 48 kHz; `None` when already 48 kHz.
    resampler: Option<LinearResampler>,
    /// Mono 48 kHz samples decoded but not yet delivered.
    pending: VecDeque<f32>,
    /// 48 kHz mono samples handed to the caller - the real playback position.
    delivered: u64,
    duration_ms: u64,
    sample_rate: u32,
    channels: u16,
    path: PathBuf,
    /// Set once the container has no more packets.
    eof: bool,
    /// Reused interleaved decode buffer (allocated on first packet).
    sample_buf: Option<SampleBuffer<f32>>,
    /// Scratch for resampling output.
    resampled: Vec<f32>,
}

fn short_title(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown track".to_string())
}

impl MusicDecoder {
    /// Open a file and prepare to stream it. No audio is decoded until
    /// [`MusicDecoder::next_chunk`] is called.
    pub fn open(path: &Path) -> Result<Self, MusicError> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions {
                    enable_gapless: false,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(|e| MusicError(format!("unsupported or corrupt file: {e}")))?;

        let format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| MusicError("no decodable audio track".into()))?;

        let track_id = track.id;
        let params = track.codec_params.clone();
        let sample_rate = params.sample_rate.unwrap_or(SAMPLE_RATE);
        let channels = params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2)
            .max(1);
        let duration_ms = match (params.n_frames, params.sample_rate) {
            (Some(frames), Some(rate)) if rate > 0 => frames * 1000 / u64::from(rate),
            _ => 0,
        };

        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| MusicError(format!("no decoder for this codec: {e}")))?;

        let resampler = (sample_rate != SAMPLE_RATE).then(|| LinearResampler::new(sample_rate, SAMPLE_RATE));

        Ok(Self {
            format,
            decoder,
            track_id,
            resampler,
            pending: VecDeque::new(),
            delivered: 0,
            duration_ms,
            sample_rate,
            channels,
            path: path.to_path_buf(),
            eof: false,
            sample_buf: None,
            resampled: Vec::new(),
        })
    }

    pub fn info(&self) -> MusicTrackInfo {
        MusicTrackInfo {
            path: self.path.clone(),
            title: short_title(&self.path),
            duration_ms: self.duration_ms,
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }

    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Playback position in ms, derived from the samples actually delivered
    /// (not from the file's clock).
    #[must_use]
    pub fn position_ms(&self) -> u64 {
        self.delivered * 1000 / u64::from(SAMPLE_RATE)
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.eof && self.pending.is_empty()
    }

    /// Pull up to `out.len()` samples (480 = 10 ms when music is playing).
    ///
    /// Returns `Ok(0)` on natural end-of-file so the caller can loop or stop;
    /// an `Err` means genuine decode failure.
    pub fn next_chunk(&mut self, out: &mut [f32; 480]) -> Result<usize, MusicError> {
        let mut filled = 0;
        while filled < out.len() {
            while filled < out.len() {
                match self.pending.pop_front() {
                    Some(s) => {
                        out[filled] = s;
                        filled += 1;
                    }
                    None => break,
                }
            }
            if filled >= out.len() {
                break;
            }
            if self.eof {
                break;
            }
            self.decode_more()?;
        }
        self.delivered += filled as u64;
        Ok(filled)
    }

    /// Decode one packet into `pending` (48 kHz mono).
    fn decode_more(&mut self) -> Result<(), MusicError> {
        let packet = match self.format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                self.eof = true;
                return Ok(());
            }
            Err(SymphoniaError::ResetRequired) => {
                self.eof = true;
                return Ok(());
            }
            Err(e) => return Err(MusicError(format!("read error: {e}"))),
        };

        if packet.track_id() != self.track_id {
            return Ok(());
        }

        let audio = self
            .decoder
            .decode(&packet)
            .map_err(|e| MusicError(format!("decode error: {e}")))?;

        let spec = *audio.spec();
        let capacity = audio.capacity() as u64;
        let rate = spec.rate;
        let channels = spec.channels.count() as u16;

        let buf = self
            .sample_buf
            .get_or_insert_with(|| SampleBuffer::<f32>::new(capacity, spec));
        buf.copy_interleaved_ref(audio);
        let mono = mixdown_to_mono(buf.samples(), channels);

        match self.resampler.as_mut() {
            Some(r) => {
                self.resampled.clear();
                r.process(&mono, &mut self.resampled);
                self.pending.extend(self.resampled.iter().copied());
            }
            None => {
                // File is already 48 kHz. A rate mismatch with the decoder spec
                // would still need conversion, so guard on it explicitly.
                if rate != SAMPLE_RATE {
                    let mut r = LinearResampler::new(rate, SAMPLE_RATE);
                    self.resampled.clear();
                    r.process(&mono, &mut self.resampled);
                    self.pending.extend(self.resampled.iter().copied());
                } else {
                    self.pending.extend(mono.iter().copied());
                }
            }
        }
        Ok(())
    }

    /// Seek to `ms`. Resets the pending FIFO and the resampler so playback
    /// continues cleanly from the new position.
    pub fn seek(&mut self, ms: u64) -> Result<(), MusicError> {
        let target = SymphoniaTime::from(ms as f64 / 1000.0);
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: target,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| MusicError(format!("seek failed: {e}")))?;
        self.pending.clear();
        self.eof = false;
        self.resampled.clear();
        // New resampler => no interpolation carry-over from the old position.
        self.resampler = (self.sample_rate != SAMPLE_RATE)
            .then(|| LinearResampler::new(self.sample_rate, SAMPLE_RATE));
        self.sample_buf = None;
        self.delivered = ms * u64::from(SAMPLE_RATE) / 1000;
        Ok(())
    }

    /// Back to the start of the track.
    pub fn reset(&mut self) -> Result<(), MusicError> {
        self.seek(0)
    }

    /// Small helper for tests/UI: how long a 480-sample chunk lasts.
    #[must_use]
    pub fn chunk_duration() -> Duration {
        Duration::from_millis(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::WavWriter;
    use std::f32::consts::PI;

    /// Session 13: a 1-second 440 Hz mono WAV written by the existing writer must
    /// decode back to ~48 000 samples at roughly the source amplitude.
    #[test]
    fn decodes_generated_wav() {
        let path = std::env::temp_dir().join("verio_music_decoder_test.wav");
        let amplitude = 0.5_f32;
        {
            let mut w = WavWriter::create(&path).expect("create wav");
            let samples: Vec<f32> = (0..SAMPLE_RATE)
                .map(|i| amplitude * (2.0 * PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin())
                .collect();
            w.write_f32(&samples).expect("write wav");
            w.finalize().expect("finalize wav");
        }

        let mut dec = MusicDecoder::open(&path).expect("open wav");
        assert_eq!(dec.duration_ms(), 1000, "duration should be 1 s");

        let mut total = 0usize;
        let mut peak = 0.0_f32;
        let mut chunk = [0.0_f32; 480];
        loop {
            let n = dec.next_chunk(&mut chunk).expect("decode chunk");
            if n == 0 {
                break;
            }
            for s in &chunk[..n] {
                peak = peak.max(s.abs());
            }
            total += n;
        }

        // The linear resampler is bypassed at 48 kHz but the WAV writer rounds to
        // 16-bit, so allow a small sample-count tolerance.
        assert!(
            (total as i64 - SAMPLE_RATE as i64).abs() <= 2,
            "expected ~48000 samples, got {total}"
        );
        // 16-bit quantisation + writer rounding: within 5% of the source amplitude.
        assert!(
            (peak - amplitude).abs() / amplitude <= 0.05,
            "peak {peak} not within 5% of {amplitude}"
        );
        assert_eq!(dec.position_ms(), 1000);

        let _ = std::fs::remove_file(&path);
    }
}
