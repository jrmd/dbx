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
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct WorkspaceDocument {
    #[serde(default)]
    pub schema_baseline: Option<dbx_core::RelationalSchema>,
    pub drafts: Vec<SavedQuery>,
    pub saved: Vec<SavedQuery>,
    pub closed: Vec<String>,
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
