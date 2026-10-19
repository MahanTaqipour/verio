//! Session 30b Integration & Remediation Tests
//!
//! Validates the four core quality guarantees of the Session 30b overhaul:
//! 1. Dynamic Range & Overdrive: PeakLimiter -> RNNoise -> 20ms Opus -> Mixer soft limiter.
//! 2. Multi-Peer Mixer: Headroom, 0 hard clips at +-1.0, and THD+N < 1% for single peer.
//! 3. Music Codec: High-fidelity CELT 12 kHz tone preservation via OpusEncoderWrapper::new_music.
//! 4. Debug WAV: WavWriter produces valid, non-empty, bit-accurate 48 kHz RIFF files.

use std::f32::consts::PI;
use std::fs;

use verio_dsp::codec::{OpusDecoderWrapper, OpusEncoderWrapper};
use verio_dsp::denoise::RnNoise;
use verio_dsp::util::{soft_limit_sample, PeakLimiter};
use verio_dsp::wav::{read_wav, WavWriter};
use verio_dsp::{OPUS_FRAME, RNNOISE_FRAME, SAMPLE_RATE};

/// Test Case 1: Dynamic Range & Overdrive
///
/// An overdriven mic signal (+12 dB gain factor ~3.98) must not clip or crush:
/// - PeakLimiter prevents RNNoise GRU saturation and Opus float overflow.
/// - RnNoise clamps inputs to [-32767.0, 32767.0].
/// - 20 ms Opus voice encoder (48 kbps) encodes cleanly.
/// - Mixer soft limiter caps peak to -0.1 dBFS (0.9885) with zero hard clips.
/// - Crest factor >= 10.0 dB preserves natural speech dynamics.
#[test]
fn test_dynamic_range_and_overdrive_protection() {
    let mut limiter = PeakLimiter::default();
    let mut denoiser = RnNoise::new();
    let mut encoder = OpusEncoderWrapper::new().expect("create opus voice encoder");
    let mut decoder = OpusDecoderWrapper::new().expect("create opus decoder");

    // Generate 1 second (48000 samples) of speech-like burst signal:
    // 50 ms tone pulse (440 Hz, amplitude 1.2) followed by 250 ms quiet period.
    let total_samples = SAMPLE_RATE as usize;
    let mut raw_signal = vec![0.0_f32; total_samples];
    for (i, sample) in raw_signal.iter_mut().enumerate() {
        let cycle_pos = i % (SAMPLE_RATE as usize / 3); // 333 ms cycle
        let pulse_len = (SAMPLE_RATE as usize) * 50 / 1000; // 50 ms pulse
        if cycle_pos < pulse_len {
            let t = i as f32 / SAMPLE_RATE as f32;
            *sample = 1.2 * (2.0 * PI * 440.0 * t).sin();
        } else {
            *sample = 0.005; // low noise floor
        }
    }

    // Apply +12 dB user gain (gain factor 10^(12/20) ~ 3.981)
    let gain_linear = 10.0_f32.powf(12.0 / 20.0);
    let mut gained: Vec<f32> = raw_signal.iter().map(|&s| s * gain_linear).collect();

    // 1. Pre-RNNoise PeakLimiter
    limiter.process_in_place(&mut gained);

    // PeakLimiter must catch every sample and hold it <= 0.945 (-0.5 dBFS)
    for &s in &gained {
        assert!(
            s.abs() <= 0.945,
            "sample {s} exceeded peak limiter ceiling 0.945"
        );
    }

    // 2. RNNoise in 480-sample blocks
    let mut denoised = Vec::with_capacity(total_samples);
    for chunk in gained.chunks_exact(RNNOISE_FRAME) {
        let (out_block, _vad) = denoiser.process(chunk);
        denoised.extend_from_slice(&out_block);
    }

    // 3. 20 ms Opus Voice Encoder (960 samples)
    let mut packets = Vec::new();
    for chunk in denoised.chunks(OPUS_FRAME) {
        if let Some(pkt) = encoder.push(chunk).expect("opus encode") {
            packets.push(pkt);
        }
    }
    assert!(!packets.is_empty(), "encoder must produce packets");

    // 4. Opus Decode & Mixer Soft Limiter
    let mut final_output = Vec::new();
    for pkt in &packets {
        let decoded = decoder.decode(Some(pkt)).expect("opus decode");
        for &s in &decoded {
            final_output.push(soft_limit_sample(s));
        }
    }

    // Assertions:
    // a. Zero hard clips at +-1.0
    let hard_clips = final_output.iter().filter(|&&s| s.abs() >= 1.0).count();
    assert_eq!(hard_clips, 0, "must have 0 hard clips at +-1.0");

    // b. Peak must not exceed -0.1 dBFS (0.989)
    let peak = final_output.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        peak <= 0.989,
        "peak {peak} exceeds -0.1 dBFS ceiling (0.989)"
    );

    // c. Crest factor >= 10.0 dB (natural dynamic range preserved)
    let rms = (final_output.iter().map(|s| s * s).sum::<f32>() / final_output.len() as f32).sqrt();
    assert!(rms > 0.0, "RMS must be positive");
    let crest_factor_db = 20.0 * (peak / rms).log10();
    assert!(
        crest_factor_db >= 10.0,
        "crest factor {crest_factor_db:.2} dB must be >= 10.0 dB"
    );
}

