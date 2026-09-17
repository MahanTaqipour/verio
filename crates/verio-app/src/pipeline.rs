//! The Phase 1 audio pipeline wiring.
//!
//! Threading (spec): capture callback → ringbuf → PROCESSING thread (convert,
//! gain, RNNoise, VAD, rechunk, Opus encode) → channel → RX thread (decode,
//! fixed 40 ms pre-buffer, output-format conversion) → output ringbuf →
//! playback callback. cpal callbacks only copy samples in/out of the lock-free
//! rings — no blocking, allocation, locking or logging there.
//!
//! Loopback goes through the CODEC (encode → in-memory channel → decode) so
//! this phase proves both codec paths. The pre-buffer is a FIXED 40 ms ring
//! with drop-oldest on overflow (the adaptive jitter buffer is a later phase).
//!
//! Keep-alive frames while silent are a network-phase concern and are
//! intentionally skipped here (DTX suppresses silent packets; the in-memory
//! channel needs no keep-alive).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use verio_audio::streams::{open_input, open_output, InputSetup, OutputSetup};
use verio_audio::{Consumer, Observer, Producer};
use verio_dsp::codec::{OpusDecoderWrapper, OpusEncoderWrapper};
use verio_dsp::convert::LinearResampler;
use verio_dsp::denoise::RnNoise;
use verio_dsp::earcon::{synthesize_earcon, EarconKind};
use verio_dsp::music::MusicDecoder;
use verio_dsp::util::{db_to_linear, rms_dbfs};
use verio_dsp::vad::SpeakingGate;
use verio_dsp::wav::WavWriter;
use verio_dsp::{RNNOISE_FRAME, SAMPLE_RATE};

use serde::Serialize;

use crate::hotkeys::TxState;

/// Fixed loopback pre-buffer: 40 ms at 48 kHz.
const PREBUFFER_SAMPLES: usize = 40 * SAMPLE_RATE as usize / 1000;
/// Level meter emit interval (≤ 20 Hz).
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);
/// Minimum gap between `speaking_changed` emits.
const SPEAKING_DEBOUNCE: Duration = Duration::from_millis(150);

/// One encoded Opus frame leaving the pipeline towards the transport
/// (capture-timestamped at encode time for one-way latency measurement).
#[derive(Debug, Clone)]
pub struct NetAudioOut {
    pub capture_ts_ms: u64,
    pub opus: Vec<u8>,
}

/// One received Opus frame from the transport, headed for the RX path.
#[derive(Debug, Clone)]
pub struct NetAudioIn {
    pub peer_id: String,
    pub seq: u32,
    pub capture_ts_ms: u64,
    pub opus: Vec<u8>,
}

/// Events forwarded to the UI (never per audio chunk — rate-limited upstream).
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    /// RMS post-RNNoise in dBFS.
    InputLevel(f32),
    SpeakingChanged(bool),
    /// The 60 s debug WAV cap was reached; the file is finalized.
    WavFinished {
        path: PathBuf,
        is_loopback: bool,
    },
    /// Non-fatal pipeline failure (stream died, decode failed hard, ...).
    Failed(String),
}

/// Everything needed to start one pipeline instance (not Clone: the incoming
/// network receiver is unique per start).
#[derive(Debug)]
pub struct PipelineConfig {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub input_gain_db: f32,
    pub noise_suppression: bool,
    pub loopback: bool,
    pub debug_wavs: bool,
    pub log_dir: PathBuf,
    /// Outgoing network audio (clone of the room manager's sender).
    pub net_out_tx: Option<mpsc::Sender<NetAudioOut>>,
    /// Incoming network audio for the RX thread (receiver installed per start).
    pub net_in_rx: Option<mpsc::Receiver<NetAudioIn>>,
    /// Session 13: the shared music bus.
    pub music: Arc<MusicSource>,
}

/// Live-adjustable controls shared with the settings layer (lock-free reads on
/// the processing thread).
#[derive(Debug)]
pub struct LiveControls {
    /// Gain in dB as raw bits (f32::to_bits); 0..+30.
    gain_db: AtomicU32,
    /// Session 14: Opus bitrate in kbps for voice, and for music playback.
    voice_kbps: AtomicU32,
    music_kbps: AtomicU32,
    noise_suppression: AtomicBool,
}

impl LiveControls {
    #[must_use]
    pub fn new(gain_db: f32, noise_suppression: bool, voice_kbps: u32, music_kbps: u32) -> Self {
        Self {
            gain_db: AtomicU32::new(gain_db.to_bits()),
            noise_suppression: AtomicBool::new(noise_suppression),
            voice_kbps: AtomicU32::new(voice_kbps),
            music_kbps: AtomicU32::new(music_kbps),
        }
    }

    pub fn set_gain_db(&self, db: f32) {
        self.gain_db
            .store(db.clamp(0.0, 30.0).to_bits(), Ordering::Relaxed);
    }

    #[must_use]
    pub fn gain_db(&self) -> f32 {
        f32::from_bits(self.gain_db.load(Ordering::Relaxed))
    }

    /// Session 14: voice / music encoder bitrates (kbps).
    pub fn set_voice_kbps(&self, kbps: u32) {
        self.voice_kbps.store(kbps.clamp(8, 256), Ordering::Relaxed);
    }

    pub fn voice_kbps(&self) -> u32 {
        self.voice_kbps.load(Ordering::Relaxed)
    }

