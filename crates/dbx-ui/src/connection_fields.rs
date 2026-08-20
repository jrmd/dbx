//! A UI-friendly, secret-safe representation of a database address.

use std::fmt;

use dbx_core::DatabaseKind;
use thiserror::Error;
use url::Url;

/// Editable connection fields. `connection_string` is an optional fast path:
/// when set, [`Self::url`] returns it after validation instead of rebuilding a
/// URL from the structured fields. SQLite deliberately uses that path because
/// its address is a file or SQLite URL rather than a network address.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionFields {
    pub kind: DatabaseKind,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub connection_string: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectionFieldsError {
    #[error("connection string is required for SQLite")]
    MissingSqliteConnectionString,
    #[error("host is required for {0}")]
    MissingHost(DatabaseKind),
    #[error("port is required for {0}")]
    MissingPort(DatabaseKind),
    #[error("port must be a number between 1 and 65535")]
    InvalidPort,
    #[error("database is required for {0}")]
    MissingDatabase(DatabaseKind),
    #[error("connection string is not a valid URL")]
    InvalidUrl,
    #[error("unsupported connection URL scheme: {0}")]
    UnsupportedScheme(String),
    #[error("connection string scheme does not match {expected}")]
    MismatchedScheme { expected: DatabaseKind },
}

impl ConnectionFields {
    /// Create blank editable fields with the conventional port for `kind`.
    pub fn new(kind: DatabaseKind) -> Self {
        Self {
            kind,
            port: kind.default_port().map(str::to_owned).unwrap_or_default(),
            host: String::new(),
            username: String::new(),
            password: String::new(),
            database: String::new(),
            connection_string: String::new(),
        }
    }

    /// Parse a supplied URL into editable fields. The original is retained as
    /// the fast path, preserving query parameters and less-common URL options.
    pub fn from_url(connection_string: impl Into<String>) -> Result<Self, ConnectionFieldsError> {
        let connection_string = connection_string.into();
        let url = Url::parse(&connection_string).map_err(|_| ConnectionFieldsError::InvalidUrl)?;
        let kind = kind_for_scheme(url.scheme())?;
        if kind == DatabaseKind::SQLite {
            return Ok(Self {
                kind,
                connection_string,
                ..Self::new(kind)
            });
        }

        let host = url
            .host_str()
            .ok_or(ConnectionFieldsError::MissingHost(kind))?
            .trim_matches(['[', ']']);
        let username = decode(url.username());
        let password = url.password().map(decode).unwrap_or_default();
        let database = decode(url.path().trim_start_matches('/'));
        Ok(Self {
            kind,
            host: host.to_owned(),
            port: url
                .port_or_known_default()
                .map(|port| port.to_string())
                .unwrap_or_default(),
            username,
            password,
            database,
            connection_string,
        })
    }

    /// Stop using the supplied-string fast path and build a URL from the
    /// structured fields instead.
    pub fn use_structured_fields(&mut self) {
        self.connection_string.clear();
    }

    /// Validate and return the address used to open a connection.
    pub fn url(&self) -> Result<String, ConnectionFieldsError> {
        if !self.connection_string.trim().is_empty() {
            let url = Url::parse(&self.connection_string)
                .map_err(|_| ConnectionFieldsError::InvalidUrl)?;
            let actual = kind_for_scheme(url.scheme())?;
            if actual != self.kind {
                return Err(ConnectionFieldsError::MismatchedScheme {
                    expected: self.kind,
                });
            }
            return Ok(self.connection_string.clone());
        }
        self.structured_url()
    }

    /// Return the address without a password, suitable for labels and logs.
    pub fn redacted_url(&self) -> Result<String, ConnectionFieldsError> {
        let mut url = Url::parse(&self.url()?).map_err(|_| ConnectionFieldsError::InvalidUrl)?;
        let _ = url.set_password(None);
        Ok(url.to_string())
    }

    fn structured_url(&self) -> Result<String, ConnectionFieldsError> {
        if self.kind == DatabaseKind::SQLite {
            return Err(ConnectionFieldsError::MissingSqliteConnectionString);
        }
        if self.host.trim().is_empty() {
            return Err(ConnectionFieldsError::MissingHost(self.kind));
        }
        if self.port.trim().is_empty() {
            return Err(ConnectionFieldsError::MissingPort(self.kind));
        }
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| ConnectionFieldsError::InvalidPort)?;
        if port == 0 {
            return Err(ConnectionFieldsError::InvalidPort);
        }
        if matches!(self.kind, DatabaseKind::PostgreSQL | DatabaseKind::MySQL)
            && self.database.trim().is_empty()
        {
            return Err(ConnectionFieldsError::MissingDatabase(self.kind));
        }

        let mut url = Url::parse(&format!("{}://placeholder", self.kind.scheme()))
            .expect("database schemes are valid URLs");
        let host = self.host.trim();
        let url_host = if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        url.set_host(Some(&url_host))
            .map_err(|_| ConnectionFieldsError::InvalidUrl)?;
        url.set_port(Some(port))
            .map_err(|_| ConnectionFieldsError::InvalidPort)?;
        if !self.username.is_empty() {
            url.set_username(&self.username)
                .map_err(|_| ConnectionFieldsError::InvalidUrl)?;
        }
        if !self.password.is_empty() {
            url.set_password(Some(&self.password))
                .map_err(|_| ConnectionFieldsError::InvalidUrl)?;
        }
        if !self.database.is_empty() {
            url.path_segments_mut()
                .map_err(|_| ConnectionFieldsError::InvalidUrl)?
                .push(&self.database);
        }
        Ok(url.to_string())
    }
}

