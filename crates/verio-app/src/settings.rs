//! Settings persistence: JSON at `<app_config_dir>/verio/settings.json`.
//!
//! Loaded once at launch; saved debounced (500 ms) on change by a background
//! persister thread. A corrupt/unreadable file falls back to defaults with a
//! warning — never panics.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Debounce window before a change hits disk.
const SAVE_DEBOUNCE_MS: u64 = 500;
pub const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct HotkeySettings {
    pub mute: String,
    pub deafen: String,
    pub ptt: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            mute: "Ctrl+Shift+M".to_string(),
            deafen: "Ctrl+Shift+D".to_string(),
            ptt: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransportModeSetting {
    #[default]
    Auto,
    CloudRelay,
    PeerHost,
    DirectP2P,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Settings {
    pub nickname: String,
    /// `None` = system default device.
    pub input_device: Option<String>,
    /// `None` = system default device.
    pub output_device: Option<String>,
    /// Input gain in dB, range 0..+30.
    pub input_gain_db: f32,
    pub noise_suppression: bool,
    pub loopback: bool,
    pub debug_wavs: bool,
    pub hotkeys: HotkeySettings,
    /// Persistent peer identity (uuid v4) — one per profile. Generated on first
    /// load; the LARGER uuid becomes the WebRTC offerer (deterministic role).
    pub peer_id: String,
    /// TCP port for direct-connect signaling. Default 49860.
    pub signaling_port: u16,
    /// Last 5 direct-connect addresses (most recent first).
    pub direct_recent: Vec<String>,
    /// VPS signaling server address (ws:// or wss://). Default ws://127.0.0.1:8443.
    pub vps_address: String,
    /// Preferred voice transport mode.
    pub transport_mode: TransportModeSetting,
    /// Local UDP port for Mode 2 ("Host on My PC"). Default 8444.
    pub host_port: u16,
    /// Session 13: music player. Play state and position are deliberately NOT
    /// persisted - music always starts idle after a restart.
    pub music_volume: f32,
    pub music_loop: bool,
    pub music_monitor: bool,
    /// Last folder the music file dialog was pointed at.
    pub music_last_dir: Option<String>,
    /// Session 14: Opus bitrate for voice (kbps). 32 is the tuned default.
    pub voice_bitrate_kbps: u32,
    /// Session 14: Opus bitrate while music is playing (kbps).
    pub music_bitrate_kbps: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            nickname: String::new(),
            input_device: None,
            output_device: None,
            input_gain_db: 0.0,
            noise_suppression: true,
            loopback: false,
            debug_wavs: false,
            hotkeys: HotkeySettings::default(),
            peer_id: String::new(),
            signaling_port: verio_discovery::DEFAULT_SIGNALING_PORT,
            direct_recent: Vec::new(),
            vps_address: "ws://127.0.0.1:8443".to_string(),
            transport_mode: TransportModeSetting::Auto,
            host_port: 8444,
            music_volume: 1.0,
            music_loop: false,
            music_monitor: false,
            music_last_dir: None,
            voice_bitrate_kbps: 32,
            music_bitrate_kbps: 96,
        }
    }
}

impl Settings {
    /// Clamp free-form fields to valid ranges after a load or update.
    pub fn sanitize(&mut self) {
        self.input_gain_db = self.input_gain_db.clamp(0.0, 30.0);
        if self.nickname.len() > 64 {
            self.nickname.truncate(64);
        }
        if self.signaling_port == 0 {
            self.signaling_port = verio_discovery::DEFAULT_SIGNALING_PORT;
        }
        if self.host_port == 0 {
            self.host_port = 8444;
        }
        self.direct_recent.truncate(5);
        for addr in &mut self.direct_recent {
            if addr.len() > 128 {
                addr.truncate(128);
            }
        }
        if self.hotkeys.ptt == "Ctrl+Shift+Space" {
            self.hotkeys.ptt.clear();
        }
    }

    /// Generate the persistent peer uuid (uuid v4) if missing. Idempotent.
    pub fn ensure_peer_id(&mut self) {
        if self.peer_id.is_empty() {
            self.peer_id = uuid::Uuid::new_v4().to_string();
            tracing::info!(peer_id = %self.peer_id, "generated persistent peer uuid");
        }
    }

    /// Record a direct-connect address as most-recently-used (dedup, max 5).
    pub fn remember_direct_addr(&mut self, addr: &str) {
        let addr = addr.trim();
        if addr.is_empty() {
            return;
        }
        self.direct_recent.retain(|a| a != addr);
        self.direct_recent.insert(0, addr.to_string());
        self.direct_recent.truncate(5);
    }
}

/// Shared, thread-safe settings handle with debounced persistence.
#[derive(Clone)]
pub struct SettingsStore {
    dir: PathBuf,
    inner: Arc<Mutex<Settings>>,
    /// Notify channel to the persister thread.
    notify: Arc<mpsc::Sender<()>>,
}

