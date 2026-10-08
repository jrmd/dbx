//! Installed native backup tools. No shell, password argv, or direct writes to
//! the final archive. PostgreSQL restore is one transaction; MySQL DDL is not.
use crate::{ConnectionConfig, DatabaseKind, DbxError, Result, TransferControl};
use std::{io::Write, path::Path, process::Stdio, time::Duration};
use tokio::io::BufReader;
use url::Url;
use zeroize::Zeroizing;

fn invalid(message: &str) -> DbxError {
    DbxError::InvalidConfig(message.into())
}
fn io(error: std::io::Error) -> DbxError {
    DbxError::Io(error.to_string())
}
fn pg_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(':', "\\:")
}
fn option_escape(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}
fn decoded(value: &str) -> Result<String> {
    crate::connectors::decode(value)
}

pub async fn native_tool_version(kind: DatabaseKind, restore: bool) -> Result<String> {
    let program = tool(kind, restore)?;
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(program)
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| invalid("Native tool version check timed out"))?
    .map_err(|_| invalid(&format!("Install {program} and make it available in PATH")))?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return Err(invalid("Native tool version check failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
fn tool(kind: DatabaseKind, restore: bool) -> Result<&'static str> {
    match (kind, restore) {
        (DatabaseKind::PostgreSQL, false) => Ok("pg_dump"),
        (DatabaseKind::PostgreSQL, true) => Ok("pg_restore"),
        (DatabaseKind::MySQL, false) => Ok("mysqldump"),
        (DatabaseKind::MySQL, true) => Ok("mysql"),
        _ => Err(invalid("Native backup supports PostgreSQL and MySQL")),
    }
}

/// A source is executable database input. Callers must display target and
/// obtain a restore confirmation before invoking this function with restore.
pub async fn native_backup(
    mut config: ConnectionConfig,
    path: &Path,
    restore: bool,
    control: TransferControl,
) -> Result<String> {
    config.validate()?;
    if restore && config.read_only {
        return Err(invalid("Protected connection: restore is disabled"));
    }
    if config.kind == DatabaseKind::MySQL && (config.ssh.is_some() || config.socket.is_some()) {
        let url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL"))?;
        if url.query_pairs().any(|(key, mode)| {
            (key == "sslmode" || key == "ssl-mode")
                && !matches!(
                    mode.to_lowercase().as_str(),
                    "disable" | "disabled" | "prefer" | "preferred"
                )
        }) {
            return Err(invalid(
                "The native MySQL client does not verify TLS through Unix sockets. Use a direct verified TCP profile for native backup/restore; DBX query sessions retain TLS verification over SSH.",
            ));
        }
    }
    let version = native_tool_version(config.kind, restore).await?;
    control.add_log(&version);
    if !restore && config.kind == DatabaseKind::PostgreSQL {
        let engine = crate::DatabaseEngine::connect(config.clone()).await?;
        let server = engine
            .query(
                "SELECT current_setting('server_version_num')::bigint",
                crate::QueryOptions { max_rows: Some(1) },
            )
            .await?;
        let number = server
            .rows
            .first()
            .and_then(|row| row.values.first())
            .map(ToString::to_string)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| invalid("Cannot determine PostgreSQL server version"))?;
        let client_major = version
            .split_whitespace()
            .find_map(|word| word.split('.').next()?.parse::<u64>().ok())
            .ok_or_else(|| invalid("Cannot determine pg_dump version"))?;
        let server_major = number / 10000;
        if server_major > client_major {
            return Err(invalid(
                "pg_dump is older than the server. Install a matching or newer PostgreSQL client.",
            ));
        }
        control.add_log(&format!(
            "Server major version: {server_major}; client major version: {client_major}"
        ));
    }
    if let Some(provider) = config.cloud_auth {
        let token = Zeroizing::new(crate::cloud_database_password(&config, provider).await?);
        let mut url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL"))?;
        url.set_password(Some(&token.replace('%', "%25")))
            .map_err(|_| invalid("Invalid credential URL"))?;
        config.url = url.into();
    }
    let original = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL"))?;
    let host = original.host_str().unwrap_or("localhost").to_owned();
    let user = decoded(original.username())?;
    let password = Zeroizing::new(
        original
            .password()
            .map(decoded)
            .transpose()?
            .unwrap_or_default(),
    );
    let database = decoded(original.path().trim_start_matches('/'))?;
    if database.is_empty() || database.starts_with('-') || database.contains(['\0', '\n', '\r']) {
        return Err(invalid(
            "Choose an explicit database for native backup or restore",
        ));
    }
    let kind = config.kind;
    // Native libpq ignores TLS on sockets. Always use TCP for PostgreSQL SSH
    // and pass hostaddr separately. The tunnel validates the original TLS mode.
    let (config, _tunnel) = if kind == DatabaseKind::PostgreSQL && config.ssh.is_some() {
        crate::transport::prepare_native_postgres(config).await?
    } else {
        crate::transport::prepare(config).await?
    };
    let url = Url::parse(&config.url).map_err(|_| invalid("Invalid forwarded URL"))?;
    let directory = tempfile::Builder::new()
        .prefix("dbx-backup-")
        .tempdir()
        .map_err(io)?;
    let credentials = directory.path().join("credentials");
    let mut secret_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&credentials)
        .map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o600))
            .map_err(io)?;
    }
    #[cfg(not(unix))]
    return Err(invalid(
        "Native backup credential-file isolation currently requires Unix",
    ));
    let program = tool(kind, restore)?;
    let mut command = tokio::process::Command::new(program);
    // Child-specific credentials replace ambient login files and environment.
    command
        .env_remove("PGPASSWORD")
        .env_remove("MYSQL_PWD")
        .env_remove("PGSERVICE")
        .env_remove("PGSERVICEFILE");
    if kind == DatabaseKind::PostgreSQL {
        let port = url.port().unwrap_or(5432);
        writeln!(
            secret_file,
            "{}:{}:{}:{}:{}",
            pg_escape(&host),
            port,
            pg_escape(&database),
            pg_escape(&user),
            pg_escape(&password)
        )
        .map_err(io)?;
        command
            .env("PGPASSFILE", &credentials)
            .env("PGHOST", &host)
            .env("PGPORT", port.to_string())
            .env("PGDATABASE", &database)
            .env("PGUSER", &user)
            .env(
                "PGCONNECT_TIMEOUT",
                config.connect_timeout_ms.div_ceil(1000).to_string(),
            );
        if config.ssh.is_none() && url.host_str() == Some("127.0.0.1") && host != "127.0.0.1" {
            command.env("PGHOSTADDR", "127.0.0.1");
        } else if let Some(socket) = &config.socket {
            command.env("PGHOST", socket);
        } else {
            command.env_remove("PGHOSTADDR");
        }
        for (key, variable) in [
            ("sslmode", "PGSSLMODE"),
            ("sslrootcert", "PGSSLROOTCERT"),
            ("sslcert", "PGSSLCERT"),
            ("sslkey", "PGSSLKEY"),
        ] {
            if let Some((_, value)) = original
                .query_pairs()
                .find(|(candidate, _)| candidate == key)
            {
                command.env(variable, value.as_ref());
            } else {
                command.env_remove(variable);
            }
        }
        command.arg("--no-password");
        if restore {
            command
                .args(["--exit-on-error", "--single-transaction", "--dbname"])
                .arg(format!(
                    "dbname='{}'",
                    database.replace('\\', "\\\\").replace('\'', "\\'")
                ))
                .arg(path);
        } else {
            command.args(["--format=custom", "--verbose"]);
        }
    } else {
        writeln!(
            secret_file,
            "[client]\nuser={}\npassword={}",
            option_escape(&user),
            option_escape(&password)
        )
        .map_err(io)?;
        command
            .arg(format!("--defaults-file={}", credentials.display()))
            .arg("--no-login-paths");
        command
            .arg(format!("--host={host}"))
            .arg(format!("--port={}", url.port().unwrap_or(3306)));
        if let Some(socket) = &config.socket {
            command
                .arg("--protocol=SOCKET")
                .arg(format!("--socket={}", socket.display()));
        } else {
            command
                .arg("--protocol=TCP")
                .arg(format!("--host={}", url.host_str().unwrap_or("localhost")));
        }
        let options = original.query_pairs().collect::<Vec<_>>();
        if let Some((_, mode)) = options
            .iter()
            .find(|(key, _)| key == "sslmode" || key == "ssl-mode")
        {
            let mode = match mode.to_lowercase().as_str() {
                "disable" | "disabled" => "DISABLED",
                "prefer" | "preferred" => "PREFERRED",
                "require" | "required" => "REQUIRED",
                "verify-ca" | "verify_ca" => "VERIFY_CA",
                "verify-full" | "verify_identity" | "verify-identity" => "VERIFY_IDENTITY",
                _ => return Err(invalid("Unknown MySQL TLS mode")),
            };
            command.arg(format!("--ssl-mode={mode}"));
        }
        for option in ["ssl-ca", "ssl-cert", "ssl-key"] {
            if let Some((_, value)) = options.iter().find(|(key, _)| key == option) {
                command.arg(format!("--{option}={value}"));
            }
        }
        if !restore {
            command.args([
                "--single-transaction",
                "--routines",
                "--events",
                "--triggers",
                "--hex-blob",
                "--verbose",
            ]);
        }
        command.arg(&database);
    }
    secret_file.sync_all().map_err(io)?;
    drop(secret_file);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let output = if restore {
        None
    } else {
        Some(tempfile::NamedTempFile::new_in(parent).map_err(io)?)
    };
    if let Some(output) = &output {
        command
            .stdout(Stdio::from(output.reopen().map_err(io)?))
            .stdin(Stdio::null());
    } else if kind == DatabaseKind::MySQL {
        command
            .stdin(Stdio::from(std::fs::File::open(path).map_err(io)?))
            .stdout(Stdio::null());
    } else {
        command.stdin(Stdio::null()).stdout(Stdio::null());
    }
    command.stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().map_err(io)?;
    let mut stderr = BufReader::new(child.stderr.take().expect("piped stderr"));
    let mut line = Vec::new();
    let wait = async {
        // Bound each read as well as the retained log, so a tool cannot emit
        // one unbounded diagnostic line. Drain all output to avoid deadlock.
        use tokio::io::AsyncReadExt;
        let mut buffer = [0u8; 4096];
        let mut omitted = false;
        loop {
            let count = stderr.read(&mut buffer).await.map_err(io)?;
            if count == 0 {
                break;
            }
            for byte in &buffer[..count] {
                if *byte == b'\n' {
                    if omitted {
                        control.add_log("[oversized diagnostic omitted]");
                    } else {
                        control.add_log(&redact_diagnostic(&line, &password));
                    }
                    line.clear();
                    omitted = false;
                } else if !omitted {
                    // Passwords cannot straddle redaction boundaries: hold the
                    // entire line or omit it. Native tool diagnostics are UTF-8.
                    line.push(*byte);
                    if line.len() > 64 * 1024 {
                        line.clear();
                        omitted = true;
                    }
                }
            }
            if let Some(output) = &output {
                control.set_bytes(output.as_file().metadata().map_err(io)?.len());
            }
        }
        if omitted {
            control.add_log("[oversized diagnostic omitted]");
        } else if !line.is_empty() {
            control.add_log(&redact_diagnostic(&line, &password));
        }
        child.wait().await.map_err(io)
    };
    let progress = async {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            if let Some(output) = &output
                && let Ok(metadata) = output.as_file().metadata()
            {
                control.set_bytes(metadata.len());
            }
        }
    };
    let status = tokio::select! {
        biased;
        _ = control.cancelled() => { return Err(DbxError::Interrupted(if restore && kind == DatabaseKind::MySQL { "Restore cancelled. MySQL DDL may already have committed; reconnect and inspect the target before retrying." } else { "Native job cancelled. An unfinished backup is removed; PostgreSQL restore rolls back its transaction." }.into())); },
        result = wait => result?,
        _ = progress => unreachable!("progress runs until the native job ends"),
    };
    if !status.success() {
        return Err(DbxError::Query(format!(
            "{program} exited with {status}. {}",
            control.log()
        )));
    }
    if let Some(output) = output {
        output.as_file().sync_all().map_err(io)?;
        output.persist(path).map_err(|error| io(error.error))?;
    }
    Ok(version)
}

fn redact_diagnostic(line: &[u8], password: &str) -> String {
    let line = String::from_utf8_lossy(line);
    if password.is_empty() {
        line.into_owned()
    } else {
        line.replace(password, "<redacted>")
    }
}
