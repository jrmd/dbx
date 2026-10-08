//! Connection transport ownership. A tunnel lives exactly as long as its engine.
use std::{path::PathBuf, process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use url::Url;

use crate::{ConnectionConfig, DatabaseKind, DbxError, Result};

/// OpenSSH authentication uses the existing agent/config or an identity file.
/// No key contents or SSH passwords are persisted in connection metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    /// OpenSSH `ProxyJump` hops: comma-separated `[user@]host[:port]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host: Option<String>,
}

/// Accept only `[user@]host[:port]` hops, so a jump specification can never
/// become another OpenSSH option.
fn valid_jump_hosts(value: &str) -> bool {
    !value.is_empty()
        && value.split(',').all(|hop| {
            let (user, address) = match hop.split_once('@') {
                Some((user, address)) => (Some(user), address),
                None => (None, hop),
            };
            let user_ok = user.is_none_or(|user| {
                !user.is_empty()
                    && !user.starts_with('-')
                    && user
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            });
            let (host, port) = match address.strip_prefix('[') {
                Some(rest) => match rest.split_once(']') {
                    Some((host, port)) => (host, port.strip_prefix(':')),
                    None => return false,
                },
                None => match address.rsplit_once(':') {
                    Some((host, port)) => (host, Some(port)),
                    None => (address, None),
                },
            };
            let port_ok = port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port > 0));
            user_ok && port_ok && safe_host(host)
        })
}

fn invalid(message: &str) -> DbxError {
    DbxError::InvalidConfig(message.into())
}

fn safe_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_:".contains(&byte))
}

