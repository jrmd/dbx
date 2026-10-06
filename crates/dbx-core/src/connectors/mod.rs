//! Native and HTTP connectors. Each retains its provider's query language.
mod clickhouse;
mod d1_binding;
mod duck;
mod http;
mod kafka;
mod mongo;
mod snowflake;
mod sqlserver;

use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, Page, QueryOptions,
    QueryResult, Result, RowData, TableRef,
};
use serde_json::{Value, json};
use std::time::Instant;

pub(crate) async fn connect(config: ConnectionConfig) -> Result<Box<dyn Engine>> {
    match config.kind {
        DatabaseKind::ClickHouse => Ok(Box::new(
            clickhouse::ClickHouseEngine::connect(config).await?,
        )),
        DatabaseKind::DuckDB => Ok(Box::new(duck::DuckEngine::connect(config).await?)),
        DatabaseKind::MongoDB => Ok(Box::new(mongo::MongoEngine::connect(config).await?)),
        DatabaseKind::Kafka => Ok(Box::new(kafka::KafkaEngine::connect(config).await?)),
        DatabaseKind::Snowflake => Ok(Box::new(snowflake::SnowflakeEngine::connect(config).await?)),
        DatabaseKind::SqlServer => Ok(Box::new(sqlserver::SqlServerEngine::connect(config).await?)),
        DatabaseKind::Elasticsearch
        | DatabaseKind::BigQuery
        | DatabaseKind::Turso
        | DatabaseKind::CloudflareD1 => Ok(Box::new(http::HttpEngine::connect(config).await?)),
        _ => Err(DbxError::InvalidConfig(
            "No connector for this database type".into(),
        )),
    }
}

pub(crate) fn browse_command(
    kind: DatabaseKind,
    table: &TableRef,
    page: Option<Page>,
) -> Result<String> {
    let page = page.unwrap_or(Page {
        limit: 100,
        offset: 0,
    });
    Ok(match kind {
        DatabaseKind::MongoDB => json!({"find": table.name, "filter": {}, "limit": page.limit, "skip": page.offset}).to_string(),
        DatabaseKind::Elasticsearch => {
            let path: String = url::form_urlencoded::byte_serialize(table.name.as_bytes()).collect();
            format!("POST /{path}/_search\n{}", json!({"query":{"match_all":{}}, "size": page.limit, "from":page.offset}))
        },
        DatabaseKind::Kafka => json!({"action":"consume", "topic":table.name, "limit":page.limit, "offset":page.offset}).to_string(),
        _ => return Err(DbxError::Unsupported { operation: "browse collection".into(), kind }),
    })
}

pub(super) fn column(
    name: impl Into<String>,
    data_type: impl Into<String>,
    ordinal: usize,
) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        data_type: data_type.into(),
        enum_values: Vec::new(),
        nullable: true,
        ordinal,
        primary_key: false,
        default_value: None,
    }
}
pub(super) fn cell(value: &Value) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Bool(v) => CellValue::Boolean(*v),
        Value::Number(n) => n
            .as_i64()
            .map(CellValue::Integer)
            .or_else(|| n.as_u64().map(CellValue::Unsigned))
            .unwrap_or_else(|| CellValue::Real(n.as_f64().unwrap_or_default())),
        Value::String(v) => CellValue::Text(v.clone()),
        other => CellValue::Json(other.clone()),
    }
}
pub(super) fn json_rows(
    values: Vec<Value>,
    options: QueryOptions,
    started: Instant,
) -> QueryResult {
    let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
    let truncated = values.len() > limit;
    let values: Vec<_> = values.into_iter().take(limit).collect();
    let mut names = Vec::new();
    for value in &values {
        if let Some(object) = value.as_object() {
            for name in object.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        } else if !names.iter().any(|name| name == "value") {
            names.push("value".into());
        }
    }
    let columns = names
        .iter()
        .enumerate()
        .map(|(i, n)| column(n, "JSON", i))
        .collect();
    let rows = values
        .iter()
        .map(|value| {
            RowData::new(
                names
                    .iter()
                    .map(|name| {
                        if value.is_object() {
                            cell(&value[name])
                        } else if name == "value" {
                            cell(value)
                        } else {
                            CellValue::Null
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    crate::engine::query_result(columns, rows, None, truncated, started)
}
pub(super) fn decode(raw: &str) -> Result<String> {
    percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| DbxError::InvalidConfig("URL contains invalid UTF-8".into()))
}
pub(super) fn text(row: &RowData, i: usize) -> String {
    row.values
        .get(i)
        .map(ToString::to_string)
        .unwrap_or_default()
}