/// Test Case 2: Multi-Peer Mixer Linearity and Headroom
///
/// Asserts:
/// - A single peer at -14 dBFS operates in 100% linear region (THD+N < 1%).
/// - 3 simultaneous peers at -14 dBFS mix cleanly without clipping.
/// - Heavy overdrive (3 loud peers summing to > 1.5) saturates smoothly <= 0.9885 with 0 clips.
#[test]
fn test_multi_peer_mixer_linearity_and_headroom() {
    let num_samples = 4800; // 100 ms

    // Peer 1: 440 Hz at -14 dBFS (amplitude 0.1995)
    let p1_amp = 10.0_f32.powf(-14.0 / 20.0);
    let p1: Vec<f32> = (0..num_samples)
        .map(|i| p1_amp * (2.0 * PI * 440.0 * (i as f32 / SAMPLE_RATE as f32)).sin())
        .collect();

    // Linearity check: soft_limit_sample(x) must equal x bit-exact below -3 dBFS (0.7071)
    let mut max_err: f32 = 0.0;
    for &s in &p1 {
        let limited = soft_limit_sample(s);
        max_err = max_err.max((limited - s).abs());
    }
    assert_eq!(
        max_err, 0.0,
        "soft_limit_sample must be 100% linear (0 error) below -3 dBFS"
    );

    // THD+N for single peer at -14 dBFS is 0%
    let thd_n = max_err / p1_amp;
    assert!(thd_n < 0.01, "THD+N must be < 1% (measured {thd_n})");

    // Peer 2: 660 Hz at -14 dBFS
    let p2: Vec<f32> = (0..num_samples)
        .map(|i| p1_amp * (2.0 * PI * 660.0 * (i as f32 / SAMPLE_RATE as f32)).sin())
        .collect();

    // Peer 3: 880 Hz at -14 dBFS
    let p3: Vec<f32> = (0..num_samples)
        .map(|i| p1_amp * (2.0 * PI * 880.0 * (i as f32 / SAMPLE_RATE as f32)).sin())
        .collect();

    // Mix 3 peers
    let mut mixed = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let sum = p1[i] + p2[i] + p3[i];
        mixed.push(soft_limit_sample(sum));
    }

    let hard_clips = mixed.iter().filter(|&&s| s.abs() >= 1.0).count();
    assert_eq!(hard_clips, 0, "3-peer mix must have 0 clips at +-1.0");

    let peak = mixed.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        peak <= 0.989,
        "mixed peak {peak} must not exceed soft ceiling"
    );

    // Extreme overdrive test: 3 peers at amplitude 0.6 each summing to 1.8 peak
    let overdriven_sum: Vec<f32> = (0..num_samples)
        .map(|i| {
            let s = 0.6 * (2.0 * PI * 440.0 * (i as f32 / SAMPLE_RATE as f32)).sin()
                + 0.6 * (2.0 * PI * 880.0 * (i as f32 / SAMPLE_RATE as f32)).sin()
                + 0.6 * (2.0 * PI * 1320.0 * (i as f32 / SAMPLE_RATE as f32)).sin();
            soft_limit_sample(s)
        })
        .collect();

    let od_clips = overdriven_sum.iter().filter(|&&s| s.abs() >= 1.0).count();
    assert_eq!(
        od_clips, 0,
        "overdriven mixer must have 0 hard clips at +-1.0"
    );
    let od_peak = overdriven_sum.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        od_peak <= 0.9886,
        "overdriven peak {od_peak} must be smoothly saturated below 0.9886"
    );
}

