//! Opus codec wrappers (audiopus).
//!
//! Encoder: 48 kHz mono, 32 000 bps, complexity 5, in-band FEC ON, DTX ON,
//! fullband, VOIP application. Handles the 480 → 960 rechunk: callers push
//! denoised 10 ms chunks and receive a packet whenever a full 20 ms frame has
//! accumulated. Encoding continues while silent — DTX suppresses the packets.
//!
//! DTX has no high-level method in audiopus 0.2, so it is enabled via the raw
//! encoder CTL request `OPUS_SET_DTX` (= 4016, opus_defines.h).

use audiopus::{coder::Encoder, Application, Bitrate, Channels, SampleRate};

use crate::OPUS_FRAME;

/// `OPUS_SET_DTX_REQUEST` from opus_defines.h (audiopus 0.2 exposes no
/// high-level setter for DTX).
const OPUS_SET_DTX_REQUEST: i32 = 4016;

fn opus_err(context: &str, e: audiopus::Error) -> String {
    format!("opus {context}: {e:?}")
}

pub struct OpusEncoderWrapper {
    inner: Encoder,
    accum: Vec<f32>,
}

impl OpusEncoderWrapper {
    pub fn new() -> Result<Self, String> {
        let mut inner =
            Encoder::new(SampleRate::Hz48000, Channels::Mono, Application::Voip)
                .map_err(|e| opus_err("encoder init", e))?;
        inner
            .set_bitrate(Bitrate::BitsPerSecond(32_000))
            .map_err(|e| opus_err("set bitrate", e))?;
        inner
            .set_complexity(5)
            .map_err(|e| opus_err("set complexity", e))?;
        inner
            .enable_inband_fec()
            .map_err(|e| opus_err("enable in-band FEC", e))?;
        inner
            .set_encoder_ctl_request(OPUS_SET_DTX_REQUEST, 1)
            .map_err(|e| opus_err("enable DTX", e))?;
        Ok(Self {
            inner,
            accum: Vec::with_capacity(OPUS_FRAME),
        })
    }

    /// Push denoised samples (any length; pipeline pushes 480-sample chunks).
    /// Returns `Some(packet)` each time a full 10 ms frame has accumulated.
    pub fn push(&mut self, samples: &[f32]) -> Result<Option<Vec<u8>>, String> {
        self.accum.extend_from_slice(samples);
        if self.accum.len() < OPUS_FRAME {
            return Ok(None);
        }
        let frame: Vec<f32> = self.accum.drain(..OPUS_FRAME).collect();
        let mut packet = vec![0_u8; 4000];
        let len = self
            .inner
            .encode_float(&frame, &mut packet)
            .map_err(|e| opus_err("encode", e))?;
        packet.truncate(len);
        Ok(Some(packet))
    }

    /// Drop a partially accumulated frame (e.g. when transmission gates close).
    pub fn reset_accum(&mut self) {
        self.accum.clear();
    }

    /// Session 14: change the encoder bitrate at runtime, so voice and music can
    /// use different quality tiers (32 kbps voice, 96/128 kbps music).
    pub fn set_bitrate_bps(&mut self, bps: i32) -> Result<(), String> {
        self.inner
            .set_bitrate(Bitrate::BitsPerSecond(bps))
            .map_err(|e| opus_err("set bitrate", e))
    }

    /// Session 13: enable/disable DTX. Music playback must turn DTX OFF (or it
    /// would collapse sustained tones into near-silent packets) and restore it
    /// when music stops. Called only on transitions, never per frame.
    pub fn set_dtx(&mut self, on: bool) -> Result<(), String> {
        self.inner
            .set_encoder_ctl_request(OPUS_SET_DTX_REQUEST, i32::from(on))
            .map_err(|e| opus_err("set DTX", e))
    }