pub(crate) fn validate(config: &ConnectionConfig) -> Result<()> {
    if !config.kind.supports_transport() && (config.socket.is_some() || config.ssh.is_some()) {
        return Err(invalid(
            "This connector does not support Unix sockets or SSH forwarding.",
        ));
    }
    if let Some(socket) = &config.socket {
        if !socket.is_absolute() || socket.as_os_str().is_empty() {
            return Err(invalid("Socket path must be absolute."));
        }
        if !cfg!(unix) {
            return Err(invalid("Unix sockets are unavailable on this platform."));
        }
    }
    if let Some(ssh) = &config.ssh {
        if !safe_host(&ssh.host) || ssh.port == 0 {
            return Err(invalid("Enter a valid SSH host and port."));
        }
        if ssh.username.is_empty()
            || ssh.username.starts_with('-')
            || !ssh
                .username
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(invalid("Enter a valid SSH username."));
        }
        if let Some(path) = &ssh.identity_file
            && !path.is_absolute()
        {
            return Err(invalid("SSH private key path must be absolute."));
        }
        if let Some(jump) = &ssh.jump_host
            && !valid_jump_hosts(jump)
        {
            return Err(invalid(
                "Enter jump hosts as user@host:port, separated by commas.",
            ));
        }
        let url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL."))?;
        if config.socket.is_none()
            && !safe_host(url.host_str().unwrap_or("").trim_matches(['[', ']']))
        {
            return Err(invalid("Enter a valid database host for SSH forwarding."));
        }
        // Unix forwarding separates the dial socket from the original TLS host.
        // Reject strict TLS elsewhere rather than weakening verification.
        for (key, value) in url.query_pairs() {
            if !cfg!(unix)
                && matches!(key.as_ref(), "sslmode" | "ssl-mode")
                && strict_tls_mode(&value)
            {
                return Err(invalid(
                    "SSH forwarding cannot preserve TLS hostname verification in this driver. Use a direct connection for this TLS mode.",
                ));
            }
            if matches!(key.as_ref(), "host" | "hostaddr" | "socket" | "port") {
                return Err(invalid(
                    "With SSH, set the database address and socket in the connection fields rather than URL address overrides.",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) struct Tunnel {
    supervisor: tokio::task::JoinHandle<()>,
    local_socket: Option<PathBuf>,
    #[cfg(test)]
    initial_pid: u32,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.supervisor.abort();
        // The supervised child uses kill_on_drop, including during reconnect.
    }
}

fn forwarded_address(config: &ConnectionConfig, port: u16) -> Result<String> {
    if let Some(path) = &config.socket {
        let path = if config.kind.dialect() == DatabaseKind::PostgreSQL {
            path.join(format!(".s.PGSQL.{port}"))
        } else {
            path.clone()
        };
        let path = path
            .to_str()
            .ok_or_else(|| invalid("Socket path must be UTF-8 for SSH."))?;
        if path.contains([':', '\n', '\r']) {
            return Err(invalid(
                "Remote socket path cannot contain colons or newlines.",
            ));
        }
        return Ok(path.to_owned());
    }
    let url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL."))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("Database host is required."))?;
    Ok(format!("{host}:{port}"))
}

pub(crate) async fn prepare(
    mut config: ConnectionConfig,
) -> Result<(ConnectionConfig, Option<Tunnel>)> {
    config.validate()?;
    if config.ssh.is_none() {
        return Ok((config, None));
    }
    let (tunnel, local_port) = start(&config, "ssh").await?;
    let mut url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL."))?;
    if let Some(socket) = &tunnel.local_socket {
        // SQLx retains url.host as the certificate identity while dialing this
        // tunnel-owned socket. PostgreSQL derives its socket filename from port.
        config.socket = Some(if config.kind.dialect() == DatabaseKind::PostgreSQL {
            socket.parent().expect("tunnel directory").to_path_buf()
        } else {
            socket.clone()
        });
        if config.kind.dialect() == DatabaseKind::PostgreSQL {
            url.set_port(Some(local_port))
                .map_err(|_| invalid("Invalid database port."))?;
        }
    } else {
        url.set_host(Some("127.0.0.1"))
            .map_err(|_| invalid("Invalid database URL."))?;
        url.set_port(Some(local_port))
            .map_err(|_| invalid("Invalid database port."))?;
        config.socket = None;
    }
    config.url = url.into();
    config.ssh = None;
    config.ssh_password = None;
    Ok((config, Some(tunnel)))
}

fn strict_tls_mode(value: &str) -> bool {
    ["verify-full", "verify_identity", "verify-identity"]
        .iter()
        .any(|mode| value.eq_ignore_ascii_case(mode))
}

fn tunnel_socket(
    config: &ConnectionConfig,
    directory: &std::path::Path,
    port: u16,
) -> Result<Option<PathBuf>> {
    let url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL."))?;
    let strict = url.query_pairs().any(|(key, value)| {
        matches!(key.as_ref(), "sslmode" | "ssl-mode") && strict_tls_mode(&value)
    });
    if !strict || !cfg!(unix) {
        return Ok(None);
    }
    let filename = if config.kind.dialect() == DatabaseKind::PostgreSQL {
        format!(".s.PGSQL.{port}")
    } else {
        "mysql.sock".into()
    };
    let path = directory.join(filename);
    if path.as_os_str().len() >= 104 {
        return Err(invalid(
            "SSH socket path is too long. Use a shorter temporary directory.",
        ));
    }
    Ok(Some(path))
}

async fn start(config: &ConnectionConfig, program: &str) -> Result<(Tunnel, u16)> {
    start_with_socket(config, program, true).await
}

/// libpq can separate TCP dial address from TLS identity using hostaddr.
pub(crate) async fn prepare_native_postgres(
    mut config: ConnectionConfig,
) -> Result<(ConnectionConfig, Option<Tunnel>)> {
    config.validate()?;
    if config.ssh.is_none() {
        return Ok((config, None));
    }
    let (tunnel, port) = start_with_socket(&config, "ssh", false).await?;
    let mut url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL"))?;
    url.set_host(Some("127.0.0.1"))
        .map_err(|_| invalid("Invalid forwarded host"))?;
    url.set_port(Some(port))
        .map_err(|_| invalid("Invalid forwarded port"))?;
    config.url = url.into();
    config.socket = None;
    config.ssh = None;
    config.ssh_password = None;
    Ok((config, Some(tunnel)))
}

async fn start_with_socket(
    config: &ConnectionConfig,
    program: &str,
    allow_socket: bool,
) -> Result<(Tunnel, u16)> {
    let ssh = config.ssh.as_ref().expect("validated SSH configuration");
    let url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL."))?;
    let remote_port = url.port().unwrap_or(match config.kind {
        DatabaseKind::PostgreSQL => 5432,
        DatabaseKind::CockroachDB => 26257,
        DatabaseKind::MySQL => 3306,
        DatabaseKind::Redis => 6379,
        _ => unreachable!("transport validated"),
    });
    let endpoint = forwarded_address(config, remote_port)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| invalid("Cannot allocate an SSH forwarding port."))?;
    let local_port = listener
        .local_addr()
        .map_err(|_| invalid("Cannot allocate an SSH forwarding port."))?
        .port();
    let directory = tempfile::Builder::new()
        .prefix("dbx-ssh-")
        .tempdir()
        .map_err(|_| invalid("Cannot create SSH tunnel directory."))?;
    let local_socket = if allow_socket {
        tunnel_socket(config, directory.path(), local_port)?
    } else {
        None
    };
    let forwarding = match &local_socket {
        Some(socket) => format!("{}:{endpoint}", socket.display()),
        None => format!("127.0.0.1:{local_port}:{endpoint}"),
    };
    let control = directory.path().join("control");
    let log = directory.path().join("stderr");
    let log_file =
        std::fs::File::create(&log).map_err(|_| invalid("Cannot create SSH diagnostics."))?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(["-N", "-T", "-a", "-M", "-S"])
        .arg(&control)
        .args([
            "-o",
            if config.ssh_password.is_some() {
                "BatchMode=no"
            } else {
                "BatchMode=yes"
            },
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ControlPersist=no",
            "-o",
            "ForkAfterAuthentication=no",
            "-o",
            "ServerAliveInterval=30",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "PermitLocalCommand=no",
            "-o",
            "ClearAllForwardings=no",
        ])
        .arg("-o")
        .arg(format!(
            "ConnectTimeout={}",
            config.connect_timeout_ms.div_ceil(1000)
        ))
        .arg("-p")
        .arg(ssh.port.to_string())
        .arg("-l")
        .arg(&ssh.username)
        .arg("-L")
        .arg(forwarding);
    if let Some(password) = &config.ssh_password {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::PermissionsExt;
            let secret_path = directory.path().join("password");
            let askpass_path = directory.path().join("askpass");
            let mut secret = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&secret_path)
                .map_err(|_| invalid("Cannot create SSH authentication file"))?;
            std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| invalid("Cannot protect SSH authentication file"))?;
            secret
                .write_all(password.as_bytes())
                .map_err(|_| invalid("Cannot prepare SSH authentication"))?;
            std::fs::write(
                &askpass_path,
                "#!/bin/sh\nexec cat -- \"$DBX_SSH_PASSWORD_FILE\"\n",
            )
            .map_err(|_| invalid("Cannot prepare SSH password helper"))?;
            std::fs::set_permissions(&askpass_path, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| invalid("Cannot protect SSH password helper"))?;
            command
                .env("SSH_ASKPASS", askpass_path)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", "dbx")
                .env("DBX_SSH_PASSWORD_FILE", secret_path)
                .args(["-o", "NumberOfPasswordPrompts=1"]);
        }
        #[cfg(not(unix))]
        return Err(invalid(
            "SSH password authentication currently requires Unix",
        ));
    }
    if let Some(identity) = &ssh.identity_file {
        command
            .arg("-i")
            .arg(identity)
            .args(["-o", "IdentitiesOnly=yes"]);
    }
    if let Some(jump) = &ssh.jump_host {
        command.arg("-J").arg(jump);
    }
    command
        .arg("--")
        .arg(&ssh.host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file))
        .kill_on_drop(true);
    // OpenSSH owns the actual listener. ExitOnForwardFailure prevents a stolen
    // port from being mistaken for a tunnel established by this process.
    drop(listener);
    let mut child = command.spawn().map_err(|_| {
        DbxError::Connection(
            "Could not start OpenSSH. Install the ssh client and try again.".into(),
        )
    })?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(config.connect_timeout_ms);
    loop {
        if child
            .try_wait()
            .map_err(|_| invalid("Cannot inspect SSH process."))?
            .is_some()
        {
            let error = std::fs::read_to_string(&log).unwrap_or_default();
            let detail: String = error.chars().take(4096).collect();
            return Err(DbxError::Connection(format!(
                "SSH tunnel failed: {}. Verify the host in a terminal with ssh first, and load encrypted keys into your SSH agent.",
                detail.trim()
            )));
        }
        if control.exists() {
            let ready = tokio::process::Command::new(program)
                .arg("-S")
                .arg(&control)
                .args(["-O", "check", "--"])
                .arg(&ssh.host)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status();
            if tokio::time::timeout_at(deadline, ready)
                .await
                .is_ok_and(|result| result.is_ok_and(|status| status.success()))
            {
                #[cfg(test)]
                let initial_pid = child.id().unwrap();
                let restart_socket = local_socket.clone();
                let supervisor = tokio::spawn(async move {
                    let _directory = directory;
                    loop {
                        let _ = child.wait().await;
                        // Re-establish only the transport, on the same forwarding
                        // port. Database statements are never replayed here.
                        loop {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let _ = std::fs::remove_file(&control);
                            if let Some(socket) = &restart_socket {
                                let _ = std::fs::remove_file(socket);
                            }
                            match command.spawn() {
                                Ok(replacement) => {
                                    child = replacement;
                                    break;
                                }
                                Err(_) => continue,
                            }
                        }
                    }
                });
                return Ok((
                    Tunnel {
                        supervisor,
                        local_socket,
                        #[cfg(test)]
                        initial_pid,
                    },
                    local_port,
                ));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(DbxError::Connection(
                "SSH authentication or forwarding timed out. Check the host, key and SSH agent."
                    .into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "run scripts/test-transports.py for the local SSH fixture"]
    async fn ssh_tunnel_lifetime_integration() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let root = PathBuf::from(std::env::var("DBX_TEST_TRANSPORT_DIRECTORY").unwrap());
        let mut config = ConnectionConfig::new(
            DatabaseKind::Redis,
            format!(
                "redis://127.0.0.1:{}/0",
                std::env::var("DBX_TEST_TRANSPORT_REDIS_PORT").unwrap()
            ),
        );
        config.ssh = Some(SshConfig {
            host: "127.0.0.1".into(),
            port: std::env::var("DBX_TEST_SSH_PORT").unwrap().parse().unwrap(),
            username: std::env::var("DBX_TEST_SSH_USER").unwrap(),
            identity_file: Some(root.join("identity")),
            jump_host: None,
        });
        let (forwarded, tunnel) = prepare(config).await.unwrap();
        assert!(forwarded.ssh.is_none());
        let url = Url::parse(&forwarded.url).unwrap();
        let port = url.port().unwrap();
        let mut connection = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        connection.write_all(b"*1\r\n$4\r\nPING\r\n").await.unwrap();
        let mut response = [0u8; 7];
        tokio::time::timeout(Duration::from_secs(3), connection.read_exact(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&response, b"+PONG\r\n");
        let pid = tunnel.as_ref().unwrap().initial_pid;
        assert!(
            tokio::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()
                .await
                .unwrap()
                .success()
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            if let Ok(mut recovered) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await
                && recovered.write_all(b"*1\r\n$4\r\nPING\r\n").await.is_ok()
                && tokio::time::timeout(
                    Duration::from_millis(500),
                    recovered.read_exact(&mut response),
                )
                .await
                .is_ok_and(|result| result.is_ok())
            {
                assert_eq!(&response, b"+PONG\r\n");
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "SSH tunnel did not reconnect on its original port"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(tunnel);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "SSH listener leaked after dropping its guard"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    #[ignore = "run scripts/test-transports.py for disposable password-authenticated SSH"]
    async fn ssh_password_integration() {
        let mut config = ConnectionConfig::new(DatabaseKind::Redis, "redis://127.0.0.1:6379/0");
        config.ssh = Some(SshConfig {
            host: "127.0.0.1".into(),
            port: std::env::var("DBX_TEST_SSH_PASSWORD_PORT")
                .unwrap()
                .parse()
                .unwrap(),
            username: "dbx_password".into(),
            identity_file: None,
            jump_host: None,
        });
        config.ssh_password = Some("disposable-fixture-password".into());
        let engine = crate::DatabaseEngine::connect(config).await.unwrap();
        let result = engine
            .query("PING", crate::QueryOptions::default())
            .await
            .unwrap();
        assert_eq!(result.rows[0].values[0].to_string(), "PONG");
    }

    #[test]
    fn legacy_connections_default_to_direct_tcp() {
        let config: ConnectionConfig =
            serde_json::from_str(r#"{"kind":"postgresql","url":"postgres://localhost/app"}"#)
                .unwrap();
        assert_eq!(config.socket, None);
        assert_eq!(config.ssh, None);
        assert!(config.validate().is_ok());
    }

    fn config() -> ConnectionConfig {
        let mut config = ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            "postgres://user:secret@db.internal:5433/app?sslmode=require",
        );
        config.ssh = Some(SshConfig {
            host: "bastion.example".into(),
            port: 22,
            username: "alice".into(),
            identity_file: None,
            jump_host: None,
        });
        config
    }

    #[test]
    fn validates_transports_and_keeps_secrets_redacted() {
        let mut config = config();
        assert!(config.validate().is_ok());
        assert!(!format!("{config:?}").contains("secret"));
        config.ssh.as_mut().unwrap().host = "-oProxyCommand=bad".into();
        assert!(config.validate().is_err());
        config.ssh.as_mut().unwrap().host = "bastion.example".into();
        config.url = "postgres://db/app?sslmode=verify-full".into();
        assert_eq!(config.validate().is_ok(), cfg!(unix));
        if cfg!(unix) {
            let socket =
                tunnel_socket(&config, std::path::Path::new("/tmp/dbx-test"), 5433).unwrap();
            assert_eq!(socket, Some(PathBuf::from("/tmp/dbx-test/.s.PGSQL.5433")));
        }
        config.ssh = None;
        config.socket = Some("relative.sock".into());
        assert!(config.validate().is_err());
        config.socket = Some("/tmp".into());
        config.kind = DatabaseKind::SQLite;
        assert!(config.validate().is_err());
    }

    #[test]
    fn jump_hosts_accept_only_user_host_port_hops() {
        for valid in [
            "bastion",
            "ops@bastion:2222,jump.internal",
            "[::1]:22",
            "a@[fe80::1]",
        ] {
            assert!(valid_jump_hosts(valid), "{valid}");
        }
        for invalid in [
            "",
            "-oProxyCommand=x",
            "a@-b",
            "host:0",
            "host:port",
            "a b",
            "[::1",
        ] {
            assert!(!valid_jump_hosts(invalid), "{invalid}");
        }
        let mut config = config();
        config.ssh.as_mut().unwrap().jump_host = Some("-J evil".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn remote_sockets_use_the_database_port_and_paths() {
        let mut config = config();
        config.socket = Some("/var/run/postgresql".into());
        assert_eq!(
            forwarded_address(&config, 5433).unwrap(),
            "/var/run/postgresql/.s.PGSQL.5433"
        );
        config.kind = DatabaseKind::MySQL;
        config.socket = Some("/tmp/mysql.sock".into());
        assert_eq!(forwarded_address(&config, 3306).unwrap(), "/tmp/mysql.sock");
    }
}
