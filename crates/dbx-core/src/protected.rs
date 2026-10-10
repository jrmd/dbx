//! Protected profiles reduce accidental writes. Database privileges remain the
//! permission boundary, including side effects inside user-defined functions.
use crate::{
    ColumnInfo, DatabaseKind, DbxError, Engine, QueryOptions, QueryResult, RelationalSchema,
    Result, SqlStatement, TableInfo, TableRef, TableStructure,
};
use async_trait::async_trait;

pub(crate) fn ensure_query(kind: DatabaseKind, query: &str) -> Result<()> {
    let allowed = if kind.is_sql() {
        crate::script::checked_split_sql_for(Some(kind), query)?
            .iter()
            .all(|statement| {
                let first = crate::sqlx_engine::top_level_operation_keyword(statement)
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                let words = sql_words_for(Some(kind), statement);
                let forbidden = [
                    "INSERT", "UPDATE", "DELETE", "REPLACE", "MERGE", "CREATE", "ALTER", "DROP",
                    "TRUNCATE", "COPY", "CALL", "ATTACH", "DETACH", "VACUUM", "INTO", "SET",
                    "RESET",
                ];
                if words.iter().any(|word| forbidden.contains(&word.as_str())) {
                    return false;
                }
                if first == "PRAGMA" {
                    return !statement.contains('=')
                        && words.get(1).is_some_and(|word| {
                            [
                                "TABLE_INFO",
                                "TABLE_XINFO",
                                "FOREIGN_KEY_LIST",
                                "INDEX_LIST",
                                "INDEX_INFO",
                                "DATABASE_LIST",
                                "FOREIGN_KEY_CHECK",
                                "INTEGRITY_CHECK",
                                "QUICK_CHECK",
                            ]
                            .contains(&word.as_str())
                        });
                }
                [
                    "SELECT", "VALUES", "TABLE", "SHOW", "DESCRIBE", "DESC", "EXPLAIN", "BEGIN",
                    "COMMIT", "ROLLBACK", "END",
                ]
                .contains(&first.as_str())
            })
    } else {
        match kind {
            DatabaseKind::Redis => {
                let first = query
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                [
                    "GET",
                    "MGET",
                    "HGET",
                    "HGETALL",
                    "HMGET",
                    "HLEN",
                    "HEXISTS",
                    "HSCAN",
                    "LRANGE",
                    "LLEN",
                    "LINDEX",
                    "SMEMBERS",
                    "SCARD",
                    "SISMEMBER",
                    "SSCAN",
                    "ZRANGE",
                    "ZREVRANGE",
                    "ZSCORE",
                    "ZCARD",
                    "ZSCAN",
                    "XRANGE",
                    "XREVRANGE",
                    "XLEN",
                    "SCAN",
                    "TYPE",
                    "TTL",
                    "PTTL",
                    "EXISTS",
                    "DBSIZE",
                    "PING",
                    "INFO",
                    "COMMAND",
                ]
                .contains(&first.as_str())
            }
            DatabaseKind::Kafka => serde_json::from_str::<serde_json::Value>(query)
                .ok()
                .and_then(|v| v.get("action").and_then(|v| v.as_str()).map(str::to_owned))
                .is_some_and(|action| ["topics", "consume"].contains(&action.as_str())),
            DatabaseKind::MongoDB => serde_json::from_str::<serde_json::Value>(query)
                .ok()
                .is_some_and(|value| {
                    let read = [
                        "find",
                        "aggregate",
                        "count",
                        "distinct",
                        "listCollections",
                        "listIndexes",
                        "dbStats",
                        "collStats",
                        "ping",
                    ]
                    .iter()
                    .any(|key| value.get(key).is_some());
                    read && !contains_write_stage(&value)
                }),
            DatabaseKind::Elasticsearch => {
                let first = query.lines().next().unwrap_or_default();
                first.starts_with("GET ")
                    || (first.starts_with("POST ") && first.trim_end().ends_with("/_search"))
            }
            _ => false,
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(DbxError::Query("Protected connection: this command is not permitted. Use a writable profile and appropriate database permissions to make changes.".into()))
    }
}

pub(crate) struct ProtectedEngine(pub Box<dyn Engine>);
#[async_trait]
impl Engine for ProtectedEngine {
    fn kind(&self) -> DatabaseKind {
        self.0.kind()
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn open_query_session(&self) -> Result<Option<Box<dyn Engine>>> {
        Ok(self
            .0
            .open_query_session()
            .await?
            .map(|engine| Box::new(ProtectedEngine(engine)) as Box<dyn Engine>))
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        self.0.list_tables().await
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.0.list_databases().await
    }
    async fn current_database(&self) -> Result<String> {
        self.0.current_database().await
    }
    async fn use_database(&self, name: &str) -> Result<()> {
        self.0.use_database(name).await
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        self.0.describe_table(table).await
    }
    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        self.0.table_structure(table).await
    }
    async fn relational_schema(&self) -> Result<RelationalSchema> {
        self.0.relational_schema().await
    }
    async fn query(&self, query: &str, options: QueryOptions) -> Result<QueryResult> {
        ensure_query(self.kind(), query)?;
        self.0.query(query, options).await
    }
    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        ensure_query(self.kind(), &statement.sql)?;
        self.0.query_statement(statement, options).await
    }
    async fn execute(&self, _: &SqlStatement) -> Result<crate::ExecResult> {
        Err(DbxError::Query(
            "Protected connection: writes are disabled".into(),
        ))
    }
}

