//! Optional "unlock on this device" for the DBX Vault.
//!
//! The vault's derived key (never the passphrase) is kept in the operating
//! system's credential store: the login Keychain on macOS, the Secret Service
//! on Linux. The store only releases it to the logged-in user, and on macOS
//! only to builds carrying the same code signature, so a stable local signing
//! identity keeps it from prompting after every rebuild. The key opens this
//! one vault; a recreated vault gets a new salt and a new key.

use std::path::Path;

use keyring::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretBox};
use zeroize::Zeroize;

const SERVICE: &str = "dev.jrmd.dbx.vault";
const KEY_LEN: usize = 32;

pub type VaultKey = SecretBox<[u8; KEY_LEN]>;

#[derive(Clone, Debug)]
pub struct DeviceUnlock {
    account: String,
}

impl DeviceUnlock {
    /// One keychain item per vault file, so the disposable QA vault never
    /// shares an entry with the real one.
    pub fn for_vault(vault_path: &Path) -> Self {
        Self {
            account: format!("vault-key:{}", vault_path.display()),
        }
    }

    fn entry(&self) -> Result<Entry, String> {
        Entry::new(SERVICE, &self.account).map_err(describe)
    }

    /// The remembered key, or `None` when this device has not been trusted.
    pub fn load(&self) -> Result<Option<VaultKey>, String> {
        let mut bytes = match self.entry()?.get_secret() {
            Ok(bytes) => bytes,
            Err(KeyringError::NoEntry) => return Ok(None),
            Err(error) => return Err(describe(error)),
        };
        let key = <[u8; KEY_LEN]>::try_from(bytes.as_slice()).ok();
        bytes.zeroize();
        match key {
            Some(key) => Ok(Some(SecretBox::new(Box::new(key)))),
            // Not something DBX wrote; drop it rather than retrying forever.
            None => self.forget().map(|()| None),
        }
    }

    pub fn store(&self, key: &VaultKey) -> Result<(), String> {
        self.entry()?
            .set_secret(key.expose_secret())
            .map_err(describe)
    }

    pub fn forget(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(describe(error)),
        }
    }
}

/// Never formats the error itself: encoding variants carry the stored bytes.
fn describe(error: KeyringError) -> String {
    match error {
        KeyringError::NoDefaultStore | KeyringError::NoStorageAccess(_) => {
            "The system keychain is unavailable".into()
        }
        _ => "The system keychain couldn’t store the vault key".into(),
    }
}