impl Default for ConnectionFields {
    fn default() -> Self {
        Self::new(DatabaseKind::SQLite)
    }
}

impl fmt::Debug for ConnectionFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionFields")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("database", &self.database)
            .field("connection_string", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Display for ConnectionFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} connection", self.kind)
    }
}

fn kind_for_scheme(scheme: &str) -> Result<DatabaseKind, ConnectionFieldsError> {
    match scheme {
        "postgres" | "postgresql" => Ok(DatabaseKind::PostgreSQL),
        "mysql" => Ok(DatabaseKind::MySQL),
        "redis" => Ok(DatabaseKind::Redis),
        "sqlite" => Ok(DatabaseKind::SQLite),
        _ => Err(ConnectionFieldsError::UnsupportedScheme(scheme.to_owned())),
    }
}

fn decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

trait DatabaseKindUrlExt {
    fn default_port(self) -> Option<&'static str>;
}

impl DatabaseKindUrlExt for DatabaseKind {
    fn default_port(self) -> Option<&'static str> {
        match self {
            DatabaseKind::PostgreSQL => Some("5432"),
            DatabaseKind::MySQL => Some("3306"),
            DatabaseKind::Redis => Some("6379"),
            DatabaseKind::SQLite => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_postgres_url_with_percent_encoded_credentials_and_database() {
        let mut fields = ConnectionFields::new(DatabaseKind::PostgreSQL);
        fields.host = "db.example.test".into();
        fields.username = "al ice@example".into();
        fields.password = "p@ss word/#?".into();
        fields.database = "team/data".into();

        assert_eq!(
            fields.url().unwrap(),
            "postgres://al%20ice%40example:p%40ss%20word%2F%23%3F@db.example.test:5432/team%2Fdata"
        );
    }

    #[test]
    fn builds_urls_with_default_ports() {
        for (kind, expected) in [
            (DatabaseKind::PostgreSQL, "postgres://localhost:5432/main"),
            (DatabaseKind::MySQL, "mysql://localhost:3306/main"),
            (DatabaseKind::Redis, "redis://localhost:6379"),
        ] {
            let mut fields = ConnectionFields::new(kind);
            fields.host = "localhost".into();
            if kind != DatabaseKind::Redis {
                fields.database = "main".into();
            }
            assert_eq!(fields.url().unwrap(), expected);
        }
    }

    #[test]
    fn supports_ipv6_hosts() {
        let mut fields = ConnectionFields::new(DatabaseKind::Redis);
        fields.host = "::1".into();
        assert_eq!(fields.url().unwrap(), "redis://[::1]:6379");
    }

    #[test]
    fn parses_network_url_into_decoded_fields_and_round_trips_fast_path() {
        let input = "postgresql://al%20ice:p%40ss%2Fword@[::1]:5433/team%2Fdata?sslmode=require";
        let fields = ConnectionFields::from_url(input).unwrap();

        assert_eq!(fields.kind, DatabaseKind::PostgreSQL);
        assert_eq!(fields.host, "::1");
        assert_eq!(fields.port, "5433");
        assert_eq!(fields.username, "al ice");
        assert_eq!(fields.password, "p@ss/word");
        assert_eq!(fields.database, "team/data");
        assert_eq!(fields.url().unwrap(), input);
    }

    #[test]
    fn sqlite_keeps_file_connection_string() {
        let fields = ConnectionFields::from_url("sqlite://dbx.db?mode=rwc").unwrap();
        assert_eq!(fields.kind, DatabaseKind::SQLite);
        assert_eq!(fields.url().unwrap(), "sqlite://dbx.db?mode=rwc");
    }

    #[test]
    fn validates_required_structured_fields() {
        let fields = ConnectionFields::new(DatabaseKind::MySQL);
        assert_eq!(
            fields.url(),
            Err(ConnectionFieldsError::MissingHost(DatabaseKind::MySQL))
        );

        let mut fields = ConnectionFields::new(DatabaseKind::PostgreSQL);
        fields.host = "localhost".into();
        fields.port = "nope".into();
        assert_eq!(fields.url(), Err(ConnectionFieldsError::InvalidPort));

        let mut fields = ConnectionFields::new(DatabaseKind::MySQL);
        fields.host = "localhost".into();
        assert_eq!(
            fields.url(),
            Err(ConnectionFieldsError::MissingDatabase(DatabaseKind::MySQL))
        );
    }

    #[test]
    fn debug_and_display_redact_passwords() {
        let fields =
            ConnectionFields::from_url("redis://user:top-secret@localhost:6379/0").unwrap();
        assert!(!format!("{fields:?}").contains("top-secret"));
        assert!(!fields.to_string().contains("top-secret"));
        assert_eq!(
            fields.redacted_url().unwrap(),
            "redis://user@localhost:6379/0"
        );
    }
}
