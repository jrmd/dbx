//! Durable connection profiles for the DBX desktop client.
//!
//! The profile document contains only connection metadata.  Passwords and
//! other userinfo secrets are kept in the operating system credential store.
//! A secret-bearing profile is not saved when that durable store is unavailable.
//! `ProfileStore` reads the document for each operation instead of retaining a
//! second, in-memory copy of profiles.

#[cfg(test)]
use std::collections::HashMap;
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use dbx_core::{ConnectionConfig, DatabaseKind};
use keyring::{Entry, Error as KeyringError};
#[cfg(test)]
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

/// Version of the on-disk profile document.
pub const PROFILE_FILE_VERSION: u32 = 1;

/// Deployment environment label a connection can be tagged with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionEnvironment {
    Production,
    Staging,
    Develop,
    #[default]
    Local,
}

impl ConnectionEnvironment {
    /// Every label in display order for pickers.
    pub const ALL: [ConnectionEnvironment; 4] = [
        ConnectionEnvironment::Production,
        ConnectionEnvironment::Staging,
        ConnectionEnvironment::Develop,
        ConnectionEnvironment::Local,
    ];
}

impl fmt::Display for ConnectionEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            ConnectionEnvironment::Production => "Production",
            ConnectionEnvironment::Staging => "Staging",
            ConnectionEnvironment::Develop => "Develop",
            ConnectionEnvironment::Local => "Local",
        };
        f.write_str(label)
    }
}

/// Name of the profile file below the platform configuration directory.
pub const PROFILE_FILE_NAME: &str = "connections.json";

/// Service name used for credentials in the platform keyring.
pub const KEYRING_SERVICE: &str = "dev.dbx.app.connections";

/// Errors returned by a secret backend.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum SecretStoreError {
    #[error("secret store error: {0}")]
    Message(String),
}

/// A small abstraction around credential storage.
///
/// Keeping this trait separate from [`ProfileStore`] means tests can provide a
/// deterministic fake store and the UI can use another backend in the future.
/// Implementations must not log or otherwise persist `secret` values.
pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, secret: &str) -> Result<(), SecretStoreError>;
    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError>;
    fn delete(&self, key: &str) -> Result<(), SecretStoreError>;
}

/// The default secret backend.
///
/// Keyring calls are synchronous, as required by the `keyring` crate. If the
/// platform credential service is unavailable, saving fails instead of
/// silently keeping a connection secret only in process memory.
#[derive(Clone, Default)]
pub struct SystemSecretStore;

impl fmt::Debug for SystemSecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SystemSecretStore").finish()
    }
}

impl SystemSecretStore {
    pub fn new() -> Self {
        Self
    }
}

impl SecretStore for SystemSecretStore {
    fn set(&self, key: &str, secret: &str) -> Result<(), SecretStoreError> {
        Entry::new(KEYRING_SERVICE, key)
            .and_then(|entry| entry.set_password(secret))
            .map_err(keyring_error)
    }

    fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        match Entry::new(KEYRING_SERVICE, key).and_then(|entry| entry.get_password()) {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        match Entry::new(KEYRING_SERVICE, key).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}

fn keyring_error(error: KeyringError) -> SecretStoreError {
    SecretStoreError::Message(error.to_string())
}

/// A profile as exposed to the UI.  The URL is always password-free.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedConnection {
    pub id: Uuid,
    pub name: String,
    pub kind: DatabaseKind,
    /// A normalized connection URL with no password in its userinfo.
    pub url: String,
    pub environment: ConnectionEnvironment,
    pub max_connections: u32,
    pub connect_timeout_ms: u64,
    secret_key: Option<String>,
}

impl SavedConnection {
    /// Whether this profile has a password stored in the keyring/session
    /// backend.
    #[cfg(test)]
    pub fn has_secret(&self) -> bool {
        self.secret_key.is_some()
    }

    /// Return the selected SQLite file, if this is a file-backed SQLite
    /// profile.  `None` is returned for non-SQLite profiles and in-memory
    /// SQLite connections.
    #[cfg(test)]
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        sqlite_path_from_url(self.kind, &self.url)
    }
}

