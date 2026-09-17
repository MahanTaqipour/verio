//! AEC stage slot.
//!
//! Phase 1 does NOT build an echo canceller. This module only fixes the trait
//! boundary where a future AEC (e.g. webrtc-audio-processing) will be inserted
//! between capture and input gain, so the processing path never has to change.

/// One stage in the capture path, operating in place on 48 kHz mono f32 audio.
pub trait AecStage: Send {
    fn process(&mut self, frame: &mut [f32]);
}

/// Placeholder that does nothing. The pipeline always holds one, so a real AEC
/// can later be swapped in without touching the pipeline code.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullAec;

impl AecStage for NullAec {
    fn process(&mut self, _frame: &mut [f32]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_aec_leaves_frame_untouched() {
        let mut frame = vec![0.25_f32; 480];
        NullAec.process(&mut frame);
        assert!(frame.iter().all(|s| *s == 0.25));
    }
}