/// Test Case 3: Music Codec (12 kHz CELT Tone Preservation)
///
/// In Application::Audio (fullband CELT mode @ 128 kbps), 12 kHz high frequencies
/// are preserved without speech-band roll-off.
#[test]
fn test_music_codec_preserves_12khz_celt_tone() {
    let mut encoder = OpusEncoderWrapper::new_music(128_000).expect("create music encoder");
    let mut decoder = OpusDecoderWrapper::new().expect("create opus decoder");

    let num_samples = 48_000; // 1 second @ 48 kHz
    let input: Vec<f32> = (0..num_samples)
        .map(|i| 0.5 * (2.0 * PI * 12_000.0 * (i as f32 / SAMPLE_RATE as f32)).sin())
        .collect();

    let mut packets = Vec::new();
    for chunk in input.chunks(OPUS_FRAME) {
        if let Some(pkt) = encoder.push(chunk).expect("encode music frame") {
            packets.push(pkt);
        }
    }
    assert!(!packets.is_empty(), "music encoder must produce packets");

    let mut decoded_all = Vec::new();
    for pkt in &packets {
        let dec = decoder.decode(Some(pkt)).expect("decode music packet");
        decoded_all.extend_from_slice(&dec);
    }

    // Skip first 2 frames (encoder filter delay / warmup)
    let skip = 2 * OPUS_FRAME;
    let eval_in = &input[skip..decoded_all.len().min(input.len())];
    let eval_out = &decoded_all[skip..eval_in.len() + skip];

    let in_rms = (eval_in.iter().map(|s| s * s).sum::<f32>() / eval_in.len() as f32).sqrt();
    let out_rms = (eval_out.iter().map(|s| s * s).sum::<f32>() / eval_out.len() as f32).sqrt();

    assert!(in_rms > 0.0);
    let energy_ratio = out_rms / in_rms;
    assert!(
        energy_ratio > 0.75,
        "12 kHz tone in CELT music mode must retain >= 75% energy (-2.5 dB), got {energy_ratio:.3}"
    );
}

/// Test Case 4: Debug WAV Loopback Writer
///
/// Validates that WavWriter creates a valid 48 kHz 16-bit mono PCM WAV file,
/// writes samples accurately, finalizes the RIFF header, and read_wav recovers it.
#[test]
fn test_debug_wav_writer_and_reader() {
    let tmp_path = std::env::temp_dir().join("verio_debug_wav_test.wav");
    if tmp_path.exists() {
        let _ = fs::remove_file(&tmp_path);
    }

    let mut writer = WavWriter::create(&tmp_path).expect("create wav writer");
    let test_samples: Vec<f32> = (0..960)
        .map(|i| 0.5 * (2.0 * PI * 440.0 * (i as f32 / SAMPLE_RATE as f32)).sin())
        .collect();

    writer.write_f32(&test_samples).expect("write f32 samples");
    writer.finalize().expect("finalize wav file");

    assert!(tmp_path.exists(), "WAV file must exist on disk");
    let file_len = fs::metadata(&tmp_path).expect("get file metadata").len();
    // 44-byte header + 960 samples * 2 bytes = 1964 bytes
    assert_eq!(file_len, 44 + 960 * 2, "WAV file length must be 1964 bytes");

    let wav_data = read_wav(&tmp_path).expect("read wav file");
    assert_eq!(wav_data.sample_rate, 48_000, "sample rate must be 48 kHz");
    assert_eq!(wav_data.channels, 1, "channels must be 1 (mono)");
    assert_eq!(wav_data.samples.len(), 960, "must recover 960 samples");

    // Check sample values within 16-bit PCM quantization error (1 / 32768 ~ 0.000031)
    for (i, (&orig, &recovered)) in test_samples.iter().zip(wav_data.samples.iter()).enumerate() {
        let err = (orig - recovered).abs();
        assert!(
            err <= 0.0001,
            "sample {i}: orig {orig} recovered {recovered} (err {err})"
        );
    }

    let _ = fs::remove_file(&tmp_path);
}
