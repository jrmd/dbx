//! Passphrase-encrypted, local credential vault.
//!
//! The on-disk document deliberately has a small authenticated binary header
//! followed by an XChaCha20-Poly1305 ciphertext.  Profile metadata never
//! enters this file and credential plaintext never enters profile JSON.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroize;

const MAGIC: &[u8; 8] = b"DBXVAULT";
const FORMAT_VERSION: u8 = 1;
const KDF_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const HEADER_LEN: usize = 8 + 1 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_LEN;
const MEMORY_KIB: u32 = 64 * 1024;
const PASSES: u32 = 3;
const PARALLELISM: u32 = 1;
const MAX_VAULT_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultState {
    Uninitialized,
    Locked,
    Unlocked,
}

#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum VaultError {
    #[error("credential vault is not initialized")]
    Uninitialized,
    #[error("credential vault is locked")]
    Locked,
    #[error("credential vault authentication failed")]
    Authentication,
    #[error("credential vault I/O error: {0}")]
    Io(String),
    #[error("credential vault data is invalid")]
    Invalid,
}

type VaultResult<T> = Result<T, VaultError>;

/// A small, synchronous credential store intended to run off the render loop.
#[derive(Clone)]
pub struct CredentialVault {
    path: PathBuf,
    state: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for CredentialVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialVault")
            .field("path", &self.path)
            .finish()
    }
}

enum Inner {
    Uninitialized,
    Locked,
    Unlocked(UnlockedVault),
}

struct UnlockedVault {
    key: SecretBox<[u8; KEY_LEN]>,
    salt: [u8; SALT_LEN],
    entries: BTreeMap<String, SecretString>,
}

impl CredentialVault {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let state = if path.exists() {
            Inner::Locked
        } else {
            Inner::Uninitialized
        };
        Self {
            path,
            state: Arc::new(Mutex::new(state)),
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn state(&self) -> VaultState {
        match *self.state.lock().expect("vault state lock") {
            Inner::Uninitialized => VaultState::Uninitialized,
            Inner::Locked => VaultState::Locked,
            Inner::Unlocked(_) => VaultState::Unlocked,
        }
    }

    pub fn create(&self, passphrase: impl Into<SecretString>) -> VaultResult<()> {
        let passphrase = passphrase.into();
        if passphrase.expose_secret().chars().count() < 12 {
            return Err(VaultError::Invalid);
        }
        let mut state = self.lock_state()?;
        if !matches!(*state, Inner::Uninitialized) {
            return Err(VaultError::Invalid);
        }
        let mut salt = [0_u8; SALT_LEN];
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut salt).map_err(|_| VaultError::Invalid)?;
        getrandom::fill(&mut nonce).map_err(|_| VaultError::Invalid)?;
        let key = derive_key(passphrase.expose_secret(), &salt)?;
        let unlocked = UnlockedVault {
            key,
            salt,
            entries: BTreeMap::new(),
        };
        write_vault(&self.path, &unlocked, nonce)?;
        *state = Inner::Unlocked(unlocked);
        Ok(())
    }

    pub fn unlock(&self, passphrase: impl Into<SecretString>) -> VaultResult<()> {
        let passphrase = passphrase.into();
        let mut state = self.lock_state()?;
        if matches!(*state, Inner::Uninitialized) {
            return Err(VaultError::Uninitialized);
        }
        let bytes = read_vault_bytes(&self.path)?;
        let (header, ciphertext) = Header::parse(&bytes).map_err(|_| VaultError::Authentication)?;
        let key = derive_key(passphrase.expose_secret(), &header.salt)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.expose_secret())
            .map_err(|_| VaultError::Authentication)?;
        let nonce = XNonce::try_from(&header.nonce[..]).map_err(|_| VaultError::Authentication)?;
        let mut plaintext = cipher
            .decrypt(
                &nonce,
                chacha20poly1305::aead::Payload {
                    msg: ciphertext,
                    aad: &bytes[..HEADER_LEN],
                },
            )
            .map_err(|_| VaultError::Authentication)?;
        let entries = match decode_entries(&plaintext) {
            Ok(entries) => entries,
            Err(()) => {
                plaintext.zeroize();
                return Err(VaultError::Authentication);
            }
        };
        plaintext.zeroize();
        // Do not change the current state until all authentication and decoding succeeds.
        *state = Inner::Unlocked(UnlockedVault {
            key,
            salt: header.salt,
            entries,
        });
        Ok(())
    }

    pub fn lock(&self) -> VaultResult<()> {
        let mut state = self.lock_state()?;
        if matches!(*state, Inner::Uninitialized) {
            return Err(VaultError::Uninitialized);
        }
        *state = Inner::Locked;
        Ok(())
    }

    pub fn set(&self, key: impl Into<String>, secret: impl Into<SecretString>) -> VaultResult<()> {
        let mut state = self.lock_state()?;
        let Inner::Unlocked(unlocked) = &mut *state else {
            return Err(state_error(&state));
        };
        let key = key.into();
        let old = unlocked.entries.insert(key.clone(), secret.into());
        let result = persist_unlocked(&self.path, unlocked);
        if result.is_err() {
            if let Some(old) = old {
                unlocked.entries.insert(key, old);
            } else {
                unlocked.entries.remove(&key);
            }
        }
        result
    }

    pub fn get(&self, key: &str) -> VaultResult<Option<SecretString>> {
        let state = self.lock_state()?;
        let Inner::Unlocked(unlocked) = &*state else {
            return Err(state_error(&state));
        };
        Ok(unlocked
            .entries
            .get(key)
            .map(|value| SecretString::from(value.expose_secret().to_owned())))
    }

    pub fn delete(&self, key: &str) -> VaultResult<()> {
        let mut state = self.lock_state()?;
        let Inner::Unlocked(unlocked) = &mut *state else {
            return Err(state_error(&state));
        };
        let old = unlocked.entries.remove(key);
        let result = persist_unlocked(&self.path, unlocked);
        if result.is_err()
            && let Some(old) = old
        {
            unlocked.entries.insert(key.to_owned(), old);
        }
        result
    }

    fn lock_state(&self) -> VaultResult<std::sync::MutexGuard<'_, Inner>> {
        self.state.lock().map_err(|_| VaultError::Invalid)
    }
}

