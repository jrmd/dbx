//! Explicit, bounded profile portability. Imported profiles never connect.
use crate::profiles::{ConnectionProfileDraft, ConnectionTag, ProfileStore};
use dbx_core::{ConnectionConfig, DatabaseKind, SshConfig};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROFILES: usize = 500;

#[derive(Clone, Deserialize, Serialize)]
pub struct PortableProfile {
    pub name: String,
    pub config: ConnectionConfig,
    pub tag: Option<ConnectionTag>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_password: Option<String>,
}
impl Drop for PortableProfile {
    fn drop(&mut self) {
        self.config.url.zeroize();
        if let Some(secret) = &mut self.config.ssh_password {
            secret.zeroize();
        }
        if let Some(secret) = &mut self.ssh_password {
            secret.zeroize();
        }
    }
}
#[derive(Deserialize, Serialize)]
struct Envelope {
    version: u32,
    profiles: Vec<PortableProfile>,
}
pub struct ImportPreview {
    pub profiles: Vec<PortableProfile>,
    pub warnings: Vec<String>,
}

pub fn review_label(profile: &PortableProfile) -> String {
    format!(
        "{} · {} · protected\n{}\n{} · tag: {} · cloud auth: {:?}",
        profile.name,
        profile.config.kind,
        password_free(&profile.config.url).unwrap_or_else(|_| "Review URL manually".into()),
        profile
            .config
            .ssh
            .as_ref()
            .map(|ssh| format!(
                "SSH {}@{}:{}; key {}; jumps {}",
                ssh.username,
                ssh.host,
                ssh.port,
                ssh.identity_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "password / agent".into()),
                ssh.jump_host.as_deref().unwrap_or("none")
            ))
            .unwrap_or_else(|| "Direct connection".into()),
        profile
            .tag
            .as_ref()
            .map(|tag| tag.name.as_str())
            .unwrap_or("none"),
        profile.config.cloud_auth
    )
}

pub fn read_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Profile file exceeds 8 MiB".into());
    }
    Ok(bytes)
}