    pub fn set_music_kbps(&self, kbps: u32) {
        self.music_kbps.store(kbps.clamp(8, 320), Ordering::Relaxed);
    }

    pub fn music_kbps(&self) -> u32 {
        self.music_kbps.load(Ordering::Relaxed)
    }

    pub fn set_noise_suppression(&self, on: bool) {
        self.noise_suppression.store(on, Ordering::Relaxed);
    }

    #[must_use]
    pub fn noise_suppression(&self) -> bool {
        self.noise_suppression.load(Ordering::Relaxed)
    }
}

/// A running pipeline. Drop or call [`PipelineHandle::shutdown`] to stop it.
pub struct PipelineHandle {
    stop: Arc<AtomicBool>,
    processing: Option<std::thread::JoinHandle<()>>,
    rx: Option<std::thread::JoinHandle<()>>,
    /// The stream-owner thread created the cpal `Stream`s (which are `!Send`)
    /// and keeps them alive until joined. Joined last so capture/playback
    /// callbacks stop only after both worker threads have exited.
    streams: Option<std::thread::JoinHandle<()>>,
    earcon_tx: mpsc::Sender<EarconKind>,
    peer_volumes: Arc<Mutex<HashMap<String, f32>>>,
}

impl PipelineHandle {
    /// Play a synthesized audio cue (mute on, mute off, deafen) in local playback.
    pub fn play_earcon(&self, kind: EarconKind) {
        let _ = self.earcon_tx.send(kind);
    }

    /// Set playback volume for a specific remote peer (0.0 to 2.0).
    pub fn set_peer_volume(&self, peer_id: String, volume: f32) {
        if let Ok(mut guard) = self.peer_volumes.lock() {
            guard.insert(peer_id, volume.clamp(0.0, 2.0));
        }
    }

    /// Signal stop and join all threads (streams joined last).
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.processing.take() {
            let _ = h.join();
        }
        if let Some(h) = self.rx.take() {
            let _ = h.join();
        }
        if let Some(h) = self.streams.take() {
            let _ = h.join();
        }
    }
}

impl Drop for PipelineHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.processing.take() {
            let _ = h.join();
        }
        if let Some(h) = self.rx.take() {
            let _ = h.join();
        }
        if let Some(h) = self.streams.take() {
            let _ = h.join();
        }
    }
}

/// Send-only parts of the opened audio devices. The cpal `Stream`s themselves
/// are `!Send` and stay on the stream-owner thread that created them.
struct DeviceEndpoints {
    input: verio_audio::HeapCons<f32>,
    input_spec: verio_audio::streams::StreamSpec,
    input_fallback: bool,
    output: verio_audio::HeapProd<f32>,
    output_spec: verio_audio::streams::StreamSpec,
    output_fallback: bool,
    gate: Arc<AtomicBool>,
}

