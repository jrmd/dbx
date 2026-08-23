//! Bounded, local query history for the SQL console.
//!
//! This module deliberately accepts a connection *identity*, never a
//! `ConnectionConfig`: URLs and credentials must not enter the history file.
//! Query text is also rejected before it reaches disk when it appears to
//! assign or provide a credential. We skip those entries instead of trying to
//! redact them, because redaction would leave misleading, executable-looking
//! history behind.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use dbx_core::DatabaseKind;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Version of the on-disk query-history document.
pub const QUERY_HISTORY_FILE_VERSION: u32 = 1;
/// Query-history file name below the DBX configuration directory.
pub const QUERY_HISTORY_FILE_NAME: &str = "query-history.json";
/// The maximum number of entries retained for one connection.
pub const MAX_QUERY_HISTORY_ENTRIES_PER_CONNECTION: usize = 100;
/// The maximum number of entries retained across all connections.
pub const MAX_QUERY_HISTORY_ENTRIES: usize = 5_000;
/// Keep a single pasted script from turning the small local history file into
/// an unbounded cache. Query execution is unaffected when history declines an
/// oversized entry.
pub const MAX_QUERY_HISTORY_SQL_BYTES: usize = 256 * 1024;

/// A non-secret connection identity suitable for durable local history.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryHistoryConnection {
    /// A saved connection profile. Its UUID is stable without exposing its URL.
    Profile { profile_id: Uuid },
    /// An unsaved session, identified only by user-visible, non-secret metadata.
    Session {
        display_name: String,
        kind: DatabaseKind,
        database: String,
    },
}

impl QueryHistoryConnection {
    pub const fn profile(profile_id: Uuid) -> Self {
        Self::Profile { profile_id }
    }

    /// Create an identity for an unsaved session.
    ///
    /// `database` must be a display/database name, not a connection string.
    pub fn session(
        display_name: impl Into<String>,
        kind: DatabaseKind,
        database: impl Into<String>,
    ) -> QueryHistoryResult<Self> {
        let display_name = validate_identity_part("connection display name", display_name.into())?;
        let database = validate_identity_part("database", database.into())?;
        Ok(Self::Session {
            display_name,
            kind,
            database,
        })
    }

    fn validate(&self) -> QueryHistoryResult<()> {
        if let Self::Session {
            display_name,
            database,
            ..
        } = self
        {
            validate_identity_part("connection display name", display_name.clone())?;
            validate_identity_part("database", database.clone())?;
        }
        Ok(())
    }
}

/// The outcome recorded beside an executed query.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "status", content = "summary", rename_all = "snake_case")]
pub enum QueryHistoryOutcome {
    Success(String),
    Failure(String),
}

impl QueryHistoryOutcome {
    pub fn success(summary: impl Into<String>) -> Self {
        Self::Success(sanitize_summary(summary.into()))
    }

    pub fn failure(summary: impl Into<String>) -> Self {
        Self::Failure(sanitize_summary(summary.into()))
    }
}

/// One executed query, stored in newest-last order in the document.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct QueryHistoryEntry {
    pub connection: QueryHistoryConnection,
    pub sql: String,
    /// Unix time in milliseconds, avoiding locale-dependent timestamp parsing.
    pub executed_at_ms: u64,
    pub outcome: QueryHistoryOutcome,
}

#[derive(Debug, Error)]
pub enum QueryHistoryError {
    #[error("query history storage I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid query history JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("platform configuration directory is unavailable")]
    ConfigDirectoryUnavailable,
    #[error("unsupported query history document version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("invalid query history: {0}")]
    Invalid(String),
    #[error("query history entry was not stored because its SQL may contain a credential")]
    SensitiveSql,
}

pub type QueryHistoryResult<T> = Result<T, QueryHistoryError>;

/// File-backed, bounded query history repository.
#[derive(Clone, Debug)]
pub struct QueryHistoryStore {
    path: PathBuf,
    operation_lock: Arc<Mutex<()>>,
}