fn password_free(raw: &str) -> Result<String, String> {
    // Profile storage already strips authority passwords. Mongo seed-list URLs
    // require that parser rather than Url's single-host parser.
    let fields = crate::connection_fields::normalize_connection_string(raw)
        .map_err(|error| error.to_string())?;
    if crate::connection_fields::is_mongo_seed_list(&fields) {
        if let Some((prefix, suffix)) = fields.rsplit_once('@') {
            let username = prefix
                .trim_start_matches("mongodb://")
                .split(':')
                .next()
                .unwrap_or_default();
            let (authority_path, query) = suffix.split_once('?').unwrap_or((suffix, ""));
            let allowed = [
                "authSource",
                "replicaSet",
                "tls",
                "ssl",
                "tlsCAFile",
                "tlsCertificateKeyFile",
                "readPreference",
                "directConnection",
                "retryWrites",
            ];
            let mut output = format!("mongodb://{username}@{authority_path}");
            let pairs = url::form_urlencoded::parse(query.as_bytes())
                .filter(|(key, _)| allowed.contains(&key.as_ref()))
                .collect::<Vec<_>>();
            if !pairs.is_empty() {
                output.push('?');
                output.push_str(
                    &url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(pairs)
                        .finish(),
                );
            }
            return Ok(output);
        }
        return Ok(fields);
    }
    if fields.starts_with("sqlite:") || fields.starts_with("duckdb:") {
        return Ok(fields);
    }
    let mut url = Url::parse(&fields).map_err(|_| "Invalid profile URL")?;
    url.set_password(None).map_err(|_| "Invalid profile URL")?;
    let allowed = [
        "sslmode",
        "ssl-mode",
        "sslrootcert",
        "sslcert",
        "sslkey",
        "ssl-ca",
        "ssl-cert",
        "ssl-key",
        "schema",
        "warehouse",
        "role",
        "encrypt",
        "trust_server_certificate",
        "application_name",
        "connect_timeout",
        "mode",
        "cache",
    ];
    let options = url
        .query_pairs()
        .filter(|(key, _)| allowed.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !options.is_empty() {
        url.query_pairs_mut().extend_pairs(options);
    }
    Ok(url.into())
}

pub fn export(store: &ProfileStore, path: &Path, passphrase: Option<&str>) -> Result<(), String> {
    let mut profiles = Vec::new();
    for profile in store.list().map_err(|error| error.to_string())? {
        let (config, ssh_password) = if passphrase.is_some() {
            let mut loaded = store.load(profile.id).map_err(|error| error.to_string())?;
            let ssh = loaded.config.ssh_password.take();
            (loaded.config, ssh)
        } else {
            let mut config = ConnectionConfig::new(profile.kind, password_free(&profile.url)?);
            config.read_only = profile.read_only;
            config.cloud_auth = profile.cloud_auth;
            config.max_connections = profile.max_connections;
            config.connect_timeout_ms = profile.connect_timeout_ms;
            config.socket = profile.socket;
            config.ssh = profile.ssh;
            (config, None)
        };
        profiles.push(PortableProfile {
            name: profile.name,
            config,
            tag: profile.tag,
            ssh_password,
        });
    }
    let plain = Zeroizing::new(
        serde_json::to_vec_pretty(&Envelope {
            version: 1,
            profiles,
        })
        .map_err(|error| error.to_string())?,
    );
    let output = match passphrase {
        Some(passphrase) => {
            crate::vault::seal_bundle(&plain, passphrase).map_err(|error| error.to_string())?
        }
        None => plain.to_vec(),
    };
    dbx_core::write_atomic_export(path, &output).map_err(|error| error.to_string())
}

pub fn decode(bytes: &[u8], passphrase: &str) -> Result<ImportPreview, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Profile file exceeds 8 MiB".into());
    }
    let plaintext = if bytes.starts_with(b"DBXBNDL1") {
        crate::vault::open_bundle(bytes, passphrase).map_err(|error| error.to_string())?
    } else if bytes.starts_with(b"TPRO") {
        open_tablepro(bytes, passphrase)?
    } else {
        Zeroizing::new(bytes.to_vec())
    };
    let value: serde_json::Value = serde_json::from_slice(&plaintext).map_err(|_| "Expected a DBX or TablePro JSON profile export. TablePlus .tableplusconnection is proprietary; migrate using connection URLs.")?;
    let mut preview = if value.get("formatVersion").is_some() {
        decode_tablepro(&value)?
    } else {
        let envelope: Envelope =
            serde_json::from_value(value).map_err(|_| "Invalid DBX profile bundle")?;
        if envelope.version != 1 {
            return Err("Unsupported DBX bundle version".into());
        }
        ImportPreview {
            profiles: envelope.profiles,
            warnings: Vec::new(),
        }
    };
    if preview.profiles.is_empty() || preview.profiles.len() > MAX_PROFILES {
        return Err("Import requires 1–500 profiles".into());
    }
    for profile in &mut preview.profiles {
        if profile.name.trim().is_empty() || profile.name.chars().count() > 200 {
            return Err("Invalid imported profile name".into());
        }
        // Always start protected. Imported commands and credentials cannot
        // initiate a connection or a query; enabling writes is a separate edit.
        profile.config.read_only = true;
        profile.config.ssh_password = profile.ssh_password.clone();
        profile
            .config
            .validate()
            .map_err(|error| error.to_string())?;
    }
    preview.warnings.push("Imported profiles start protected. No connections or startup commands will run. Paths to key/certificate files are references; those files are not copied.".into());
    Ok(preview)
}

fn open_tablepro(bytes: &[u8], passphrase: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    use aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    };
    if bytes.len() < 65 || bytes[4] != 1 {
        return Err("Unsupported or malformed encrypted TablePro export".into());
    }
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(passphrase.as_bytes(), &bytes[5..37], 600_000, &mut *key);
    let cipher = Aes256Gcm::new_from_slice(&*key).map_err(|_| "Invalid encryption key")?;
    let nonce = Nonce::from_slice(&bytes[37..49]);
    cipher
        .decrypt(nonce, &bytes[49..])
        .map(Zeroizing::new)
        .map_err(|_| "TablePro passphrase or file authentication failed".into())
}