fn contains_write_stage(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            ["$out", "$merge", "$function", "$accumulator", "$where"].contains(&key.as_str())
                || contains_write_stage(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_write_stage),
        _ => false,
    }
}
/// Square brackets quote identifiers only in SQL Server and SQLite dialects;
/// elsewhere they are array subscripts or list literals.
fn brackets_quote_identifiers(kind: Option<DatabaseKind>) -> bool {
    kind.is_none_or(|kind| {
        matches!(
            kind.dialect(),
            DatabaseKind::SqlServer | DatabaseKind::SQLite
        )
    })
}
/// SQL keywords outside literals, quoted identifiers and comments.
pub(crate) fn sql_words_for(kind: Option<DatabaseKind>, sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut words = Vec::new();
    while index < bytes.len() {
        if bytes[index..].starts_with(b"--")
            || (bytes[index] == b'#' && crate::script::sql_hash_comments(kind))
        {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index..].starts_with(b"/*") {
            if bytes[index..].starts_with(b"/*!") {
                words.push("SET".into());
            }
            index += 2;
            let mut depth = 1;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
        } else if b"\'\"`".contains(&bytes[index])
            || (bytes[index] == b'[' && brackets_quote_identifiers(kind))
        {
            let escaped =
                bytes[index] == b'\'' && crate::script::sql_backslash_escapes(kind, &sql[..index]);
            let end = if bytes[index] == b'[' {
                b']'
            } else {
                bytes[index]
            };
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' && escaped {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == end {
                    index += 1;
                    if index < bytes.len() && bytes[index] == end {
                        index += 1;
                    } else {
                        break;
                    }
                } else {
                    index += 1;
                }
            }
        } else if bytes[index] == b'$' && crate::script::sql_dollar_quotes(kind) {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'$' {
                index += 1;
                let tag = &sql[start..index];
                if let Some(end) = sql[index..].find(tag) {
                    index += end + tag.len();
                } else {
                    index = bytes.len();
                }
            }
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            words.push(sql[start..index].to_ascii_uppercase());
        } else {
            index += 1;
        }
    }
    words
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protection_ignores_literals_but_blocks_writing_ctes_and_encoded_stages() {
        ensure_query(
            DatabaseKind::PostgreSQL,
            "SELECT 'update', \"delete\", $$DROP$$ -- INSERT\n",
        )
        .unwrap();
        assert!(
            ensure_query(
                DatabaseKind::PostgreSQL,
                "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x"
            )
            .is_err()
        );
        assert!(
            ensure_query(
                DatabaseKind::MongoDB,
                r#"{"aggregate":"t","pipeline":[{"\u0024out":"other"}]}"#
            )
            .is_err()
        ); // `#` is an operator in PostgreSQL, so it cannot hide what follows;
        // `SELECT ... INTO` creates a table there.
        assert!(ensure_query(DatabaseKind::PostgreSQL, "SELECT 5 # 3 INTO copy FROM t").is_err());
        ensure_query(DatabaseKind::MySQL, "SELECT 1 # INTO, UPDATE\n").unwrap();
    }

    #[test]
    fn brackets_only_quote_identifiers_where_the_engine_allows_it() {
        // PostgreSQL brackets are array subscripts, so a `]` inside a string
        // there must not unbalance the scan and hide the DELETE that follows.
        let hidden = "SELECT (ARRAY[1])[ (']')::int ] INTO copy FROM t";
        assert!(ensure_query(DatabaseKind::PostgreSQL, hidden).is_err());
        assert!(ensure_query(DatabaseKind::MySQL, hidden).is_err());
        // SQL Server and SQLite still quote identifiers with brackets.
        ensure_query(DatabaseKind::SQLite, "SELECT [delete] FROM [update]").unwrap();
        ensure_query(DatabaseKind::SqlServer, "SELECT [drop] FROM t").unwrap();
    }
}
