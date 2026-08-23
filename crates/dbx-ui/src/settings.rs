//! Versioned, durable user preferences for the DBX desktop client.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::theme::Appearance;

/// Version of the on-disk settings document.
pub const SETTINGS_FILE_VERSION: u32 = 1;
/// Settings file stored beneath DBX's platform config directory.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Settings {
    pub version: u32,
    #[serde(default)]
    pub appearance: Appearance,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new(Appearance::Dark)
    }
}

impl Settings {
    pub fn new(appearance: Appearance) -> Self {
        Self {
            version: SETTINGS_FILE_VERSION,
            appearance,
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings storage I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid settings JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("platform configuration directory is unavailable")]
    ConfigDirectoryUnavailable,
    #[error("unsupported settings document version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
}

pub type SettingsResult<T> = Result<T, SettingsError>;

/// File-backed settings repository. Each operation reads or writes one small,
/// versioned document so a partial write cannot leave invalid preferences.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new() -> SettingsResult<Self> {
        Ok(Self::at(default_settings_path()?))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> SettingsResult<Settings> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let settings = serde_json::from_slice::<Settings>(&bytes)?;
                if settings.version != SETTINGS_FILE_VERSION {
                    return Err(SettingsError::UnsupportedVersion {
                        found: settings.version,
                        expected: SETTINGS_FILE_VERSION,
                    });
                }
                Ok(settings)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, settings: Settings) -> SettingsResult<()> {
        if settings.version != SETTINGS_FILE_VERSION {
            return Err(SettingsError::UnsupportedVersion {
                found: settings.version,
                expected: SETTINGS_FILE_VERSION,
            });
        }
        atomic_write(&self.path, &serde_json::to_vec_pretty(&settings)?)?;
        Ok(())
    }
}

fn default_settings_path() -> SettingsResult<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or(SettingsError::ConfigDirectoryUnavailable)?
        .join("dbx")
        .join(SETTINGS_FILE_NAME))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SETTINGS_FILE_NAME);
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
        replace_file(&temporary_path, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if destination.exists() => {
            fs::remove_file(destination)?;
            fs::rename(source, destination)
        }
        Err(error) => Err(error),
    }
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

    fn test_store() -> (tempfile::TempDir, SettingsStore) {
        let directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(directory.path().join(SETTINGS_FILE_NAME));
        (directory, store)
    }

    #[test]
    fn missing_settings_default_to_dark() {
        let (_directory, store) = test_store();
        assert_eq!(store.load().expect("load defaults"), Settings::default());
        assert_eq!(Settings::default().appearance, Appearance::Dark);
    }

    #[test]
    fn saves_and_reloads_appearance_atomically() {
        let (_directory, store) = test_store();
        let settings = Settings::new(Appearance::Light);
        store.save(settings).expect("save settings");

        assert_eq!(store.load().expect("reload settings"), settings);
        let json = fs::read_to_string(&store.path).expect("read settings file");
        assert!(json.contains("\"version\": 1"));
        assert!(json.contains("\"appearance\": \"light\""));
        assert!(!store.path.with_extension("tmp").exists());
    }

    #[test]
    fn replaces_an_existing_settings_document() {
        let (_directory, store) = test_store();
        store
            .save(Settings::new(Appearance::Light))
            .expect("save light appearance");
        store
            .save(Settings::new(Appearance::Dark))
            .expect("replace appearance");

        assert_eq!(
            store.load().expect("reload replacement").appearance,
            Appearance::Dark
        );
    }

    #[test]
    fn rejects_unknown_document_versions() {
        let (_directory, store) = test_store();
        fs::write(&store.path, r#"{"version":2,"appearance":"dark"}"#)
            .expect("write future settings");

        assert!(matches!(
            store.load(),
            Err(SettingsError::UnsupportedVersion {
                found: 2,
                expected: 1
            })
        ));
    }
}
