//! Bind query-document parameters with the types the server expects.
//!
//! PostgreSQL never coerces a typed parameter between type categories:
//! `uuid_column = $1` fails when `$1` is bound as `text`, and so does
//! `name = $1` when `$1` is a `bigint`. The server knows what each
//! placeholder should be, so it is asked, and mismatched values are sent as
//! text and cast to that type, just as psql's untyped literals would be.

use std::collections::HashMap;

use sqlx::{Either, Executor, TypeInfo, postgres::PgTypeInfo};

use crate::{CellValue, DatabaseKind, Result, SqlStatement};

/// Rewrite `statement` so every parameter matches its inferred type. A
/// statement the server cannot describe is returned unchanged so running it
/// reports the server's own error.
pub(crate) async fn adapt_postgres_parameters(
    connection: &mut sqlx::PgConnection,
    statement: &SqlStatement,
) -> Result<SqlStatement> {
    if statement.params.is_empty() {
        return Ok(statement.clone());
    }
    let Ok(described) = (&mut *connection).describe(&statement.sql).await else {
        return Ok(statement.clone());
    };
    let Some(Either::Left(types)) = described.parameters() else {
        return Ok(statement.clone());
    };
    let mut params = statement.params.clone();
    let mut casts = HashMap::new();
    for (index, (value, expected)) in params.iter_mut().zip(types).enumerate() {
        if binds_compatibly(value, expected) {
            continue;
        }
        if let Some(text) = text_form(value) {
            *value = CellValue::Text(text);
        }
        if text_like(expected) {
            continue;
        }
        let Some(oid) = expected.oid() else {
            continue;
        };
        let name: Option<String> = sqlx::query_scalar("SELECT format_type($1::oid, NULL)")
            .bind(i64::from(oid.0))
            .fetch_one(&mut *connection)
            .await?;
        if let Some(name) = name {
            casts.insert(index + 1, name);
        }
    }
    Ok(SqlStatement::new(
        rewrite_placeholders(&statement.sql, &casts),
        params,
    ))
}

/// Whether binding `value` as DBX normally does already suits `expected`.
fn binds_compatibly(value: &CellValue, expected: &PgTypeInfo) -> bool {
    let name = expected.name().to_ascii_uppercase();
    let numeric = matches!(
        name.as_str(),
        "INT2" | "INT4" | "INT8" | "NUMERIC" | "FLOAT4" | "FLOAT8" | "OID"
    );
    match value {
        CellValue::Text(_) => text_like(expected),
        CellValue::Integer(_) | CellValue::Unsigned(_) | CellValue::Real(_) => numeric,
        CellValue::Boolean(_) => name == "BOOL",
        CellValue::Bytes(_) => name == "BYTEA",
        CellValue::Json(_) => matches!(name.as_str(), "JSON" | "JSONB"),
        // A NULL is bound as text, so it needs the same cast as text.
        CellValue::Null => text_like(expected),
    }
}

fn text_like(expected: &PgTypeInfo) -> bool {
    matches!(
        expected.name().to_ascii_uppercase().as_str(),
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "UNKNOWN" | "CITEXT"
    )
}

/// PostgreSQL's text input form of a value, which every type can parse.
fn text_form(value: &CellValue) -> Option<String> {
    Some(match value {
        CellValue::Null => return None,
        CellValue::Boolean(value) => value.to_string(),
        CellValue::Integer(value) => value.to_string(),
        CellValue::Unsigned(value) => value.to_string(),
        CellValue::Real(value) => value.to_string(),
        CellValue::Text(value) => value.clone(),
        CellValue::Bytes(value) => {
            let mut text = String::from("\\x");
            for byte in value {
                text.push_str(&format!("{byte:02x}"));
            }
            text
        }
        CellValue::Json(value) => value.to_string(),
    })
}

/// Replace `$n` placeholders outside strings, quoted identifiers, comments
/// and dollar-quoted bodies with `CAST($n AS type)`.
fn rewrite_placeholders(sql: &str, casts: &HashMap<usize, String>) -> String {
    if casts.is_empty() {
        return sql.to_owned();
    }
    let bytes = sql.as_bytes();
    let mut output = String::with_capacity(sql.len() + casts.len() * 16);
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let rest = &sql[index..];
        if rest.starts_with("--") {
            index += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            let mut depth = 0usize;
            while index < bytes.len() {
                if sql[index..].starts_with("/*") {
                    depth += 1;
                    index += 2;
                } else if sql[index..].starts_with("*/") {
                    depth -= 1;
                    index += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    index += 1;
                }
            }
        } else if byte == b'\'' || byte == b'"' {
            let escapes = byte == b'\''
                && crate::sql_backslash_escapes(Some(DatabaseKind::PostgreSQL), &sql[..index]);
            index += 1;
            while index < bytes.len() {
                if escapes && bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == byte {
                    index += 1;
                    if bytes.get(index) != Some(&byte) {
                        break;
                    }
                    index += 1;
                } else {
                    index += 1;
                }
            }
        } else if byte == b'$' {
            let digits = rest[1..]
                .bytes()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let after_identifier =
                index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
            if digits > 0 && !after_identifier {
                let end = index + 1 + digits;
                if let Some(cast) = sql[index + 1..end]
                    .parse::<usize>()
                    .ok()
                    .and_then(|position| casts.get(&position))
                {
                    output.push_str(&sql[copied..index]);
                    output.push_str(&format!("CAST({} AS {cast})", &sql[index..end]));
                    copied = end;
                }
                index = end;
            } else if let Some(tag_length) = dollar_tag_length(rest) {
                let tag = &rest[..tag_length];
                index += tag_length;
                index += sql[index..]
                    .find(tag)
                    .map_or(sql.len() - index, |end| end + tag_length);
            } else {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    output.push_str(&sql[copied..]);
    output
}

/// The byte length of a `$tag$` opener at the start of `text`.
fn dollar_tag_length(text: &str) -> Option<usize> {
    let tag = text[1..].find('$')?;
    text[1..1 + tag]
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        .then_some(tag + 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_cast_only_outside_literals() {
        let casts = HashMap::from([(1, "uuid".to_owned()), (12, "date".to_owned())]);
        assert_eq!(
            rewrite_placeholders(
                "SELECT '$1', \"$1\", $$ $1 $$, $tag$ $1 $tag$ -- $1\n FROM t /* $1 /* $1 */ */ WHERE id = $1 AND d > $12 AND x$1 = $2",
                &casts
            ),
            "SELECT '$1', \"$1\", $$ $1 $$, $tag$ $1 $tag$ -- $1\n FROM t /* $1 /* $1 */ */ WHERE id = CAST($1 AS uuid) AND d > CAST($12 AS date) AND x$1 = $2"
        );
        assert_eq!(
            rewrite_placeholders("SELECT E'\\'$1' || $1", &casts),
            "SELECT E'\\'$1' || CAST($1 AS uuid)"
        );
    }
}
