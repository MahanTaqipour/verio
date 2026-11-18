//! Settings: the `Settings` shape, and the store that owns it.
//!
//! Session 28 moved persistence into the local database ([`crate::db`]) at
//! `<config_dir>/verio.db`. What used to be `<config_dir>/settings.json` is now
//! **imported once and renamed** — see [`import_legacy`]. Nothing reads or writes
//! the JSON afterwards, which is the point: two stores for one setting is how a
//! setting silently stops taking effect.
//!
//! Saves are debounced (500 ms) by a background thread. An unreadable stored blob
//! falls back to the legacy file, then to defaults, with a warning — never a panic.

use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::db::Db;

/// Debounce window before a change hits the database.
const SAVE_DEBOUNCE_MS: u64 = 500;

/// The pre-Session-28 store. Read once by [`import_legacy`], then renamed to
/// `settings.json.imported` so it cannot be mistaken for the live settings.
pub const LEGACY_SETTINGS_FILE_NAME: &str = "settings.json";

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
    /// Session 29b: may another peer stop the music this user is hosting?
    ///
    /// Defaults to `true` — the plan's "default: ON" — and is stored here, in the
    /// settings blob in the database, so it survives a restart.
    ///
    /// Applied at the moment the user *becomes* host: it is read when the bot is
    /// spawned and fixed for that session's life. So toggling it changes the next
    /// bot, never the running one, which is what lets the value be announced once
    /// at join instead of broadcast as it changes.
    pub music_force_stop: bool,
    /// Last folder the music file dialog was pointed at.
    pub music_last_dir: Option<String>,
    /// Session 14: Opus bitrate for voice (kbps). 32 is the tuned default.
    pub voice_bitrate_kbps: u32,
    /// Session 14: Opus bitrate while music is playing (kbps).
    pub music_bitrate_kbps: u32,
}

// Session 28 removed `peer_id` from this struct. It was generated on every load
// and read by nothing — the wire identity has come from a per-process uuid since
// two-instance testing needed it. The persistent half of the identity now lives
// in `identity.user_id` (see `crate::db`), which is the only authority for it.

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
            signaling_port: verio_discovery::DEFAULT_SIGNALING_PORT,
            direct_recent: Vec::new(),
            vps_address: "ws://127.0.0.1:8443".to_string(),
            transport_mode: TransportModeSetting::Auto,
            host_port: 8444,
            music_volume: 1.0,
            music_loop: false,
            music_monitor: false,
            music_force_stop: true,
            music_last_dir: None,
            voice_bitrate_kbps: 48,
            music_bitrate_kbps: 128,
        }
    }
}