fn state_error(state: &Inner) -> VaultError {
    match state {
        Inner::Uninitialized => VaultError::Uninitialized,
        Inner::Locked => VaultError::Locked,
        Inner::Unlocked(_) => unreachable!(),
    }
}

fn derive_key(passphrase: &str, salt: &[u8; SALT_LEN]) -> VaultResult<SecretBox<[u8; KEY_LEN]>> {
    let params = Params::new(MEMORY_KIB, PASSES, PARALLELISM, Some(KEY_LEN))
        .map_err(|_| VaultError::Invalid)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|_| VaultError::Invalid)?;
    Ok(SecretBox::new(Box::new(key)))
}

fn encode_entries(entries: &BTreeMap<String, SecretString>) -> VaultResult<Vec<u8>> {
    let mut plain = BTreeMap::new();
    for (key, value) in entries {
        plain.insert(key.clone(), value.expose_secret().to_owned());
    }
    let result = serde_json::to_vec(&plain).map_err(|_| VaultError::Invalid);
    for value in plain.values_mut() {
        value.zeroize();
    }
    result
}

fn decode_entries(bytes: &[u8]) -> Result<BTreeMap<String, SecretString>, ()> {
    let mut plain: BTreeMap<String, String> = serde_json::from_slice(bytes).map_err(|_| ())?;
    let mut entries = BTreeMap::new();
    for (key, mut value) in std::mem::take(&mut plain) {
        entries.insert(key, SecretString::from(std::mem::take(&mut value)));
        value.zeroize();
    }
    Ok(entries)
}

struct Header {
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
}

impl Header {
    fn bytes(salt: [u8; SALT_LEN], nonce: [u8; NONCE_LEN]) -> [u8; HEADER_LEN] {
        let mut header = [0_u8; HEADER_LEN];
        header[..8].copy_from_slice(MAGIC);
        header[8] = FORMAT_VERSION;
        header[9] = KDF_VERSION;
        header[10..14].copy_from_slice(&MEMORY_KIB.to_le_bytes());
        header[14..18].copy_from_slice(&PASSES.to_le_bytes());
        header[18..22].copy_from_slice(&PARALLELISM.to_le_bytes());
        header[22..38].copy_from_slice(&salt);
        header[38..62].copy_from_slice(&nonce);
        header
    }
    fn parse(bytes: &[u8]) -> Result<(Self, &[u8]), ()> {
        if bytes.len() <= HEADER_LEN
            || &bytes[..8] != MAGIC
            || bytes[8] != FORMAT_VERSION
            || bytes[9] != KDF_VERSION
            || bytes[10..14] != MEMORY_KIB.to_le_bytes()
            || bytes[14..18] != PASSES.to_le_bytes()
            || bytes[18..22] != PARALLELISM.to_le_bytes()
        {
            return Err(());
        }
        let mut salt = [0; SALT_LEN];
        salt.copy_from_slice(&bytes[22..38]);
        let mut nonce = [0; NONCE_LEN];
        nonce.copy_from_slice(&bytes[38..62]);
        Ok((Self { salt, nonce }, &bytes[HEADER_LEN..]))
    }
}

fn persist_unlocked(path: &Path, unlocked: &UnlockedVault) -> VaultResult<()> {
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| VaultError::Invalid)?;
    write_vault(path, unlocked, nonce)
}

