//! Local store: SQLite at `<config_dir>/verio.db`.
//!
//! Session 28. Two jobs.
//!
//! **A real place to keep things.** Until now the only local persistence was
//! `settings.json`, so anything new — a room-wide mute list, per-peer volumes —
//! had to be smuggled into the settings blob or thrown away on exit.
//!
//! **Two-tier identity.** `Settings::peer_id` was generated on every load and
//! *never read by anything*: the wire identity has come from a per-process
//! `Uuid::new_v4()` since two-instance testing needed it (see PROGRESS.md
//! Session 28). Both halves are now explicit and named:
//!
//! - [`Db::user_id`] — persistent, one per profile, survives restarts.
//! - [`Db::session_id`] — minted per process and **deliberately never stored**.
//!
//! The two must not be conflated: the persistent id is what another peer would
//! mute, and the session id is what must differ between two instances on one PC.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OpenFlags};
use uuid::Uuid;

pub const DB_FILE_NAME: &str = "verio.db";

/// Ordered and append-only. The index in this array, plus one, is the schema
/// version that entry brings the database to.
///
/// **Never edit an entry that has shipped** — an existing database will not
/// re-run it, so the edit would apply to fresh installs only and the two would
/// silently diverge. Add a new entry instead.
///
/// `mute_list` and `volume_prefs` are created here but not yet written: they are
/// the approved v1 schema that Session 29 (room-wide mute) and its playlist work
/// fill in. An empty table is not a feature; it is a shape agreed in advance.
pub const MIGRATIONS: &[&str] = &[
    // v1 — initial schema.
    r"
    CREATE TABLE schema_version (
        version    INTEGER PRIMARY KEY,
        applied_at INTEGER NOT NULL
    );

    CREATE TABLE identity (
        id         INTEGER PRIMARY KEY CHECK (id = 1),
        user_id    TEXT    NOT NULL,
        created_at INTEGER NOT NULL
    );

    CREATE TABLE settings (
        id         INTEGER PRIMARY KEY CHECK (id = 1),
        data       TEXT    NOT NULL,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE mute_list (
        user_id    TEXT    PRIMARY KEY,
        muted      INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE volume_prefs (
        peer_id    TEXT    PRIMARY KEY,
        volume     REAL    NOT NULL,
        updated_at INTEGER NOT NULL
    );
    ",
];

/// The tables v1 is required to have. Asserted by a test, so a migration that
/// forgets one fails there rather than at first use.
#[cfg(test)]
const REQUIRED_TABLES: [&str; 5] = [
    "schema_version",
    "identity",
    "settings",
    "mute_list",
    "volume_prefs",
];

/// Handle to the local store. Cloneable; all clones share one connection.
#[derive(Clone)]
pub struct Db {
    path: PathBuf,
    conn: Arc<Mutex<Connection>>,
    /// Fresh per process, never persisted. See the module docs.
    session_id: Uuid,
}

impl Db {
    /// Open (creating if absent) `<dir>/verio.db` and bring it to the latest
    /// schema version.
    ///
    /// # Errors
    /// The directory cannot be created, SQLite cannot open the file, a migration
    /// fails, or the file was written by a newer build (see [`Self::migrate`]).
    pub fn open(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("create config dir {}: {e}", dir.display()))?;
        let path = dir.join(DB_FILE_NAME);
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| format!("open {}: {e}", path.display()))?;

        // WAL: an interrupted write cannot leave a half-updated database, and a
        // reader never blocks the writer. busy_timeout so a second instance on
        // the same profile waits for the write lock instead of failing.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| format!("configure {}: {e}", path.display()))?;

        let version = Self::migrate(&conn)?;
        tracing::info!(
            db = %path.display(),
            schema_version = version,
            "local store open"
        );

        Ok(Self {
            path,
            conn: Arc::new(Mutex::new(conn)),
            session_id: Uuid::new_v4(),
        })
    }

    /// Apply every migration newer than the database's current version, each in
    /// its own transaction, and return the resulting version.
    fn migrate(conn: &Connection) -> Result<i64, String> {
        let current = schema_version(conn)?;
        let target = MIGRATIONS.len() as i64;
        if current > target {
            // Refusing is the only safe answer: the file contains tables and
            // columns this build has never heard of, and "migrating down" would
            // mean destroying them.
            return Err(format!(
                "database schema is v{current} but this build only knows v{target}; \
                 it was written by a newer Verio"
            ));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let version = (i + 1) as i64;
            if version <= current {
                continue;
            }
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin migration v{version}: {e}"))?;
            tx.execute_batch(sql)
                .map_err(|e| format!("migration v{version} failed: {e}"))?;
            tx.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                params![version, now_unix()],
            )
            .map_err(|e| format!("record migration v{version}: {e}"))?;
            tx.commit()
                .map_err(|e| format!("commit migration v{version}: {e}"))?;
            tracing::info!(version, "applied schema migration");
        }
        Ok(target)
    }

    /// Path of the database file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The session half of the identity: fresh for every process, never written
    /// to disk. Two instances on one machine must not share it.
    #[must_use]
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// The persistent half of the identity, generated on first call and stable
    /// from then on.
    ///
    /// # Errors
    /// The row cannot be read or written.
    pub fn user_id(&self) -> Result<String, String> {
        let conn = self.conn.lock().expect("db lock");
        if let Some(existing) = read_identity(&conn)? {
            return Ok(existing);
        }
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO identity (id, user_id, created_at) VALUES (1, ?1, ?2)",
            params![&id, now_unix()],
        )
        .map_err(|e| format!("create identity: {e}"))?;
        tracing::info!(user_id = %id, "generated persistent user id");
        Ok(id)
    }

    /// Seed the persistent identity from the legacy `settings.json` `peer_id`.
    ///
    /// Returns whether it was adopted. A blank candidate is ignored, and an
    /// identity that already exists is **never** overwritten — a stale file must
    /// not be able to re-key a user.
    ///
    /// # Errors
    /// The row cannot be read or written.
    pub fn adopt_user_id(&self, candidate: &str) -> Result<bool, String> {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            return Ok(false);
        }
        let conn = self.conn.lock().expect("db lock");
        if read_identity(&conn)?.is_some() {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO identity (id, user_id, created_at) VALUES (1, ?1, ?2)",
            params![candidate, now_unix()],
        )
        .map_err(|e| format!("adopt identity: {e}"))?;
        tracing::info!(
            user_id = %candidate,
            "adopted the persistent user id from the legacy settings.json"
        );
        Ok(true)
    }

    /// The stored settings blob, or `None` when the database has never held one.
    ///
    /// # Errors
    /// The row cannot be read.
    pub fn load_settings_json(&self) -> Result<Option<String>, String> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn
            .prepare("SELECT data FROM settings WHERE id = 1")
            .map_err(|e| format!("prepare settings read: {e}"))?;
        let mut rows = stmt.query([]).map_err(|e| format!("read settings: {e}"))?;
        match rows.next().map_err(|e| format!("read settings row: {e}"))? {
            Some(row) => row.get(0).map(Some).map_err(|e| format!("settings column: {e}")),
            None => Ok(None),
        }
    }

    /// Write the settings blob, replacing any previous one.
    ///
    /// # Errors
    /// The row cannot be written.
    pub fn save_settings_json(&self, json: &str) -> Result<(), String> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO settings (id, data, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
            params![json, now_unix()],
        )
        .map_err(|e| format!("save settings: {e}"))?;
        Ok(())
    }
}

