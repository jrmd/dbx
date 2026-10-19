//! An explicitly selected plaintext SQL library, separate from encrypted vault saves.
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_QUERY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FILES: usize = 500;
const FAVORITES_FILE: &str = ".dbx-favorites.json";

#[derive(Clone)]
pub struct SqlFileEntry {
    pub relative_path: PathBuf,
    pub favorite: bool,
}
impl SqlFileEntry {
    pub fn label(&self) -> String {
        self.relative_path.to_string_lossy().into_owned()
    }
}

type CachedEntries = Option<Result<Vec<SqlFileEntry>, String>>;

#[derive(Clone)]
pub struct SqlLibrary {
    root: PathBuf,
    preferences: PathBuf,
    cache: RefCell<CachedEntries>,
}
#[derive(Deserialize, Serialize)]
struct Preferences {
    directory: PathBuf,
}

impl Default for SqlLibrary {
    fn default() -> Self {
        let base = crate::config::directory().unwrap_or_else(|| PathBuf::from("."));
        let preferences = base.join("sql-library.json");
        let default_root = crate::config::override_directory()
            .map(|path| path.join("queries"))
            .unwrap_or_else(|| {
                dirs::document_dir()
                    .map(|path| path.join("DBX/queries"))
                    .unwrap_or_else(|| base.join("queries"))
            });
        let root = read_bounded(&preferences, 64 * 1024)
            .ok()
            .filter(|bytes| bytes.len() < 64 * 1024)
            .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
            .map(|prefs| prefs.directory)
            .unwrap_or(default_root);
        Self {
            root,
            preferences,
            cache: Default::default(),
        }
    }
}

impl SqlLibrary {
    #[cfg(test)]
    pub fn at(root: PathBuf) -> Self {
        Self {
            preferences: root.join(".test-library-preferences.json"),
            root,
            cache: Default::default(),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn choose_directory(&mut self, directory: PathBuf) -> Result<(), String> {
        if !directory.is_dir() {
            return Err("Choose an existing SQL library folder".into());
        }
        let directory = directory
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if let Some(parent) = self.preferences.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(&Preferences {
            directory: directory.clone(),
        })
        .map_err(|error| error.to_string())?;
        dbx_core::write_atomic_export(&self.preferences, &bytes)
            .map_err(|error| error.to_string())?;
        self.root = directory;
        self.invalidate();
        Ok(())
    }
    fn favorites(&self) -> Result<BTreeSet<PathBuf>, String> {
        match read_bounded(&self.root.join(FAVORITES_FILE), 256 * 1024) {
            Ok(bytes) if bytes.len() <= 256 * 1024 => {
                serde_json::from_slice(&bytes).map_err(|_| "Invalid SQL favorites file".into())
            }
            Ok(_) => Err("SQL favorites file exceeds 256 KiB".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
            Err(error) => Err(error.to_string()),
        }
    }
    pub fn invalidate(&self) {
        *self.cache.borrow_mut() = None;
    }
    /// Render-safe snapshot. Filesystem access belongs in a background refresh.
    pub fn cached_entries(&self) -> Result<Vec<SqlFileEntry>, String> {
        self.cache
            .borrow()
            .clone()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
    pub fn needs_refresh(&self) -> bool {
        self.cache.borrow().is_none()
    }
    pub fn cache_entries(&self, entries: Result<Vec<SqlFileEntry>, String>) {
        *self.cache.borrow_mut() = Some(entries);
    }
    /// Explicit scan; callers in the application run this on a worker thread.
    pub fn entries(&self) -> Result<Vec<SqlFileEntry>, String> {
        self.scan()
    }
    fn scan(&self) -> Result<Vec<SqlFileEntry>, String> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let favorites = self.favorites()?;
        let mut entries = Vec::new();
        let mut pending = vec![(self.root.clone(), 0)];
        let mut directories = 0;
        let mut scanned_entries = 0;
        while let Some((directory, depth)) = pending.pop() {
            directories += 1;
            if directories > 1000 {
                return Err("SQL library exceeds 1000 folders; choose a smaller folder".into());
            }
            for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
                scanned_entries += 1;
                if scanned_entries > 10_000 {
                    return Err(
                        "SQL library exceeds 10000 directory entries; choose a smaller folder"
                            .into(),
                    );
                }
                let entry = entry.map_err(|error| error.to_string())?;
                let kind = entry.file_type().map_err(|error| error.to_string())?;
                let path = entry.path();
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir()
                    && depth < 8
                    && !entry.file_name().to_string_lossy().starts_with('.')
                {
                    pending.push((path, depth + 1));
                } else if kind.is_file() && is_sql(&path) {
                    let relative_path = path
                        .strip_prefix(&self.root)
                        .map_err(|_| "Invalid library path")?
                        .to_owned();
                    entries.push(SqlFileEntry {
                        favorite: favorites.contains(&relative_path),
                        relative_path,
                    });
                    if entries.len() > MAX_FILES {
                        return Err("SQL library exceeds 500 files; choose a smaller folder".into());
                    }
                }
            }
        }
        entries.sort_by_key(|entry| (!entry.favorite, entry.label().to_lowercase()));
        Ok(entries)
    }
    fn resolve(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
            || !is_sql(relative)
        {
            return Err("SQL library entries must be relative .sql paths".into());
        }
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let path = root
            .join(relative)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !path.starts_with(root) || !path.is_file() {
            return Err("SQL library file is outside its folder".into());
        }
        Ok(path)
    }
    pub fn read(&self, relative: &Path) -> Result<String, String> {
        read_sql(&self.resolve(relative)?)
    }
    pub fn toggle_favorite(&self, relative: &Path) -> Result<(), String> {
        self.resolve(relative)?;
        let mut favorites = self.favorites()?;
        if !favorites.remove(relative) {
            favorites.insert(relative.to_owned());
        }
        let bytes = serde_json::to_vec_pretty(&favorites).map_err(|error| error.to_string())?;
        dbx_core::write_atomic_export(&self.root.join(FAVORITES_FILE), &bytes)
            .map_err(|error| error.to_string())?;
        self.invalidate();
        Ok(())
    }
}

fn read_bounded(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SQL library metadata exceeds its size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SQL library metadata exceeds its size limit",
        ));
    }
    Ok(bytes)
}