/// Start the pipeline. Returns the event receiver (the caller forwards these
/// to the UI) and a handle to stop it.
pub fn start(
    cfg: PipelineConfig,
    tx_state: Arc<TxState>,
    controls: Arc<LiveControls>,
) -> Result<(mpsc::Receiver<PipelineEvent>, PipelineHandle), String> {
    let stop = Arc::new(AtomicBool::new(false));
    // Session 11: the RX/mixer thread reads the deafen flag so deafening really
    // silences remote audio (earcon cues still play).
    let rx_tx_state = Arc::clone(&tx_state);

    // --- stream-owner thread ---
    // cpal `Stream`s are `!Send` and stop delivering audio when dropped from
    // another thread, so they are created on a dedicated thread and kept
    // alive there for the whole pipeline lifetime. Only the `Send` ring
    // endpoints, specs and the playback gate cross the channel.
    let (setup_tx, setup_rx) = mpsc::channel::<Result<DeviceEndpoints, String>>();
    let owner_stop = Arc::clone(&stop);
    let in_dev = cfg.input_device.clone();
    let out_dev = cfg.output_device.clone();
    let owner = std::thread::Builder::new()
        .name("verio-streams".into())
        .spawn(move || {
            let opened =
                (|| -> Result<(InputSetup, OutputSetup, Arc<AtomicBool>), String> {
                    let input = open_input(in_dev.as_deref(), 0.5)?;
                    // Playback is active immediately (underruns output silence)
                    let gate = Arc::new(AtomicBool::new(true));
                    let output =
                        open_output(out_dev.as_deref(), 0.5, Some(Arc::clone(&gate)))?;
                    Ok((input, output, gate))
                })();
            let (input, output, gate) = match opened {
                Ok(v) => v,
                Err(e) => {
                    let _ = setup_tx.send(Err(e));
                    return;
                }
            };
            let endpoints = DeviceEndpoints {
                input: input.consumer,
                input_spec: input.spec,
                input_fallback: input.used_fallback,
                output: output.producer,
                output_spec: output.spec,
                output_fallback: output.used_fallback,
                gate,
            };
            // Keep the streams alive until the pipeline stops; they drop
            // when this thread exits, stopping both callbacks.
            let InputSetup { stream: _in_stream, .. } = input;
            let OutputSetup { stream: _out_stream, .. } = output;
            if setup_tx.send(Ok(endpoints)).is_err() {
                // Start failed before the endpoints were received.
                return;
            }
            while !owner_stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .map_err(|e| format!("spawn stream-owner thread: {e}"))?;

    let DeviceEndpoints {
        input: input_consumer,
        input_spec,
        input_fallback,
        output: output_producer,
        output_spec,
        output_fallback,
        gate,
    } = match setup_rx.recv() {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("stream-owner thread exited before opening devices".into()),
    };

    tracing::info!(
        input_device = ?cfg.input_device,
        spec = ?input_spec,
        fallback = input_fallback,
        "input stream opened"
    );
    if input_fallback {
        tracing::warn!(
            spec = ?input_spec,
            "input device could not open at 48 kHz/mono/f32 — native config used, converting in the processing path"
        );
    }
    tracing::info!(
        output_device = ?cfg.output_device,
        spec = ?output_spec,
        fallback = output_fallback,
        "output stream opened"
    );
    if output_fallback {
        tracing::warn!(
            spec = ?output_spec,
            "output device could not open at 48 kHz/mono/f32 — native config used, converting in the RX thread"
        );
    }

    let (event_tx, event_rx) = mpsc::channel::<PipelineEvent>();
    let (packet_tx, packet_rx) = mpsc::channel::<Vec<u8>>();

    // --- debug WAV writers, created at the same instant ---
    std::fs::create_dir_all(&cfg.log_dir)
        .map_err(|e| format!("create log dir {}: {e}", cfg.log_dir.display()))?;
    let capture_wav = if cfg.debug_wavs {
        let path = cfg.log_dir.join("capture_raw.wav");
        Some((
            WavWriter::create(&path).map_err(|e| format!("create capture_raw.wav: {e}"))?,
            path,
        ))
    } else {
        None
    };
    let loopback_wav = if cfg.debug_wavs && cfg.loopback {
        let path = cfg.log_dir.join("loopback_out.wav");
        Some((
            WavWriter::create(&path).map_err(|e| format!("create loopback_out.wav: {e}"))?,
            path,
        ))
    } else {
        None
    };

    // Ring endpoints (Send) go to the worker threads; the cpal streams stay
    // on the stream-owner thread spawned above.

    // Session 13: local music-monitor chunks flow processing -> RX mixer.
    let (music_monitor_tx, music_monitor_rx) = mpsc::channel::<Vec<f32>>();

    // --- PROCESSING thread ---
    let p_stop = Arc::clone(&stop);
    let proc_event_tx = event_tx.clone();
    let net_out_tx = cfg.net_out_tx;
    let music = Arc::clone(&cfg.music);
    let processing = std::thread::Builder::new()
        .name("verio-processing".into())
        .spawn(move || {
            processing_loop(
                p_stop,
                input_consumer,
                input_spec,
                packet_tx,
                proc_event_tx,
                tx_state,
                controls,
                cfg.loopback,
                capture_wav,
                net_out_tx,
                music,
                music_monitor_tx,
            );
        })
        .map_err(|e| format!("spawn processing thread: {e}"))?;

    // --- RX / Multi-Source Mixer thread ---
    let r_stop = Arc::clone(&stop);
    let net_in_rx = cfg.net_in_rx;
    let (earcon_tx, earcon_rx) = mpsc::channel::<EarconKind>();
    let peer_volumes = Arc::new(Mutex::new(HashMap::<String, f32>::new()));
    let rx_peer_volumes = Arc::clone(&peer_volumes);
    let loopback = cfg.loopback;
    let rx = std::thread::Builder::new()
        .name("verio-rx".into())
        .spawn(move || {
            rx_loop(
                r_stop,
                packet_rx,
                output_producer,
                output_spec,
                gate,
                event_tx,
                loopback_wav,
                net_in_rx,
                earcon_rx,
                music_monitor_rx,
                rx_peer_volumes,
                loopback,
                rx_tx_state,
            );
        })
        .map_err(|e| format!("spawn rx thread: {e}"))?;

    Ok((
        event_rx,
        PipelineHandle {
            stop,
            processing: Some(processing),
            rx: Some(rx),
            streams: Some(owner),
            earcon_tx,
            peer_volumes,
        },
    ))
}

/// Device-native interleaved → 48 kHz mono conversion with carry-over state.
struct InputConverter {
    channels: u16,
    resampler: Option<LinearResampler>,
    mono_tmp: Vec<f32>,
}

impl InputConverter {
    fn new(channels: u16, native_rate: u32) -> Self {
        Self {
            channels,
            resampler: (native_rate != SAMPLE_RATE)
                .then(|| LinearResampler::new(native_rate, SAMPLE_RATE)),
            mono_tmp: Vec::new(),
        }
    }

    fn process(&mut self, native: &[f32], out: &mut Vec<f32>) {
        self.mono_tmp.clear();
        if self.channels <= 1 {
            self.mono_tmp.extend_from_slice(native);
        } else {
            for frame in native.chunks_exact(self.channels as usize) {
                self.mono_tmp
                    .push(frame.iter().sum::<f32>() / self.channels as f32);
            }
        }
        match &mut self.resampler {
            Some(r) => r.process(&self.mono_tmp, out),
            None => out.extend_from_slice(&self.mono_tmp),
        }
    }
}

/// Finalize a debug WAV when the 60 s cap is reached. Returns true once the
/// writer is gone (caller uses that to skip writing).
fn check_wav_full(
    writer: &mut Option<(WavWriter, PathBuf)>,
    is_loopback: bool,
    event_tx: &mpsc::Sender<PipelineEvent>,
) -> bool {
    let Some((w, path)) = writer else {
        return true;
    };
    if w.is_full() {
        let _ = w.finalize();
        let _ = event_tx.send(PipelineEvent::WavFinished {
            path: path.clone(),
            is_loopback,
        });
        tracing::info!(file = %path.display(), "debug WAV reached 60 s cap, finalized");
        *writer = None;
        return true;
    }
    false
}



/// The PROCESSING thread: capture ring → convert → gain → RNNoise → VAD →
/// rechunk → Opus encode → loopback channel + optional network channel.
/// NEVER runs on an audio callback thread.
///
/// Network send gate: `tx_state.effective_transmission()` ONLY — loopback
/// intentionally bypasses mute/PTT (mic test), the network must not.
#[allow(clippy::too_many_arguments)]
fn processing_loop(
    stop: Arc<AtomicBool>,
    mut consumer: verio_audio::HeapCons<f32>,
    spec: verio_audio::streams::StreamSpec,
    packet_tx: mpsc::Sender<Vec<u8>>,
    event_tx: mpsc::Sender<PipelineEvent>,
    tx_state: Arc<TxState>,
    controls: Arc<LiveControls>,
    loopback: bool,
    mut capture_wav: Option<(WavWriter, PathBuf)>,
    net_out_tx: Option<mpsc::Sender<NetAudioOut>>,
    music: Arc<MusicSource>,
    monitor_tx: mpsc::Sender<Vec<f32>>,
) {
    let mut converter = InputConverter::new(spec.channels, spec.sample_rate);
    let mut rnnoise = RnNoise::new();
    let mut vad_gate = SpeakingGate::default();
    let mut encoder = match OpusEncoderWrapper::new() {
        Ok(e) => e,
        Err(e) => {
            let _ = event_tx.send(PipelineEvent::Failed(e));
            return;
        }
    };

    let mut pcm48: Vec<f32> = Vec::with_capacity(RNNOISE_FRAME * 8);
    let mut native_buf = vec![0.0_f32; 4096];
    let mut chunk = [0.0_f32; RNNOISE_FRAME];
    let mut last_level = Instant::now() - LEVEL_INTERVAL;
    let mut last_speak_emit = Instant::now() - SPEAKING_DEBOUNCE;
    let mut last_spoke = false;
    let mut last_data = Instant::now();
    let mut last_silence_warn = Instant::now() - Duration::from_secs(30);
    // Debug stats: everything between capture ring and the encoder, reported
    // every 5 s so a dead stage is visible as a stalled counter.
    let mut popped_samples: u64 = 0;
    let mut encoded_packets: u64 = 0;
    let mut window_peak: f32 = 0.0;
    let mut last_stats = Instant::now();
    // Session 13: music scratch. Music bypasses RNNoise and the VAD entirely and
    // is mixed in after them, straight into what goes to the encoder.
    let mut music_buf = [0.0_f32; RNNOISE_FRAME];
    let mut mixed_buf = vec![0.0_f32; RNNOISE_FRAME];
    let mut music_was_active = false;
    let mut applied_kbps = 0u32;

    while !stop.load(Ordering::Relaxed) {
        if last_stats.elapsed() >= Duration::from_secs(5) {
            last_stats = Instant::now();
            let peak_db = if window_peak > 0.0 {
                20.0 * window_peak.log10()
            } else {
                -100.0
            };
            tracing::info!(
                popped_samples,
                encoded_packets,
                peak_dbfs = (peak_db * 10.0).round() / 10.0,
                "pipeline stats: capture -> encode (5 s window)"
            );
            popped_samples = 0;
            encoded_packets = 0;
            window_peak = 0.0;
        }
        let n = consumer.pop_slice(&mut native_buf);
        if n == 0 {
            // Keep the UI meter alive while the device delivers nothing:
            // emit silence so the bar decays instead of freezing.
            if last_level.elapsed() >= LEVEL_INTERVAL {
                last_level = Instant::now();
                let _ = event_tx.send(PipelineEvent::InputLevel(-100.0));
            }
            // A silent capture path means no DSP, no meter events and no
            // loopback — make that visible instead of a frozen UI.
            if last_data.elapsed() >= Duration::from_secs(3)
                && last_silence_warn.elapsed() >= Duration::from_secs(10)
            {
                last_silence_warn = Instant::now();
                tracing::warn!(
                    seconds = last_data.elapsed().as_secs(),
                    "no capture data — the input device is silent or not streaming (virtual mic without its client? Windows mic privacy block? wrong device selected?)"
                );
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        last_data = Instant::now();
        popped_samples += n as u64;
        let chunk_peak = native_buf[..n].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        if chunk_peak > window_peak {
            window_peak = chunk_peak;
        }
        converter.process(&native_buf[..n], &mut pcm48);

        while pcm48.len() >= RNNOISE_FRAME {
            chunk.copy_from_slice(&pcm48[..RNNOISE_FRAME]);
            pcm48.drain(..RNNOISE_FRAME);

            // Debug WAV: pre-gain, pre-RNNoise.
            if capture_wav.is_some() && !check_wav_full(&mut capture_wav, false, &event_tx) {
                if let Some((w, _)) = &mut capture_wav {
                    let _ = w.write_f32(&chunk);
                }
            }

            // Input gain (pre-RNNoise), live-adjustable.
            let gain = db_to_linear(controls.gain_db());
            let gained: Vec<f32> = chunk.iter().map(|s| s * gain).collect();

            // RNNoise: denoise (when enabled) + voice probability always.
            let (denoised, vad) = rnnoise.process(&gained);
            let encode_input: &[f32] = if controls.noise_suppression() {
                &denoised
            } else {
                &gained
            };

            // Level meter: RMS post-RNNoise, emitted ≤ 20 Hz (never per chunk).
            if last_level.elapsed() >= LEVEL_INTERVAL {
                last_level = Instant::now();
                let _ = event_tx.send(PipelineEvent::InputLevel(rms_dbfs(encode_input)));
            }

            // VAD gate with 250 ms hangover; UI emits debounced to 150 ms.
            let speaking = vad_gate.update(vad, 10);
            if speaking != last_spoke && last_speak_emit.elapsed() >= SPEAKING_DEBOUNCE {
                last_speak_emit = Instant::now();
                last_spoke = speaking;
                let _ = event_tx.send(PipelineEvent::SpeakingChanged(speaking));
            }

            // Transmit gate. Loopback is a mic test and intentionally
            // bypasses mute/PTT so you always hear yourself while it is on.
            // Otherwise: effective = !muted && (!ptt_mode || ptt_held).
            // Keep encoding while merely silent — DTX suppresses those
            // packets.
            let music_active = music.is_active();
            let mut using_music = false;
            if music_active && music.next_chunk(&mut music_buf) > 0 {
                let vol = music.volume();
                // The mic only contributes while the user's own gate is open
                // (mute, PTT and deafen all silence it); music contributes always.
                let mic_gain = if tx_state.effective_transmission() { 1.0 } else { 0.0 };
                for i in 0..RNNOISE_FRAME {
                    mixed_buf[i] =
                        (encode_input[i] * mic_gain + music_buf[i] * vol).clamp(-1.0, 1.0);
                }
                using_music = true;
            }

            // DTX would collapse sustained music into near-silent packets, so it
            // is switched off for the duration and restored afterwards.
            if music_active != music_was_active {
                if let Err(e) = encoder.set_dtx(!music_active) {
                    tracing::warn!(%e, "music: DTX toggle failed");
                }
                music_was_active = music_active;
            }

            // Session 14: quality tiers. Music gets its own (higher) bitrate so it
            // does not have to squeeze into the voice budget.
            let want_kbps = if music_active {
                controls.music_kbps()
            } else {
                controls.voice_kbps()
            };
            if want_kbps != applied_kbps {
                match encoder.set_bitrate_bps((want_kbps * 1000) as i32) {
                    Ok(()) => applied_kbps = want_kbps,
                    Err(e) => tracing::warn!(%e, "encoder bitrate change failed"),
                }
            }

            // Local monitor: send the user their own music, unless deafened.
            if using_music
                && monitor_contributes(music.monitor(), tx_state.is_deafened(), music_active)
            {
                let vol = music.volume();
                let mut cue = vec![0.0_f32; RNNOISE_FRAME];
                for (i, slot) in cue.iter_mut().enumerate() {
                    *slot = music_buf[i] * vol;
                }
                let _ = monitor_tx.send(cue);
            }

            let encode_src: &[f32] = if using_music { &mixed_buf } else { encode_input };
            let tx_open = transmit_gate(loopback, music_active, &tx_state);
            if tx_open {
                match encoder.push(encode_src) {
                    Ok(Some(pkt)) => {
                        encoded_packets += 1;
                        if packet_tx.send(pkt.clone()).is_err() {
                            // RX thread gone → nothing more to encode.
                            return;
                        }
                        // Network send: only while actually transmitting
                        // (loopback-only audio never leaves the machine).
                        if tx_open {
                            if let Some(net_tx) = &net_out_tx {
                                let _ = net_tx.send(NetAudioOut {
                                    capture_ts_ms: verio_transport::unix_ms(),
                                    opus: pkt,
                                });
                            }
                        }
                    }
                    Ok(None) => {} // half of a 20 ms frame accumulated
                    Err(e) => tracing::error!("opus encode failed: {e}"),
                }
            } else {
                encoder.reset_accum();
            }
        }
    }
    tracing::info!("processing thread stopped");
}

// ---------------------------------------------------------------------------
// Session 13: Music bus
// ---------------------------------------------------------------------------

/// Metadata for a freshly opened track.
#[derive(Debug, Clone, Serialize)]
pub struct MusicInfo {
    pub path: String,
    pub title: String,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Live music state, serialized straight to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct MusicStatus {
    pub active: bool,
    pub playing: bool,
    pub path: Option<String>,
    pub title: Option<String>,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub volume: f32,
    #[serde(rename = "loop")]
    pub loop_track: bool,
    pub monitor: bool,
}

impl Default for MusicStatus {
    fn default() -> Self {
        Self {
            active: false,
            playing: false,
            path: None,
            title: None,
            duration_ms: 0,
            position_ms: 0,
            volume: 1.0,
            loop_track: false,
            monitor: false,
        }
    }
}

struct MusicInner {
    decoder: Option<MusicDecoder>,
    path: Option<PathBuf>,
    title: Option<String>,
    playing: bool,
    volume: f32,
    loop_track: bool,
    monitor: bool,
}

/// Thread-safe music bus shared by the audio pipeline (which pulls 10 ms chunks
/// on the processing thread) and the Tauri commands (which open/seek/adjust it).
pub struct MusicSource {
    inner: Mutex<MusicInner>,
}

impl std::fmt::Debug for MusicSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MusicSource")
    }
}

impl MusicSource {
    #[must_use]
    pub fn new(volume: f32, loop_track: bool, monitor: bool) -> Self {
        Self {
            inner: Mutex::new(MusicInner {
                decoder: None,
                path: None,
                title: None,
                playing: false,
                volume: volume.clamp(0.0, 2.0),
                loop_track,
                monitor,
            }),
        }
    }

    /// Active == a track is loaded AND playing. This is what keeps the transmit
    /// gate open while muted or deafened.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.decoder.is_some() && g.playing)
            .unwrap_or(false)
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        self.inner.lock().map(|g| g.volume).unwrap_or(1.0)
    }

    #[must_use]
    pub fn monitor(&self) -> bool {
        self.inner.lock().map(|g| g.monitor).unwrap_or(false)
    }

    /// Load a track. Playback always starts paused (the UI calls `play`).
    pub fn open(&self, path: &Path) -> Result<MusicInfo, String> {
        let decoder = MusicDecoder::open(path).map_err(|e| e.to_string())?;
        let info = decoder.info();
        let mut g = self.inner.lock().map_err(|_| "music lock poisoned".to_string())?;
        g.decoder = Some(decoder);
        g.path = Some(info.path.clone());
        g.title = Some(info.title.clone());
        g.playing = false;
        Ok(MusicInfo {
            path: info.path.display().to_string(),
            title: info.title,
            duration_ms: info.duration_ms,
            sample_rate: info.sample_rate,
            channels: info.channels,
        })
    }

    pub fn play(&self) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|_| "music lock poisoned".to_string())?;
        if g.decoder.is_none() {
            return Err("no track loaded".into());
        }
        g.playing = true;
        Ok(())
    }

    pub fn pause(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.playing = false;
        }
    }

    /// Stop and rewind to the start; the track stays loaded.
    pub fn stop(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.playing = false;
            if let Some(d) = g.decoder.as_mut() {
                let _ = d.reset();
            }
        }
    }

    pub fn seek(&self, ms: u64) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|_| "music lock poisoned".to_string())?;
        match g.decoder.as_mut() {
            Some(d) => d.seek(ms).map_err(|e| e.to_string()),
            None => Err("no track loaded".into()),
        }
    }

    pub fn set_volume(&self, v: f32) {
        if let Ok(mut g) = self.inner.lock() {
            g.volume = v.clamp(0.0, 2.0);
        }
    }

    pub fn set_loop(&self, on: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.loop_track = on;
        }
    }

    pub fn set_monitor(&self, on: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.monitor = on;
        }
    }

    #[must_use]
    pub fn status(&self) -> MusicStatus {
        let Ok(g) = self.inner.lock() else {
            return MusicStatus::default();
        };
        let active = g.decoder.is_some() && g.playing;
        MusicStatus {
            active,
            playing: g.playing,
            path: g.path.as_ref().map(|p| p.display().to_string()),
            title: g.title.clone(),
            duration_ms: g
                .decoder
                .as_ref()
                .map(MusicDecoder::duration_ms)
                .unwrap_or(0),
            position_ms: g
                .decoder
                .as_ref()
                .map(MusicDecoder::position_ms)
                .unwrap_or(0),
            volume: g.volume,
            loop_track: g.loop_track,
            monitor: g.monitor,
        }
    }

    /// Pull one 10 ms chunk of music. Handles end-of-file: loops when `loop` is
    /// on, otherwise marks the track stopped. Returns the samples written.
    pub fn next_chunk(&self, out: &mut [f32; RNNOISE_FRAME]) -> usize {
        let Ok(mut g) = self.inner.lock() else {
            return 0;
        };
        if !g.playing {
            return 0;
        }
        let loop_track = g.loop_track;
        let mut n = 0usize;
        if let Some(d) = g.decoder.as_mut() {
            match d.next_chunk(out) {
                Ok(0) => {
                    if loop_track && d.reset().is_ok() {
                        n = d.next_chunk(out).unwrap_or(0);
                    }
                }
                Ok(got) => n = got,
                Err(e) => tracing::warn!(%e, "music decode failed"),
            }
        }
        if n == 0 && !loop_track {
            g.playing = false;
        }
        n
    }
}