/// Input used to create or update a saved profile.
///
/// `id == None` creates a profile with a generated UUID.  A password can be
/// provided separately, or it can be extracted from `config.url`; in either
/// case it is removed before the profile is written to disk.
pub struct ConnectionProfileDraft {
    pub id: Option<Uuid>,
    pub name: String,
    pub config: ConnectionConfig,
    pub environment: ConnectionEnvironment,
    pub secret: Option<String>,
}

impl fmt::Debug for ConnectionProfileDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionProfileDraft")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("config", &self.config)
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl ConnectionProfileDraft {
    pub fn new(name: impl Into<String>, kind: DatabaseKind, url: impl Into<String>) -> Self {
        Self::from_config(name, ConnectionConfig::new(kind, url))
    }

    pub fn from_config(name: impl Into<String>, config: ConnectionConfig) -> Self {
        Self {
            id: None,
            name: name.into(),
            config,
            environment: ConnectionEnvironment::default(),
            secret: None,
        }
    }

    /// Create a profile for a user-selected SQLite file.
    #[cfg(test)]
    pub fn sqlite(name: impl Into<String>, path: impl AsRef<Path>) -> Self {
        Self::from_config(
            name,
            ConnectionConfig::new(DatabaseKind::SQLite, sqlite_url(path.as_ref())),
        )
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    /// Tag the profile with a deployment environment label.
    pub fn with_environment(mut self, environment: ConnectionEnvironment) -> Self {
        self.environment = environment;
        self
    }

    #[cfg(test)]
    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }
}

/// A profile plus a complete connection URL ready for `dbx-core`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConnection {
    pub profile: SavedConnection,
    pub config: ConnectionConfig,
}

/// Errors while reading, writing, or resolving a saved connection.
#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile storage I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid profile JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("platform configuration directory is unavailable")]
    ConfigDirectoryUnavailable,
    #[error("unsupported profile document version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("profile {0} was not found")]
    NotFound(Uuid),
    #[error("profile {0} has no available credential; enter its password again")]
    MissingSecret(Uuid),
    #[error("invalid connection profile: {0}")]
    Invalid(String),
    #[error("secret store error: {0}")]
    Secret(#[from] SecretStoreError),
}

pub type ProfileResult<T> = Result<T, ProfileError>;

/// Persistent profile repository.
///
/// Only the file path, secret backend, and a short-lived operation lock are
/// retained.  Profiles are loaded from disk for each operation and are not
/// cached in this repository.
#[derive(Clone)]
pub struct ProfileStore {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    operation_lock: Arc<Mutex<()>>,
}

impl fmt::Debug for ProfileStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ProfileStore {
    /// Open the profile store in the platform's normal application config
    /// directory.
    pub fn new() -> ProfileResult<Self> {
        Ok(Self::at(default_profile_path()?))
    }

