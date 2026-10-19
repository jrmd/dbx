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

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Settings {
    pub version: u32,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub theme: crate::theme::catalog::ThemePreferences,
    /// Replace glass materials with opaque surfaces.
    #[serde(default)]
    pub reduce_transparency: bool,
    #[serde(default)]
    pub motion: crate::motion::MotionPreference,
    #[serde(default = "text_scale_default")]
    pub ui_scale: u16,
    #[serde(default = "text_scale_default")]
    pub editor_scale: u16,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub mono_font_family: Option<String>,
    #[serde(default = "font_size_default")]
    pub font_size: u16,
    /// Older settings used font_size for both families.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mono_font_size: Option<u16>,
    #[serde(default = "remember_device_default")]
    pub drag_row_selection: bool,
    /// Keep the vault key in the system keychain so launches skip the
    /// passphrase.
    #[serde(default = "remember_device_default")]
    pub remember_device: bool,
    #[serde(default)]
    pub agents: crate::agents::AgentPreferences,
    /// Use modal Vim bindings in SQL query editors only.
    #[serde(default)]
    pub vim_mode: bool,
    /// Dragged widths of the explorer sidebar and row inspector, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explorer_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspector_width: Option<u32>,
}

fn font_size_default() -> u16 {
    12
}

fn text_scale_default() -> u16 {
    100
}

fn remember_device_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self::new(Appearance::System)
    }
}

impl Settings {
    pub fn new(appearance: Appearance) -> Self {
        Self {
            version: SETTINGS_FILE_VERSION,
            appearance,
            theme: Default::default(),
            reduce_transparency: false,
            motion: crate::motion::MotionPreference::System,
            ui_scale: 100,
            editor_scale: 100,
            font_family: None,
            mono_font_family: None,
            font_size: 12,
            mono_font_size: None,
            drag_row_selection: true,
            remember_device: true,
            agents: crate::agents::AgentPreferences::default(),
            vim_mode: false,
            explorer_width: None,
            inspector_width: None,
        }
    }

    pub fn with_reduce_transparency(mut self, reduce: bool) -> Self {
        self.reduce_transparency = reduce;
        self
    }

    pub fn with_remember_device(mut self, remember: bool) -> Self {
        self.remember_device = remember;
        self
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
    Ok(crate::config::directory()
        .ok_or(SettingsError::ConfigDirectoryUnavailable)?
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
        // The new content is already in place; a failed directory sync only
        // weakens durability and must not report the save as failed.
        let _ = sync_directory(parent);
        Ok(())
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
    fn legacy_settings_migrate_and_preferences_survive_restart() {
        let (_directory, store) = test_store();
        fs::write(&store.path, r#"{"version":1,"appearance":"dark"}"#).unwrap();
        let mut settings = store.load().unwrap();
        assert_eq!(settings.appearance, Appearance::Dark);
        assert!(!settings.reduce_transparency);
        assert_eq!(settings.ui_scale, 100);
        assert_eq!(settings.motion, crate::motion::MotionPreference::System);
        assert_eq!(settings.editor_scale, 100);
        assert_eq!(settings.theme, Default::default());
        settings.theme.preset = crate::theme::catalog::Preset::Ocean;
        settings
            .theme
            .edit("grid_active", "#aabbcc", false)
            .unwrap();
        assert_eq!(settings.font_size, 12);
        assert!(settings.font_family.is_none());
        assert!(settings.mono_font_family.is_none());
        assert!(settings.drag_row_selection);
        settings.font_family = Some("Helvetica".into());
        settings.mono_font_family = Some("Menlo".into());
        settings.font_size = 16;
        settings.mono_font_size = Some(14);
        settings.drag_row_selection = false;
        store.save(settings.clone()).unwrap();
        assert_eq!(
            SettingsStore::at(store.path.clone()).load().unwrap(),
            settings
        );
    }
}