impl QueryHistoryStore {
    pub fn new() -> QueryHistoryResult<Self> {
        Ok(Self::at(default_query_history_path()?))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Load all entries in chronological order (oldest first).
    pub fn load(&self) -> QueryHistoryResult<Vec<QueryHistoryEntry>> {
        let _lock = self.lock()?;
        Ok(self.read_document()?.entries)
    }

    /// Record a query with the current wall-clock timestamp.
    pub fn record(
        &self,
        connection: QueryHistoryConnection,
        sql: impl Into<String>,
        outcome: QueryHistoryOutcome,
    ) -> QueryHistoryResult<()> {
        self.record_at(connection, sql, now_ms()?, outcome)
    }

    /// Record a query at a caller-provided timestamp. Useful when execution
    /// already has a timestamp and for deterministic tests.
    pub fn record_at(
        &self,
        connection: QueryHistoryConnection,
        sql: impl Into<String>,
        executed_at_ms: u64,
        outcome: QueryHistoryOutcome,
    ) -> QueryHistoryResult<()> {
        connection.validate()?;
        let sql = validate_sql(sql.into())?;
        let _lock = self.lock()?;
        let mut document = self.read_document()?;
        let entry = QueryHistoryEntry {
            connection: connection.clone(),
            sql,
            executed_at_ms,
            outcome,
        };

        // Consecutive means consecutive executions for this connection, even
        // if another connection ran a query in between.
        if let Some(index) = document
            .entries
            .iter()
            .rposition(|existing| existing.connection == connection)
            .filter(|&index| document.entries[index].sql == entry.sql)
        {
            document.entries.remove(index);
        }
        document.entries.push(entry);
        trim_connection_entries(&mut document.entries, &connection);
        trim_total_entries(&mut document.entries);
        self.write_document(&document)
    }

    /// Return up to `limit` newest entries for one connection.
    pub fn recent(
        &self,
        connection: &QueryHistoryConnection,
        limit: usize,
    ) -> QueryHistoryResult<Vec<QueryHistoryEntry>> {
        connection.validate()?;
        let _lock = self.lock()?;
        let document = self.read_document()?;
        Ok(document
            .entries
            .iter()
            .rev()
            .filter(|entry| &entry.connection == connection)
            .take(limit)
            .cloned()
            .collect())
    }

    /// Clear history for one connection, returning the number of removed entries.
    pub fn clear(&self, connection: &QueryHistoryConnection) -> QueryHistoryResult<usize> {
        connection.validate()?;
        let _lock = self.lock()?;
        let mut document = self.read_document()?;
        let previous_len = document.entries.len();
        document
            .entries
            .retain(|entry| &entry.connection != connection);
        let removed = previous_len - document.entries.len();
        if removed != 0 {
            self.write_document(&document)?;
        }
        Ok(removed)
    }

    fn lock(&self) -> QueryHistoryResult<MutexGuard<'_, ()>> {
        self.operation_lock
            .lock()
            .map_err(|_| QueryHistoryError::Invalid("query history operation lock poisoned".into()))
    }

