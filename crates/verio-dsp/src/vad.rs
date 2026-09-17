//! VAD gate driven by RNNoise's voice probability.
//!
//! Enter "speaking" when voice probability exceeds [`ENTER_PROB`]; leave only
//! after [`HANGOVER_MS`] of continuous silence, so short gaps between words
//! don't flicker the UI indicator.

/// Voice probability threshold to enter the speaking state.
pub const ENTER_PROB: f32 = 0.6;
/// Time below threshold before going back to silent.
pub const HANGOVER_MS: u32 = 250;

#[derive(Debug, Default)]
pub struct SpeakingGate {
    speaking: bool,
    silent_ms: u32,
}

impl SpeakingGate {
    /// Feed one chunk's voice probability; `chunk_ms` is the chunk duration.
    /// Returns the current speaking state.
    pub fn update(&mut self, voice_prob: f32, chunk_ms: u32) -> bool {
        if voice_prob > ENTER_PROB {
            self.speaking = true;
            self.silent_ms = 0;
        } else if self.speaking {
            self.silent_ms += chunk_ms;
            if self.silent_ms >= HANGOVER_MS {
                self.speaking = false;
                self.silent_ms = 0;
            }
        }
        self.speaking
    }

    #[must_use]
    pub fn speaking(&self) -> bool {
        self.speaking
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enters_on_voice_exits_after_hangover() {
        let mut gate = SpeakingGate::default();
        assert!(gate.update(0.7, 10)); // 0.7 > 0.6 → speaking immediately
        assert!(gate.update(0.7, 10));
        // 24 silent chunks = 240 ms < 250 ms hangover → still speaking.
        for _ in 0..24 {
            assert!(gate.update(0.1, 10), "still within hangover");
        }
        assert!(!gate.update(0.1, 10), "hangover expired at 250 ms");
    }

    #[test]
    fn voice_resets_hangover() {
        let mut gate = SpeakingGate::default();
        assert!(gate.update(0.7, 10));
        for _ in 0..20 {
            assert!(gate.update(0.1, 10));
        }
        gate.update(0.8, 10); // voice again → hangover resets
        for _ in 0..24 {
            assert!(gate.update(0.1, 10));
        }
        assert!(!gate.update(0.1, 10));
    }
}
