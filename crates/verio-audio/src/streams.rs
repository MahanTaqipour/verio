//! cpal stream setup.
//!
//! Preference order for input AND output: 48 kHz / mono / f32 (the pipeline
//! format). If a device won't open that way, its native default config is used
//! and conversion happens in the processing path (input) / RX thread (output).
//! The `used_fallback` flag lets the caller log the fallback once at init.
//!
//! Callbacks: capture only copies (format-converted) samples into the
//! lock-free ring buffer; playback only drains it and zero-fills underruns.

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig, SupportedStreamConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;

use crate::devices;
use crate::TARGET_SAMPLE_RATE;

#[derive(Debug, Clone, Copy)]
pub struct StreamSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct InputSetup {
    pub consumer: crate::HeapCons<f32>,
    pub stream: cpal::Stream,
    pub spec: StreamSpec,
    pub used_fallback: bool,
}

/// The pair of cpal Streams a pipeline keeps alive. cpal `Stream` is `!Send`
/// and dies silently when dropped, so this must stay on the thread that
/// created it for the whole pipeline lifetime.
pub type StreamPair = (cpal::Stream, cpal::Stream);

pub struct OutputSetup {
    pub producer: crate::HeapProd<f32>,
    pub stream: cpal::Stream,
    pub spec: StreamSpec,
    pub used_fallback: bool,
}

/// Pick the preferred config (48 kHz/mono/f32), falling back to the device
/// default. Returns (stream config, spec, used_fallback).
fn pick_config(
    ranges: &[cpal::SupportedStreamConfigRange],
    preferred: &SupportedStreamConfig,
) -> (StreamConfig, StreamSpec, bool) {
    let target = cpal::SampleRate(TARGET_SAMPLE_RATE);
    for range in ranges {
        if range.channels() == 1
            && range.sample_format() == SampleFormat::F32
            && range.min_sample_rate() <= target
            && range.max_sample_rate() >= target
        {
            let cfg = range.with_sample_rate(target);
            let spec = StreamSpec {
                sample_rate: cfg.sample_rate().0,
                channels: cfg.channels(),
            };
            return (StreamConfig::from(cfg), spec, false);
        }
    }
    let spec = StreamSpec {
        sample_rate: preferred.sample_rate().0,
        channels: preferred.channels(),
    };
    (StreamConfig::from(preferred.clone()), spec, true)
}

