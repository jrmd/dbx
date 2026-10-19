//! All user state shares one root, including isolated native QA profiles.
use std::path::PathBuf;

pub fn override_directory() -> Option<PathBuf> {
    std::env::var_os("DBX_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn directory() -> Option<PathBuf> {
    override_directory().or_else(|| dirs::config_dir().map(|path| path.join("dbx")))
}
