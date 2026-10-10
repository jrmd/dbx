//! Drafts and named queries share the vault's encryption and lock lifecycle.
use crate::{query_history::QueryHistoryConnection, vault::CredentialVault};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct SavedQuery {
    pub name: String,
    pub sql: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", content = "document")]
pub enum SavedTab {
    Query(SavedQuery),
    Data(dbx_core::TableRef),
    RecoveredData(SavedChangeset),
    Structure(dbx_core::TableRef),
    Diagram,
}
/// Recovery stores identities and originals, never page row indices.
#[derive(Clone, Deserialize, Serialize)]
pub struct SavedChangeset {
    #[serde(default)]
    pub connection_identity: Option<[u8; 32]>,
    pub table: dbx_core::TableRef,
    pub database: Option<String>,
    pub columns: Vec<dbx_core::ColumnInfo>,
    pub changes: Vec<dbx_core::RowChange>,
}
impl SavedChangeset {
    pub async fn validate(
        &self,
        engine: &dbx_core::DatabaseEngine,
        current_target: Option<[u8; 32]>,
    ) -> dbx_core::Result<()> {
        if self.connection_identity.is_none() || self.connection_identity != current_target {
            return Err(dbx_core::DbxError::Query("Recovered draft belongs to different connection settings. Review or discard it; it cannot be applied to this target.".into()));
        }
        if self
            .database
            .as_ref()
            .is_some_and(|database| database.is_empty())
        {
            return Err(dbx_core::DbxError::Query(
                "Recovered changeset has no database identity".into(),
            ));
        }
        if self.database.as_ref() != Some(&engine.current_database().await?)
            || engine.describe_table(&self.table).await? != self.columns
            || self
                .changes
                .iter()
                .any(|change| change.table() != &self.table)
        {
            return Err(dbx_core::DbxError::Query("Recovered changeset no longer matches this database and column schema. Review the draft and discard it before editing again.".into()));
        }
        Ok(())
    }
}

/// How the user arranged one table's grid. Columns are named, so a layout
/// survives added or dropped columns.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TableLayout {
    #[serde(default)]
    pub widths: std::collections::BTreeMap<String, f32>,
    #[serde(default)]
    pub hidden: std::collections::BTreeSet<String>,
    /// Columns pinned to the left edge, in pin order.
    #[serde(default)]
    pub pinned: Vec<String>,
    /// Display order of the unpinned columns. Unlisted columns follow in
    /// table order.
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub saved_filters: Vec<SavedFilter>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedFilter {
    pub name: String,
    pub filters: Vec<dbx_core::Filter>,
}

/// The key for a table's layout within its connection's workspace.
pub fn table_layout_key(table: &dbx_core::TableRef) -> String {
    match &table.schema {
        Some(schema) => format!("{schema}.{}", table.name),
        None => table.name.clone(),
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct WorkspaceDocument {
    /// History is plaintext; opt-out and retention are persisted per identity.
    #[serde(default)]
    pub history_disabled: bool,
    #[serde(default = "default_history_retention")]
    pub history_retention: usize,
    /// Per-table grid layouts and saved filters, keyed by [`table_layout_key`].
    #[serde(default)]
    pub table_layouts: HashMap<String, TableLayout>,
    #[serde(default)]
    pub schema_baseline: Option<dbx_core::RelationalSchema>,
    pub drafts: Vec<SavedQuery>,
    pub saved: Vec<SavedQuery>,
    pub closed: Vec<String>,
    /// Tables open in data tabs, reopened on the next connection.
    #[serde(default)]
    pub open_tables: Vec<dbx_core::TableRef>,
    /// Ordered documents and the selected document within this connection.
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active_tab: Option<usize>,
    #[serde(default)]
    pub current_database: Option<String>,
}
impl Default for WorkspaceDocument {
    fn default() -> Self {
        Self {
            history_disabled: false,
            history_retention: 100,
            table_layouts: HashMap::new(),
            schema_baseline: None,
            drafts: Vec::new(),
            saved: Vec::new(),
            closed: Vec::new(),
            open_tables: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            current_database: None,
        }
    }
}
fn default_history_retention() -> usize {
    100
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct StartupWorkspace {
    pub connections: Vec<StartupConnection>,
    pub active: Option<usize>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct StartupConnection {
    pub profile_id: uuid::Uuid,
    pub database: Option<String>,
    #[serde(default)]
    pub tabs: Option<Vec<SavedTab>>,
    #[serde(default)]
    pub active_tab: Option<usize>,
}
pub fn connection_key(connection: &QueryHistoryConnection) -> String {
    let identity = serde_json::to_vec(connection).unwrap_or_default();
    format!("workspace-v1-{:x}", Sha256::digest(identity))
}
const STARTUP_KEY: &str = "workspace-startup-v1";
pub struct WorkspaceStore {
    vault: Arc<CredentialVault>,
    revisions: Mutex<HashMap<String, u64>>,
}
impl WorkspaceStore {
    pub fn new(vault: Arc<CredentialVault>) -> Self {
        Self {
            vault,
            revisions: Mutex::new(HashMap::new()),
        }
    }
    pub fn matches_vault(&self, vault: &Arc<CredentialVault>) -> bool {
        Arc::ptr_eq(&self.vault, vault)
    }
    pub fn load_startup(&self) -> Result<StartupWorkspace, String> {
        match self
            .vault
            .get(STARTUP_KEY)
            .map_err(|error| error.to_string())?
        {
            None => Ok(StartupWorkspace::default()),
            Some(secret) => serde_json::from_str(secret.expose_secret())
                .map_err(|_| "Saved connections could not be decoded".into()),
        }
    }
    pub fn save_startup(&self, document: &StartupWorkspace) -> Result<(), String> {
        self.vault
            .set(
                STARTUP_KEY,
                serde_json::to_string(document).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
    }
    /// Claims the next startup-index revision. Only the newest claim may
    /// write, so a delayed save can never overwrite a later layout change.
    pub fn register_startup(&self) -> u64 {
        self.register(STARTUP_KEY)
    }

    /// Whether `revision` is still the newest claim for `key`.
    pub fn is_current(&self, key: &str, revision: u64) -> bool {
        self.revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            == Some(&revision)
    }

    pub fn save_startup_at(
        &self,
        revision: u64,
        document: &StartupWorkspace,
    ) -> Result<(), String> {
        let revisions = self
            .revisions
            .lock()
            .map_err(|_| "Workspace save lock failed")?;
        if revisions.get(STARTUP_KEY) != Some(&revision) {
            return Ok(());
        }
        self.save_startup(document)
    }

    pub fn load(&self, key: &str) -> Result<WorkspaceDocument, String> {
        match self.vault.get(key).map_err(|error| error.to_string())? {
            None => Ok(WorkspaceDocument::default()),
            Some(secret) => serde_json::from_str(secret.expose_secret())
                .map_err(|_| "Saved query workspace could not be decoded".into()),
        }
    }
    pub fn register(&self, key: &str) -> u64 {
        let mut revisions = self
            .revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let revision = revisions.entry(key.to_owned()).or_default();
        *revision = revision.saturating_add(1);
        *revision
    }
    pub fn save(
        &self,
        key: &str,
        revision: u64,
        document: &WorkspaceDocument,
    ) -> Result<(), String> {
        let revisions = self
            .revisions
            .lock()
            .map_err(|_| "Workspace save lock failed")?;
        if revisions.get(key) != Some(&revision) {
            return Ok(());
        }
        let serialized =
            serde_json::to_string(document).map_err(|_| "Workspace serialization failed")?;
        if serialized.len() > 2 * 1024 * 1024 {
            return Err("Query workspace exceeds 2 MiB; save large scripts to files or remove saved queries".into());
        }
        self.vault
            .set(key, serialized)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_preserves_separate_layouts_for_instances_of_the_same_profile() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Arc::new(CredentialVault::at(directory.path().join("vault")));
        vault.create("fixture passphrase").unwrap();
        let store = WorkspaceStore::new(vault.clone());
        let profile_id = uuid::Uuid::new_v4();
        let connection = |name: &str| StartupConnection {
            profile_id,
            database: Some("app".into()),
            tabs: Some(vec![
                SavedTab::Query(SavedQuery {
                    name: name.into(),
                    sql: format!("SELECT '{name}'"),
                }),
                SavedTab::Data(dbx_core::TableRef::new("items")),
            ]),
            active_tab: Some(1),
        };
        store
            .save_startup(&StartupWorkspace {
                connections: vec![
                    connection("first private draft"),
                    connection("second private draft"),
                ],
                active: Some(1),
            })
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&std::fs::read(vault.path()).unwrap())
                .contains("private draft")
        );
        vault.lock().unwrap();
        vault.unlock("fixture passphrase").unwrap();
        let reopened = WorkspaceStore::new(vault).load_startup().unwrap();
        assert_eq!(reopened.active, Some(1));
        assert_eq!(reopened.connections.len(), 2);
        for (index, expected) in ["first private draft", "second private draft"]
            .iter()
            .enumerate()
        {
            assert_eq!(reopened.connections[index].active_tab, Some(1));
            let SavedTab::Query(query) = &reopened.connections[index].tabs.as_ref().unwrap()[0]
            else {
                panic!("query layout lost")
            };
            assert_eq!(&query.name, expected);
        }
    }

    #[test]
    fn drafts_are_encrypted_recoverable_and_old_saves_cannot_overwrite_new_work() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Arc::new(CredentialVault::at(directory.path().join("vault")));
        vault.create("workspace passphrase").unwrap();
        let store = WorkspaceStore::new(vault.clone());
        let old = store.register("workspace");
        let new = store.register("workspace");
        let document = WorkspaceDocument {
            drafts: vec![SavedQuery {
                name: "Private investigation".into(),
                sql: "SELECT 'sensitive literal'".into(),
            }],
            ..Default::default()
        };
        store.save("workspace", new, &document).unwrap();
        store
            .save("workspace", old, &WorkspaceDocument::default())
            .unwrap();
        let bytes = std::fs::read(vault.path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sensitive literal"));
        vault.lock().unwrap();
        assert!(store.load("workspace").is_err());
        vault.unlock("workspace passphrase").unwrap();
        assert_eq!(
            store.load("workspace").unwrap().drafts[0].sql,
            "SELECT 'sensitive literal'"
        );
    }
}

#[cfg(test)]
mod changeset_tests {
    use super::*;
    #[tokio::test]
    async fn encrypted_changesets_recover_without_execution_and_reject_schema_drift() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Arc::new(CredentialVault::at(directory.path().join("vault")));
        vault.create("fixture vault passphrase").unwrap();
        let store = WorkspaceStore::new(vault.clone());
        let engine = dbx_core::DatabaseEngine::connect(dbx_core::ConnectionConfig::new(
            dbx_core::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT)")
            .await
            .unwrap();
        let table = dbx_core::TableRef::new("items");
        let saved = SavedChangeset {
            connection_identity: Some([1; 32]),
            table: table.clone(),
            database: Some(engine.current_database().await.unwrap()),
            columns: engine.describe_table(&table).await.unwrap(),
            changes: vec![dbx_core::RowChange::Insert(
                dbx_core::InsertRequest::from_row(
                    table.clone(),
                    vec![
                        ("id".into(), dbx_core::CellValue::Integer(1)),
                        (
                            "value".into(),
                            dbx_core::CellValue::Text("private staged data".into()),
                        ),
                    ],
                ),
            )],
        };
        let document = WorkspaceDocument {
            tabs: vec![SavedTab::RecoveredData(saved.clone())],
            ..Default::default()
        };
        let revision = store.register("fixture");
        store.save("fixture", revision, &document).unwrap();
        vault.lock().unwrap();
        vault.unlock("fixture vault passphrase").unwrap();
        let recovered = store.load("fixture").unwrap();
        let SavedTab::RecoveredData(recovered) = &recovered.tabs[0] else {
            panic!("missing changeset");
        };
        assert_eq!(recovered.changes, saved.changes);
        recovered.validate(&engine, Some([1; 32])).await.unwrap();
        assert!(recovered.validate(&engine, Some([2; 32])).await.is_err());
        assert_eq!(engine.count_rows(&table, &[], None).await.unwrap(), 0);
        assert!(
            !String::from_utf8_lossy(&std::fs::read(directory.path().join("vault")).unwrap())
                .contains("private staged data")
        );
        engine
            .execute_sql("ALTER TABLE items ADD COLUMN other TEXT")
            .await
            .unwrap();
        assert!(recovered.validate(&engine, Some([1; 32])).await.is_err());
    }

    #[test]
    fn a_delayed_startup_save_never_overwrites_a_newer_one() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Arc::new(CredentialVault::at(directory.path().join("vault")));
        vault.create("fixture passphrase").unwrap();
        let store = WorkspaceStore::new(vault);
        let layout = |active| StartupWorkspace {
            connections: Vec::new(),
            active,
        };
        let older = store.register_startup();
        let newer = store.register_startup();
        store.save_startup_at(newer, &layout(Some(2))).unwrap();
        store.save_startup_at(older, &layout(Some(1))).unwrap();
        assert_eq!(store.load_startup().unwrap().active, Some(2));
        assert!(store.is_current(STARTUP_KEY, newer));
        assert!(!store.is_current(STARTUP_KEY, older));
    }
}