pub fn open_input(device_name: Option<&str>, ring_seconds: f32) -> Result<InputSetup, String> {
    let device = devices::find_input_device(device_name)?;
    let dev_name = device.name().unwrap_or_else(|_| "<unnamed>".into());

    let supported: Vec<cpal::SupportedStreamConfigRange> = device
        .supported_input_configs()
        .map_err(|e| format!("input configs for {dev_name}: {e}"))?
        .collect();
    let preferred = device
        .default_input_config()
        .map_err(|e| format!("default input config for {dev_name}: {e}"))?;
    let (stream_cfg, spec, used_fallback) = pick_config(&supported, &preferred);
    let sample_format = if used_fallback {
        preferred.sample_format()
    } else {
        SampleFormat::F32
    };

    let capacity =
        (f64::from(spec.sample_rate) * f64::from(spec.channels) * f64::from(ring_seconds)) as usize;
    let (producer, consumer) = HeapRb::<f32>::new(capacity.max(1)).split();
    let mut producer_opt = Some(producer);

    let err_fn = |e: cpal::StreamError| tracing::warn!("input stream error: {e}");
    let build_result = match sample_format {
        SampleFormat::F32 => {
            let mut prod = producer_opt.take().expect("producer");
            device.build_input_stream(
                &stream_cfg,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    for s in data {
                        let _ = prod.try_push(*s);
                    }
                },
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let mut prod = producer_opt.take().expect("producer");
            device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    for s in data {
                        let _ = prod.try_push(f32::from(*s) / 32_768.0);
                    }
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let mut prod = producer_opt.take().expect("producer");
            device.build_input_stream(
                &stream_cfg,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    for s in data {
                        let _ = prod.try_push((f32::from(*s) - 32_768.0) / 32_768.0);
                    }
                },
                err_fn,
                None,
            )
        }
        other => {
            return Err(format!(
                "unsupported input sample format {other:?} on {dev_name}"
            ))
        }
    };
    let stream = build_result.map_err(|e| format!("open input stream {dev_name}: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("start input stream {dev_name}: {e}"))?;

    Ok(InputSetup {
        consumer,
        stream,
        spec,
        used_fallback,
    })
}

/// Open the output stream. `gate`: when `Some(flag)`, the callback outputs
/// silence (without draining the ring) until the flag goes true — used to arm
/// playback only after the fixed 40 ms loopback pre-buffer has filled.
pub fn open_output(
    device_name: Option<&str>,
    ring_seconds: f32,
    gate: Option<Arc<AtomicBool>>,
) -> Result<OutputSetup, String> {
    let device = devices::find_output_device(device_name)?;
    let dev_name = device.name().unwrap_or_else(|_| "<unnamed>".into());

    let supported: Vec<cpal::SupportedStreamConfigRange> = device
        .supported_output_configs()
        .map_err(|e| format!("output configs for {dev_name}: {e}"))?
        .collect();
    let preferred = device
        .default_output_config()
        .map_err(|e| format!("default output config for {dev_name}: {e}"))?;
    let (stream_cfg, spec, used_fallback) = pick_config(&supported, &preferred);
    let sample_format = if used_fallback {
        preferred.sample_format()
    } else {
        SampleFormat::F32
    };

    let capacity =
        (f64::from(spec.sample_rate) * f64::from(spec.channels) * f64::from(ring_seconds)) as usize;
    let (producer, consumer) = HeapRb::<f32>::new(capacity.max(1)).split();

    let err_fn = |e: cpal::StreamError| tracing::warn!("output stream error: {e}");
    let build_result = match sample_format {
        SampleFormat::F32 => {
            let mut cons = consumer;
            device.build_output_stream(
                &stream_cfg,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !gate_open(&gate) {
                        data.fill(0.0);
                        return;
                    }
                    for chunk in data.chunks_mut(OUTPUT_CHUNK_LEN) {
                        let n = cons.pop_slice(chunk);
                        chunk[n..].fill(0.0);
                    }
                },
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            // Reusable conversion buffer: no per-callback allocation.
            let mut cons = consumer;
            let mut tmp = vec![0.0_f32; OUTPUT_CHUNK_LEN];
            device.build_output_stream(
                &stream_cfg,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    if !gate_open(&gate) {
                        data.fill(0);
                        return;
                    }
                    fill_output_chunks(data, &mut tmp, 0i16, |buf| cons.pop_slice(buf), f32_to_i16);
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            // Dedicated unsigned 16-bit output branch centered at 32768.
            let mut cons = consumer;
            let mut tmp = vec![0.0_f32; OUTPUT_CHUNK_LEN];
            device.build_output_stream(
                &stream_cfg,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    if !gate_open(&gate) {
                        data.fill(U16_SILENCE);
                        return;
                    }
                    fill_output_chunks(
                        data,
                        &mut tmp,
                        U16_SILENCE,
                        |buf| cons.pop_slice(buf),
                        f32_to_u16,
                    );
                },
                err_fn,
                None,
            )
        }
        other => {
            return Err(format!(
                "unsupported output sample format {other:?} on {dev_name}"
            ))
        }
    };
    let stream = build_result.map_err(|e| format!("open output stream {dev_name}: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("start output stream {dev_name}: {e}"))?;

    Ok(OutputSetup {
        producer,
        stream,
        spec,
        used_fallback,
    })
}

const OUTPUT_CHUNK_LEN: usize = 8192;
const U16_SILENCE: u16 = 32_768;

#[inline]
fn f32_to_i16(sample: f32) -> i16 {
    if !sample.is_finite() {
        return 0;
    }
    (sample.clamp(-1.0, 1.0) * 32_767.0) as i16
}

#[inline]
fn f32_to_u16(sample: f32) -> u16 {
    if !sample.is_finite() {
        return U16_SILENCE;
    }
    ((sample.clamp(-1.0, 1.0) * 32_767.0).round() + 32_768.0).clamp(0.0, 65_535.0) as u16
}

fn fill_output_chunks<T: Copy>(
    data: &mut [T],
    tmp: &mut [f32],
    silence: T,
    mut pull: impl FnMut(&mut [f32]) -> usize,
    convert: impl Fn(f32) -> T,
) {
    let chunk_cap = tmp.len().max(1);
    for chunk in data.chunks_mut(chunk_cap) {
        let n = pull(&mut tmp[..chunk.len()]);
        for (dst, src) in chunk[..n].iter_mut().zip(tmp[..n].iter()) {
            *dst = convert(*src);
        }
        chunk[n..].fill(silence);
    }
}

/// Hot-path-safe gate read: plain atomic load, no lock, no logging.
fn gate_open(gate: &Option<Arc<AtomicBool>>) -> bool {
    match gate {
        None => true,
        Some(g) => g.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_to_u16_centers_silence_clamps_and_handles_nan() {
        assert_eq!(f32_to_u16(0.0), U16_SILENCE);
        assert_eq!(f32_to_u16(1.0), 65_535);
        assert_eq!(f32_to_u16(2.5), 65_535);
        assert_eq!(f32_to_u16(-1.0), 1);
        assert_eq!(f32_to_u16(-2.5), 1);
        assert_eq!(f32_to_u16(f32::NAN), U16_SILENCE);
        assert_eq!(f32_to_u16(f32::INFINITY), U16_SILENCE);
    }

    #[test]
    fn fill_output_chunks_handles_buffers_over_8192_without_panic() {
        let total = 20_000usize;
        let src_len = 10_000usize;
        let source: Vec<f32> = (0..src_len).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let mut cursor = 0usize;
        let mut tmp = vec![0.0_f32; OUTPUT_CHUNK_LEN];
        let mut out_u16 = vec![0u16; total];

        fill_output_chunks(
            &mut out_u16,
            &mut tmp,
            U16_SILENCE,
            |buf| {
                assert!(buf.len() <= OUTPUT_CHUNK_LEN);
                let avail = source.len().saturating_sub(cursor).min(buf.len());
                buf[..avail].copy_from_slice(&source[cursor..cursor + avail]);
                cursor += avail;
                avail
            },
            f32_to_u16,
        );

        assert_eq!(out_u16[0], 65_535);
        assert_eq!(out_u16[1], 1);
        assert_eq!(out_u16[src_len - 2], 65_535);
        assert_eq!(out_u16[src_len - 1], 1);
        assert!(out_u16[src_len..].iter().all(|&s| s == U16_SILENCE));

        // Same chunk safety for i16 output.
        let mut cursor_i = 0usize;
        let mut out_i16 = vec![99i16; total];
        fill_output_chunks(
            &mut out_i16,
            &mut tmp,
            0i16,
            |buf| {
                let avail = source.len().saturating_sub(cursor_i).min(buf.len());
                buf[..avail].copy_from_slice(&source[cursor_i..cursor_i + avail]);
                cursor_i += avail;
                avail
            },
            f32_to_i16,
        );
        assert_eq!(out_i16[0], 32_767);
        assert_eq!(out_i16[1], -32_767);
        assert!(out_i16[src_len..].iter().all(|&s| s == 0));
    }
}