fn is_sql(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
}

pub fn read_sql(path: &Path) -> Result<String, String> {
    if !is_sql(path) {
        return Err("Choose a .sql file".into());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_QUERY_BYTES {
        return Err("SQL file exceeds 4 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_QUERY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_QUERY_BYTES {
        return Err("SQL file exceeds 4 MiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "SQL files must be UTF-8".into())
}

pub fn write_sql(path: &Path, sql: &str) -> Result<(), String> {
    if !is_sql(path) {
        return Err("Choose a filename ending in .sql".into());
    }
    if sql.len() as u64 > MAX_QUERY_BYTES {
        return Err("SQL file exceeds 4 MiB".into());
    }
    dbx_core::write_atomic_export(path, sql.as_bytes()).map_err(|error| error.to_string())
}

pub fn suggested_filename(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    format!(
        "{}.sql",
        if name.trim().is_empty() {
            "query"
        } else {
            name.trim()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_are_plain_git_usable_and_favorites_survive_reload() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("queries");
        fs::create_dir_all(root.join("reports")).unwrap();
        let mut library = SqlLibrary {
            root: root.clone(),
            preferences: directory.path().join("config/library.json"),
            cache: Default::default(),
        };
        library.choose_directory(root.clone()).unwrap();
        write_sql(&root.join("reports/count.sql"), "SELECT 42;\n").unwrap();
        assert_eq!(
            fs::read_to_string(root.join("reports/count.sql")).unwrap(),
            "SELECT 42;\n"
        );
        library
            .toggle_favorite(Path::new("reports/count.sql"))
            .unwrap();
        assert!(library.entries().unwrap()[0].favorite);
        let reload = SqlLibrary {
            root,
            preferences: library.preferences.clone(),
            cache: Default::default(),
        };
        assert_eq!(
            reload.read(Path::new("reports/count.sql")).unwrap(),
            "SELECT 42;\n"
        );
        assert!(reload.entries().unwrap()[0].favorite);
        reload
            .toggle_favorite(Path::new("reports/count.sql"))
            .unwrap();
        assert!(!reload.entries().unwrap()[0].favorite);
        assert!(reload.read(Path::new("../elsewhere.sql")).is_err());
    }
    #[test]
    fn rendering_reads_only_the_last_background_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let library = SqlLibrary {
            root: directory.path().join("does-not-exist"),
            preferences: directory.path().join("prefs.json"),
            cache: Default::default(),
        };
        assert!(library.needs_refresh());
        library.cache_entries(Ok(vec![SqlFileEntry {
            relative_path: "saved.sql".into(),
            favorite: true,
        }]));
        assert_eq!(library.cached_entries().unwrap()[0].label(), "saved.sql");
        assert!(!library.needs_refresh());
        library.invalidate();
        assert!(library.cached_entries().unwrap().is_empty());
        assert!(library.needs_refresh());
    }
    #[cfg(unix)]
    #[test]
    fn library_skips_and_rejects_symlink_escapes() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_sql(&outside.path().join("secret.sql"), "SELECT 'private'").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.sql"),
            directory.path().join("linked.sql"),
        )
        .unwrap();
        let library = SqlLibrary {
            root: directory.path().into(),
            preferences: directory.path().join("prefs.json"),
            cache: Default::default(),
        };
        assert!(library.entries().unwrap().is_empty());
        assert!(library.read(Path::new("linked.sql")).is_err());
    }
}