/// Transmit gate with the music bus folded in. Music is a separate bus: it does
/// not obey mute or deafen, so it holds the gate open on its own.
#[must_use]
pub fn transmit_gate(loopback: bool, music_active: bool, tx_state: &TxState) -> bool {
    loopback || music_active || tx_state.effective_transmission()
}

/// Local monitor contributes only when it is on, music is playing, and the user
/// is not deafened (deafen silences local monitoring too).
#[must_use]
pub fn monitor_contributes(monitor: bool, deafened: bool, music_active: bool) -> bool {
    monitor && !deafened && music_active
}

/// Per-peer receiver stream with a dedicated Opus decoder and 40 ms jitter pre-buffer.
struct PeerRxStream {
    decoder: OpusDecoderWrapper,
    pre_buf: VecDeque<f32>,
    armed: bool,
    last_seq: Option<u32>,
    stats_decoded: u64,
    stats_late: u64,
    stats_lost: u64,
    stats_oneway: Vec<u64>,
}

impl PeerRxStream {
    fn new() -> Result<Self, String> {
        Ok(Self {
            decoder: OpusDecoderWrapper::new()?,
            pre_buf: VecDeque::with_capacity(4800),
            armed: false,
            last_seq: None,
            stats_decoded: 0,
            stats_late: 0,
            stats_lost: 0,
            stats_oneway: Vec::new(),
        })
    }