fn read_identity(conn: &Connection) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare("SELECT user_id FROM identity WHERE id = 1")
        .map_err(|e| format!("prepare identity read: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| format!("read identity: {e}"))?;
    match rows.next().map_err(|e| format!("read identity row: {e}"))? {
        Some(row) => row.get(0).map(Some).map_err(|e| format!("identity column: {e}")),
        None => Ok(None),
    }
}

/// Current schema version; `0` for a database with no `schema_version` table.
fn schema_version(conn: &Connection) -> Result<i64, String> {
    let has_table: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("probe schema_version: {e}"))?;
    if has_table == 0 {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )
    .map_err(|e| format!("read schema_version: {e}"))
}

/// Unix seconds. Deliberately not a formatted timestamp: that would need a date
/// library for no benefit, since nothing displays these to a user.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private directory per test; SQLite will not share a file between cases.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("verio-db-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn table_names(db: &Db) -> Vec<String> {
        let conn = db.conn.lock().expect("db lock");
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare");
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");
        names
    }

    #[test]
    fn fresh_database_reaches_the_latest_schema_version() {
        let dir = temp_dir("fresh");
        let db = Db::open(&dir).expect("open");
        let conn = db.conn.lock().expect("db lock");
        assert_eq!(schema_version(&conn).expect("version"), MIGRATIONS.len() as i64);
        drop(conn);
        let names = table_names(&db);
        for required in REQUIRED_TABLES {
            assert!(
                names.iter().any(|n| n == required),
                "v1 schema is missing the `{required}` table; found {names:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrations_are_applied_once_not_reapplied() {
        let dir = temp_dir("once");
        {
            let db = Db::open(&dir).expect("first open");
            let conn = db.conn.lock().expect("db lock");
            let rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
                .expect("count");
            assert_eq!(rows, MIGRATIONS.len() as i64);
        }
        // Reopening must not try to CREATE TABLE again — that would error.
        let db = Db::open(&dir).expect("second open");
        let conn = db.conn.lock().expect("db lock");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            rows,
            MIGRATIONS.len() as i64,
            "reopening re-ran a migration instead of skipping it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_user_id_is_stable_when_the_database_is_reopened() {
        let dir = temp_dir("user-id");
        let first = {
            let db = Db::open(&dir).expect("open");
            db.user_id().expect("user id")
        };
        // A second open stands in for the next launch of the app.
        let second = {
            let db = Db::open(&dir).expect("reopen");
            db.user_id().expect("user id")
        };
        assert_eq!(first, second, "the persistent user id changed across restarts");
        assert!(!first.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_id_is_fresh_for_every_open() {
        let dir = temp_dir("session-id");
        let a = Db::open(&dir).expect("open").session_id();
        let b = Db::open(&dir).expect("reopen").session_id();
        assert_ne!(
            a, b,
            "two opens produced the same session id — it is being persisted somewhere"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adopting_a_legacy_user_id_never_overwrites_a_real_one() {
        let dir = temp_dir("adopt");
        let db = Db::open(&dir).expect("open");
        assert!(db.adopt_user_id("legacy-abc").expect("adopt"));
        assert_eq!(db.user_id().expect("read"), "legacy-abc");
        // A second, different candidate must lose.
        assert!(!db.adopt_user_id("legacy-xyz").expect("adopt"));
        assert_eq!(db.user_id().expect("read"), "legacy-abc");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_legacy_user_id_is_ignored() {
        let dir = temp_dir("adopt-blank");
        let db = Db::open(&dir).expect("open");
        assert!(!db.adopt_user_id("   ").expect("adopt"));
        // Falls through to a generated id rather than storing whitespace.
        let id = db.user_id().expect("read");
        assert!(!id.trim().is_empty());
        assert_ne!(id, "   ");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_blob_roundtrips_and_is_replaced() {
        let dir = temp_dir("settings-blob");
        let db = Db::open(&dir).expect("open");
        assert_eq!(db.load_settings_json().expect("read"), None);
        db.save_settings_json(r#"{"nickname":"first"}"#).expect("save");
        assert_eq!(
            db.load_settings_json().expect("read").as_deref(),
            Some(r#"{"nickname":"first"}"#)
        );
        db.save_settings_json(r#"{"nickname":"second"}"#).expect("save");
        assert_eq!(
            db.load_settings_json().expect("read").as_deref(),
            Some(r#"{"nickname":"second"}"#),
            "the second save appended a row instead of replacing the first"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_migrated_down() {
        let dir = temp_dir("newer");
        {
            let db = Db::open(&dir).expect("open");
            let conn = db.conn.lock().expect("db lock");
            conn.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (99, 0)",
                [],
            )
            .expect("plant a future version");
        }
        let err = match Db::open(&dir) {
            Ok(_) => panic!("a v99 database must not open"),
            Err(e) => e,
        };
        assert!(
            err.contains("v99") && err.contains("newer Verio"),
            "the refusal should name the version and the reason, got: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
