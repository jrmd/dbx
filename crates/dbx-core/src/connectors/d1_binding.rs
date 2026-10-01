//! D1's REST contract accepts string parameters. Cast bound numeric strings
//! in SQL and emit NULL as a literal so typed DBX values retain their meaning.
use crate::{CellValue, DbxError, Result, SqlStatement};
use serde_json::Value;

pub(super) fn bind(statement: &SqlStatement) -> Result<(String, Vec<Value>)> {
    if statement.params.is_empty() {
        return Ok((statement.sql.clone(), Vec::new()));
    }
    let mut sql = String::new();
    let mut params = Vec::new();
    let mut position = 0;
    let bytes = statement.sql.as_bytes();
    let mut i = 0;
    let mut quote = None;
    let mut line_comment = false;
    let mut block_comment = false;
    while i < bytes.len() {
        let character = statement.sql[i..].chars().next().unwrap();
        if line_comment {
            sql.push(character);
            i += character.len_utf8();
            if character == '\n' {
                line_comment = false;
            }
            continue;
        }
        if block_comment {
            if bytes[i..].starts_with(b"*/") {
                sql.push_str("*/");
                i += 2;
                block_comment = false;
            } else {
                sql.push(character);
                i += character.len_utf8();
            }
            continue;
        }
        if let Some(delimiter) = quote {
            sql.push(character);
            i += character.len_utf8();
            if character == delimiter {
                if bytes.get(i) == Some(&(delimiter as u8)) && delimiter != ']' {
                    sql.push(delimiter);
                    i += 1;
                } else {
                    quote = None;
                }
            }
            continue;
        }
        if bytes[i..].starts_with(b"--") {
            line_comment = true;
            sql.push_str("--");
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            block_comment = true;
            sql.push_str("/*");
            i += 2;
            continue;
        }
        if matches!(character, '\'' | '"' | '`' | '[') {
            quote = Some(if character == '[' { ']' } else { character });
            sql.push(character);
            i += 1;
            continue;
        }
        if character == '?' {
            if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                return Err(DbxError::Parse(
                    "D1 bound statements use anonymous ? parameters".into(),
                ));
            }
            let value = statement
                .params
                .get(position)
                .ok_or_else(|| DbxError::Parse("Too few bound parameters".into()))?;
            position += 1;
            let (placeholder, param) = match value {
                CellValue::Null => ("NULL", None),
                CellValue::Boolean(v) => (
                    "CAST(? AS INTEGER)",
                    Some(if *v { "1" } else { "0" }.into()),
                ),
                CellValue::Integer(v) => ("CAST(? AS INTEGER)", Some(v.to_string())),
                CellValue::Unsigned(v) => (
                    "CAST(? AS INTEGER)",
                    Some(
                        i64::try_from(*v)
                            .map_err(|_| {
                                DbxError::InvalidConfig("Integer exceeds SQLite range".into())
                            })?
                            .to_string(),
                    ),
                ),
                CellValue::Real(v) if v.is_finite() => ("CAST(? AS REAL)", Some(v.to_string())),
                CellValue::Text(v) => ("?", Some(v.clone())),
                CellValue::Json(v) => ("?", Some(v.to_string())),
                CellValue::Bytes(_) => {
                    return Err(DbxError::Unsupported {
                        operation: "binary parameters through D1 REST".into(),
                        kind: crate::DatabaseKind::CloudflareD1,
                    });
                }
                _ => return Err(DbxError::InvalidConfig("Non-finite parameter".into())),
            };
            sql.push_str(placeholder);
            if let Some(param) = param {
                params.push(Value::String(param));
            }
            i += 1;
        } else {
            sql.push(character);
            i += character.len_utf8();
        }
    }
    if position != statement.params.len() {
        return Err(DbxError::Parse("Too many bound parameters".into()));
    }
    Ok((sql, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binds_strings_under_the_rest_contract_without_changing_literals_or_null() {
        let statement = SqlStatement::new(
            "SELECT '?', ? AS integer_value, ? AS null_value, ? AS text_value /* ? */ -- ?\n",
            vec![
                CellValue::Integer(9007199254740993),
                CellValue::Null,
                CellValue::Text("O'Reilly ?".into()),
            ],
        );
        let (sql, params) = bind(&statement).unwrap();
        assert_eq!(
            sql,
            "SELECT '?', CAST(? AS INTEGER) AS integer_value, NULL AS null_value, ? AS text_value /* ? */ -- ?\n"
        );
        assert_eq!(
            params,
            vec![
                Value::String("9007199254740993".into()),
                Value::String("O'Reilly ?".into())
            ]
        );
        assert!(!sql.contains("O'Reilly"));
    }
}