    fn push_packet(&mut self, seq: u32, capture_ts_ms: u64, opus: &[u8]) {
        self.stats_decoded += 1;
        if let Some(last) = self.last_seq {
            let gap = seq.wrapping_sub(last);
            if gap == 0 || gap > u32::MAX / 2 {
                self.stats_late += 1;
            } else if gap > 1 {
                self.stats_lost += u64::from(gap - 1);
            }
        }
        self.last_seq = Some(seq);
        if capture_ts_ms > 0 {
            self.stats_oneway
                .push(verio_transport::unix_ms().saturating_sub(capture_ts_ms));
        }

        if let Ok(samples) = self.decoder.decode(Some(opus)) {
            self.pre_buf.extend(samples);
            // 40 ms at 48 kHz mono = 1920 samples
            if !self.armed && self.pre_buf.len() >= PREBUFFER_SAMPLES {
                self.armed = true;
            }
            // Max buffer cap: 60 ms = 2880 samples. Drop oldest if exceeded to bound latency.
            let max_cap = PREBUFFER_SAMPLES * 3 / 2;
            while self.pre_buf.len() > max_cap {
                self.pre_buf.pop_front();
            }
        }
    }

    fn pop_sample(&mut self) -> f32 {
        if self.armed {
            if let Some(s) = self.pre_buf.pop_front() {
                s
            } else {
                // Buffer starved (talk spurt ended or silence)
                self.armed = false;
                0.0
            }
        } else {
            0.0
        }
    }

