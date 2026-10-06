//! Fetch short-lived database credentials from an already signed-in cloud CLI.
use crate::{ConnectionConfig, DatabaseKind, DbxError, Result};
use std::process::Stdio;

#[derive(Clone, Copy)]
pub enum CloudAuthentication {
    AwsRdsIam,
    AzureEntra,
}

pub async fn cloud_database_password(
    config: &ConnectionConfig,
    provider: CloudAuthentication,
) -> Result<String> {
    let mut command = authentication_command(config, provider)?;
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .map_err(|_| DbxError::Connection("Cloud authentication timed out".into()))?
        .map_err(|_| {
            DbxError::Connection(
                "Install the cloud CLI and sign in before requesting a token".into(),
            )
        })?;
    if !output.status.success() {
        return Err(DbxError::Connection(
            "Cloud authentication failed; check CLI login, default region and database permissions"
                .into(),
        ));
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| DbxError::Connection("Cloud CLI returned an invalid token".into()))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(DbxError::Connection("Cloud CLI returned no token".into()));
    }
    Ok(token.to_owned())
}

fn authentication_command(
    config: &ConnectionConfig,
    provider: CloudAuthentication,
) -> Result<tokio::process::Command> {
    let url = url::Url::parse(&config.url)
        .map_err(|_| DbxError::InvalidConfig("Invalid database URL".into()))?;
    let command =
        match provider {
            CloudAuthentication::AwsRdsIam => {
                if !matches!(config.kind, DatabaseKind::PostgreSQL | DatabaseKind::MySQL) {
                    return Err(DbxError::InvalidConfig(
                        "RDS IAM requires PostgreSQL or MySQL".into(),
                    ));
                }
                let mut command = tokio::process::Command::new("aws");
                command
                    .args(["rds", "generate-db-auth-token", "--hostname"])
                    .arg(url.host_str().ok_or_else(|| {
                        DbxError::InvalidConfig("Database host is required".into())
                    })?)
                    .arg("--port")
                    .arg(
                        url.port()
                            .unwrap_or(if config.kind == DatabaseKind::MySQL {
                                3306
                            } else {
                                5432
                            })
                            .to_string(),
                    )
                    .arg("--username")
                    .arg(super::connectors::decode(url.username())?);
                command
            }
            CloudAuthentication::AzureEntra => {
                if config.kind != DatabaseKind::PostgreSQL {
                    return Err(DbxError::InvalidConfig(
                        "This Entra helper targets Azure PostgreSQL".into(),
                    ));
                }
                let mut command = tokio::process::Command::new("az");
                command.args([
                    "account",
                    "get-access-token",
                    "--resource-type",
                    "oss-rdbms",
                    "--query",
                    "accessToken",
                    "--output",
                    "tsv",
                ]);
                command
            }
        };
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_commands_use_database_fields_without_shell_interpolation_or_password_arguments() {
        let config = ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            "postgres://alice%40tenant:private-secret@db.example:5444/app",
        );
        let command = authentication_command(&config, CloudAuthentication::AwsRdsIam).unwrap();
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "rds",
                "generate-db-auth-token",
                "--hostname",
                "db.example",
                "--port",
                "5444",
                "--username",
                "alice@tenant"
            ]
        );
        assert!(!args.iter().any(|arg| arg.contains("private-secret")));
        let command = authentication_command(&config, CloudAuthentication::AzureEntra).unwrap();
        assert!(command.as_std().get_args().any(|arg| arg == "oss-rdbms"));
        let unsupported = ConnectionConfig::new(DatabaseKind::SQLite, "sqlite::memory:");
        assert!(authentication_command(&unsupported, CloudAuthentication::AwsRdsIam).is_err());
        assert!(authentication_command(&unsupported, CloudAuthentication::AzureEntra).is_err());
    }
}