    fn read_document(&self) -> QueryHistoryResult<QueryHistoryDocument> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let document: QueryHistoryDocument = serde_json::from_slice(&bytes)?;
                document.validate()?;
                Ok(document)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(QueryHistoryDocument::empty())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn write_document(&self, document: &QueryHistoryDocument) -> QueryHistoryResult<()> {
        document.validate()?;
        atomic_write(&self.path, &serde_json::to_vec_pretty(document)?)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct QueryHistoryDocument {
    version: u32,
    #[serde(default)]
    entries: Vec<QueryHistoryEntry>,
}

impl QueryHistoryDocument {
    fn empty() -> Self {
        Self {
            version: QUERY_HISTORY_FILE_VERSION,
            entries: Vec::new(),
        }
    }

    fn validate(&self) -> QueryHistoryResult<()> {
        if self.version != QUERY_HISTORY_FILE_VERSION {
            return Err(QueryHistoryError::UnsupportedVersion {
                found: self.version,
                expected: QUERY_HISTORY_FILE_VERSION,
            });
        }
        if self.entries.len() > MAX_QUERY_HISTORY_ENTRIES {
            return Err(QueryHistoryError::Invalid(format!(
                "more than {MAX_QUERY_HISTORY_ENTRIES} query history entries"
            )));
        }
        let mut per_connection = std::collections::HashMap::<&QueryHistoryConnection, usize>::new();
        for entry in &self.entries {
            entry.connection.validate()?;
            validate_sql(entry.sql.clone())?;
            let count = per_connection.entry(&entry.connection).or_default();
            *count += 1;
            if *count > MAX_QUERY_HISTORY_ENTRIES_PER_CONNECTION {
                return Err(QueryHistoryError::Invalid(format!(
                    "more than {MAX_QUERY_HISTORY_ENTRIES_PER_CONNECTION} entries for one connection"
                )));
            }
        }
        Ok(())
    }
}

fn trim_connection_entries(
    entries: &mut Vec<QueryHistoryEntry>,
    connection: &QueryHistoryConnection,
) {
    while entries
        .iter()
        .filter(|entry| &entry.connection == connection)
        .count()
        > MAX_QUERY_HISTORY_ENTRIES_PER_CONNECTION
    {
        let oldest = entries
            .iter()
            .position(|entry| &entry.connection == connection)
            .expect("connection count confirmed above maximum");
        entries.remove(oldest);
    }
}

fn trim_total_entries(entries: &mut Vec<QueryHistoryEntry>) {
    let excess = entries.len().saturating_sub(MAX_QUERY_HISTORY_ENTRIES);
    if excess != 0 {
        entries.drain(..excess);
    }
}

fn validate_identity_part(label: &str, value: String) -> QueryHistoryResult<String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(QueryHistoryError::Invalid(format!(
            "{label} cannot be empty"
        )));
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("://") || lower.contains("password=") || lower.contains("@") {
        return Err(QueryHistoryError::Invalid(format!(
            "{label} must not be a connection URL or credential"
        )));
    }
    Ok(value)
}

fn validate_sql(sql: String) -> QueryHistoryResult<String> {
    if sql.trim().is_empty() {
        return Err(QueryHistoryError::Invalid(
            "SQL text cannot be empty".into(),
        ));
    }
    if sql.len() > MAX_QUERY_HISTORY_SQL_BYTES {
        return Err(QueryHistoryError::Invalid(format!(
            "SQL text exceeds the {MAX_QUERY_HISTORY_SQL_BYTES}-byte history limit"
        )));
    }
    if contains_sensitive_sql(&sql) {
        return Err(QueryHistoryError::SensitiveSql);
    }
    Ok(sql)
}

/// Detect common credential-bearing SQL without attempting to interpret a
/// database dialect. This intentionally looks for a credential-shaped label
/// followed by an assignment, `TO`/`IS`, or a literal; identifiers such as
/// `token_count` therefore remain valid query text.
fn contains_sensitive_sql(sql: &str) -> bool {
    contains_url_userinfo(sql)
        || contains_credential_assignment(sql)
        || contains_database_auth_command(sql)
}

fn contains_url_userinfo(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    for scheme_start in 0..bytes.len().saturating_sub(2) {
        if bytes[scheme_start..].starts_with(b"://") {
            let authority_start = scheme_start + 3;
            let authority_end = bytes[authority_start..]
                .iter()
                .position(|byte| {
                    matches!(
                        byte,
                        b'/' | b'?' | b'#' | b'\'' | b'\"' | b' ' | b'\n' | b'\r' | b'\t'
                    )
                })
                .map_or(bytes.len(), |offset| authority_start + offset);
            if bytes[authority_start..authority_end].contains(&b'@') {
                return true;
            }
        }
    }
    false
}

fn contains_credential_assignment(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !is_identifier_byte(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_identifier_byte(bytes[index]) {
            index += 1;
        }
        let label = &sql[start..index];
        let label_end = if is_credential_label(label) {
            index
        } else if label.eq_ignore_ascii_case("api") {
            let key_start = skip_whitespace(bytes, index);
            match next_word(sql, key_start) {
                Some((word, end)) if word.eq_ignore_ascii_case("key") => end,
                _ => continue,
            }
        } else {
            continue;
        };
        let mut next = skip_whitespace(bytes, label_end);
        if matches!(bytes.get(next), Some(b'=' | b':')) {
            return true;
        }
        if matches!(bytes.get(next), Some(b'\'' | b'\"' | b'$')) {
            return true;
        }
        if let Some((word, end)) = next_word(sql, next)
            && (word.eq_ignore_ascii_case("to") || word.eq_ignore_ascii_case("is"))
        {
            next = skip_whitespace(bytes, end);
            if matches!(bytes.get(next), Some(b'\'' | b'\"' | b'$')) {
                return true;
            }
        }
    }
    false
}

/// Detect authentication command forms that carry a secret as an unlabelled
/// argument. Keeping this command-shaped (rather than rejecting every `AUTH`
/// identifier) lets ordinary column queries such as `SELECT auth FROM users`
/// remain in history.
fn contains_database_auth_command(sql: &str) -> bool {
    let words = sql_words(sql);
    let Some((first, rest)) = words.split_first() else {
        return false;
    };

    if first.eq_ignore_ascii_case("auth") {
        return !rest.is_empty();
    }
    if first.eq_ignore_ascii_case("hello")
        && words
            .iter()
            .skip(1)
            .any(|word| word.eq_ignore_ascii_case("auth"))
    {
        return true;
    }
    if first.eq_ignore_ascii_case("acl")
        && rest
            .first()
            .is_some_and(|word| word.eq_ignore_ascii_case("setuser"))
        && sql.contains('>')
    {
        return true;
    }

    let creates_or_alters_user = words.iter().enumerate().any(|(index, word)| {
        (word.eq_ignore_ascii_case("create") || word.eq_ignore_ascii_case("alter"))
            && words[index + 1..]
                .iter()
                .take(3)
                .any(|candidate| candidate.eq_ignore_ascii_case("user"))
    });
    creates_or_alters_user
        && words.iter().enumerate().any(|(identified_index, word)| {
            word.eq_ignore_ascii_case("identified")
                && words[identified_index + 1..]
                    .iter()
                    .take_while(|candidate| !candidate.eq_ignore_ascii_case("identified"))
                    .any(|candidate| candidate.eq_ignore_ascii_case("by"))
        })
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn is_credential_label(label: &str) -> bool {
    matches!(
        label.to_ascii_lowercase().as_str(),
        "password"
            | "passwd"
            | "secret"
            | "token"
            | "api_key"
            | "api-key"
            | "apikey"
            | "client_secret"
            | "client-secret"
            | "access_token"
            | "access-token"
            | "refresh_token"
            | "refresh-token"
            | "aws_secret_access_key"
            | "aws-secret-access-key"
            | "private_key"
            | "private-key"
            | "authorization"
            | "credential"
            | "credentials"
    )
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(byte) if byte.is_ascii_whitespace()) {
        index += 1;
    }
    index
}

fn next_word(sql: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = sql.as_bytes();
    if !matches!(bytes.get(start), Some(byte) if byte.is_ascii_alphabetic()) {
        return None;
    }
    let mut end = start;
    while matches!(bytes.get(end), Some(byte) if byte.is_ascii_alphabetic()) {
        end += 1;
    }
    Some((&sql[start..end], end))
}

fn sql_words(sql: &str) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_identifier_byte(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_identifier_byte(bytes[index]) {
            index += 1;
        }
        words.push(&sql[start..index]);
    }
    words
}