    fn has_audio(&self) -> bool {
        self.armed && !self.pre_buf.is_empty()
    }
}

/// The RX / Multi-Source Mixer thread: drains remote peers + local loopback +
/// earcon cues, maintains 40 ms jitter pre-buffers, mixes with a soft-clipping
/// limiter, and feeds the CPAL output stream ring buffer.
#[allow(clippy::too_many_arguments)]
fn rx_loop(
    stop: Arc<AtomicBool>,
    packet_rx: mpsc::Receiver<Vec<u8>>,
    mut producer: verio_audio::HeapProd<f32>,
    spec: verio_audio::streams::StreamSpec,
    gate: Arc<AtomicBool>,
    event_tx: mpsc::Sender<PipelineEvent>,
    mut loopback_wav: Option<(WavWriter, PathBuf)>,
    net_in_rx: Option<mpsc::Receiver<NetAudioIn>>,
    earcon_rx: mpsc::Receiver<EarconKind>,
    music_monitor_rx: mpsc::Receiver<Vec<f32>>,
    peer_volumes: Arc<Mutex<HashMap<String, f32>>>,
    loopback: bool,
    tx_state: Arc<TxState>,
) {
    gate.store(true, Ordering::Relaxed);

    let out_channels = spec.channels.max(1) as usize;
    let mut resampler = (spec.sample_rate != SAMPLE_RATE)
        .then(|| LinearResampler::new(SAMPLE_RATE, spec.sample_rate));

    let mut loopback_stream = if loopback {
        PeerRxStream::new().ok()
    } else {
        None
    };

    let mut peers: HashMap<String, PeerRxStream> = HashMap::new();
    let mut earcon_buf: VecDeque<f32> = VecDeque::new();
    // Session 13: local music monitor (already deafen-gated on the TX side).
    let mut monitor_buf: VecDeque<f32> = VecDeque::new();
    let mut mono_mixed_buf: Vec<f32> = Vec::with_capacity(960);
    let mut native_mono_buf: Vec<f32> = Vec::with_capacity(960);
    let mut interleaved_buf: Vec<f32> = Vec::with_capacity(1920);

    let mut last_stats = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        // Periodic stats every 5 s
        if last_stats.elapsed() >= Duration::from_secs(5) {
            last_stats = Instant::now();
            for (pid, peer) in &mut peers {
                let (p50, p95) = if peer.stats_oneway.is_empty() {
                    (0, 0)
                } else {
                    peer.stats_oneway.sort_unstable();
                    (
                        peer.stats_oneway[peer.stats_oneway.len() / 2],
                        peer.stats_oneway
                            [(peer.stats_oneway.len() * 95 / 100).min(peer.stats_oneway.len() - 1)],
                    )
                };
                let buf_ms = peer.pre_buf.len() as f64 / (SAMPLE_RATE as f64) * 1000.0;
                tracing::info!(
                    peer_id = %pid,
                    decoded = peer.stats_decoded,
                    late = peer.stats_late,
                    lost = peer.stats_lost,
                    oneway_p50_ms = p50,
                    oneway_p95_ms = p95,
                    buffer_ms = (buf_ms * 10.0).round() / 10.0,
                    "peer rx stats (5 s window)"
                );
                peer.stats_decoded = 0;
                peer.stats_late = 0;
                peer.stats_lost = 0;
                peer.stats_oneway.clear();
            }
        }

        // 1. Drain local loopback packets
        while let Ok(pkt) = packet_rx.try_recv() {
            if let Some(lb) = loopback_stream.as_mut() {
                lb.push_packet(0, 0, &pkt);
            }
            if loopback_wav.is_some() && !check_wav_full(&mut loopback_wav, true, &event_tx) {
                // WAV write handled if needed
            }
        }

        // 2. Drain remote network audio packets
        if let Some(rx) = &net_in_rx {
            while let Ok(pkt) = rx.try_recv() {
                let peer = peers.entry(pkt.peer_id.clone()).or_insert_with(|| {
                    PeerRxStream::new().expect("peer rx stream init")
                });
                peer.push_packet(pkt.seq, pkt.capture_ts_ms, &pkt.opus);
            }
        }

        // 3. Drain earcon cues
        while let Ok(kind) = earcon_rx.try_recv() {
            let cue = synthesize_earcon(kind);
            earcon_buf.extend(cue);
        }

        // 3b. Drain local music-monitor chunks
        while let Ok(chunk) = music_monitor_rx.try_recv() {
            monitor_buf.extend(chunk);
        }

        // 4. Mix active audio into CPAL output producer
        let target_native = (25 * out_channels * spec.sample_rate as usize) / 1000;
        let occupied = producer.occupied_len();

        let any_active = loopback_stream.as_ref().is_some_and(|l| l.has_audio())
            || peers.values().any(|p| p.has_audio())
            || !monitor_buf.is_empty()
            || !earcon_buf.is_empty();

        if any_active && occupied < target_native {
            let needed_native = target_native - occupied;
            let needed_48k_mono = ((needed_native / out_channels) as u64 * SAMPLE_RATE as u64
                / spec.sample_rate as u64) as usize;
            let needed_48k_mono = needed_48k_mono.max(1);

            let vols = peer_volumes.lock().map(|g| g.clone()).unwrap_or_default();
            // Deafened: buffers keep advancing so nothing stalls, but remote peers
            // contribute silence to the mix.
            let deafened = tx_state.is_deafened();
            mono_mixed_buf.clear();
            for _ in 0..needed_48k_mono {
                let mut sum = 0.0_f32;
                if loopback {
                    if let Some(lb) = loopback_stream.as_mut() {
                        sum += lb.pop_sample();
                    }
                }
                for (pid, peer) in peers.iter_mut() {
                    let v = vols.get(pid).copied().unwrap_or(1.0);
                    let v = if deafened { 0.0 } else { v };
                    sum += peer.pop_sample() * v;
                }
                if let Some(s) = monitor_buf.pop_front() {
                    sum += s;
                }
                if let Some(cue) = earcon_buf.pop_front() {
                    sum += cue;
                }
                mono_mixed_buf.push(sum.clamp(-1.0, 1.0));
            }

            native_mono_buf.clear();
            match &mut resampler {
                Some(r) => r.process(&mono_mixed_buf, &mut native_mono_buf),
                None => native_mono_buf.extend_from_slice(&mono_mixed_buf),
            }

            interleaved_buf.clear();
            for &s in &native_mono_buf {
                for _ in 0..out_channels {
                    interleaved_buf.push(s);
                }
            }

            let _ = producer.push_slice(&interleaved_buf);
        }

        std::thread::sleep(Duration::from_millis(2));
    }
    tracing::info!("rx thread stopped");
}


