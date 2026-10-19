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
    pub folder: Option<String>,
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
        "{} · {} · protected\n{}\n{} · tag: {} · folder: {} · cloud auth: {:?}",
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
        profile.folder.as_deref().unwrap_or("Ungrouped"),
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
    let saved = store.list().map_err(|error| error.to_string())?;
    if saved
        .iter()
        .any(|profile| profile.driver_manifest.is_some())
    {
        return Err("Portable bundles do not support locally registered driver connections. Recreate those connections explicitly on the destination after registering a trusted driver.".into());
    }
    for profile in saved {
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
            folder: profile.folder,
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
    let value: serde_json::Value = serde_json::from_slice(&plaintext)
        .or_else(|_| serde_yaml::from_slice(&plaintext))
        .unwrap_or(serde_json::Value::Null);
    let is_env = std::str::from_utf8(&plaintext).is_ok_and(|text| {
        text.lines().any(|line| {
            line.trim()
                .strip_prefix("export ")
                .unwrap_or(line.trim())
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "DATABASE_URL")
        })
    });
    let mut preview = if is_env {
        decode_env(&plaintext)?
    } else if value.get("services").is_some() {
        decode_compose(&value)?
    } else if value
        .get("connections")
        .is_some_and(serde_json::Value::is_object)
    {
        decode_dbeaver(&value)?
    } else if value.is_null() || value.is_string() {
        decode_env(&plaintext)?
    } else if value.get("formatVersion").is_some() {
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
        if profile.config.driver_manifest.is_some() {
            return Err("Imported profiles cannot contain machine-specific driver programs. Register a trusted driver locally and create its connection explicitly.".into());
        }
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

fn imported_profile(
    name: String,
    fields: crate::connection_fields::ConnectionFields,
    folder: Option<String>,
) -> Result<PortableProfile, String> {
    Ok(PortableProfile {
        name,
        config: fields.config()?,
        tag: None,
        folder,
        ssh_password: None,
    })
}

fn decode_env(bytes: &[u8]) -> Result<ImportPreview, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Import must be UTF-8")?;
    let mut profiles = Vec::new();
    for line in text.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "DATABASE_URL" {
            continue;
        }
        let value = value.trim();
        let value = if value.starts_with(['\'', '"']) {
            let quote = value.chars().next().unwrap();
            let end = value[1..]
                .find(quote)
                .ok_or("Unclosed DATABASE_URL quote")?
                + 1;
            &value[1..end]
        } else {
            value.split(" #").next().unwrap_or(value).trim()
        };
        if value.contains('$') || value.contains('`') {
            return Err("Resolve DATABASE_URL variables before importing; shell expressions are never evaluated".into());
        }
        let fields = crate::connection_fields::ConnectionFields::from_url(value)
            .map_err(|_| "Invalid DATABASE_URL (value hidden)")?;
        profiles.push(imported_profile("DATABASE_URL".into(), fields, None)?);
    }
    if profiles.is_empty() {
        return Err("Choose DBX, TablePro, DBeaver data-sources.json, Docker Compose YAML, or a .env with DATABASE_URL. TablePlus files require migration via a connection URL.".into());
    }
    Ok(ImportPreview { profiles, warnings: vec!["Only DATABASE_URL is imported. Shell commands and variable expansion are never evaluated.".into()] })
}