    /// Open a profile store at an explicit file path using the system keyring.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::with_secret_store(path, Arc::new(SystemSecretStore::new()))
    }

    /// Construct a store with an injected secret backend.  This is useful for
    /// tests and for an application-managed credential provider.
    pub fn with_secret_store(path: impl Into<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            path: path.into(),
            secrets,
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// List profiles sorted by display name, without loading any secrets.
    pub fn list(&self) -> ProfileResult<Vec<SavedConnection>> {
        let _lock = self.lock()?;
        let mut profiles = self
            .read_document()?
            .connections
            .into_iter()
            .map(StoredConnection::into_public)
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(profiles)
    }

    /// Find a profile by ID without loading its secret.
    pub fn get(&self, id: Uuid) -> ProfileResult<Option<SavedConnection>> {
        let _lock = self.lock()?;
        Ok(self
            .read_document()?
            .connections
            .into_iter()
            .find(|profile| profile.id == id)
            .map(StoredConnection::into_public))
    }

    /// Save a new profile or update the profile identified by `draft.id`.
    pub fn save(&self, draft: ConnectionProfileDraft) -> ProfileResult<SavedConnection> {
        let _lock = self.lock()?;
        validate_name(&draft.name)?;
        let (url, embedded_secret) = scrub_url(&draft.config.url)?;
        let secret = draft.secret.or(embedded_secret);
        let id = draft.id.unwrap_or_else(Uuid::new_v4);
        let mut document = self.read_document()?;
        let existing = document
            .connections
            .iter()
            .find(|profile| profile.id == id)
            .cloned();
        // The connection screen deliberately displays the password-free URL.
        // Saving edits to that selected profile must retain its existing
        // credential unless a replacement password is explicitly supplied.
        let new_secret_key = secret.as_ref().map(|_| keyring_key(id)).or_else(|| {
            existing
                .as_ref()
                .and_then(|profile| profile.secret_key.clone())
        });
        let replacement = StoredConnection {
            id,
            name: draft.name,
            kind: draft.config.kind,
            url,
            environment: draft.environment,
            max_connections: draft.config.max_connections,
            connect_timeout_ms: draft.config.connect_timeout_ms,
            secret_key: new_secret_key.clone(),
        };

        if let Some(secret) = &secret {
            self.secrets.set(
                new_secret_key
                    .as_deref()
                    .expect("a secret always has a keyring key"),
                secret,
            )?;
        }

        if existing.is_some() {
            if let Some(index) = document
                .connections
                .iter()
                .position(|profile| profile.id == id)
            {
                document.connections[index] = replacement.clone();
            }
        } else {
            document.connections.push(replacement.clone());
        }

        if let Err(error) = self.write_document(&document) {
            // Avoid leaving a newly created keyring entry behind when the
            // profile file could not be atomically replaced.  If this was an
            // update using the same key, the previous credential remains
            // valid and is intentionally not deleted here.
            if existing.is_none()
                && let Some(key) = &new_secret_key
            {
                let _ = self.secrets.delete(key);
            }
            return Err(error);
        }

        Ok(replacement.into_public())
    }

    /// Load a profile and resolve its password from the secret backend.
    pub fn load(&self, id: Uuid) -> ProfileResult<LoadedConnection> {
        let _lock = self.lock()?;
        let stored = self
            .read_document()?
            .connections
            .into_iter()
            .find(|profile| profile.id == id)
            .ok_or(ProfileError::NotFound(id))?;
        let profile = stored.clone().into_public();
        let url = if let Some(secret_key) = stored.secret_key {
            let secret = self
                .secrets
                .get(&secret_key)?
                .ok_or(ProfileError::MissingSecret(id))?;
            add_password(&stored.url, &secret)?
        } else {
            stored.url
        };
        Ok(LoadedConnection {
            profile,
            config: ConnectionConfig::new(stored.kind, url)
                .with_max_connections(stored.max_connections)
                .with_connect_timeout_ms(stored.connect_timeout_ms),
        })
    }

    /// Delete a profile and its associated credential.  Returns `true` when a
    /// profile existed and `false` when the ID was already absent.
    pub fn delete(&self, id: Uuid) -> ProfileResult<bool> {
        let _lock = self.lock()?;
        let mut document = self.read_document()?;
        let Some(index) = document
            .connections
            .iter()
            .position(|profile| profile.id == id)
        else {
            return Ok(false);
        };
        let removed = document.connections.remove(index);
        self.write_document(&document)?;
        if let Some(secret_key) = removed.secret_key {
            self.secrets.delete(&secret_key)?;
        }
        Ok(true)
    }

    fn lock(&self) -> ProfileResult<MutexGuard<'_, ()>> {
        self.operation_lock
            .lock()
            .map_err(|_| ProfileError::Invalid("profile operation lock poisoned".into()))
    }

    fn read_document(&self) -> ProfileResult<ProfileDocument> {
        if !self.path.exists() {
            return Ok(ProfileDocument::empty());
        }
        let bytes = fs::read(&self.path)?;
        let document: ProfileDocument = serde_json::from_slice(&bytes)?;
        document.validate()?;
        Ok(document)
    }

    fn write_document(&self, document: &ProfileDocument) -> ProfileResult<()> {
        document.validate()?;
        let bytes = serde_json::to_vec_pretty(document)?;
        atomic_write(&self.path, &bytes)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProfileDocument {
    version: u32,
    #[serde(default)]
    connections: Vec<StoredConnection>,
}

impl ProfileDocument {
    fn empty() -> Self {
        Self {
            version: PROFILE_FILE_VERSION,
            connections: Vec::new(),
        }
    }

    fn validate(&self) -> ProfileResult<()> {
        if self.version != PROFILE_FILE_VERSION {
            return Err(ProfileError::UnsupportedVersion {
                found: self.version,
                expected: PROFILE_FILE_VERSION,
            });
        }
        let mut ids = std::collections::HashSet::with_capacity(self.connections.len());
        for profile in &self.connections {
            validate_name(&profile.name)?;
            if !ids.insert(profile.id) {
                return Err(ProfileError::Invalid(format!(
                    "duplicate profile ID {}",
                    profile.id
                )));
            }
            if profile.url.trim().is_empty() {
                return Err(ProfileError::Invalid(format!(
                    "profile {} has an empty URL",
                    profile.id
                )));
            }
            let (_, embedded_secret) = scrub_url(&profile.url)?;
            if embedded_secret.is_some() {
                return Err(ProfileError::Invalid(format!(
                    "profile {} contains a password in its JSON URL",
                    profile.id
                )));
            }
            if profile.max_connections == 0 || profile.connect_timeout_ms == 0 {
                return Err(ProfileError::Invalid(format!(
                    "profile {} has invalid connection limits",
                    profile.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredConnection {
    id: Uuid,
    name: String,
    kind: DatabaseKind,
    url: String,
    #[serde(default)]
    environment: ConnectionEnvironment,
    max_connections: u32,
    connect_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_key: Option<String>,
}

impl StoredConnection {
    fn into_public(self) -> SavedConnection {
        SavedConnection {
            id: self.id,
            name: self.name,
            kind: self.kind,
            url: self.url,
            environment: self.environment,
            max_connections: self.max_connections,
            connect_timeout_ms: self.connect_timeout_ms,
            secret_key: self.secret_key,
        }
    }
}

fn validate_name(name: &str) -> ProfileResult<()> {
    if name.trim().is_empty() {
        return Err(ProfileError::Invalid("profile name cannot be empty".into()));
    }
    Ok(())
}

fn keyring_key(id: Uuid) -> String {
    format!("profile-{id}")
}

fn scrub_url(raw: &str) -> ProfileResult<(String, Option<String>)> {
    if raw.trim().is_empty() {
        return Err(ProfileError::Invalid(
            "connection URL cannot be empty".into(),
        ));
    }
    let mut parsed = Url::parse(raw)
        .map_err(|error| ProfileError::Invalid(format!("invalid connection URL: {error}")))?;
    let secret = parsed.password().map(ToOwned::to_owned);
    if secret.is_some() {
        parsed
            .set_password(None)
            .map_err(|_| ProfileError::Invalid("connection URL has invalid userinfo".into()))?;
    }
    Ok((parsed.to_string(), secret))
}

fn add_password(raw: &str, secret: &str) -> ProfileResult<String> {
    let mut parsed = Url::parse(raw)
        .map_err(|error| ProfileError::Invalid(format!("invalid connection URL: {error}")))?;
    parsed
        .set_password(Some(secret))
        .map_err(|_| ProfileError::Invalid("connection URL has invalid userinfo".into()))?;
    Ok(parsed.to_string())
}

pub(crate) fn sqlite_url(path: &Path) -> String {
    if path == Path::new(":memory:") {
        return "sqlite::memory:".into();
    }
    // SQLx accepts sqlite://<path>; Url performs the required escaping for
    // spaces and other characters in a selected filename.
    let mut url = Url::parse("sqlite://").expect("sqlite is a valid URL scheme");
    url.set_path(&path.to_string_lossy());
    url.to_string()
}

#[cfg(test)]
fn sqlite_path_from_url(kind: DatabaseKind, raw: &str) -> Option<PathBuf> {
    if kind != DatabaseKind::SQLite {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    if !parsed.scheme().eq_ignore_ascii_case("sqlite") {
        return None;
    }
    if parsed.path() == ":memory:" || parsed.path() == "/:memory:" {
        return None;
    }
    if !parsed.path().is_empty() {
        let is_absolute = parsed.path().starts_with('/');
        let segments = parsed.path_segments()?.collect::<Vec<_>>().join("/");
        let decoded = percent_decode_str(&segments).decode_utf8_lossy();
        let path = if is_absolute {
            format!("/{decoded}")
        } else {
            decoded.into_owned()
        };
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    parsed.host_str().map(PathBuf::from)
}

fn default_profile_path() -> ProfileResult<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or(ProfileError::ConfigDirectoryUnavailable)?
        .join("dbx")
        .join(PROFILE_FILE_NAME))
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
        .and_then(|value| value.to_str())
        .unwrap_or(PROFILE_FILE_NAME);
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> io::Result<()> {
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

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
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
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeSecretStore {
        values: Mutex<HashMap<String, String>>,
    }

    impl SecretStore for FakeSecretStore {
        fn set(&self, key: &str, secret: &str) -> Result<(), SecretStoreError> {
            self.values
                .lock()
                .expect("fake store lock")
                .insert(key.to_owned(), secret.to_owned());
            Ok(())
        }

        fn get(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            Ok(self
                .values
                .lock()
                .expect("fake store lock")
                .get(key)
                .cloned())
        }

        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            self.values.lock().expect("fake store lock").remove(key);
            Ok(())
        }
    }

    fn test_store() -> (tempfile::TempDir, ProfileStore, Arc<FakeSecretStore>) {
        let directory = tempfile::tempdir().expect("temporary profile directory");
        let secrets = Arc::new(FakeSecretStore::default());
        let store = ProfileStore::with_secret_store(
            directory.path().join(PROFILE_FILE_NAME),
            secrets.clone(),
        );
        (directory, store, secrets)
    }

    #[test]
    fn saves_named_profile_without_writing_password_to_json() {
        let (_directory, store, secrets) = test_store();
        let draft = ConnectionProfileDraft::new(
            "Production",
            DatabaseKind::PostgreSQL,
            "postgres://alice:super-secret@example.test/app",
        );
        let saved = store.save(draft).expect("save profile");

        let json = fs::read_to_string(store.path()).expect("read profile file");
        assert!(json.contains("Production"));
        assert!(json.contains("alice@example.test"));
        assert!(!json.contains("super-secret"));
        assert!(saved.has_secret());
        assert_eq!(secrets.values.lock().unwrap().len(), 1);

        let loaded = store.load(saved.id).expect("load profile");
        assert_eq!(
            loaded.config.url,
            "postgres://alice:super-secret@example.test/app"
        );
        assert_eq!(loaded.config.max_connections, 8);
    }

    #[test]
    fn explicit_secret_is_removed_from_url_and_restored_on_load() {
        let (_directory, store, _secrets) = test_store();
        let draft = ConnectionProfileDraft::new(
            "Redis",
            DatabaseKind::Redis,
            "redis://default@example.test:6379",
        )
        .with_secret("token-value");
        let saved = store.save(draft).expect("save profile");
        let json = fs::read_to_string(store.path()).expect("read profile file");
        assert!(!json.contains("token-value"));
        assert_eq!(
            store.load(saved.id).unwrap().config.url,
            "redis://default:token-value@example.test:6379"
        );
    }

    #[test]
    fn sqlite_file_selection_round_trips_as_a_path() {
        let (_directory, store, _secrets) = test_store();
        let path = PathBuf::from("/tmp/dbx selected.sqlite");
        let saved = store
            .save(ConnectionProfileDraft::sqlite("Local SQLite", &path))
            .expect("save sqlite profile");
        assert_eq!(saved.sqlite_path(), Some(path));
        assert_eq!(saved.kind, DatabaseKind::SQLite);
        assert!(!saved.has_secret());
        assert_eq!(
            store.load(saved.id).unwrap().config.url,
            "sqlite:///tmp/dbx%20selected.sqlite"
        );
    }

    #[test]
    fn update_and_delete_remove_the_old_credential() {
        let (_directory, store, secrets) = test_store();
        let saved = store
            .save(
                ConnectionProfileDraft::new(
                    "Staging",
                    DatabaseKind::MySQL,
                    "mysql://user:old-secret@example.test/app",
                )
                .with_id(Uuid::new_v4()),
            )
            .expect("save profile");
        let key = keyring_key(saved.id);
        assert_eq!(secrets.get(&key).unwrap(), Some("old-secret".into()));

        let updated = store
            .save(
                ConnectionProfileDraft::new(
                    "Staging renamed",
                    DatabaseKind::MySQL,
                    "mysql://user:new-secret@example.test/app",
                )
                .with_id(saved.id),
            )
            .expect("update profile");
        assert_eq!(updated.id, saved.id);
        assert_eq!(secrets.get(&key).unwrap(), Some("new-secret".into()));
        assert_eq!(store.list().unwrap()[0].name, "Staging renamed");

        assert!(store.delete(saved.id).unwrap());
        assert!(secrets.get(&key).unwrap().is_none());
        assert!(store.list().unwrap().is_empty());
        assert!(!store.delete(saved.id).unwrap());
    }

    #[test]
    fn password_free_profile_edit_retains_the_keyring_credential() {
        let (_directory, store, secrets) = test_store();
        let saved = store
            .save(ConnectionProfileDraft::new(
                "Production",
                DatabaseKind::PostgreSQL,
                "postgres://alice:durable-secret@example.test/app",
            ))
            .expect("save profile");

        let updated = store
            .save(
                ConnectionProfileDraft::new(
                    "Production renamed",
                    DatabaseKind::PostgreSQL,
                    saved.url.clone(),
                )
                .with_id(saved.id),
            )
            .expect("update password-free profile");

        assert!(updated.has_secret());
        assert_eq!(
            secrets.get(&keyring_key(saved.id)).unwrap(),
            Some("durable-secret".into())
        );
        assert_eq!(
            store.load(saved.id).unwrap().config.url,
            "postgres://alice:durable-secret@example.test/app"
        );
    }

    #[test]
    fn environment_labels_round_trip_and_default_for_older_documents() {
        let (_directory, store, _secrets) = test_store();
        let saved = store
            .save(
                ConnectionProfileDraft::new(
                    "Prod",
                    DatabaseKind::PostgreSQL,
                    "postgres://alice@example.test/app",
                )
                .with_environment(ConnectionEnvironment::Production),
            )
            .expect("save profile");
        assert_eq!(saved.environment, ConnectionEnvironment::Production);
        assert_eq!(
            store.list().unwrap()[0].environment,
            ConnectionEnvironment::Production
        );

        // A document written before environments existed still loads, and the
        // connection defaults to Local.
        let id = Uuid::new_v4();
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        let json = format!(
            r#"{{"version":1,"connections":[{{"id":"{id}","name":"Legacy","kind":"postgresql","url":"postgres://u@example.test/db","max_connections":8,"connect_timeout_ms":10000}}]}}"#
        );
        fs::write(store.path(), json).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed[0].environment, ConnectionEnvironment::Local);
    }

    #[test]
    fn document_version_is_checked_before_use() {
        let (_directory, store, _secrets) = test_store();
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), r#"{"version":99,"connections":[]}"#).unwrap();
        assert!(matches!(
            store.list(),
            Err(ProfileError::UnsupportedVersion {
                found: 99,
                expected: PROFILE_FILE_VERSION
            })
        ));
    }

    #[test]
    fn no_password_in_json_is_rejected_if_file_was_manually_changed() {
        let (_directory, store, _secrets) = test_store();
        let id = Uuid::new_v4();
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        let json = format!(
            r#"{{"version":1,"connections":[{{"id":"{id}","name":"Unsafe","kind":"postgresql","url":"postgres://u:p@example.test/db","max_connections":8,"connect_timeout_ms":10000}}]}}"#
        );
        fs::write(store.path(), json).unwrap();
        assert!(
            matches!(store.list(), Err(ProfileError::Invalid(message)) if message.contains("password"))
        );
    }
}
