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
                    let n = cons.pop_slice(data);
                    data[n..].fill(0.0);
                },
                err_fn,
                None,
            )
        }
        SampleFormat::I16 | SampleFormat::U16 => {
            // Reusable conversion buffer: no per-callback allocation.
            let mut cons = consumer;
            let mut tmp = vec![0.0_f32; 8192];
            device.build_output_stream(
                &stream_cfg,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    if !gate_open(&gate) {
                        data.fill(0);
                        return;
                    }
                    let n = cons.pop_slice(&mut tmp[..data.len()]);
                    for (dst, src) in data.iter_mut().zip(tmp.iter()) {
                        *dst = (src.clamp(-1.0, 1.0) * 32_767.0) as i16;
                    }
                    data[n..].fill(0);
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

/// Hot-path-safe gate read: plain atomic load, no lock, no logging.
fn gate_open(gate: &Option<Arc<AtomicBool>>) -> bool {
    match gate {
        None => true,
        Some(g) => g.load(Ordering::Relaxed),
    }
}