fn decode_dbeaver(value: &serde_json::Value) -> Result<ImportPreview, String> {
    let connections = value["connections"]
        .as_object()
        .ok_or("Invalid DBeaver connections")?;
    let mut preview = ImportPreview { profiles: Vec::new(), warnings: vec!["DBeaver encrypted credentials, SSH/TLS custom properties and startup SQL are not imported. Review authentication and TLS before connecting.".into()] };
    for connection in connections.values() {
        let name = connection["name"]
            .as_str()
            .unwrap_or("DBeaver connection")
            .to_owned();
        let config = &connection["configuration"];
        let raw = config["url"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("jdbc:");
        let mut fields = if !raw.is_empty() {
            match crate::connection_fields::ConnectionFields::from_url(raw) {
                Ok(fields) => fields,
                Err(_) => {
                    preview
                        .warnings
                        .push(format!("{name}: unsupported JDBC URL; skipped"));
                    continue;
                }
            }
        } else {
            let driver = connection["provider"]
                .as_str()
                .or_else(|| connection["driver"].as_str())
                .unwrap_or_default()
                .to_lowercase();
            let kind = if driver.contains("postgres") || driver.contains("redshift") {
                DatabaseKind::PostgreSQL
            } else if driver.contains("mysql") || driver.contains("maria") {
                DatabaseKind::MySQL
            } else if driver.contains("sqlite") {
                DatabaseKind::SQLite
            } else {
                preview
                    .warnings
                    .push(format!("{name}: unsupported DBeaver driver; skipped"));
                continue;
            };
            let mut fields = crate::connection_fields::ConnectionFields::new(kind);
            if kind.is_file() {
                fields.connection_string = config["database"].as_str().unwrap_or_default().into();
            } else {
                fields.host = config["host"].as_str().unwrap_or("localhost").into();
                fields.port = config["port"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| config["port"].as_u64().map(|p| p.to_string()))
                    .unwrap_or_else(|| fields.port.clone());
                fields.database = config["database"].as_str().unwrap_or_default().into();
                fields.username = config["user"]
                    .as_str()
                    .or_else(|| config["userName"].as_str())
                    .or_else(|| config["username"].as_str())
                    .unwrap_or_default()
                    .into();
            }
            fields
        };
        if fields.username.is_empty() && !fields.kind.is_file() {
            fields.username = config["user"]
                .as_str()
                .or_else(|| config["userName"].as_str())
                .or_else(|| config["username"].as_str())
                .unwrap_or_default()
                .into();
            if !fields.username.is_empty() && !fields.connection_string.is_empty() {
                let mut url =
                    Url::parse(&fields.connection_string).map_err(|_| "Invalid DBeaver URL")?;
                url.set_username(&fields.username.replace('%', "%25"))
                    .map_err(|_| "Invalid DBeaver username")?;
                fields.connection_string = url.into();
            }
        }
        let folder = connection["folder"]
            .as_str()
            .and_then(|folder| value["folders"][folder]["name"].as_str().or(Some(folder)))
            .map(str::to_owned);
        preview
            .profiles
            .push(imported_profile(name, fields, folder)?);
    }
    Ok(preview)
}

fn decode_compose(value: &serde_json::Value) -> Result<ImportPreview, String> {
    let services = value["services"]
        .as_object()
        .ok_or("Compose services must be a mapping")?;
    let mut preview = ImportPreview { profiles: Vec::new(), warnings: vec!["Compose is read as data. Containers, env_file files, commands and variable interpolation are never executed. Review published host ports and credentials.".into()] };
    for (name, service) in services {
        let image = service["image"].as_str().unwrap_or_default().to_lowercase();
        let (kind, prefix, default_user, default_db) = if image.contains("postgres") {
            (DatabaseKind::PostgreSQL, "POSTGRES", "postgres", "postgres")
        } else if image.contains("mariadb") {
            (DatabaseKind::MySQL, "MARIADB", "root", "mysql")
        } else if image.contains("mysql") {
            (DatabaseKind::MySQL, "MYSQL", "root", "mysql")
        } else if image.contains("redis") {
            (DatabaseKind::Redis, "REDIS", "", "")
        } else {
            continue;
        };
        let mut env = std::collections::HashMap::<String, String>::new();
        if let Some(map) = service["environment"].as_object() {
            for (key, value) in map {
                if let Some(value) = value
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| value.as_i64().map(|v| v.to_string()))
                {
                    env.insert(key.clone(), value);
                }
            }
        } else if let Some(values) = service["environment"].as_array() {
            for value in values.iter().filter_map(|v| v.as_str()) {
                if let Some((key, value)) = value.split_once('=') {
                    env.insert(key.into(), value.into());
                }
            }
        }
        if env
            .values()
            .any(|value| value.contains('$') || value.contains('`'))
        {
            preview.warnings.push(format!(
                "{name}: unresolved environment expressions; skipped"
            ));
            continue;
        }
        let mut fields = crate::connection_fields::ConnectionFields::new(kind);
        fields.host = "localhost".into();
        fields.username = env
            .get(&format!("{prefix}_USER"))
            .cloned()
            .unwrap_or(default_user.into());
        fields.database = env
            .get(&format!("{prefix}_DB"))
            .or_else(|| env.get(&format!("{prefix}_DATABASE")))
            .cloned()
            .unwrap_or_else(|| {
                if kind == DatabaseKind::PostgreSQL {
                    fields.username.clone()
                } else {
                    default_db.into()
                }
            });
        fields.password = env
            .get(&format!("{prefix}_PASSWORD"))
            .or_else(|| env.get(&format!("{prefix}_ROOT_PASSWORD")))
            .cloned()
            .unwrap_or_default();
        let mut published = None;
        for port in service["ports"].as_array().into_iter().flatten() {
            if let Some(port) = port.as_str() {
                let pieces: Vec<_> = port.trim_end_matches("/tcp").split(':').collect();
                if pieces.len() >= 2 && pieces.last().copied() == kind.default_port() {
                    published = pieces[pieces.len() - 2]
                        .parse::<u16>()
                        .ok()
                        .filter(|port| *port > 0);
                }
            } else if port["target"].as_u64().map(|v| v.to_string()).as_deref()
                == kind.default_port()
            {
                published = port["published"]
                    .as_u64()
                    .and_then(|v| u16::try_from(v).ok())
                    .or_else(|| port["published"].as_str().and_then(|v| v.parse().ok()))
                    .filter(|port| *port > 0);
            }
        }
        let Some(port) = published else {
            preview
                .warnings
                .push(format!("{name}: no fixed published database port; skipped"));
            continue;
        };
        fields.port = port.to_string();
        preview.profiles.push(imported_profile(
            name.clone(),
            fields,
            Some("Docker Compose".into()),
        )?);
    }
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
            folder: item["folderName"]
                .as_str()
                .or_else(|| item["groupName"].as_str())
                .or_else(|| item["folder"].as_str())
                .map(str::to_owned),
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
    if profiles
        .iter()
        .any(|profile| profile.config.driver_manifest.is_some())
    {
        return Err("Imported profiles cannot select local driver programs. Register a trusted driver and configure it explicitly.".into());
    }
    let tags = store.tags().map_err(|error| error.to_string())?;
    let mut created = Vec::new();
    let mut created_tags = Vec::new();
    for profile in profiles {
        let result = (|| {
            let mut draft =
                ConnectionProfileDraft::from_config(profile.name.clone(), profile.config.clone());
            draft.config.ssh_password = profile.ssh_password.clone();
            draft.folder = profile.folder.clone();
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
    fn portable_imports_reject_local_driver_programs_before_validation_or_writes() {
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("profiles.json"));
        let mut config =
            ConnectionConfig::new(DatabaseKind::SQLite, "example-sqlite:///private.sqlite");
        config.driver_manifest = Some(directory.path().join("untrusted-not-registered.json"));
        let profile = PortableProfile {
            name: "Untrusted driver".into(),
            config,
            tag: None,
            folder: None,
            ssh_password: None,
        };
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            profiles: vec![profile.clone()],
        })
        .unwrap();
        let error = match decode(&bytes, "") {
            Ok(_) => panic!("plugin import accepted"),
            Err(error) => error,
        };
        assert!(error.contains("machine-specific driver"));
        assert!(
            import(&store, vec![profile])
                .unwrap_err()
                .contains("local driver programs")
        );
        assert!(!store.path().exists());
        assert!(
            !directory
                .path()
                .join("untrusted-not-registered.json")
                .exists()
        );
    }

    #[test]
    fn portable_exports_reject_plugins_without_overwriting_destination() {
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("profiles.json"));
        store
            .vault()
            .unwrap()
            .create("fixture plugin vault passphrase")
            .unwrap();
        let config = crate::profiles::plugin_profile_fixture(
            directory.path(),
            "example-sqlite:///absolute/database.sqlite",
        );
        store
            .save(ConnectionProfileDraft::from_config("Local driver", config))
            .unwrap();
        let destination = directory.path().join("existing-export.json");
        std::fs::write(&destination, b"keep existing export").unwrap();
        for passphrase in [None, Some("portable fixture passphrase")] {
            assert!(
                export(&store, &destination, passphrase)
                    .unwrap_err()
                    .contains("locally registered driver")
            );
            assert_eq!(
                std::fs::read(&destination).unwrap(),
                b"keep existing export"
            );
        }
    }

    #[test]
    fn imports_env_without_evaluating_shell_or_leaking_credentials() {
        let preview = decode(
            b"IGNORED=value\nexport DATABASE_URL='postgres://alice:secret@localhost/example'\n",
            "",
        )
        .unwrap();
        assert_eq!(preview.profiles.len(), 1);
        assert!(preview.profiles[0].config.read_only);
        assert!(!review_label(&preview.profiles[0]).contains("secret"));
        assert!(decode(b"DATABASE_URL=$(run something)", "").is_err());
        assert!(decode(b"DATABASE_URL=postgres://${USER}:secret@host/db", "").is_err());
    }

    #[test]
    fn imports_compose_published_ports_and_skips_unresolved_secrets() {
        let yaml = b"services:\n  local-db:\n    image: postgres:17\n    ports: [\"15432:5432\"]\n    environment:\n      POSTGRES_USER: alice\n      POSTGRES_PASSWORD: secret\n      POSTGRES_DB: sample\n    command: rm -rf /\n  unresolved:\n    image: mysql:8\n    ports: [\"13306:3306\"]\n    environment: [\"MYSQL_PASSWORD=${SECRET}\"]\n";
        let preview = decode(yaml, "").unwrap();
        assert_eq!(preview.profiles.len(), 1);
        assert!(preview.profiles[0].config.url.contains(":15432/sample"));
        assert_eq!(
            preview.profiles[0].folder.as_deref(),
            Some("Docker Compose")
        );
        assert!(
            preview
                .warnings
                .iter()
                .any(|warning| warning.contains("unresolved"))
        );
        assert!(
            !serde_json::to_string(&preview.profiles)
                .unwrap()
                .contains("rm -rf")
        );
    }

    #[test]
    fn imports_dbeaver_folder_and_ignores_startup_sql_and_credentials_store() {
        let json = br#"{"folders":{"f":{"name":"Team"}},"connections":{"pg":{"name":"Example","provider":"postgresql","folder":"f","configuration":{"url":"jdbc:postgresql://localhost:5432/sample","user":"alice","bootstrap":{"initQueries":["DROP TABLE users"]}}}}}"#;
        let preview = decode(json, "").unwrap();
        assert_eq!(preview.profiles[0].folder.as_deref(), Some("Team"));
        assert_eq!(preview.profiles[0].config.kind, DatabaseKind::PostgreSQL);
        assert!(preview.profiles[0].config.read_only);
        assert!(
            !serde_json::to_string(&preview.profiles)
                .unwrap()
                .contains("DROP TABLE")
        );
    }

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
