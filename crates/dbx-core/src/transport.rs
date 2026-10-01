//! Connection transport ownership. A tunnel lives exactly as long as its engine.
use std::{path::PathBuf, process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, process::Child};
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
        let url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL."))?;
        if config.socket.is_none()
            && !safe_host(url.host_str().unwrap_or("").trim_matches(['[', ']']))
        {
            return Err(invalid("Enter a valid database host for SSH forwarding."));
        }
        // SQLx uses one host for both the network address and certificate name.
        // Never silently weaken TLS when redirecting the connection to loopback.
        for (key, value) in url.query_pairs() {
            if matches!(key.as_ref(), "sslmode" | "ssl-mode")
                && (value.eq_ignore_ascii_case("verify-full")
                    || value.eq_ignore_ascii_case("verify_identity")
                    || value.eq_ignore_ascii_case("verify-identity"))
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
    child: Child,
    // Private control socket/log directory remains until the child is stopped.
    _directory: tempfile::TempDir,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        // Tokio reaps dropped children; kill_on_drop also covers cancellation.
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
    url.set_host(Some("127.0.0.1"))
        .map_err(|_| invalid("Invalid database URL."))?;
    url.set_port(Some(local_port))
        .map_err(|_| invalid("Invalid database port."))?;
    config.url = url.into();
    config.socket = None;
    config.ssh = None;
    Ok((config, Some(tunnel)))
}

async fn start(config: &ConnectionConfig, program: &str) -> Result<(Tunnel, u16)> {
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
            "BatchMode=yes",
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
        .arg(format!("127.0.0.1:{local_port}:{endpoint}"));
    if let Some(identity) = &ssh.identity_file {
        command
            .arg("-i")
            .arg(identity)
            .args(["-o", "IdentitiesOnly=yes"]);
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
    let child = command.spawn().map_err(|_| {
        DbxError::Connection(
            "Could not start OpenSSH. Install the ssh client and try again.".into(),
        )
    })?;
    let mut tunnel = Tunnel {
        child,
        _directory: directory,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_millis(config.connect_timeout_ms);
    loop {
        if tunnel
            .child
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
                return Ok((tunnel, local_port));
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
        assert!(config.validate().is_err());
        config.ssh = None;
        config.socket = Some("relative.sock".into());
        assert!(config.validate().is_err());
        config.socket = Some("/tmp".into());
        config.kind = DatabaseKind::SQLite;
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
