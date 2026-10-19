//! Persistence: `settings.json` plus a SQLite database for history and
//! favourites.
//!
//! Upstream keeps all three in `UserDefaults` — `EZConfiguration_kQueryHistory`
//! and `EZConfiguration_kFavorites` in `Defaults.Keys+Extension.swift`, mutated
//! by `QueryRecordManager.swift`; the macOS app has no Core Data store. The
//! settings document keeps the upstream key names so an export from either app
//! can be read by the other, while the two record lists — which upstream
//! rewrites in full on every insert — get real tables so paging, search and the
//! length cap do not depend on how long the list has grown.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::{QueryRecord, Settings};
use crate::model::Language;

mod protection;

const SETTINGS_FILE: &str = "settings.json";
const DATABASE_FILE: &str = "Easydict.db";
const PROTECTED_FORMAT: &str = "easydict.windows-dpapi.v1";

/// Bumped whenever `SCHEMA` changes; kept in SQLite's `user_version`.
const SCHEMA_VERSION: i64 = 1;

/// `QueryRecordManager.maxHistoryCount` — upstream trims history to the newest
/// 1000 entries after every insert. Favourites are unbounded upstream and here.
const MAX_HISTORY: usize = 1000;

/// Column list in the order [`row_to_record`] reads it.
const RECORD_COLUMNS: &str = "query_text, from_language, to_language, timestamp, translated_text";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    query_text      TEXT    NOT NULL,
    from_language   TEXT    NOT NULL,
    to_language     TEXT    NOT NULL,
    timestamp       INTEGER NOT NULL,
    translated_text TEXT
);
CREATE TABLE IF NOT EXISTS favorites (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    query_text      TEXT    NOT NULL,
    from_language   TEXT    NOT NULL,
    to_language     TEXT    NOT NULL,
    timestamp       INTEGER NOT NULL,
    translated_text TEXT
);
CREATE INDEX IF NOT EXISTS idx_history_order ON history (timestamp DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_favorites_order ON favorites (timestamp DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_favorites_text ON favorites (query_text);
";

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    InvalidSettings(&'static str),
}

pub struct Store {
    root: PathBuf,
    db: Mutex<Connection>,
}

impl Store {
    /// Creates `<root>` and the database if they do not exist.
    pub fn open(root: PathBuf) -> StoreResult<Self> {
        fs::create_dir_all(&root)?;
        let conn = Connection::open(root.join(DATABASE_FILE))?;
        // The app is single-instance, but a second process reaching the same
        // file should wait for the lock instead of failing the operation.
        conn.busy_timeout(Duration::from_secs(5))?;
        init_schema(&conn)?;
        Ok(Store {
            root,
            db: Mutex::new(conn),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join(SETTINGS_FILE)
    }

    pub fn database_path(&self) -> PathBuf {
        self.root.join(DATABASE_FILE)
    }

    // ---- Settings ---------------------------------------------------------

    /// Only a missing file yields defaults. Unreadable settings must not be
    /// replaced by first-launch housekeeping. Valid legacy JSON is migrated
    /// atomically before any credentials are used.
    pub fn load_settings(&self) -> StoreResult<Settings> {
        let text = match fs::read_to_string(self.settings_path()) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(err) => return Err(err.into()),
        };
        let document: serde_json::Value = serde_json::from_str(&text)?;
        let legacy = document.get("format").is_none();
        let settings = decode_settings(document)?;
        if legacy {
            self.save_settings(&settings)?;
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> StoreResult<()> {
        let encrypted = protection::protect(&serde_json::to_vec(settings)?)?;
        write_json_atomic(&self.settings_path(), &serde_json::json!({
            "format": PROTECTED_FORMAT,
            "data": STANDARD.encode(encrypted),
        }))
    }

    /// Overwrites the settings with the defaults and returns them.
    pub fn reset_settings(&self) -> StoreResult<Settings> {
        let settings = Settings::default();
        self.save_settings(&settings)?;
        Ok(settings)
    }

    pub fn export_settings(&self, path: &Path, settings: &Settings, include_secrets: bool) -> StoreResult<()> {
        let path = std::path::absolute(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            let root = self.root.canonicalize()?;
            if parent.canonicalize()?.starts_with(&root)
                || path.canonicalize().is_ok_and(|target| target.starts_with(&root))
            {
                return Err(StoreError::InvalidSettings(
                    "不能导出到应用数据目录，以免覆盖加密设置或数据库。请选择其他文件夹。",
                ));
            }
        }
        let mut exported = settings.clone();
        if !include_secrets {
            exported.clear_credentials();
        }
        write_json_atomic(&path, &exported)
    }

    /// Reads a settings document written by [`Store::export_settings`]. Fails
    /// rather than falling back — importing is an explicit user action, so a
    /// bad file has to be reported.
    pub fn import_settings(&self, path: &Path) -> StoreResult<Settings> {
        decode_settings(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    // ---- History ----------------------------------------------------------

    /// Appends a query to the history.
    ///
    /// Blank queries are dropped. Re-running a query — upstream's Retry —
    /// arrives here with the same text and languages as the newest entry, and
    /// updates that entry's timestamp instead of stacking a second row.
    pub fn add_history(&self, record: &QueryRecord) -> StoreResult<()> {
        if record.query_text.trim().is_empty() {
            return Ok(());
        }
        let conn = self.db.lock();

        // "Newest" has to mean what the list shows first, hence the same
        // `timestamp DESC, id DESC` ordering the readers use: two queries
        // recorded in the same millisecond still have a defined front.
        let newest = conn
            .query_row(
                "SELECT query_text, from_language, to_language, id FROM history \
                 ORDER BY timestamp DESC, id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;

        if let Some((text, from, to, id)) = newest {
            if text == record.query_text
                && from == record.from_language.raw()
                && to == record.to_language.raw()
            {
                conn.execute(
                    "UPDATE history SET timestamp = ?1 WHERE id = ?2",
                    params![record.timestamp, id],
                )?;
                trim_history(&conn)?;
                return Ok(());
            }
        }

        conn.execute(
            "INSERT INTO history (query_text, from_language, to_language, timestamp, translated_text) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.query_text,
                record.from_language.raw(),
                record.to_language.raw(),
                record.timestamp,
                record.translated_text
            ],
        )?;
        trim_history(&conn)
    }

    /// Newest first. A `limit` of 0 returns nothing.
    pub fn history(&self, limit: usize, offset: usize) -> StoreResult<Vec<QueryRecord>> {
        let conn = self.db.lock();
        query_records(
            &conn,
            &format!(
                "SELECT {RECORD_COLUMNS} FROM history \
                 ORDER BY timestamp DESC, id DESC LIMIT ?1 OFFSET ?2"
            ),
            params![limit as i64, offset as i64],
        )
    }

    /// Case-insensitive substring match on the query text, newest first.
    ///
    /// `instr(lower(..))` rather than `LIKE` so a needle containing `%` or `_`
    /// is matched literally; `lower` folds ASCII exactly like `LIKE` does. An
    /// empty needle matches every row.
    pub fn search_history(&self, needle: &str, limit: usize) -> StoreResult<Vec<QueryRecord>> {
        let conn = self.db.lock();
        query_records(
            &conn,
            &format!(
                "SELECT {RECORD_COLUMNS} FROM history \
                 WHERE instr(lower(query_text), lower(?1)) > 0 \
                 ORDER BY timestamp DESC, id DESC LIMIT ?2"
            ),
            params![needle, limit as i64],
        )
    }

    /// Deletes the row the record came from.
    ///
    /// `QueryRecord` carries no identity field, so a row is matched on the
    /// fields that identify it — text, both languages and the timestamp, which
    /// is what tells two entries with the same text apart.
    pub fn delete_history(&self, record: &QueryRecord) -> StoreResult<()> {
        let conn = self.db.lock();
        conn.execute(
            "DELETE FROM history \
             WHERE query_text = ?1 AND from_language = ?2 AND to_language = ?3 AND timestamp = ?4",
            params![
                record.query_text,
                record.from_language.raw(),
                record.to_language.raw(),
                record.timestamp
            ],
        )?;
        Ok(())
    }

    pub fn clear_history(&self) -> StoreResult<()> {
        let conn = self.db.lock();
        conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    pub fn history_count(&self) -> StoreResult<usize> {
        let conn = self.db.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    // ---- Favourites -------------------------------------------------------

    /// Adds a favourite; an existing one is left alone.
    ///
    /// Favourites are keyed on the query text alone, matching upstream's
    /// `containsRecord(queryText:in:)` check that guards the add, so the same
    /// word starred under two language pairs stays a single entry.
    pub fn add_favorite(&self, record: &QueryRecord) -> StoreResult<()> {
        let conn = self.db.lock();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM favorites WHERE query_text = ?1)",
            params![record.query_text],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO favorites (query_text, from_language, to_language, timestamp, translated_text) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.query_text,
                record.from_language.raw(),
                record.to_language.raw(),
                record.timestamp,
                record.translated_text
            ],
        )?;
        Ok(())
    }

    /// Removes the favourite with this query text; a missing one is not an error.
    pub fn remove_favorite(&self, record: &QueryRecord) -> StoreResult<()> {
        let conn = self.db.lock();
        conn.execute(
            "DELETE FROM favorites WHERE query_text = ?1",
            params![record.query_text],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn favorites(&self) -> StoreResult<Vec<QueryRecord>> {
        let conn = self.db.lock();
        query_records(
            &conn,
            &format!("SELECT {RECORD_COLUMNS} FROM favorites ORDER BY timestamp DESC, id DESC"),
            &[],
        )
    }

    pub fn is_favorite(&self, record: &QueryRecord) -> StoreResult<bool> {
        let conn = self.db.lock();
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM favorites WHERE query_text = ?1)",
            params![record.query_text],
            |row| row.get(0),
        )?)
    }
}

fn decode_settings(document: serde_json::Value) -> StoreResult<Settings> {
    if let Some(format) = document.get("format") {
        if format.as_str() != Some(PROTECTED_FORMAT) {
            return Err(StoreError::InvalidSettings("不支持的加密设置格式；文件未被修改。"));
        }
        let data = document.get("data").and_then(|value| value.as_str())
            .ok_or(StoreError::InvalidSettings("加密设置缺少数据；文件未被修改。"))?;
        let encrypted = STANDARD.decode(data)
            .map_err(|_| StoreError::InvalidSettings("加密设置数据损坏；文件未被修改。"))?;
        let plain = protection::unprotect(&encrypted)?;
        Ok(serde_json::from_slice(&plain)?)
    } else {
        Ok(serde_json::from_value(document)?)
    }
}

/// `records.prefix(maxHistoryCount)` upstream: keep the newest rows. Run after
/// either write so the cap holds even on a database that arrived over it.
fn trim_history(conn: &Connection) -> StoreResult<()> {
    conn.execute(
        "DELETE FROM history WHERE id NOT IN \
         (SELECT id FROM history ORDER BY timestamp DESC, id DESC LIMIT ?1)",
        params![MAX_HISTORY as i64],
    )?;
    Ok(())
}

fn init_schema(conn: &Connection) -> StoreResult<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < SCHEMA_VERSION {
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn query_records(
    conn: &Connection,
    sql: &str,
    bindings: &[&dyn rusqlite::ToSql],
) -> StoreResult<Vec<QueryRecord>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(bindings, row_to_record)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueryRecord> {
    let from: String = row.get(1)?;
    let to: String = row.get(2)?;
    Ok(QueryRecord {
        query_text: row.get(0)?,
        from_language: language_from_raw(&from),
        to_language: language_from_raw(&to),
        timestamp: row.get(3)?,
        translated_text: row.get(4)?,
    })
}

/// Languages are stored as their raw `EZLanguage*` value so a row stays
/// readable in any SQLite browser. A value this build does not know — a file
/// written by a newer one — reads back as `Unsupported` rather than failing
/// the whole listing.
fn language_from_raw(raw: &str) -> Language {
    Language::from_raw(raw).unwrap_or(Language::Unsupported)
}

/// Writes `value` as pretty JSON, replacing `path` in one step.
///
/// A crash mid-write would otherwise leave a truncated settings file, which the
/// next start would treat as corrupt. The temporary is a sibling of the target
/// so the rename stays inside one volume.
fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> StoreResult<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    let tmp = temp_sibling(path);
    let mut file = File::options().write(true).create_new(true).open(&tmp)?;
    let written = (|| -> StoreResult<()> {
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);

    if let Err(err) = written {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

fn temp_sibling(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{name}.{}.tmp", uuid::Uuid::new_v4()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A unique directory under the system temp dir, removed on drop.
    ///
    /// Declare it before the `Store` so the database handle is closed first —
    /// Windows refuses to delete a directory that is still open.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "easydict-store-test-{tag}-{}",
                uuid::Uuid::new_v4()
            ));
            TempDir(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let (Ok(path), Ok(temp)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize()) {
                if path.parent() == Some(temp.as_path())
                    && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("easydict-store-test-"))
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
    }

    fn record(text: &str, from: Language, to: Language, timestamp: i64) -> QueryRecord {
        QueryRecord {
            query_text: text.to_string(),
            from_language: from,
            to_language: to,
            timestamp,
            translated_text: None,
        }
    }

    fn texts(records: Vec<QueryRecord>) -> Vec<String> {
        records.into_iter().map(|r| r.query_text).collect()
    }

    fn value(settings: &Settings) -> serde_json::Value {
        serde_json::to_value(settings).unwrap()
    }

    #[test]
    fn store_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Store>();
    }

    #[test]
    fn open_creates_missing_parent_directories() {
        let dir = TempDir::new("open-nested");
        let root = dir.path().join("nested").join("data");
        assert!(!root.exists());

        let store = Store::open(root.clone()).unwrap();
        assert!(root.is_dir());
        assert_eq!(store.root(), root.as_path());
        assert_eq!(store.settings_path(), root.join("settings.json"));
        assert_eq!(store.database_path(), root.join("Easydict.db"));
        assert!(store.database_path().is_file());
    }

    #[test]
    fn reopening_the_same_root_keeps_the_data() {
        let dir = TempDir::new("reopen");
        {
            let store = Store::open(dir.path().to_path_buf()).unwrap();
            store
                .add_history(&record("apple", Language::Auto, Language::English, 1_000))
                .unwrap();
        }

        let again = Store::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(again.history_count().unwrap(), 1);
        assert_eq!(again.history(10, 0).unwrap()[0].query_text, "apple");
    }

    #[test]
    fn settings_round_trip() {
        let dir = TempDir::new("settings-round-trip");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        let mut settings = Settings::default();
        settings.query_from_language = Language::English;
        settings.query_to_language = Language::SimplifiedChinese;
        settings.font_size_option_index = 2;
        settings.auto_play_audio = true;
        settings.bing_cookie = "cookie=1".to_string();

        store.save_settings(&settings).unwrap();
        assert!(store.settings_path().exists());

        let text = fs::read_to_string(store.settings_path()).unwrap();
        assert!(text.contains("\n  \""), "settings should be pretty-printed");

        assert!(!text.contains("cookie=1"));
        assert!(text.contains(PROTECTED_FORMAT));
        let loaded = store.load_settings().unwrap();
        assert_eq!(value(&loaded), value(&settings));
        assert_eq!(loaded.query_from_language, Language::English);
        assert_eq!(loaded.query_to_language, Language::SimplifiedChinese);
        assert_eq!(loaded.bing_cookie, "cookie=1");

        // The write is atomic, so no temporary is left behind.
        for entry in fs::read_dir(store.root()).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(!name.ends_with(".tmp"), "leftover temp file {name}");
        }
    }

    #[test]
    fn missing_settings_file_yields_defaults() {
        let dir = TempDir::new("settings-missing");
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        assert!(!store.settings_path().exists());

        let loaded = store.load_settings().unwrap();
        assert_eq!(value(&loaded), value(&Settings::default()));
        // Loading must not write anything.
        assert!(!store.settings_path().exists());
    }

    #[test]
    fn corrupt_settings_are_reported_and_never_replaced_with_defaults() {
        let dir = TempDir::new("settings-corrupt");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        fs::write(store.settings_path(), "{ not json").unwrap();
        assert!(store.load_settings().is_err());
        assert_eq!(fs::read_to_string(store.settings_path()).unwrap(), "{ not json");

        // Valid JSON with a value this build cannot represent is just as unusable.
        fs::write(store.settings_path(), r#"{"appearanceType":"Neon"}"#).unwrap();
        assert!(store.load_settings().is_err());

        // Startup must stop before first-launch or other housekeeping writes.
        assert_eq!(
            fs::read_to_string(store.settings_path()).unwrap(),
            r#"{"appearanceType":"Neon"}"#
        );
    }

    #[test]
    fn reset_export_and_import_settings() {
        let dir = TempDir::new("settings-reset-export");
        let store = Store::open(dir.path().join("data")).unwrap();

        let mut settings = Settings::default();
        settings.hide_main_window = false;
        store.save_settings(&settings).unwrap();

        let fresh = store.reset_settings().unwrap();
        assert!(fresh.hide_main_window);
        assert_eq!(value(&fresh), value(&Settings::default()));
        // The reset is persisted, not just returned.
        assert_eq!(value(&store.load_settings().unwrap()), value(&Settings::default()));

        // Export creates the parent directory when the user picked a new one.
        let export = dir.path().join("exported").join("easydict.json");
        store.export_settings(&export, &settings, false).unwrap();
        assert_eq!(value(&store.import_settings(&export).unwrap()), value(&settings));

        let missing = dir.path().join("nope.json");
        assert!(matches!(
            store.import_settings(&missing),
            Err(StoreError::Io(_))
        ));
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "nope").unwrap();
        assert!(matches!(
            store.import_settings(&bad),
            Err(StoreError::Json(_))
        ));
    }

    #[test]
    fn legacy_settings_migrate_without_losing_credentials() {
        let dir = TempDir::new("migration");
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let mut legacy = Settings::default();
        legacy.bing_cookie = "test-cookie".into();
        legacy.service_mut("CustomOpenAI#test").api_key = "test-custom-key".into();
        fs::write(store.settings_path(), serde_json::to_vec(&legacy).unwrap()).unwrap();

        assert_eq!(value(&store.load_settings().unwrap()), value(&legacy));
        let disk = fs::read_to_string(store.settings_path()).unwrap();
        assert!(disk.contains(PROTECTED_FORMAT));
        assert!(!disk.contains("test-cookie"));
        assert!(!disk.contains("test-custom-key"));
        assert_eq!(value(&store.load_settings().unwrap()), value(&legacy));
        assert_eq!(value(&store.import_settings(&store.settings_path()).unwrap()), value(&legacy));
    }

    #[test]
    fn invalid_envelopes_never_silently_reset_settings() {
        let dir = TempDir::new("bad-envelope");
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        for document in [
            serde_json::json!({"format": "unknown", "data": ""}),
            serde_json::json!({"format": PROTECTED_FORMAT}),
            serde_json::json!({"format": PROTECTED_FORMAT, "data": "invalid base64!"}),
            serde_json::json!({"format": PROTECTED_FORMAT, "data": STANDARD.encode(b"invalid DPAPI")}),
        ] {
            let original = serde_json::to_vec(&document).unwrap();
            fs::write(store.settings_path(), &original).unwrap();
            assert!(store.load_settings().is_err());
            assert_eq!(fs::read(store.settings_path()).unwrap(), original);
            assert!(store.import_settings(&store.settings_path()).is_err());
        }
    }

    #[test]
    fn exports_omit_every_configured_credential_unless_explicitly_requested() {
        let dir = TempDir::new("redacted-export");
        let store = Store::open(dir.path().join("data")).unwrap();
        let mut document = value(&Settings::default());
        let credentials = [
            "deepLAuth", "youdaoWebApiKey", "bingCookie", "niuTransApiKey", "caiyunToken",
            "tencentSecretId", "tencentSecretKey", "aliAccessKeyId", "aliAccessKeySecret",
            "baiduAppId", "baiduSecretKey", "volcanoAccessKeyId", "volcanoSecretAccessKey", "doubaoApiKey",
        ];
        for field in credentials {
            assert!(document.get(field).is_some(), "unknown credential field {field}");
            document[field] = serde_json::json!(format!("test-sensitive-{field}"));
        }
        let mut settings: Settings = serde_json::from_value(document).unwrap();
        for service in settings.services.values_mut() {
            service.api_key = "test-sensitive-key".into();
        }
        settings.service_mut("CustomOpenAI#example").api_key = "test-sensitive-custom".into();
        settings.service_mut("CustomOpenAI#example").endpoint = "http://localhost:8321/v1".into();

        let redacted = dir.path().join("redacted.json");
        store.export_settings(&redacted, &settings, false).unwrap();
        assert!(!fs::read_to_string(&redacted).unwrap().contains("test-sensitive-"));
        let loaded = store.import_settings(&redacted).unwrap();
        assert_eq!(loaded.service("CustomOpenAI#example").endpoint, "http://localhost:8321/v1");
        for field in credentials {
            assert_eq!(value(&loaded)[field], "");
        }

        let full = dir.path().join("full.json");
        store.export_settings(&full, &settings, true).unwrap();
        assert_eq!(value(&store.import_settings(&full).unwrap()), value(&settings));
        assert!(settings.bing_cookie.starts_with("test-sensitive-"));
        // Import reads portable files without encrypting or otherwise changing the source.
        assert!(fs::read_to_string(&full).unwrap().contains("test-sensitive-"));
    }

    #[test]
    fn export_cannot_overwrite_live_settings_or_database() {
        let dir = TempDir::new("export-target");
        let store = Store::open(dir.path().join("data")).unwrap();
        let settings = Settings::default();
        store.save_settings(&settings).unwrap();
        let before = fs::read(store.settings_path()).unwrap();
        for path in [
            store.settings_path(),
            store.database_path(),
            store.root().join("nested").join("export.json"),
            store.root().join("..").join("data").join("settings.json"),
        ] {
            assert!(store.export_settings(&path, &settings, true).is_err());
        }
        assert_eq!(fs::read(store.settings_path()).unwrap(), before);
        assert_eq!(store.history_count().unwrap(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn failed_migration_preserves_the_legacy_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = TempDir::new("migration-failure");
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let original = br#"{"bingCookie":"test-cookie"}"#;
        fs::write(store.settings_path(), original).unwrap();
        let lock = File::options().read(true).share_mode(1).open(store.settings_path()).unwrap();
        assert!(store.load_settings().is_err());
        drop(lock);
        assert_eq!(fs::read(store.settings_path()).unwrap(), original);
        assert!(!fs::read_dir(store.root()).unwrap()
            .any(|entry| entry.unwrap().path().extension().is_some_and(|ext| ext == "tmp")));
    }

    #[test]
    fn history_orders_newest_first_and_dedupes() {
        let dir = TempDir::new("history-order");
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (from, to) = (Language::Auto, Language::SimplifiedChinese);

        store.add_history(&record("apple", from, to, 1_000)).unwrap();
        store.add_history(&record("banana", from, to, 2_000)).unwrap();
        store.add_history(&record("cherry", from, to, 3_000)).unwrap();

        assert_eq!(store.history_count().unwrap(), 3);
        assert_eq!(
            texts(store.history(10, 0).unwrap()),
            vec!["cherry", "banana", "apple"]
        );
        assert_eq!(texts(store.history(1, 1).unwrap()), vec!["banana"]);

        // Retrying the newest query moves its timestamp instead of adding a row.
        store.add_history(&record("cherry", from, to, 5_000)).unwrap();
        assert_eq!(store.history_count().unwrap(), 3);
        let all = store.history(10, 0).unwrap();
        assert_eq!(all[0].timestamp, 5_000);
        assert_eq!(texts(all), vec!["cherry", "banana", "apple"]);

        // The same text in another language pair is another query.
        store
            .add_history(&record("cherry", Language::Auto, Language::English, 6_000))
            .unwrap();
        assert_eq!(store.history_count().unwrap(), 4);
        assert_eq!(
            texts(store.history(10, 0).unwrap()),
            vec!["cherry", "cherry", "banana", "apple"]
        );

        // Blank queries never reach the table.
        for blank in ["", "   ", "\t\n"] {
            store
                .add_history(&record(blank, Language::Auto, Language::English, 7_000))
                .unwrap();
        }
        assert_eq!(store.history_count().unwrap(), 4);
    }

    #[test]
    fn delete_history_removes_the_matching_row() {
        let dir = TempDir::new("history-delete");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        let apple = record("apple", Language::Auto, Language::English, 1_000);
        let banana = record("banana", Language::Auto, Language::English, 2_000);
        let cherry = record("cherry", Language::Auto, Language::English, 3_000);
        for r in [&apple, &banana, &cherry] {
            store.add_history(r).unwrap();
        }

        store.delete_history(&banana).unwrap();
        assert_eq!(store.history_count().unwrap(), 2);
        assert_eq!(texts(store.history(10, 0).unwrap()), vec!["cherry", "apple"]);

        // Deleting twice, or a record that was never stored, is not an error.
        store.delete_history(&banana).unwrap();
        store
            .delete_history(&record("durian", Language::Auto, Language::English, 4_000))
            .unwrap();
        assert_eq!(store.history_count().unwrap(), 2);
    }

    #[test]
    fn search_history_matches_substring_case_insensitively() {
        let dir = TempDir::new("history-search");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        for (text, ts) in [
            ("Hello World", 1_000),
            ("hello there", 2_000),
            ("Goodbye", 3_000),
            ("100% sure", 4_000),
        ] {
            store
                .add_history(&record(text, Language::Auto, Language::English, ts))
                .unwrap();
        }

        assert_eq!(
            texts(store.search_history("hello", 10).unwrap()),
            vec!["hello there", "Hello World"]
        );
        assert_eq!(
            texts(store.search_history("WORLD", 10).unwrap()),
            vec!["Hello World"]
        );
        assert_eq!(
            texts(store.search_history("lo wo", 10).unwrap()),
            vec!["Hello World"]
        );
        // `%` is a literal, not a wildcard.
        assert_eq!(
            texts(store.search_history("%", 10).unwrap()),
            vec!["100% sure"]
        );
        assert!(store.search_history("zzz", 10).unwrap().is_empty());

        // An empty needle matches everything, newest first, and the limit holds.
        assert_eq!(
            texts(store.search_history("", 10).unwrap()),
            vec!["100% sure", "Goodbye", "hello there", "Hello World"]
        );
        assert_eq!(
            texts(store.search_history("", 2).unwrap()),
            vec!["100% sure", "Goodbye"]
        );
    }

    #[test]
    fn history_cap_keeps_the_newest_rows() {
        let dir = TempDir::new("history-cap");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        // Bulk-load straight into the table: going through `add_history` would
        // pay one transaction per row.
        let mut conn = Connection::open(store.database_path()).unwrap();
        {
            let tx = conn.transaction().unwrap();
            for i in 0..MAX_HISTORY {
                tx.execute(
                    "INSERT INTO history \
                     (query_text, from_language, to_language, timestamp, translated_text) \
                     VALUES (?1, 'auto', 'English', ?2, NULL)",
                    params![format!("old-{i}"), i as i64],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        assert_eq!(store.history_count().unwrap(), MAX_HISTORY);

        store
            .add_history(&record(
                "newest",
                Language::Auto,
                Language::English,
                MAX_HISTORY as i64 + 1,
            ))
            .unwrap();

        assert_eq!(store.history_count().unwrap(), MAX_HISTORY);
        let all = store.history(MAX_HISTORY + 10, 0).unwrap();
        assert_eq!(all.len(), MAX_HISTORY);
        assert_eq!(all[0].query_text, "newest");

        // Exactly the oldest row fell off the end.
        let count = |text: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM history WHERE query_text = ?1",
                params![text],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count("old-0"), 0);
        assert_eq!(count("old-1"), 1);
    }

    #[test]
    fn favorites_add_remove_and_idempotence() {
        let dir = TempDir::new("favorites");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        let mut apple = record("apple", Language::Auto, Language::English, 1_000);
        apple.translated_text = Some("苹果".to_string());
        store.add_favorite(&apple).unwrap();
        assert!(store.is_favorite(&apple).unwrap());

        let favorites = store.favorites().unwrap();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].translated_text.as_deref(), Some("苹果"));

        // Adding the same favourite again is a no-op.
        store.add_favorite(&apple).unwrap();
        assert_eq!(store.favorites().unwrap().len(), 1);

        // Even under another language pair: a favourite is the query text.
        let other = record("apple", Language::English, Language::Auto, 2_000);
        assert!(store.is_favorite(&other).unwrap());
        store.add_favorite(&other).unwrap();
        let favorites = store.favorites().unwrap();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].timestamp, 1_000);

        // Newest first.
        store
            .add_favorite(&record("banana", Language::Auto, Language::English, 3_000))
            .unwrap();
        assert_eq!(texts(store.favorites().unwrap()), vec!["banana", "apple"]);

        store.remove_favorite(&apple).unwrap();
        assert!(!store.is_favorite(&apple).unwrap());
        assert_eq!(texts(store.favorites().unwrap()), vec!["banana"]);

        // Removing it again is not an error.
        store.remove_favorite(&apple).unwrap();
        assert_eq!(texts(store.favorites().unwrap()), vec!["banana"]);
    }

    #[test]
    fn clear_history_empties_only_history() {
        let dir = TempDir::new("history-clear");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        store
            .add_history(&record("apple", Language::Auto, Language::English, 1_000))
            .unwrap();
        store
            .add_history(&record("banana", Language::Auto, Language::English, 2_000))
            .unwrap();
        store
            .add_favorite(&record("cherry", Language::Auto, Language::English, 3_000))
            .unwrap();
        assert_eq!(store.history_count().unwrap(), 2);

        store.clear_history().unwrap();
        assert_eq!(store.history_count().unwrap(), 0);
        assert!(store.history(10, 0).unwrap().is_empty());
        // Favourites have their own table.
        assert_eq!(texts(store.favorites().unwrap()), vec!["cherry"]);

        // Clearing an empty table is fine.
        store.clear_history().unwrap();
        assert_eq!(store.history_count().unwrap(), 0);
    }

    #[test]
    fn rows_hold_the_raw_upstream_language_strings() {
        let dir = TempDir::new("raw-languages");
        let store = Store::open(dir.path().to_path_buf()).unwrap();

        // The stored form is the serde form, so exports of either kind read back.
        for language in Language::all() {
            assert_eq!(
                format!("\"{}\"", language.raw()),
                serde_json::to_string(language).unwrap()
            );
        }

        store
            .add_history(&record(
                "apple",
                Language::SimplifiedChinese,
                Language::BrazilianPortuguese,
                1_000,
            ))
            .unwrap();
        store
            .add_favorite(&record(
                "banana",
                Language::Auto,
                Language::Unsupported,
                2_000,
            ))
            .unwrap();

        let conn = Connection::open(store.database_path()).unwrap();
        let (from, to): (String, String) = conn
            .query_row(
                "SELECT from_language, to_language FROM history",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(from, "Simplified-Chinese");
        assert_eq!(to, "Brazilian-Portuguese");

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        // And they round-trip through the API.
        let history = store.history(10, 0).unwrap();
        assert_eq!(history[0].from_language, Language::SimplifiedChinese);
        assert_eq!(history[0].to_language, Language::BrazilianPortuguese);

        let favorites = store.favorites().unwrap();
        assert_eq!(favorites[0].from_language, Language::Auto);
        assert_eq!(favorites[0].to_language, Language::Unsupported);
    }
}
