//! Hotkey-driven transmit state machine.
//!
//! Semantics (Phase 1 spec):
//! - `deafen = true` forces `muted = true`; undeafen restores the pre-deafen
//!   mute state.
//! - PTT opens the mic ONLY while held AND not muted.
//! - Effective transmission = `!muted && (!ptt_mode || ptt_held)`.
//!
//! This struct is lock-free (atomics only) so the global-shortcut handler and
//! the audio processing thread can share it without ever blocking a hot path.

use std::sync::atomic::{AtomicBool, Ordering};

/// A hotkey binding as sent to / received from the frontend (Tauri
/// global-shortcut format, e.g. `"Ctrl+Shift+M"`).
pub type Binding = String;

#[derive(Debug, Default)]
pub struct TxState {
    muted: AtomicBool,
    deafened: AtomicBool,
    ptt_held: AtomicBool,
    /// Push-to-talk mode: when true, the mic opens only while the PTT key is
    /// held. Driven by whether a PTT binding is configured (live-settable).
    ptt_mode: AtomicBool,
    /// Mute state to restore when deafen is turned off.
    pre_deafen_muted: AtomicBool,
}

/// Which hotkey action a binding maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Mute,
    Deafen,
    Ptt,
}

impl TxState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle mute; returns the new muted state. Muting while deafened keeps
    /// deafened=true (deafen already implies mute).
    pub fn toggle_mute(&self) -> bool {
        let new_muted = !self.muted.load(Ordering::Relaxed);
        self.set_muted(new_muted);
        new_muted
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Toggle deafen. Turning deafen ON forces mute and remembers the previous
    /// mute state; turning it OFF restores that state. Returns
    /// `(deafened, muted)` after the toggle.
    pub fn toggle_deafen(&self) -> (bool, bool) {
        if self.deafened.load(Ordering::Relaxed) {
            self.deafened.store(false, Ordering::Relaxed);
            self.muted
                .store(self.pre_deafen_muted.load(Ordering::Relaxed), Ordering::Relaxed);
        } else {
            self.pre_deafen_muted
                .store(self.muted.load(Ordering::Relaxed), Ordering::Relaxed);
            self.deafened.store(true, Ordering::Relaxed);
            self.muted.store(true, Ordering::Relaxed);
        }
        (self.is_deafened(), self.is_muted())
    }

    pub fn set_deafened(&self, deafened: bool) {
        let was = self.deafened.load(Ordering::Relaxed);
        if deafened == was {
            return;
        }
        self.toggle_deafen();
    }

    pub fn set_ptt_held(&self, held: bool) {
        self.ptt_held.store(held, Ordering::Relaxed);
    }

    /// Push-to-talk mode toggle (true = mic opens only while PTT is held).
    pub fn set_ptt_mode(&self, on: bool) {
        self.ptt_mode.store(on, Ordering::Relaxed);
        if !on {
            // Leaving PTT mode must not leave a stuck "held" flag.
            self.ptt_held.store(false, Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn is_ptt_mode(&self) -> bool {
        self.ptt_mode.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_deafened(&self) -> bool {
        self.deafened.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_ptt_held(&self) -> bool {
        self.ptt_held.load(Ordering::Relaxed)
    }

    /// Whether the mic may currently transmit.
    /// Effective = `!muted && (!ptt_mode || ptt_held)`.
    #[must_use]
    pub fn effective_transmission(&self) -> bool {
        !self.is_muted() && (!self.is_ptt_mode() || self.is_ptt_held())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_toggle_roundtrip() {
        let st = TxState::new();
        assert!(!st.is_muted());
        assert!(st.toggle_mute());
        assert!(!st.toggle_mute());
    }

    #[test]
    fn deafen_forces_mute_and_restores() {
        let st = TxState::new();
        // Not muted before → undeafen restores transmitting.
        let (deaf, muted) = st.toggle_deafen();
        assert!(deaf && muted);
        assert!(!st.effective_transmission());
        let (deaf, muted) = st.toggle_deafen();
        assert!(!deaf && !muted);
        assert!(st.effective_transmission());
    }

    #[test]
    fn undeafen_restores_pre_deafen_mute() {
        let st = TxState::new();
        st.set_muted(true); // user muted themselves first
        st.toggle_deafen(); // deafen on
        st.toggle_deafen(); // deafen off → still muted (pre-deafen state)
        assert!(st.is_muted());
        assert!(!st.effective_transmission());
    }

    #[test]
    fn ptt_only_transmits_while_held_and_unmuted() {
        let st = TxState::new();
        // PTT mode on, not held → no transmission.
        st.set_ptt_mode(true);
        assert!(!st.effective_transmission());
        st.set_ptt_held(true);
        assert!(st.effective_transmission());
        // Held but muted → still no transmission.
        st.set_muted(true);
        assert!(!st.effective_transmission());
        st.set_muted(false);
        assert!(st.effective_transmission());
        // Released → off again.
        st.set_ptt_held(false);
        assert!(!st.effective_transmission());
        // Voice-activated mode (PTT off) ignores held state.
        st.set_ptt_mode(false);
        assert!(st.effective_transmission());
        assert!(!st.is_ptt_held()); // leaving PTT mode clears held
    }
}