impl Settings {
    /// Clamp free-form fields to valid ranges after a load or update.
    pub fn sanitize(&mut self) {
        self.input_gain_db = self.input_gain_db.clamp(-12.0, 12.0);
        if self.nickname.chars().count() > 64 {
            self.nickname = self.nickname.chars().take(64).collect();
        }
        if self.signaling_port == 0 {
            self.signaling_port = verio_discovery::DEFAULT_SIGNALING_PORT;
        }
        if self.host_port == 0 {
            self.host_port = 8444;
        }
        self.direct_recent.truncate(5);
        for addr in &mut self.direct_recent {
            if addr.chars().count() > 128 {
                *addr = addr.chars().take(128).collect();
            }
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
    db: Db,
    inner: Arc<Mutex<Settings>>,
    /// Notify channel to the persister thread.
    notify: Arc<mpsc::Sender<()>>,
}

impl SettingsStore {
    /// Load settings, importing the legacy `settings.json` from `legacy_dir` the
    /// first time. The persister thread is started either way.
    #[must_use]
    pub fn load(db: Db, legacy_dir: &Path) -> Self {
        let mut settings = import_legacy(&db, legacy_dir);
        let mut migrated = false;
        if settings.input_gain_db > 6.0 {
            tracing::warn!(
                old = settings.input_gain_db,
                "migrated legacy high input_gain_db to 0.0 dB unity"
            );
            settings.input_gain_db = 0.0;
            migrated = true;
        }
        if settings.voice_bitrate_kbps == 32 {
            tracing::info!(
                old = settings.voice_bitrate_kbps,
                "migrated legacy voice_bitrate_kbps from 32 to 48 kbps"
            );
            settings.voice_bitrate_kbps = 48;
            migrated = true;
        }
        if settings.music_bitrate_kbps == 96 {
            tracing::info!(
                old = settings.music_bitrate_kbps,
                "migrated legacy music_bitrate_kbps from 96 to 128 kbps"
            );
            settings.music_bitrate_kbps = 128;
            migrated = true;
        }
        settings.sanitize();
        if migrated {
            if let Err(e) = save_settings(&db, &settings) {
                tracing::warn!("failed to persist migrated settings: {e}");
            }
        }
        tracing::info!(settings = ?settings, "settings loaded");
        let inner = Arc::new(Mutex::new(settings));

        let (tx, rx) = mpsc::channel::<()>();
        let persist_db = db.clone();
        let persist_inner = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("settings-persister".into())
            .spawn(move || persister_loop(persist_db, persist_inner, rx))
            .expect("spawn settings persister");

        Self {
            db,
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
        if let Err(e) = save_settings(&self.db, &snapshot) {
            tracing::error!("failed to save settings: {e}");
        }
    }
}

/// What the legacy file held, if it was there at all.
struct LegacyFile {
    settings: Settings,
    /// The old `peer_id`, which was the persistent identity before Session 28.
    peer_id: Option<String>,
    /// The file existed but could not be read or parsed.
    corrupt: bool,
}

/// Read `settings.json` if present. `None` means "no legacy file", which is the
/// normal case for a fresh install and for every launch after the first.
fn read_legacy_settings(path: &Path) -> Option<LegacyFile> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(file = %path.display(), error = %e, "legacy settings.json is unreadable");
            return Some(LegacyFile {
                settings: Settings::default(),
                peer_id: None,
                corrupt: true,
            });
        }
    };
    // Read `peer_id` from the raw value, not from `Settings`: the field is gone
    // from the struct, but the value is still the user's persistent identity.
    let peer_id = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("peer_id").and_then(|p| p.as_str()).map(str::to_string));
    match serde_json::from_str::<Settings>(&text) {
        Ok(mut settings) => {
            settings.sanitize();
            Some(LegacyFile {
                settings,
                peer_id,
                corrupt: false,
            })
        }
        Err(e) => {
            tracing::debug!(error = %e, "legacy settings.json parse error detail");
            Some(LegacyFile {
                settings: Settings::default(),
                peer_id,
                corrupt: true,
            })
        }
    }
}

/// Import the legacy `settings.json` into the database, exactly once.
///
/// Returns the settings to run with. The database wins whenever it already holds
/// a blob, so after the first launch this only reads the database. A corrupt
/// legacy file is **left on disk untouched** — the app runs on defaults, and the
/// file stays for the user to inspect rather than being silently destroyed.
fn import_legacy(db: &Db, legacy_dir: &Path) -> Settings {
    let legacy_path = legacy_dir.join(LEGACY_SETTINGS_FILE_NAME);
    let legacy = read_legacy_settings(&legacy_path);

    // Identity first: the old `peer_id` is the user's persistent id, so adopt it
    // before anything can generate a replacement. `adopt_user_id` refuses to
    // overwrite an identity that already exists.
    if let Some(peer_id) = legacy.as_ref().and_then(|f| f.peer_id.as_deref()) {
        match db.adopt_user_id(peer_id) {
            Ok(true) => tracing::info!("persistent identity carried over from settings.json"),
            Ok(false) => {}
            Err(e) => tracing::warn!("could not adopt the legacy peer_id: {e}"),
        }
    }

    // The database is authoritative once it holds a blob.
    match db.load_settings_json() {
        Ok(Some(json)) => match serde_json::from_str::<Settings>(&json) {
            Ok(mut stored) => {
                stored.sanitize();
                return stored;
            }
            // Unreadable blob: fall through and re-import rather than run on
            // defaults while pretending the stored values are still in play.
            Err(e) => {
                tracing::warn!(error = %e, "stored settings are unreadable; re-importing");
            }
        },
        Ok(None) => {}
        Err(e) => tracing::warn!("could not read settings from the database: {e}"),
    }

    let (mut settings, imported) = match legacy {
        Some(found) => (found.settings, !found.corrupt),
        None => (Settings::default(), false),
    };
    settings.sanitize();

    if let Err(e) = save_settings(db, &settings) {
        tracing::error!("could not write settings to the database: {e}");
    }

    if imported {
        // The file is no longer read. Rename it so nobody edits a file that has
        // stopped doing anything. The database already holds the values, so a
        // failure here is cosmetic.
        let done = legacy_path.with_extension("json.imported");
        match std::fs::rename(&legacy_path, &done) {
            Ok(()) => tracing::info!(
                to = %done.display(),
                "legacy settings.json imported into verio.db and renamed"
            ),
            Err(e) => tracing::warn!("imported settings.json but could not rename it: {e}"),
        }
    } else if legacy_path.exists() {
        tracing::warn!(
            file = %legacy_path.display(),
            "legacy settings.json could not be parsed; running on defaults and leaving the file in place"
        );
    }

    settings
}

