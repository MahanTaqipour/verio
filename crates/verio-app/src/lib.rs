//! Verio application core.
//!
//! Phase 0: minimal shared application state + app metadata, used to prove the
//! workspace wiring (`ui/src-tauri` depends on this crate). The room state machine
//! and the wiring between verio-audio / verio-dsp / verio-transport / verio-discovery land in
//! later phases.
//!
//! Architecture rule: audio NEVER crosses the Tauri IPC boundary. The frontend is a
//! control panel only — Tauri commands in, Tauri events out.

pub mod hotkeys;
pub mod pipeline;
pub mod room;
pub mod settings;

pub use verio_dsp::earcon::EarconKind;
pub use verio_transport::TransportDiagnostics;

use serde::Serialize;

pub const APP_NAME: &str = "Verio";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shared application state managed by the Tauri builder.
///
/// Phase 1+ will hold settings, the room state machine, and handles to the
/// audio/DSP/transport tasks.
#[derive(Debug, Default)]
pub struct AppState {
    // Phase 1+: settings store, room machine, task handles.
    _private: (),
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Static app metadata surfaced to the frontend for the round-trip proof.
#[derive(Debug, Serialize, Clone)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[must_use]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: APP_NAME,
        version: APP_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_correctly() {
        let info = app_info();
        assert_eq!(info.name, "Verio");
        assert!(!info.version.is_empty());
    }
}