fn write_vault(path: &Path, unlocked: &UnlockedVault, nonce: [u8; NONCE_LEN]) -> VaultResult<()> {
    let header = Header::bytes(unlocked.salt, nonce);
    let mut plaintext = encode_entries(&unlocked.entries)?;
    let cipher = XChaCha20Poly1305::new_from_slice(unlocked.key.expose_secret())
        .map_err(|_| VaultError::Invalid)?;
    let nonce = XNonce::try_from(&nonce[..]).map_err(|_| VaultError::Invalid)?;
    let ciphertext = match cipher.encrypt(
        &nonce,
        chacha20poly1305::aead::Payload {
            msg: &plaintext,
            aad: &header,
        },
    ) {
        Ok(ciphertext) => ciphertext,
        Err(_) => {
            plaintext.zeroize();
            return Err(VaultError::Invalid);
        }
    };
    plaintext.zeroize();
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&ciphertext);
    let result = atomic_write(path, &bytes).map_err(io_error);
    bytes.zeroize();
    result
}

fn io_error(error: io::Error) -> VaultError {
    VaultError::Io(error.to_string())
}

fn read_vault_bytes(path: &Path) -> VaultResult<Vec<u8>> {
    let file = File::open(path).map_err(io_error)?;
    let mut bytes = Vec::with_capacity(HEADER_LEN);
    file.take(MAX_VAULT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_VAULT_FILE_BYTES {
        bytes.zeroize();
        return Err(VaultError::Authentication);
    }
    Ok(bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let missing = !parent.exists();
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    if missing {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let file_name = path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("credentials.vault");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        // Rename is the commit point. Directory sync improves crash
        // durability, but a failure after commit must not roll state back.
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault() -> (tempfile::TempDir, CredentialVault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = CredentialVault::at(dir.path().join("vault.bin"));
        (dir, vault)
    }

    #[test]
    fn create_relaunch_unlocks_reserved_unicode_secret() {
        let (_dir, vault) = vault();
        vault.create("a passphrase").unwrap();
        vault.set("postgres", "p@ss:word/?#%[]é").unwrap();
        let reloaded = CredentialVault::at(vault.path());
        assert_eq!(reloaded.state(), VaultState::Locked);
        reloaded.unlock("a passphrase").unwrap();
        assert_eq!(
            reloaded.get("postgres").unwrap().unwrap().expose_secret(),
            "p@ss:word/?#%[]é"
        );
    }
    #[test]
    fn wrong_passphrase_does_not_unlock_or_overwrite() {
        let (_dir, vault) = vault();
        vault.create("right passphrase").unwrap();
        vault.set("key", "secret").unwrap();
        vault.lock().unwrap();
        assert_eq!(
            vault.unlock("wrong passphrase"),
            Err(VaultError::Authentication)
        );
        assert_eq!(vault.state(), VaultState::Locked);
        vault.unlock("right passphrase").unwrap();
        assert_eq!(vault.get("key").unwrap().unwrap().expose_secret(), "secret");
    }
    #[test]
    fn tampering_is_an_authentication_error() {
        let (_dir, vault) = vault();
        vault.create("a passphrase").unwrap();
        vault.set("key", "secret").unwrap();
        let mut bytes = fs::read(vault.path()).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(vault.path(), bytes).unwrap();
        let reloaded = CredentialVault::at(vault.path());
        assert_eq!(
            reloaded.unlock("a passphrase"),
            Err(VaultError::Authentication)
        );
    }
    #[test]
    fn locked_vault_rejects_secret_operations() {
        let (_dir, vault) = vault();
        assert!(matches!(vault.get("x"), Err(VaultError::Uninitialized)));
        assert_eq!(vault.create("too short"), Err(VaultError::Invalid));
        vault.create("a passphrase").unwrap();
        vault.lock().unwrap();
        assert_eq!(vault.set("x", "y"), Err(VaultError::Locked));
        assert_eq!(vault.delete("x"), Err(VaultError::Locked));
    }
    #[test]
    fn every_write_uses_a_new_nonce() {
        let (_dir, vault) = vault();
        vault.create("a passphrase").unwrap();
        let first = fs::read(vault.path()).unwrap();
        vault.set("x", "y").unwrap();
        let second = fs::read(vault.path()).unwrap();
        assert_ne!(&first[38..62], &second[38..62]);
    }

    #[test]
    fn oversized_vault_is_rejected_before_reading_it() {
        let (_dir, vault) = vault();
        fs::write(vault.path(), vec![0_u8; MAX_VAULT_FILE_BYTES as usize + 1]).unwrap();
        let reloaded = CredentialVault::at(vault.path());
        assert_eq!(
            reloaded.unlock("a passphrase"),
            Err(VaultError::Authentication)
        );
    }
    #[cfg(unix)]
    #[test]
    fn vault_and_new_parent_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let vault = CredentialVault::at(dir.path().join("private").join("vault.bin"));
        vault.create("a passphrase").unwrap();
        assert_eq!(
            fs::metadata(vault.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(vault.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