fn decode_tablepro(value: &serde_json::Value) -> Result<ImportPreview, String> {
    if value["formatVersion"].as_u64() != Some(1) {
        return Err("Unsupported TablePro format version".into());
    }
    let connections = value["connections"]
        .as_array()
        .ok_or("TablePro connections must be an array")?;
    if connections.len() > MAX_PROFILES {
        return Err("Too many imported profiles".into());
    }
    let mut preview = ImportPreview {
        profiles: Vec::new(),
        warnings: Vec::new(),
    };
    for (index, item) in connections.iter().enumerate() {
        let field = |key| {
            item[key]
                .as_str()
                .ok_or_else(|| format!("TablePro connection {} is missing {key}", index + 1))
        };
        let type_name = field("type")?;
        let kind = DatabaseKind::ALL
            .into_iter()
            .find(|kind| {
                kind.to_string().eq_ignore_ascii_case(type_name)
                    || kind.scheme().eq_ignore_ascii_case(type_name)
            })
            .or_else(|| (type_name == "MariaDB").then_some(DatabaseKind::MySQL))
            .ok_or_else(|| format!("Unsupported imported database type: {type_name}"))?;
        let name = field("name")?.to_owned();
        let mut url = if kind.is_file() {
            Url::parse(&format!("{}://{}", kind.scheme(), field("database")?))
                .map_err(|_| "Invalid file database path")?
        } else {
            let mut url = Url::parse(kind.default_url()).map_err(|_| "Invalid engine URL")?;
            url.set_host(Some(field("host")?))
                .map_err(|_| "Invalid imported host")?;
            let port = item["port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port > 0)
                .ok_or("Invalid imported port")?;
            url.set_port(Some(port))
                .map_err(|_| "Invalid imported port")?;
            url.set_username(field("username")?)
                .map_err(|_| "Invalid imported username")?;
            url.set_path(field("database")?);
            url
        };
        let credentials = &value["credentials"][index.to_string()];
        if let Some(password) = credentials["password"].as_str() {
            crate::connection_fields::set_url_password(&mut url, Some(password))
                .map_err(|_| "Invalid imported credentials")?;
        }
        if let Some(ssl) = item.get("sslConfig").filter(|ssl| !ssl.is_null()) {
            let mode = ssl["mode"]
                .as_str()
                .unwrap_or("prefer")
                .to_lowercase()
                .replace(' ', "-");
            let mode = match mode.as_str() {
                "verifyfull" | "verify-identity" | "verify-full" => "verify-full",
                "verifyca" | "verify-ca" => "verify-ca",
                "preferred" | "prefer" => "prefer",
                "required" | "require" => "require",
                "disabled" | "disable" => "disable",
                other => {
                    return Err(format!(
                        "Unsupported TLS mode {other}; review TLS manually for {name}"
                    ));
                }
            };
            if !matches!(
                kind.dialect(),
                DatabaseKind::PostgreSQL | DatabaseKind::MySQL
            ) {
                return Err(format!(
                    "Review TLS manually for imported {name}; this mapping supports PostgreSQL and MySQL"
                ));
            }
            url.query_pairs_mut().append_pair(
                "sslmode",
                if kind == DatabaseKind::MySQL && mode == "verify-full" {
                    "verify_identity"
                } else {
                    mode
                },
            );
            for (source, pg, mysql) in [
                ("caCertificatePath", "sslrootcert", "ssl-ca"),
                ("clientCertificatePath", "sslcert", "ssl-cert"),
                ("clientKeyPath", "sslkey", "ssl-key"),
            ] {
                if let Some(path) = ssl[source].as_str().filter(|path| !path.is_empty()) {
                    let path = expand_path(path)?;
                    url.query_pairs_mut().append_pair(
                        if kind == DatabaseKind::MySQL {
                            mysql
                        } else {
                            pg
                        },
                        &path.to_string_lossy(),
                    );
                }
            }
        }
        let mut config = ConnectionConfig::new(kind, url.to_string());
        if let Some(seconds) = item["connectTimeoutSeconds"].as_u64() {
            config.connect_timeout_ms = seconds.saturating_mul(1000);
        }
        if let Some(ssh) = item.get("sshConfig").filter(|ssh| ssh["enabled"] == true) {
            if ssh
                .get("jumpHosts")
                .is_some_and(|jumps| jumps.as_array().is_some_and(|jumps| !jumps.is_empty()))
            {
                return Err(format!(
                    "Review jump-host authentication manually for imported {name}"
                ));
            }
            config.ssh = Some(SshConfig {
                host: ssh["host"].as_str().ok_or("Missing SSH host")?.into(),
                port: ssh["port"]
                    .as_u64()
                    .unwrap_or(22)
                    .try_into()
                    .map_err(|_| "Invalid SSH port")?,
                username: ssh["username"].as_str().ok_or("Missing SSH user")?.into(),
                identity_file: ssh["privateKeyPath"]
                    .as_str()
                    .filter(|path| !path.is_empty())
                    .map(expand_path)
                    .transpose()?,
                jump_host: None,
            });
        }
        if [
            "startupCommands",
            "tunnelCommand",
            "additionalFields",
            "credentialProfileName",
            "sshProfileId",
        ]
        .iter()
        .any(|key| item.get(key).is_some_and(|field| !field.is_null()))
        {
            preview.warnings.push(format!("{name}: startup commands, custom tunnel commands, credential-profile references and additional fields were not imported. Review them manually."));
        }
        if [
            "keyPassphrase",
            "sslClientKeyPassphrase",
            "totpSecret",
            "pluginSecureFields",
        ]
        .iter()
        .any(|key| credentials.get(key).is_some_and(|field| !field.is_null()))
        {
            return Err(format!(
                "{name}: unsupported encrypted-key/TOTP/plugin credentials require manual migration"
            ));
        }
        let tag = item["tagName"]
            .as_str()
            .or_else(|| item["tagNames"].as_array()?.first()?.as_str())
            .or_else(|| item["groupName"].as_str())
            .map(|name| ConnectionTag {
                id: uuid::Uuid::new_v4(),
                name: name.chars().take(32).collect(),
                color: 0x82aaff,
            });
        preview.profiles.push(PortableProfile {
            name,
            config,
            tag,
            ssh_password: credentials["sshPassword"].as_str().map(str::to_owned),
        });
    }
    Ok(preview)
}
fn expand_path(path: &str) -> Result<std::path::PathBuf, String> {
    if let Some(path) = path.strip_prefix("~/") {
        Ok(dirs::home_dir()
            .ok_or("Home directory unavailable")?
            .join(path))
    } else {
        Ok(path.into())
    }
}

pub fn import(store: &ProfileStore, profiles: Vec<PortableProfile>) -> Result<usize, String> {
    let tags = store.tags().map_err(|error| error.to_string())?;
    let mut created = Vec::new();
    let mut created_tags = Vec::new();
    for profile in profiles {
        let result = (|| {
            let mut draft =
                ConnectionProfileDraft::from_config(profile.name.clone(), profile.config.clone());
            draft.config.ssh_password = profile.ssh_password.clone();
            draft.tag = if let Some(tag) = &profile.tag {
                if let Some(existing) = tags
                    .iter()
                    .find(|existing| existing.name.eq_ignore_ascii_case(&tag.name))
                {
                    Some(existing.clone())
                } else {
                    let mut tag = tag.clone();
                    tag.id = uuid::Uuid::new_v4();
                    // Re-read because several imported profiles may share a tag.
                    if let Some(existing) = store
                        .tags()
                        .map_err(|error| error.to_string())?
                        .into_iter()
                        .find(|existing| existing.name.eq_ignore_ascii_case(&tag.name))
                    {
                        Some(existing)
                    } else {
                        store
                            .save_tag(tag.clone())
                            .map_err(|error| error.to_string())?;
                        created_tags.push(tag.id);
                        Some(tag)
                    }
                }
            } else {
                None
            };
            store.save(draft).map_err(|error| error.to_string())
        })();
        match result {
            Ok(saved) => created.push(saved.id),
            Err(error) => {
                let mut rollback_errors = Vec::new();
                for id in created.iter().rev() {
                    if let Err(error) = store.delete(*id) {
                        rollback_errors.push(error.to_string());
                    }
                }
                for id in created_tags.iter().rev() {
                    if let Err(error) = store.delete_tag(*id) {
                        rollback_errors.push(error.to_string());
                    }
                }
                return Err(if rollback_errors.is_empty() {
                    error
                } else {
                    format!(
                        "{error}; import rollback incomplete: {}",
                        rollback_errors.join("; ")
                    )
                });
            }
        }
    }
    Ok(created.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_profiles_are_password_free_or_authenticated_and_import_protected() {
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        store
            .vault()
            .unwrap()
            .create("fixture vault passphrase")
            .unwrap();
        let mut config = ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            "postgres://fixture:database-secret@localhost/example?sslmode=verify-full&vendor_option=hidden",
        );
        config.ssh = Some(SshConfig {
            host: "bastion.example".into(),
            port: 22,
            username: "fixture".into(),
            identity_file: None,
            jump_host: Some("fixture@jump.example:22".into()),
        });
        config.ssh_password = Some("ssh-secret".into());
        let saved = store
            .save(ConnectionProfileDraft::from_config("Example", config))
            .unwrap();
        let duplicate = store.duplicate(saved.id).unwrap();
        assert_ne!(duplicate.id, saved.id);
        assert!(
            store
                .load(duplicate.id)
                .unwrap()
                .config
                .url
                .contains("database-secret")
        );
        let path = directory.path().join("profiles.json");
        export(&store, &path, None).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        for secret in ["database-secret", "ssh-secret", "hidden"] {
            assert!(!text.contains(secret));
        }
        let preview = decode(&bytes, "").unwrap();
        assert!(
            preview
                .profiles
                .iter()
                .all(|profile| profile.config.read_only)
        );
        assert_eq!(
            preview.profiles[0]
                .config
                .ssh
                .as_ref()
                .unwrap()
                .jump_host
                .as_deref(),
            Some("fixture@jump.example:22")
        );
        let path = directory.path().join("profiles.dbxbundle");
        export(&store, &path, Some("portable test passphrase")).unwrap();
        let mut bytes = std::fs::read(path).unwrap();
        assert!(decode(&bytes, "wrong passphrase").is_err());
        let preview = decode(&bytes, "portable test passphrase").unwrap();
        assert!(preview.profiles[0].config.url.contains("database-secret"));
        assert_eq!(
            preview.profiles[0].ssh_password.as_deref(),
            Some("ssh-secret")
        );
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(decode(&bytes, "portable test passphrase").is_err());
        store.delete(duplicate.id).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }
    #[test]
    fn tablepro_modes_and_encrypted_export_interoperate_without_running_startup_commands() {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };
        let value = serde_json::json!({"formatVersion":1,"connections":[{"name":"Imported","type":"PostgreSQL","host":"example.test","port":5432,"username":"fixture","database":"sample","sslConfig":{"mode":"Verify Identity"},"startupCommands":["DROP TABLE data"]}],"credentials":{"0":{"password":"fixture-secret"}}});
        let plain = serde_json::to_vec(&value).unwrap();
        let salt = [5u8; 32];
        let nonce = [8u8; 12];
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(b"fixture password", &salt, 600_000, &mut key);
        let ciphertext = Aes256Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(Nonce::from_slice(&nonce), plain.as_slice())
            .unwrap();
        let mut file = b"TPRO".to_vec();
        file.push(1);
        file.extend(salt);
        file.extend(nonce);
        file.extend(ciphertext);
        let preview = decode(&file, "fixture password").unwrap();
        assert!(
            preview.profiles[0]
                .config
                .url
                .contains("sslmode=verify-full")
        );
        assert!(preview.profiles[0].config.read_only);
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("startup commands"))
        );
        assert!(decode(&file, "incorrect").is_err());
    }
}
