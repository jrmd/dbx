use std::fmt;

use thiserror::Error;

/// Errors returned by DBX's connection and query layer.
#[derive(Debug, Error)]
pub enum DbxError {
    #[error("invalid database configuration: {0}")]
    InvalidConfig(String),

    #[error("unsupported operation `{operation}` for {kind}")]
    Unsupported {
        operation: String,
        kind: crate::DatabaseKind,
    },

    #[error("database connection failed: {0}")]
    Connection(String),

    #[error("database query failed: {0}")]
    Query(String),

    #[error("database value could not be decoded: {0}")]
    Decode(String),

    #[error("database command could not be parsed: {0}")]
    Parse(String),

    #[error("database driver error: {0}")]
    Driver(String),

    #[error("local file operation failed: {0}")]
    Io(String),
}

impl From<sqlx::Error> for DbxError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_) => Self::Connection(error.to_string()),
            other => Self::Query(other.to_string()),
        }
    }
}

impl From<redis::RedisError> for DbxError {
    fn from(error: redis::RedisError) -> Self {
        Self::Driver(error.to_string())
    }
}

/// Result alias used by all public operations.
pub type Result<T, E = DbxError> = std::result::Result<T, E>;

/// A redacted connection error suitable for displaying in the UI.
pub(crate) fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return "<redacted>".to_owned();
    };
    let scheme = &url[..scheme_end + 3];
    let Some(at) = url[scheme_end + 3..].find('@') else {
        return format!("{scheme}<redacted>");
    };
    format!("{scheme}<redacted>@{}", &url[scheme_end + 3 + at + 1..])
}

/// Formats an error with a connection URL stripped of credentials.
pub(crate) fn connection_message(url: &str, error: impl fmt::Display) -> String {
    let redacted = redact_url(url);
    let mut detail = error.to_string().replace(url, &redacted);
    if let Some(scheme_end) = url.find("://") {
        let authority = &url[scheme_end + 3..];
        if let Some(at) = authority.find('@') {
            let userinfo = &authority[..at];
            if !userinfo.is_empty() {
                detail = detail.replace(userinfo, "<redacted>");
            }
        }
    }
    format!("{redacted} ({detail})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_errors_do_not_repeat_userinfo_from_driver_messages() {
        let url = "postgres://alice:secret@example.test/app";
        let message = connection_message(url, format!("could not connect to {url}"));
        assert!(!message.contains("alice"));
        assert!(!message.contains("secret"));
        assert!(message.contains("example.test"));
    }
}