impl SettingsStore {
    /// Load settings from `dir/settings.json`. A missing file → defaults (no
    /// warning). A corrupt/unreadable file → defaults + warning. The persister
    /// thread is started either way.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        let file = dir.join(SETTINGS_FILE_NAME);
        let (mut settings, corrupt) = read_settings(&file);
        // Phase 2: every load produces a valid, stable peer uuid per profile and
        // clamps the new network fields.
        settings.ensure_peer_id();
        settings.sanitize();
        if corrupt {
            tracing::warn!(
                file = %file.display(),
                "settings file unreadable/corrupt — falling back to defaults (overwritten on next change)"
            );
        } else {
            tracing::info!(file = %file.display(), "settings loaded");
        }
        let inner = Arc::new(Mutex::new(settings));

        let (tx, rx) = mpsc::channel::<()>();
        let persist_dir = dir.to_path_buf();
        let persist_inner = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("settings-persister".into())
            .spawn(move || persister_loop(persist_dir, persist_inner, rx))
            .expect("spawn settings persister");

        Self {
            dir: dir.to_path_buf(),
            inner,
            notify: Arc::new(tx),
        }
    }

    /// Snapshot of the current settings.
    #[must_use]
    pub fn get(&self) -> Settings {
        self.inner.lock().expect("settings lock").clone()
    }

    /// Apply `mutate` to the settings, then trigger a debounced save.
    pub fn update(&self, mutate: impl FnOnce(&mut Settings)) {
        {
            let mut guard = self.inner.lock().expect("settings lock");
            mutate(&mut guard);
            guard.sanitize();
        }
        tracing::info!(settings = ?self.get(), "settings changed");
        // A failed notify means the persister died; try to save inline.
        if self.notify.send(()).is_err() {
            tracing::warn!("settings persister gone — saving synchronously");
            self.flush();
        }
    }

    /// Save immediately (used on exit / when the persister is unavailable).
    pub fn flush(&self) {
        let snapshot = self.inner.lock().expect("settings lock").clone();
        if let Err(e) = write_settings(&self.dir.join(SETTINGS_FILE_NAME), &snapshot) {
            tracing::error!("failed to save settings: {e}");
        }
    }
}

/// Returns `(settings, corrupt)` — corrupt=true means the file existed but
/// could not be read/parsed.
fn read_settings(path: &Path) -> (Settings, bool) {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.sanitize();
                (s, false)
            }
            Err(e) => {
                tracing::debug!(error = %e, "settings parse error detail");
                (Settings::default(), true)
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), false),
        Err(_) => (Settings::default(), true),
    }
}

fn write_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| format!("serialize: {e}"))?;
    // Write-then-rename so an interrupted write never corrupts the file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {e}", path.display()))
}

fn persister_loop(dir: PathBuf, inner: Arc<Mutex<Settings>>, rx: mpsc::Receiver<()>) {
    while rx.recv().is_ok() {
        // Debounce window: coalesce any notifications arriving while we wait.
        std::thread::sleep(std::time::Duration::from_millis(SAVE_DEBOUNCE_MS));
        while rx.try_recv().is_ok() {}
        let snapshot = inner.lock().expect("settings lock").clone();
        match write_settings(&dir.join(SETTINGS_FILE_NAME), &snapshot) {
            Ok(()) => tracing::debug!("settings saved"),
            Err(e) => tracing::error!("failed to save settings: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_via_json() {
        let dir = std::env::temp_dir().join("verio-settings-test-rt");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(SETTINGS_FILE_NAME);
        let mut s = Settings { nickname: "Mahan".into(), input_gain_db: 12.5, ..Settings::default() };
        s.hotkeys.mute = "Ctrl+Alt+Z".into();
        write_settings(&path, &s).expect("write");
        let (loaded, corrupt) = read_settings(&path);
        assert!(!corrupt);
        assert_eq!(loaded, s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join("verio-settings-test-corrupt");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(SETTINGS_FILE_NAME);
        std::fs::write(&path, "{ not json !!!").expect("write junk");
        let (s, corrupt) = read_settings(&path);
        assert!(corrupt);
        assert_eq!(s, Settings::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_not_corrupt() {
        let (s, corrupt) = read_settings(Path::new("Z:/definitely/not/here/settings.json"));
        assert!(!corrupt);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn sanitize_clamps_gain() {
        let mut s = Settings { input_gain_db: 55.0, ..Settings::default() };
        s.sanitize();
        assert!((s.input_gain_db - 30.0).abs() < 1e-6);
    }

    #[test]
    fn serde_defaults_fill_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"nickname":"x"}"#).expect("parse");
        assert_eq!(s.nickname, "x");
        assert_eq!(s.hotkeys.mute, "Ctrl+Shift+M");
        assert!(s.noise_suppression);
        assert!(!s.loopback);
    }

    #[test]
    fn store_roundtrip_keeps_nonempty_nickname() {
        let dir = std::env::temp_dir().join("verio-settings-test-nickname");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        {
            let store = SettingsStore::load(&dir);
            store.update(|s| s.nickname = "Mahan".into());
            store.flush();
        }
        // Reload exactly as the app does at launch.
        let reloaded = SettingsStore::load(&dir);
        assert_eq!(reloaded.get().nickname, "Mahan");
        // And the on-disk JSON really contains it.
        let (from_disk, corrupt) = read_settings(&dir.join(SETTINGS_FILE_NAME));
        assert!(!corrupt);
        assert_eq!(from_disk.nickname, "Mahan");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