fn sanitize_summary(summary: String) -> String {
    let summary = summary.trim();
    let lower = summary.to_ascii_lowercase();
    if lower.contains("://") || lower.contains("password=") || contains_sensitive_sql(summary) {
        return "Database error details redacted.".into();
    }
    summary.chars().take(512).collect()
}

fn now_ms() -> QueryHistoryResult<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .map_err(|error| {
            QueryHistoryError::Invalid(format!("system clock before Unix epoch: {error}"))
        })
}

fn default_query_history_path() -> QueryHistoryResult<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or(QueryHistoryError::ConfigDirectoryUnavailable)?
        .join("dbx")
        .join(QUERY_HISTORY_FILE_NAME))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let parent_was_missing = !parent.exists();
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    if parent_was_missing {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(QUERY_HISTORY_FILE_NAME);
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, QueryHistoryStore) {
        let directory = tempfile::tempdir().expect("temporary history directory");
        let store = QueryHistoryStore::at(directory.path().join(QUERY_HISTORY_FILE_NAME));
        (directory, store)
    }

    fn connection() -> QueryHistoryConnection {
        QueryHistoryConnection::profile(Uuid::nil())
    }

    #[test]
    fn records_and_returns_newest_entries_first() {
        let (_directory, store) = store();
        store
            .record_at(
                connection(),
                "select 1",
                10,
                QueryHistoryOutcome::success("1 row"),
            )
            .expect("record query");
        store
            .record_at(
                connection(),
                "select 2",
                20,
                QueryHistoryOutcome::failure("bad SQL"),
            )
            .expect("record query");

        let entries = store.recent(&connection(), 10).expect("recent history");
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.sql.as_str())
                .collect::<Vec<_>>(),
            ["select 2", "select 1"]
        );
        assert_eq!(entries[0].executed_at_ms, 20);
        assert!(matches!(
            entries[0].outcome,
            QueryHistoryOutcome::Failure(_)
        ));
    }

    #[test]
    fn deduplicates_consecutive_queries_per_connection() {
        let (_directory, store) = store();
        let other = QueryHistoryConnection::profile(Uuid::new_v4());
        store
            .record_at(
                connection(),
                "select 1",
                10,
                QueryHistoryOutcome::success("one"),
            )
            .unwrap();
        store
            .record_at(other, "select 9", 11, QueryHistoryOutcome::success("one"))
            .unwrap();
        store
            .record_at(
                connection(),
                "select 1",
                12,
                QueryHistoryOutcome::success("again"),
            )
            .unwrap();

        let entries = store.recent(&connection(), 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].executed_at_ms, 12);
    }

    #[test]
    fn bounds_entries_per_connection_without_losing_other_connections() {
        let (_directory, store) = store();
        let other = QueryHistoryConnection::profile(Uuid::new_v4());
        for index in 0..=MAX_QUERY_HISTORY_ENTRIES_PER_CONNECTION {
            store
                .record_at(
                    connection(),
                    format!("select {index}"),
                    index as u64,
                    QueryHistoryOutcome::success("ok"),
                )
                .unwrap();
        }
        store
            .record_at(
                other.clone(),
                "select other",
                999,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();

        let entries = store.recent(&connection(), usize::MAX).unwrap();
        assert_eq!(entries.len(), MAX_QUERY_HISTORY_ENTRIES_PER_CONNECTION);
        assert_eq!(entries.last().unwrap().sql, "select 1");
        assert_eq!(store.recent(&other, 10).unwrap().len(), 1);
    }

    #[test]
    fn rejects_corrupt_and_future_documents() {
        let (directory, store) = store();
        fs::write(directory.path().join(QUERY_HISTORY_FILE_NAME), b"not JSON").unwrap();
        assert!(matches!(store.load(), Err(QueryHistoryError::Json(_))));

        fs::write(
            directory.path().join(QUERY_HISTORY_FILE_NAME),
            br#"{"version":2,"entries":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            store.load(),
            Err(QueryHistoryError::UnsupportedVersion {
                found: 2,
                expected: 1
            })
        ));
    }

    #[test]
    fn clear_only_removes_requested_connection() {
        let (_directory, store) = store();
        let other = QueryHistoryConnection::profile(Uuid::new_v4());
        store
            .record_at(
                connection(),
                "select 1",
                1,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();
        store
            .record_at(
                other.clone(),
                "select 2",
                2,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();
        assert_eq!(store.clear(&connection()).unwrap(), 1);
        assert!(store.recent(&connection(), 10).unwrap().is_empty());
        assert_eq!(store.recent(&other, 10).unwrap().len(), 1);
    }

    #[test]
    fn session_identity_and_summary_do_not_allow_urls_or_credentials() {
        assert!(
            QueryHistoryConnection::session("local", DatabaseKind::SQLite, "sqlite:///tmp/db")
                .is_err()
        );
        assert_eq!(
            QueryHistoryOutcome::failure("postgres://alice:secret@host/db"),
            QueryHistoryOutcome::Failure("Database error details redacted.".into())
        );
    }

    #[test]
    fn redacts_credential_shaped_summaries_but_keeps_ordinary_identifiers() {
        for summary in [
            "database rejected token=not-for-history",
            "database rejected api_key: not-for-history",
            "database rejected authorization: Bearer not-for-history",
        ] {
            assert_eq!(
                QueryHistoryOutcome::failure(summary),
                QueryHistoryOutcome::Failure("Database error details redacted.".into())
            );
        }
        assert_eq!(
            QueryHistoryOutcome::success("returned token_count for 3 rows"),
            QueryHistoryOutcome::Success("returned token_count for 3 rows".into())
        );
    }

    #[test]
    fn rejects_oversized_queries_without_changing_existing_history() {
        let (_directory, store) = store();
        store
            .record_at(
                connection(),
                "select 1",
                1,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();

        let oversized = "x".repeat(MAX_QUERY_HISTORY_SQL_BYTES + 1);
        assert!(matches!(
            store.record_at(
                connection(),
                oversized,
                2,
                QueryHistoryOutcome::success("ok"),
            ),
            Err(QueryHistoryError::Invalid(_))
        ));
        assert_eq!(store.recent(&connection(), 10).unwrap().len(), 1);
    }

    #[test]
    fn rejects_credential_bearing_sql_without_changing_the_history_file() {
        let (directory, store) = store();
        let path = directory.path().join(QUERY_HISTORY_FILE_NAME);
        store
            .record_at(
                connection(),
                "select token_count from metrics",
                1,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();
        let before = fs::read(&path).unwrap();

        for sql in [
            "alter user alice password 'not-for-history'",
            "set api_key = 'not-for-history'",
            "set api key = 'not-for-history'",
            "set client_secret = 'not-for-history'",
            "set access_token = 'not-for-history'",
            "set refresh_token = 'not-for-history'",
            "set aws_secret_access_key = 'not-for-history'",
            "set private_key = 'not-for-history'",
            "select * from requests where authorization: 'Bearer secret'",
            "select 'postgres://alice:secret@db.example/app'",
            "create user alice identified by 'not-for-history'",
            "create user alice identified with caching_sha2_password by 'not-for-history'",
            "auth not-for-history",
            "hello 3 auth alice not-for-history",
            "acl setuser alice >not-for-history",
        ] {
            assert!(matches!(
                store.record_at(connection(), sql, 2, QueryHistoryOutcome::success("ok")),
                Err(QueryHistoryError::SensitiveSql)
            ));
            assert_eq!(fs::read(&path).unwrap(), before);
        }

        let entries = store.recent(&connection(), 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sql, "select token_count from metrics");
    }

    #[test]
    fn allows_ordinary_auth_identifier_queries() {
        let (_directory, store) = store();
        store
            .record_at(
                connection(),
                "select auth, token_count from audit_metrics",
                1,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();
        assert_eq!(store.recent(&connection(), 10).unwrap().len(), 1);
    }

    #[test]
    fn bounds_total_entries_across_unique_connections() {
        let (_directory, store) = store();
        for index in 0..=MAX_QUERY_HISTORY_ENTRIES {
            store
                .record_at(
                    QueryHistoryConnection::profile(Uuid::from_u128(index as u128 + 1)),
                    "select 1",
                    index as u64,
                    QueryHistoryOutcome::success("ok"),
                )
                .unwrap();
        }

        let entries = store.load().unwrap();
        assert_eq!(entries.len(), MAX_QUERY_HISTORY_ENTRIES);
        assert_eq!(entries.first().unwrap().executed_at_ms, 1);
        assert_eq!(
            entries.last().unwrap().executed_at_ms,
            MAX_QUERY_HISTORY_ENTRIES as u64
        );
    }

    #[cfg(unix)]
    #[test]
    fn writes_private_file_and_directory() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("private")
            .join(QUERY_HISTORY_FILE_NAME);
        let store = QueryHistoryStore::at(&path);
        store
            .record_at(
                connection(),
                "select 1",
                1,
                QueryHistoryOutcome::success("ok"),
            )
            .unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(directory.path().join("private"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