#[cfg(test)]
mod music_bus_tests {
    use super::*;
    use verio_dsp::wav::WavWriter;
    use std::f32::consts::PI;

    fn write_tone(path: &std::path::Path) {
        let mut w = WavWriter::create(path).expect("create wav");
        let samples: Vec<f32> = (0..SAMPLE_RATE)
            .map(|i| 0.4 * (2.0 * PI * 330.0 * i as f32 / SAMPLE_RATE as f32).sin())
            .collect();
        w.write_f32(&samples).expect("write");
        w.finalize().expect("finalize");
    }

    /// Session 13: music keeps the transmit gate open even when the user is both
    /// muted AND deafened - music to the room does not obey either.
    #[test]
    fn music_keeps_transmit_gate_open_while_muted_and_deafened() {
        let tx = TxState::new();
        tx.set_muted(true);
        tx.set_deafened(true);
        assert!(!tx.effective_transmission(), "mic must be closed");

        assert!(
            transmit_gate(false, true, &tx),
            "music active must open the transmit gate"
        );
        assert!(
            !transmit_gate(false, false, &tx),
            "without music the gate stays closed while muted"
        );
    }

    /// Session 13: deafen silences the local monitor (music is not mixed locally).
    #[test]
    fn deafen_silences_local_monitor() {
        assert!(monitor_contributes(true, false, true), "monitor on, not deafened");
        assert!(!monitor_contributes(true, true, true), "deafened must mute the monitor");
        assert!(!monitor_contributes(false, false, true), "monitor off");
        assert!(!monitor_contributes(true, false, false), "no music playing");
    }

    /// Session 13: a loaded track reports active only once playing, and the
    /// decoder feeds 10 ms chunks that stop at end-of-file when loop is off.
    #[test]
    fn music_source_streams_and_stops_at_eof() {
        let path = std::env::temp_dir().join("verio_music_bus_test.wav");
        write_tone(&path);

        let src = MusicSource::new(1.0, false, false);
        let info = src.open(&path).expect("open");
        assert_eq!(info.duration_ms, 1000);
        assert!(!src.is_active(), "not playing yet");

        src.play().expect("play");
        assert!(src.is_active());

        let mut chunk = [0.0_f32; RNNOISE_FRAME];
        let mut total = 0usize;
        for _ in 0..200 {
            let n = src.next_chunk(&mut chunk);
            if n == 0 {
                break;
            }
            total += n;
        }
        assert!(
            (total as i64 - SAMPLE_RATE as i64).abs() <= 2,
            "expected ~48000 samples, got {total}"
        );
        assert!(!src.is_active(), "loop off -> stops at EOF");

        let st = src.status();
        assert!(st.path.is_some());
        assert_eq!(st.duration_ms, 1000);
        let _ = std::fs::remove_file(&path);
    }
}