    /// Generate the network keep-alive frame: encode consecutive
    /// 10 ms frames of digital silence and keep the SMALLEST packet. Opus DTX
    /// only kicks in after sustained silence, so a few frames are needed; the
    /// result is a tiny (typically 2-byte) valid Opus frame that keeps the
    /// remote decoder warm and NAT mappings alive.
    pub fn opus_silence_frame() -> Result<Vec<u8>, String> {
        let mut enc = OpusEncoderWrapper::new()?;
        let silence = vec![0.0_f32; OPUS_FRAME];
        let mut best: Option<Vec<u8>> = None;
        for _ in 0..50 {
            if let Some(pkt) = enc.push(&silence)? {
                if best.as_ref().is_none_or(|b| pkt.len() < b.len()) {
                    best = Some(pkt);
                }
                if best.as_ref().is_some_and(|b| b.len() <= 2) {
                    break; // DTX reached its minimum
                }
            }
        }
        best.ok_or_else(|| "encoder produced no silence packet".to_string())
    }
}

pub struct OpusDecoderWrapper {
    inner: audiopus::coder::Decoder,
    last_packet: Vec<u8>,
}

impl OpusDecoderWrapper {
    pub fn new() -> Result<Self, String> {
        let inner = audiopus::coder::Decoder::new(SampleRate::Hz48000, Channels::Mono)
            .map_err(|e| opus_err("decoder init", e))?;
        Ok(Self {
            inner,
            last_packet: Vec::new(),
        })
    }

    /// Decode one packet into 480 samples. `None` = lost packet → concealment
    /// with in-band FEC if a previous packet exists, otherwise PLC.
    pub fn decode(&mut self, packet: Option<&[u8]>) -> Result<Vec<f32>, String> {
        if let Some(p) = packet {
            self.last_packet.clear();
            self.last_packet.extend_from_slice(p);
        }
        let mut out = vec![0.0_f32; OPUS_FRAME];
        let result = match packet {
            Some(p) => self.inner.decode_float(Some(p), &mut out, false),
            None if !self.last_packet.is_empty() => {
                // Lost packet with FEC data available in the previous packet.
                self.inner
                    .decode_float(Some(&self.last_packet), &mut out, true)
            }
            None => self.inner.decode_float(None::<&[u8]>, &mut out, false), // PLC
        };
        match result {
            Ok(_) => Ok(out),
            Err(e) => {
                // Anything unexpected → full PLC concealment instead of failing.
                tracing::warn!("opus decode failed ({e:?}), falling back to PLC");
                self.inner
                    .decode_float(None::<&[u8]>, &mut out, false)
                    .map_err(|e| opus_err("PLC", e))?;
                Ok(out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_one_chunk_produces_packet() {
        let mut enc = OpusEncoderWrapper::new().expect("encoder");
        let pkt = enc
            .push(&[0.0; 480])
            .expect("push")
            .expect("10 ms frame → immediate packet");
        assert!(!pkt.is_empty());
    }

    #[test]
    fn silence_encodes_to_tiny_dtx_packet() {
        let mut enc = OpusEncoderWrapper::new().expect("encoder");
        let mut tiny_seen = false;
        // DTX needs sustained silence (~0.5 s+) before it starts suppressing;
        // encode 4 s worth of 10 ms chunks and require at least one tiny frame.
        for _ in 0..400 {
            if let Some(p) = enc.push(&[0.0; 480]).expect("push") {
                if p.len() <= 4 {
                    tiny_seen = true;
                    break;
                }
            }
        }
        assert!(tiny_seen, "no DTX-sized packet within 4 s of silence");
    }

    #[test]
    fn roundtrip_and_plc() {
        let mut enc = OpusEncoderWrapper::new().expect("encoder");
        let mut dec = OpusDecoderWrapper::new().expect("decoder");
        let tone: Vec<f32> = (0..OPUS_FRAME)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI / 96.0).sin() * 0.5)
            .collect();
        let pkt = enc.push(&tone).expect("push").expect("packet");
        let decoded = dec.decode(Some(&pkt)).expect("decode");
        assert_eq!(decoded.len(), OPUS_FRAME);
        let peak = decoded.iter().cloned().fold(0.0_f32, f32::max);
        assert!(peak > 0.05, "decoded peak {peak}");
        // Lost packet → FEC/PLC concealment, still 480 samples.
        let concealed = dec.decode(None).expect("conceal");
        assert_eq!(concealed.len(), OPUS_FRAME);
    }
}
