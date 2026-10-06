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
    Structure(dbx_core::TableRef),
    Diagram,
}
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct WorkspaceDocument {
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
            .get("workspace-startup-v1")
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
                "workspace-startup-v1",
                serde_json::to_string(document).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
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