fn save_settings(db: &Db, settings: &Settings) -> Result<(), String> {
    let json = serde_json::to_string(settings).map_err(|e| format!("serialize settings: {e}"))?;
    db.save_settings_json(&json)
}

fn persister_loop(db: Db, inner: Arc<Mutex<Settings>>, rx: mpsc::Receiver<()>) {
    while rx.recv().is_ok() {
        // Debounce window: coalesce any notifications arriving while we wait.
        std::thread::sleep(std::time::Duration::from_millis(SAVE_DEBOUNCE_MS));
        while rx.try_recv().is_ok() {}
        let snapshot = inner.lock().expect("settings lock").clone();
        match save_settings(&db, &snapshot) {
            Ok(()) => tracing::debug!("settings saved"),
            Err(e) => tracing::error!("failed to save settings: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A private directory per test — SQLite will not share a file between cases.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("verio-settings-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn open(dir: &Path) -> Db {
        Db::open(dir).expect("open db")
    }

    #[test]
    fn sanitize_clamps_gain() {
        let mut s = Settings {
            input_gain_db: 55.0,
            ..Settings::default()
        };
        s.sanitize();
        assert!((s.input_gain_db - 12.0).abs() < 1e-6);

        let mut s2 = Settings {
            input_gain_db: -55.0,
            ..Settings::default()
        };
        s2.sanitize();
        assert!((s2.input_gain_db - (-12.0)).abs() < 1e-6);
    }

    #[test]
    fn legacy_high_input_gain_and_bitrates_are_migrated_on_load() {
        let dir = temp_dir("migration");
        let legacy = Settings {
            nickname: "Mahan".into(),
            input_gain_db: 15.0,
            voice_bitrate_kbps: 32,
            music_bitrate_kbps: 96,
            ..Settings::default()
        };
        std::fs::write(
            dir.join(LEGACY_SETTINGS_FILE_NAME),
            serde_json::to_string(&legacy).expect("serialize"),
        )
        .expect("write legacy");

        let store = SettingsStore::load(open(&dir), &dir);
        assert_eq!(store.get().input_gain_db, 0.0);
        assert_eq!(store.get().voice_bitrate_kbps, 48);
        assert_eq!(store.get().music_bitrate_kbps, 128);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serde_defaults_fill_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"nickname":"x"}"#).expect("parse");
        assert_eq!(s.nickname, "x");
        assert_eq!(s.hotkeys.mute, "Ctrl+Shift+M");
        assert!(s.noise_suppression);
        assert!(!s.loopback);
    }

    // -- Session 29b Force Stop ----------------------------------------------

    /// The plan's "default: ON" is a requirement, not a comment. A default of
    /// `false` would ship a feature that is invisible until somebody finds it, and
    /// would make a listener's Stop button dead on first use.
    #[test]
    fn force_stop_defaults_to_on() {
        assert!(
            Settings::default().music_force_stop,
            "Force Stop has to default to on"
        );
    }

    /// And it stays ON for a blob written before the field existed. This leans on the
    /// container-level `#[serde(default)]` filling a missing field from
    /// `Settings::default()`, which is worth pinning: a `bool` that quietly defaulted
    /// to `false` would disable the feature for every existing user without a word.
    #[test]
    fn a_stored_blob_without_force_stop_gets_it_on() {
        let s: Settings =
            serde_json::from_str(r#"{"nickname":"x","music_volume":0.5}"#).expect("parse");
        assert_eq!(s.nickname, "x");
        assert!(
            s.music_force_stop,
            "an older settings blob must not silently disable Force Stop"
        );
    }

    /// A setting that did not survive a restart would reset the moment the app
    /// closed, which is indistinguishable from a toggle that does not work.
    #[test]
    fn force_stop_survives_a_restart() {
        let dir = temp_dir("force-stop");
        {
            let store = SettingsStore::load(open(&dir), &dir);
            assert!(store.get().music_force_stop, "defaults on");
            store.update(|s| s.music_force_stop = false);
            store.flush();
        }
        let reloaded = SettingsStore::load(open(&dir), &dir);
        assert!(
            !reloaded.get().music_force_stop,
            "the toggle did not survive a restart"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_install_needs_no_json_file() {
        let dir = temp_dir("fresh");
        let store = SettingsStore::load(open(&dir), &dir);
        assert_eq!(store.get().nickname, "");
        // The database is the only sink: nothing wrote a JSON file.
        assert!(
            !dir.join(LEGACY_SETTINGS_FILE_NAME).exists(),
            "loading a fresh install created a settings.json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_settings_json_is_imported_and_then_renamed() {
        let dir = temp_dir("import");
        let legacy = Settings {
            nickname: "Mahan".into(),
            input_gain_db: 4.5,
            ..Settings::default()
        };
        std::fs::write(
            dir.join(LEGACY_SETTINGS_FILE_NAME),
            serde_json::to_string(&legacy).expect("serialize"),
        )
        .expect("write legacy");

        let store = SettingsStore::load(open(&dir), &dir);
        assert_eq!(store.get().nickname, "Mahan");
        assert!((store.get().input_gain_db - 4.5).abs() < 1e-6);

        // The JSON is no longer the store, and must not look like it still is.
        assert!(!dir.join(LEGACY_SETTINGS_FILE_NAME).exists());
        assert!(dir.join("settings.json.imported").exists());
        // And the value really reached the database.
        let db = open(&dir);
        let blob = db.load_settings_json().expect("read").expect("a blob");
        let stored: Settings = serde_json::from_str(&blob).expect("parse");
        assert_eq!(stored.nickname, "Mahan");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_database_wins_over_a_reintroduced_legacy_file() {
        let dir = temp_dir("db-wins");
        std::fs::write(
            dir.join(LEGACY_SETTINGS_FILE_NAME),
            serde_json::to_string(&Settings {
                nickname: "from-file".into(),
                ..Settings::default()
            })
            .expect("serialize"),
        )
        .expect("write legacy");

        {
            let store = SettingsStore::load(open(&dir), &dir);
            assert_eq!(store.get().nickname, "from-file");
            store.update(|s| s.nickname = "from-db".into());
            store.flush();
        }

        // Someone drops a fresh settings.json back in. It must not take over.
        std::fs::write(
            dir.join(LEGACY_SETTINGS_FILE_NAME),
            serde_json::to_string(&Settings {
                nickname: "stale".into(),
                ..Settings::default()
            })
            .expect("serialize"),
        )
        .expect("write legacy again");

        let store = SettingsStore::load(open(&dir), &dir);
        assert_eq!(
            store.get().nickname,
            "from-db",
            "a stale settings.json overrode the database"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_legacy_file_is_left_on_disk_and_does_not_break_the_load() {
        let dir = temp_dir("corrupt");
        let path = dir.join(LEGACY_SETTINGS_FILE_NAME);
        std::fs::write(&path, "{ not json !!!").expect("write junk");

        let store = SettingsStore::load(open(&dir), &dir);
        assert_eq!(store.get(), Settings::default());
        // Not renamed: the user's file is preserved for inspection.
        assert!(path.exists(), "the corrupt file was renamed or deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_legacy_peer_id_becomes_the_persistent_user_id() {
        let dir = temp_dir("peer-id");
        // `peer_id` is no longer part of `Settings`, so build the JSON by hand —
        // exactly the shape the old build wrote.
        std::fs::write(
            dir.join(LEGACY_SETTINGS_FILE_NAME),
            r#"{"nickname":"Mahan","peer_id":"6cd0d574-0ac9-45db-8837-d096800f0226"}"#,
        )
        .expect("write legacy");

        let db = open(&dir);
        let _store = SettingsStore::load(db.clone(), &dir);
        assert_eq!(
            db.user_id().expect("user id"),
            "6cd0d574-0ac9-45db-8837-d096800f0226",
            "the persistent identity was replaced instead of carried over"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_nickname_survives_a_restart() {
        let dir = temp_dir("restart");
        {
            let store = SettingsStore::load(open(&dir), &dir);
            store.update(|s| s.nickname = "Mahan".into());
            store.flush();
        }
        // Reload exactly as the app does at launch.
        let reloaded = SettingsStore::load(open(&dir), &dir);
        assert_eq!(reloaded.get().nickname, "Mahan");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_stored_blob_falls_back_instead_of_failing() {
        let dir = temp_dir("bad-blob");
        let db = open(&dir);
        db.save_settings_json("{ this is not a Settings }")
            .expect("plant a bad blob");

        let store = SettingsStore::load(db, &dir);
        assert_eq!(store.get(), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_truncates_utf8_on_char_boundaries_and_keeps_ctrl_shift_space() {
        let mut s = Settings {
            nickname: "😀".repeat(80),
            direct_recent: vec!["🌐".repeat(150)],
            hotkeys: HotkeySettings {
                ptt: "Ctrl+Shift+Space".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        s.sanitize();

        assert_eq!(s.nickname.chars().count(), 64);
        assert_eq!(s.direct_recent[0].chars().count(), 128);
        assert_eq!(s.hotkeys.ptt, "Ctrl+Shift+Space");
    }
}
