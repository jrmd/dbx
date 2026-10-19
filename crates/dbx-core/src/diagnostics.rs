//! Provider-aware diagnostics and reviewable schema migration drafts.
use crate::sql::{quote_identifier, quote_table};
use crate::{
    CellValue, ColumnInfo, DatabaseKind, DbxError, EntityKind, QueryResult, RelationalSchema,
    Result, RowData, TableRef,
};

#[derive(Clone, Copy)]
pub enum Monitor {
    Sessions,
    Locks,
}
pub fn monitor_query(kind: DatabaseKind, monitor: Monitor) -> Result<String> {
    Ok(match (kind, monitor) {
        (DatabaseKind::PostgreSQL, Monitor::Sessions) => "SELECT pid, usename, datname, state, wait_event_type, wait_event, query_start, query FROM pg_stat_activity WHERE datname=current_database() ORDER BY query_start".into(),
        (DatabaseKind::PostgreSQL, Monitor::Locks) => "SELECT l.pid, a.usename, l.locktype, l.mode, l.granted, l.relation::regclass::text AS relation, pg_blocking_pids(l.pid)::text AS blocking_pids, a.query FROM pg_locks l LEFT JOIN pg_stat_activity a ON a.pid=l.pid WHERE a.datname=current_database() ORDER BY l.granted, l.pid".into(),
        (DatabaseKind::MySQL, Monitor::Sessions) => "SHOW FULL PROCESSLIST".into(),
        (DatabaseKind::MySQL, Monitor::Locks) => "SELECT * FROM performance_schema.data_lock_waits".into(),
        (DatabaseKind::SqlServer, Monitor::Sessions) => "SELECT s.session_id, s.login_name, s.host_name, s.program_name, s.status, DB_NAME(s.database_id) AS database_name, s.cpu_time, s.memory_usage, s.last_request_start_time, t.text AS current_sql FROM sys.dm_exec_sessions s LEFT JOIN sys.dm_exec_requests r ON r.session_id = s.session_id OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t WHERE s.is_user_process = 1 ORDER BY s.session_id".into(),
        (DatabaseKind::SqlServer, Monitor::Locks) => "SELECT r.session_id, r.blocking_session_id, r.wait_type, r.wait_time, r.wait_resource, r.status, DB_NAME(r.database_id) AS database_name, t.text AS current_sql FROM sys.dm_exec_requests r OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t WHERE r.blocking_session_id <> 0 OR r.wait_type LIKE 'LCK%' ORDER BY r.wait_time DESC".into(),
        _ => return Err(DbxError::Unsupported { operation: "server session/lock monitoring".into(), kind }),
    })
}
pub fn execution_plan_query(kind: DatabaseKind, sql: &str) -> Result<String> {
    let statements = crate::script::checked_split_sql_for(Some(kind), sql)?;
    if statements.len() != 1 {
        return Err(DbxError::Parse("Select one statement to explain".into()));
    }
    crate::protected::ensure_query(kind, &statements[0])?;
    let operation = crate::sqlx_engine::top_level_operation_keyword(&statements[0])
        .unwrap_or_default()
        .to_ascii_uppercase();
    if !matches!(operation.as_str(), "SELECT" | "VALUES" | "TABLE") {
        return Err(DbxError::Parse(
            "Explain accepts a read query; it does not execute ANALYZE".into(),
        ));
    }
    let prefix = match kind {
        DatabaseKind::PostgreSQL => "EXPLAIN (FORMAT JSON)",
        DatabaseKind::MySQL => "EXPLAIN FORMAT=JSON",
        DatabaseKind::SQLite | DatabaseKind::Turso | DatabaseKind::CloudflareD1 => {
            "EXPLAIN QUERY PLAN"
        }
        DatabaseKind::DuckDB | DatabaseKind::CockroachDB | DatabaseKind::ClickHouse => "EXPLAIN",
        _ => {
            return Err(DbxError::Unsupported {
                operation: "execution plan".into(),
                kind,
            });
        }
    };
    Ok(format!("{prefix} {}", statements[0]))
}

/// ANALYZE executes a query; callers must use QuerySession::explain_analyze
/// to enforce an isolated read-only transaction. Unsupported server versions
/// reject the command; DBX never retries with a writing transaction.
pub fn execution_plan_analyze_query(kind: DatabaseKind, sql: &str) -> Result<String> {
    let plain = execution_plan_query(kind, sql)?;
    let (prefix, analyze) = match kind {
        DatabaseKind::PostgreSQL => (
            "EXPLAIN (FORMAT JSON) ",
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)",
        ),
        DatabaseKind::MySQL => ("EXPLAIN FORMAT=JSON ", "EXPLAIN ANALYZE FORMAT=TREE"),
        DatabaseKind::CockroachDB => ("EXPLAIN ", "EXPLAIN ANALYZE (PLAN)"),
        _ => {
            return Err(DbxError::Unsupported {
                operation: "isolated read-only EXPLAIN ANALYZE".into(),
                kind,
            });
        }
    };
    Ok(format!(
        "{analyze} {}",
        plain.strip_prefix(prefix).expect("matching plan prefix")
    ))
}
fn plan_columns() -> Vec<ColumnInfo> {
    [
        "Depth",
        "Operation",
        "Estimated rows",
        "Estimated cost",
        "Actual rows",
        "Actual time (ms)",
        "Actual loops",
        "Detail",
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, name)| ColumnInfo::result(name, ordinal, "TEXT"))
    .collect()
}

/// Native text plans retain the original node and attributes in Detail. Only
/// explicitly named metrics are normalized; missing measurements stay blank.
fn format_native_plan(kind: DatabaseKind, result: &QueryResult) -> Option<QueryResult> {
    if !matches!(kind, DatabaseKind::MySQL | DatabaseKind::CockroachDB) {
        return None;
    }
    let mut output = QueryResult::empty(None, result.elapsed_ms);
    output.columns = plan_columns();
    let mut indent_stack = Vec::new();
    let mut preamble = Vec::new();
    let metric = |text: &str, key: &str| -> CellValue {
        text.split_once(key)
            .and_then(|(_, tail)| tail.split([' ', ')']).next())
            .filter(|value| !value.is_empty())
            .map(|value| CellValue::Text(value.into()))
            .unwrap_or_default()
    };
    for line in result
        .rows
        .iter()
        .filter_map(|row| row.values.first())
        .filter_map(|value| match value {
            CellValue::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .flat_map(str::lines)
    {
        let marker = if kind == DatabaseKind::MySQL {
            line.find("-> ").map(|index| (index, 3))
        } else {
            line.find('•').map(|index| (index, '•'.len_utf8()))
        };
        let marker = marker.filter(|(index, _)| {
            line[..*index].chars().all(|ch| {
                ch.is_whitespace()
                    || (kind == DatabaseKind::CockroachDB && matches!(ch, '│' | '├' | '└' | '─'))
            })
        });
        if let Some((indent, marker_length)) = marker {
            while indent_stack
                .last()
                .is_some_and(|previous| *previous >= indent)
            {
                indent_stack.pop();
            }
            let depth = indent_stack.len();
            indent_stack.push(indent);
            let node = line[indent + marker_length..].trim();
            let operation = node
                .split_once("  (")
                .map_or(node, |(operation, _)| operation);
            let actual = node.split_once("(actual ").map(|(_, actual)| actual);
            let elapsed = actual
                .map(|actual| metric(actual, "time="))
                .map(|value| match value {
                    CellValue::Text(text) => {
                        CellValue::Text(text.rsplit("..").next().unwrap_or_default().into())
                    }
                    value => value,
                })
                .unwrap_or_default();
            output.rows.push(RowData::new(vec![
                CellValue::Integer(depth as i64),
                CellValue::Text(operation.into()),
                metric(node.split("(actual ").next().unwrap_or_default(), "rows="),
                metric(node, "cost="),
                actual
                    .map(|actual| metric(actual, "rows="))
                    .unwrap_or_default(),
                elapsed,
                actual
                    .map(|actual| metric(actual, "loops="))
                    .unwrap_or_default(),
                CellValue::Text(node.into()),
            ]));
        } else if kind == DatabaseKind::CockroachDB {
            let detail = line.trim();
            if let Some(row) = output.rows.last_mut() {
                if let CellValue::Text(text) = &mut row.values[7] {
                    text.push('\n');
                    text.push_str(detail);
                }
                if let Some(count) = detail.strip_prefix("actual row count: ") {
                    row.values[4] = CellValue::Text(count.into());
                }
                if let Some(count) = detail.strip_prefix("estimated row count: ") {
                    row.values[2] = CellValue::Text(count.into());
                }
            } else if !detail.is_empty() {
                preamble.push(detail.to_string());
            }
        }
    }
    if let Some(row) = output.rows.first_mut()
        && !preamble.is_empty()
        && let CellValue::Text(detail) = &mut row.values[7]
    {
        *detail = format!("{}\n{detail}", preamble.join("\n"));
    }
    (!output.rows.is_empty()).then_some(output)
}

pub fn format_execution_plan(kind: DatabaseKind, result: &QueryResult) -> QueryResult {
    if let Some(plan) = format_native_plan(kind, result) {
        return plan;
    }
    if !matches!(kind, DatabaseKind::PostgreSQL | DatabaseKind::MySQL) {
        return result.clone();
    }
    let Some(value) = result.rows.first().and_then(|row| row.values.first()) else {
        return result.clone();
    };
    let json = match value {
        CellValue::Json(value) => Some(value.clone()),
        CellValue::Text(value) => serde_json::from_str(value).ok(),
        _ => None,
    };
    let Some(json) = json else {
        return result.clone();
    };
    let mut output = QueryResult::empty(None, result.elapsed_ms);
    output.columns = plan_columns();
    fn walk(value: &serde_json::Value, depth: usize, rows: &mut Vec<RowData>) {
        match value {
            serde_json::Value::Object(object) => {
                let operation = object
                    .get("Node Type")
                    .or_else(|| object.get("access_type"));
                let emitted = operation.is_some();
                if let Some(operation) = operation {
                    let detail = [
                        "Relation Name",
                        "Index Name",
                        "table_name",
                        "key",
                        "Filter",
                        "Index Cond",
                        "attached_condition",
                    ]
                    .iter()
                    .filter_map(|key| object.get(*key).map(|value| format!("{key}: {value}")))
                    .collect::<Vec<_>>()
                    .join(" · ");
                    rows.push(RowData::new(vec![
                        CellValue::Integer(depth as i64),
                        CellValue::Text(operation.as_str().unwrap_or_default().into()),
                        object
                            .get("Plan Rows")
                            .or_else(|| object.get("rows_examined_per_scan"))
                            .map(|value| CellValue::Text(value.to_string()))
                            .unwrap_or_default(),
                        object
                            .get("Total Cost")
                            .or_else(|| object.get("cost_info"))
                            .map(|value| CellValue::Text(value.to_string()))
                            .unwrap_or_default(),
                        object
                            .get("Actual Rows")
                            .or_else(|| object.get("r_rows"))
                            .map(|value| CellValue::Text(value.to_string()))
                            .unwrap_or_default(),
                        object
                            .get("Actual Total Time")
                            .or_else(|| object.get("r_table_time_ms"))
                            .map(|value| CellValue::Text(value.to_string()))
                            .unwrap_or_default(),
                        object
                            .get("Actual Loops")
                            .or_else(|| object.get("r_loops"))
                            .map(|value| CellValue::Text(value.to_string()))
                            .unwrap_or_default(),
                        CellValue::Text(detail),
                    ]));
                }
                for child in object.values() {
                    if child.is_object() || child.is_array() {
                        walk(child, depth + usize::from(emitted), rows);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    walk(value, depth, rows);
                }
            }
            _ => {}
        }
    }
    walk(&json, 0, &mut output.rows);
    if output.rows.is_empty() {
        result.clone()
    } else {
        output
    }
}
pub fn compare_execution_plans(before: &QueryResult, after: &QueryResult) -> QueryResult {
    let mut result = QueryResult::empty(None, after.elapsed_ms);
    result.columns = ["Step", "Change", "Previous", "Current"]
        .iter()
        .enumerate()
        .map(|(i, name)| ColumnInfo::result(*name, i, "TEXT"))
        .collect();
    for index in 0..before.rows.len().max(after.rows.len()) {
        let previous = before.rows.get(index);
        let current = after.rows.get(index);
        let change = if previous == current {
            "Unchanged"
        } else if previous.is_none() {
            "Added"
        } else if current.is_none() {
            "Removed"
        } else {
            "Changed"
        };
        result.rows.push(RowData::new(vec![
            CellValue::Integer(index as i64 + 1),
            CellValue::Text(change.into()),
            CellValue::Text(
                previous
                    .map(|row| serde_json::to_string(&row.values).unwrap_or_default())
                    .unwrap_or_default(),
            ),
            CellValue::Text(
                current
                    .map(|row| serde_json::to_string(&row.values).unwrap_or_default())
                    .unwrap_or_default(),
            ),
        ]));
    }
    result
}

/// Full-table server aggregates. Native comparisons retain the source type;
/// decimal/integer extrema are never converted through floating point.
/// The generated query deliberately has no page/filter limit.
pub fn table_column_statistics_query(
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[ColumnInfo],
) -> Result<String> {
    if !matches!(
        kind,
        DatabaseKind::PostgreSQL
            | DatabaseKind::MySQL
            | DatabaseKind::SQLite
            | DatabaseKind::CockroachDB
    ) {
        return Err(DbxError::Unsupported {
            operation: "full-table column statistics".into(),
            kind,
        });
    }
    if columns.is_empty() {
        return Err(DbxError::Parse("Table has no columns to summarize".into()));
    }
    let mut expressions = vec!["COUNT(*) AS dbx_rows".to_string()];
    for (index, column) in columns.iter().enumerate() {
        let name = quote_identifier(kind, &column.name)?;
        let (distinct, range) = statistics_operators(kind, &column.data_type);
        expressions.push(format!("COUNT(*) - COUNT({name}) AS dbx_nulls_{index}"));
        expressions.push(format!(
            "{} AS dbx_distinct_{index}",
            if distinct {
                format!("COUNT(DISTINCT {name})")
            } else {
                "NULL".into()
            }
        ));
        for operator in ["MIN", "MAX"] {
            expressions.push(format!(
                "{} AS dbx_{}_{index}",
                if range {
                    format!("{operator}({name})")
                } else {
                    "NULL".into()
                },
                operator.to_ascii_lowercase()
            ));
        }
    }
    Ok(format!(
        "SELECT\n  {}\nFROM {}",
        expressions.join(",\n  "),
        quote_table(kind, table)?
    ))
}

fn statistics_operators(kind: DatabaseKind, data_type: &str) -> (bool, bool) {
    let data_type = data_type.trim().to_ascii_lowercase();
    if data_type.contains('[')
        || data_type.contains("json")
        || data_type.contains("blob")
        || data_type == "bytea"
    {
        return (false, false);
    }
    if kind == DatabaseKind::SQLite {
        // SQLite defines comparison order for all its native storage classes.
        return (true, true);
    }
    let base = data_type.split(['(', ' ']).next().unwrap_or_default();
    let range = matches!(
        base,
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "integer"
            | "bigint"
            | "int2"
            | "int4"
            | "int8"
            | "numeric"
            | "decimal"
            | "dec"
            | "real"
            | "float"
            | "float4"
            | "float8"
            | "double"
            | "text"
            | "varchar"
            | "character"
            | "char"
            | "bpchar"
            | "string"
            | "date"
            | "datetime"
            | "timestamp"
            | "timestamptz"
            | "time"
            | "timetz"
            | "interval"
            | "year"
    );
    (
        range || matches!(base, "bool" | "boolean" | "uuid" | "bit"),
        range,
    )
}

pub fn format_table_column_statistics(
    kind: DatabaseKind,
    columns: &[ColumnInfo],
    raw: &QueryResult,
) -> Result<QueryResult> {
    let values = raw
        .rows
        .first()
        .map(|row| &row.values)
        .filter(|values| values.len() == 1 + 4 * columns.len())
        .ok_or_else(|| {
            DbxError::Query("Full-table aggregate returned an unexpected result shape".into())
        })?;
    let mut output = QueryResult::empty(None, raw.elapsed_ms);
    output.columns = [
        "Column",
        "Source type",
        "Full table rows",
        "Nulls",
        "Distinct non-null (server)",
        "Minimum (source type)",
        "Maximum (source type)",
        "Scope / support",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        ColumnInfo::result(
            name,
            index,
            if (2..=4).contains(&index) {
                "INTEGER"
            } else {
                "TEXT"
            },
        )
    })
    .collect();
    for (index, column) in columns.iter().enumerate() {
        let offset = 1 + 4 * index;
        let (distinct, range) = statistics_operators(kind, &column.data_type);
        output.rows.push(RowData::new(vec![
            CellValue::Text(column.name.clone()),
            CellValue::Text(column.data_type.clone()),
            values[0].clone(),
            values[offset].clone(),
            values[offset + 1].clone(),
            values[offset + 2].clone(),
            values[offset + 3].clone(),
            CellValue::Text(
                if !distinct {
                    "Full table · distinct/min/max unsupported for this type"
                } else if !range {
                    "Full table · min/max unsupported for this type"
                } else {
                    "Full table · server comparison and collation"
                }
                .into(),
            ),
        ]));
    }
    Ok(output)
}

/// Statistics over the supplied client result only, never an implicit query
/// over a whole table. Distinctness uses typed client values, not DB collation.
pub fn loaded_column_statistics(source: &QueryResult) -> QueryResult {
    use std::collections::BTreeSet;
    let mut output = QueryResult::empty(None, 0);
    output.columns = [
        "Column",
        "Type",
        "Loaded rows",
        "Nulls",
        "Distinct non-null (client)",
        "Numeric count",
        "Numeric min (~)",
        "Numeric max (~)",
        "Numeric mean (~)",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        ColumnInfo::result(
            name,
            index,
            if index < 2 {
                "TEXT"
            } else if index < 6 {
                "INTEGER"
            } else {
                "REAL"
            },
        )
    })
    .collect();
    for (index, column) in source.columns.iter().enumerate() {
        let mut distinct = BTreeSet::new();
        let mut nulls = 0;
        let mut numbers = 0;
        let mut minimum = f64::INFINITY;
        let mut maximum = f64::NEG_INFINITY;
        let mut mean = 0.;
        let numeric_type = ["int", "real", "float", "double", "decimal", "numeric"]
            .iter()
            .any(|name| column.data_type.to_ascii_lowercase().contains(name));
        for row in &source.rows {
            let value = row.values.get(index).unwrap_or(&CellValue::Null);
            if *value == CellValue::Null {
                nulls += 1;
                continue;
            }
            distinct.insert(format!("{value:?}"));
            let number = match value {
                CellValue::Integer(value) => Some(*value as f64),
                CellValue::Unsigned(value) => Some(*value as f64),
                CellValue::Real(value) => Some(*value),
                CellValue::Text(value) if numeric_type => value.parse::<f64>().ok(),
                _ => None,
            }
            .filter(|number| number.is_finite());
            if let Some(number) = number {
                numbers += 1;
                minimum = minimum.min(number);
                maximum = maximum.max(number);
                mean += (number - mean) / numbers as f64;
            }
        }
        output.rows.push(RowData::new(vec![
            CellValue::Text(column.name.clone()),
            CellValue::Text(column.data_type.clone()),
            CellValue::Integer(source.rows.len() as i64),
            CellValue::Integer(nulls),
            CellValue::Integer(distinct.len() as i64),
            CellValue::Integer(numbers),
            if numbers > 0 {
                CellValue::Real(minimum)
            } else {
                CellValue::Null
            },
            if numbers > 0 {
                CellValue::Real(maximum)
            } else {
                CellValue::Null
            },
            if numbers > 0 && mean.is_finite() {
                CellValue::Real(mean)
            } else {
                CellValue::Null
            },
        ]));
    }
    output
}

#[derive(Clone, Debug)]
pub struct MigrationDraft {
    pub sql: String,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
}
/// Compare available column/key metadata. Destructive or ambiguous operations
/// stay commented out; callers present this document for editing and review.
pub fn schema_migration(
    kind: DatabaseKind,
    before: &RelationalSchema,
    after: &RelationalSchema,
) -> Result<MigrationDraft> {
    let details = before.details_captured && after.details_captured;
    let mut draft = MigrationDraft { sql: "-- Migration from existing schema to desired schema. Review before running.\n-- Scope: columns, defaults, primary keys, check constraints, indexes and foreign keys. Views, triggers, sequences and routines are compared when captured; grants are not captured.\n".into(), changes: Vec::new(), warnings: Vec::new() };
    if !details {
        draft.warnings.push("The captured baseline predates default, index and check capture; recapture it to compare them".into());
    }
    fn comment(value: &str) -> String {
        value
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect()
    }
    if before.objects_captured && after.objects_captured {
        for object in &after.objects {
            let old = before.objects.iter().find(|old| {
                old.kind == object.kind && old.schema == object.schema && old.name == object.name
            });
            if old == Some(object) {
                continue;
            }
            draft.changes.push(format!(
                "{} {:?} {}",
                if old.is_some() { "Changed" } else { "Added" },
                object.kind,
                object.name
            ));
            draft.sql.push_str(&format!(
                "-- Review {:?} {} and its dependencies:\n",
                object.kind,
                comment(&object.name)
            ));
            if let Some(definition) = &object.definition {
                for line in definition.lines() {
                    draft.sql.push_str(&format!("-- {line}\n"));
                }
            } else {
                draft
                    .warnings
                    .push(format!("Definition unavailable for {}", object.name));
            }
        }
        for object in &before.objects {
            if !after.objects.iter().any(|new| {
                new.kind == object.kind && new.schema == object.schema && new.name == object.name
            }) {
                draft
                    .changes
                    .push(format!("Removed {:?} {}", object.kind, object.name));
                draft.sql.push_str(&format!(
                    "-- Removed {:?} {}. Review dependencies before dropping.\n",
                    object.kind,
                    comment(&object.name)
                ));
            }
        }
    }
    for table in after
        .tables
        .iter()
        .filter(|table| table.table.kind == EntityKind::View)
    {
        let old = before.tables.iter().find(|old| old.table == table.table);
        if old.is_some_and(|old| old.structure.definition == table.structure.definition) {
            continue;
        }
        draft
            .changes
            .push(format!("View definition changed: {}", table.table.name));
        if let Some(definition) = &table.structure.definition {
            draft
                .sql
                .push_str(&format!("-- Review view {}:\n", comment(&table.table.name)));
            for line in definition.lines() {
                draft.sql.push_str(&format!("-- {line}\n"));
            }
        }
    }
    for table in before
        .tables
        .iter()
        .filter(|table| table.table.kind == EntityKind::View)
    {
        if !after.tables.iter().any(|new| new.table == table.table) {
            draft
                .changes
                .push(format!("Removed view: {}", table.table.name));
            draft.sql.push_str(&format!(
                "-- DROP VIEW {}; -- Review dependencies before dropping.\n",
                quote_table(
                    kind,
                    &TableRef {
                        name: table.table.name.clone(),
                        schema: table.table.schema.clone()
                    }
                )?
            ));
        }
    }
    let mut added = Vec::new();
    let refs: Vec<_> = after
        .tables
        .iter()
        .filter(|table| table.table.kind == EntityKind::Table)
        .map(|table| TableRef {
            name: table.table.name.clone(),
            schema: table.table.schema.clone(),
        })
        .collect();
    for table in &after.tables {
        if table.table.kind != EntityKind::Table {
            continue;
        }
        let reference = TableRef {
            name: table.table.name.clone(),
            schema: table.table.schema.clone(),
        };
        let quoted = quote_table(kind, &reference)?;
        let old = before.tables.iter().find(|old| old.table == table.table);
        let Some(old) = old else {
            draft.changes.push(format!("Added table {quoted}"));
            let definition = if matches!(
                kind.dialect(),
                DatabaseKind::PostgreSQL | DatabaseKind::MySQL
            ) {
                crate::transfer::render_sql_schema_without_foreign_keys(
                    kind,
                    &reference,
                    &table.structure,
                    &refs,
                )?
            } else {
                crate::render_sql_schema(kind, &reference, &table.structure, &refs)?
            };
            draft.sql.push_str(&format!("{definition};\n"));
            for statement in crate::render_sql_indexes(kind, &reference, &table.structure)? {
                if statement.starts_with("--") {
                    draft.sql.push_str(&format!("{statement}\n"));
                } else {
                    draft.sql.push_str(&format!("{statement};\n"));
                }
            }
            added.push((reference, &table.structure));
            continue;
        };
        for column in &table.structure.columns {
            let quoted_column = quote_identifier(kind, &column.name)?;
            match old
                .structure
                .columns
                .iter()
                .find(|old| old.name == column.name)
            {
                None => {
                    draft
                        .changes
                        .push(format!("Added {quoted}.{quoted_column}"));
                    let default = match column.default_value.as_deref() {
                        Some(default) => format!(
                            " DEFAULT {}",
                            crate::transfer::safe_schema_expression(default)?
                        ),
                        None => String::new(),
                    };
                    let statement = format!(
                        "ALTER TABLE {quoted} ADD COLUMN {quoted_column} {}{default}{};",
                        crate::transfer::safe_schema_type(&column.data_type)?,
                        if column.nullable { "" } else { " NOT NULL" }
                    );
                    // A default backfills existing rows, so a required column
                    // with one needs no manual strategy.
                    if column.primary_key || (!column.nullable && default.is_empty()) {
                        draft.warnings.push(format!(
                            "{quoted}.{quoted_column} requires a key/backfill strategy"
                        ));
                        draft
                            .sql
                            .push_str(&format!("-- MANUAL: {}\n", comment(&statement)));
                    } else {
                        draft.sql.push_str(&format!("{statement}\n"));
                    }
                }
                Some(previous)
                    if previous.data_type == column.data_type
                        && previous.nullable == column.nullable
                        && previous.primary_key == column.primary_key
                        && previous.enum_values == column.enum_values
                        && previous.ordinal == column.ordinal =>
                {
                    if details && previous.default_value != column.default_value {
                        draft
                            .changes
                            .push(format!("Changed default of {quoted}.{quoted_column}"));
                        let action = match column.default_value.as_deref() {
                            Some(default) => format!(
                                "SET DEFAULT {}",
                                crate::transfer::safe_schema_expression(default)?
                            ),
                            None => "DROP DEFAULT".into(),
                        };
                        let statement =
                            format!("ALTER TABLE {quoted} ALTER COLUMN {quoted_column} {action};");
                        if kind.dialect() == DatabaseKind::SQLite {
                            draft.warnings.push(format!(
                                "SQLite cannot alter the default of {quoted}.{quoted_column}; rebuild the table"
                            ));
                            draft
                                .sql
                                .push_str(&format!("-- MANUAL: {}\n", comment(&statement)));
                        } else {
                            draft.sql.push_str(&format!("{statement}\n"));
                        }
                    }
                }
                Some(previous) if previous != column => {
                    draft.changes.push(format!("Changed {quoted}.{quoted_column}: {} -> {} (nullable {} -> {}, primary key {} -> {})", previous.data_type, column.data_type, previous.nullable, column.nullable, previous.primary_key, column.primary_key));
                    draft.warnings.push(format!(
                        "Review type, constraint or order change on {quoted}.{quoted_column}"
                    ));
                    draft.sql.push_str(&format!("-- MANUAL: {}\n", comment(&format!("alter {quoted}.{quoted_column}; verify casts, constraints and data preservation."))));
                }
                _ => {}
            }
        }
        for column in &old.structure.columns {
            if !table
                .structure
                .columns
                .iter()
                .any(|current| current.name == column.name)
            {
                let column = quote_identifier(kind, &column.name)?;
                draft.changes.push(format!("Removed {quoted}.{column}"));
                draft.warnings.push(format!(
                    "Dropping {quoted}.{column} destroys data; check for a rename"
                ));
                draft.sql.push_str(&format!(
                    "-- DESTRUCTIVE: {}\n",
                    comment(&format!("ALTER TABLE {quoted} DROP COLUMN {column};"))
                ));
            }
        }
        if details {
            migrate_indexes(
                kind,
                &reference,
                &old.structure,
                &table.structure,
                &mut draft,
            )?;
            migrate_checks(kind, &quoted, &old.structure, &table.structure, &mut draft)?;
        }
        if old.structure.foreign_keys != table.structure.foreign_keys {
            draft
                .changes
                .push(format!("Foreign keys changed on {quoted}"));
            draft
                .warnings
                .push(format!("Review foreign key changes on {quoted}"));
            draft.sql.push_str(&format!(
                "-- MANUAL: {}\n",
                comment(&format!("reconcile foreign keys on {quoted}."))
            ));
        }
    }
    if matches!(
        kind.dialect(),
        DatabaseKind::PostgreSQL | DatabaseKind::MySQL
    ) {
        let mut constraints = Vec::new();
        for (table, structure) in added {
            crate::transfer::append_sql_foreign_keys(
                kind,
                &table,
                structure,
                &refs,
                &mut constraints,
            )?;
        }
        draft.sql.push_str(
            &String::from_utf8(constraints)
                .map_err(|_| DbxError::Decode("Invalid migration encoding".into()))?,
        );
    }
    for old in &before.tables {
        if old.table.kind == EntityKind::Table
            && !after.tables.iter().any(|table| table.table == old.table)
        {
            let table = quote_table(
                kind,
                &TableRef {
                    name: old.table.name.clone(),
                    schema: old.table.schema.clone(),
                },
            )?;
            draft.changes.push(format!("Removed table {table}"));
            draft.warnings.push(format!(
                "Dropping {table} destroys data; check for a rename"
            ));
            draft.sql.push_str(&format!(
                "-- DESTRUCTIVE: {}\n",
                comment(&format!("DROP TABLE {table};"))
            ));
        }
    }
    Ok(draft)
}

fn migrate_indexes(
    kind: DatabaseKind,
    table: &TableRef,
    before: &crate::TableStructure,
    after: &crate::TableStructure,
    draft: &mut MigrationDraft,
) -> Result<()> {
    let secondary = |structure: &crate::TableStructure| {
        structure
            .indexes
            .iter()
            .filter(|index| !index.primary)
            .cloned()
            .collect::<Vec<_>>()
    };
    let (old, new) = (secondary(before), secondary(after));
    let quoted = quote_table(kind, table)?;
    for index in &old {
        if new.iter().any(|current| current == index) {
            continue;
        }
        let name = quote_identifier(kind, &index.name)?;
        draft
            .changes
            .push(format!("Removed or changed index {name} on {quoted}"));
        let statement = match kind.dialect() {
            DatabaseKind::MySQL => format!("DROP INDEX {name} ON {quoted};"),
            DatabaseKind::PostgreSQL => match &table.schema {
                Some(schema) => format!("DROP INDEX {}.{name};", quote_identifier(kind, schema)?),
                None => format!("DROP INDEX {name};"),
            },
            _ => format!("DROP INDEX {name};"),
        };
        draft.sql.push_str(&format!("{statement}\n"));
    }
    let added = new
        .into_iter()
        .filter(|index| !old.contains(index))
        .collect::<Vec<_>>();
    if added.is_empty() {
        return Ok(());
    }
    for index in &added {
        draft.changes.push(format!(
            "Added or changed index {} on {quoted}",
            quote_identifier(kind, &index.name)?
        ));
        if index.unique {
            draft.warnings.push(format!(
                "Unique index {} fails if {quoted} already holds duplicates",
                quote_identifier(kind, &index.name)?
            ));
        }
    }
    let structure = crate::TableStructure {
        indexes: added,
        ..Default::default()
    };
    for statement in crate::render_sql_indexes(kind, table, &structure)? {
        if statement.starts_with("--") {
            draft.sql.push_str(&format!("{statement}\n"));
        } else {
            draft.sql.push_str(&format!("{statement};\n"));
        }
    }
    Ok(())
}

fn migrate_checks(
    kind: DatabaseKind,
    quoted: &str,
    before: &crate::TableStructure,
    after: &crate::TableStructure,
    draft: &mut MigrationDraft,
) -> Result<()> {
    for check in &before.checks {
        if after.checks.contains(check) {
            continue;
        }
        let Some(name) = &check.name else {
            continue;
        };
        let name = quote_identifier(kind, name)?;
        draft
            .changes
            .push(format!("Removed or changed check {name} on {quoted}"));
        draft
            .sql
            .push_str(&format!("ALTER TABLE {quoted} DROP CONSTRAINT {name};\n"));
    }
    for check in &after.checks {
        if before.checks.contains(check) {
            continue;
        }
        let constraint = match &check.name {
            Some(name) => format!("CONSTRAINT {} ", quote_identifier(kind, name)?),
            None => String::new(),
        };
        draft
            .changes
            .push(format!("Added or changed check {constraint}on {quoted}"));
        draft.warnings.push(format!(
            "New check constraints on {quoted} fail if existing rows violate them"
        ));
        draft.sql.push_str(&format!(
            "ALTER TABLE {quoted} ADD {constraint}CHECK ({});\n",
            crate::transfer::safe_schema_expression(&check.expression)?
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explain_never_executes_analyze_or_writing_ctes() {
        assert!(
            execution_plan_query(
                DatabaseKind::PostgreSQL,
                "WITH changed AS (DELETE FROM t RETURNING *) SELECT * FROM changed"
            )
            .is_err()
        );
        assert!(execution_plan_query(DatabaseKind::SQLite, "SELECT 1; DELETE FROM t").is_err());
        assert_eq!(
            execution_plan_query(DatabaseKind::SQLite, "SELECT 'DELETE'").unwrap(),
            "EXPLAIN QUERY PLAN SELECT 'DELETE'"
        );
    }

    #[test]
    fn table_statistics_quote_identifiers_and_skip_unsupported_operators() {
        let columns = vec![
            ColumnInfo::result("a\"b", 0, "NUMERIC(40,12)"),
            ColumnInfo::result("doc", 1, "JSONB"),
            ColumnInfo::result("flag", 2, "BOOLEAN"),
        ];
        let sql = table_column_statistics_query(
            DatabaseKind::PostgreSQL,
            &TableRef::in_schema("public", "a.b"),
            &columns,
        )
        .unwrap();
        assert!(sql.contains("MIN(\"a\"\"b\")"));
        assert!(sql.contains("FROM \"public\".\"a.b\""));
        assert!(!sql.contains("MIN(\"doc\")"));
        assert!(!sql.contains("COUNT(DISTINCT \"doc\")"));
        assert!(sql.contains("COUNT(DISTINCT \"flag\")"));
        assert!(!sql.contains("MIN(\"flag\")"));
        assert!(!sql.contains("LIMIT"));
    }

    #[test]
    fn analyze_requires_supported_engine_and_one_read_statement() {
        assert!(execution_plan_analyze_query(DatabaseKind::SQLite, "SELECT 1").is_err());
        assert_eq!(
            execution_plan_analyze_query(DatabaseKind::MySQL, "SELECT 1").unwrap(),
            "EXPLAIN ANALYZE FORMAT=TREE SELECT 1"
        );
        assert!(execution_plan_analyze_query(DatabaseKind::PostgreSQL, "DELETE FROM t").is_err());
        assert!(
            execution_plan_analyze_query(DatabaseKind::PostgreSQL, "SELECT 1; COMMIT").is_err()
        );
        assert!(
            execution_plan_analyze_query(
                DatabaseKind::PostgreSQL,
                "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x"
            )
            .is_err()
        );
        assert_eq!(
            execution_plan_analyze_query(DatabaseKind::PostgreSQL, "SELECT 1").unwrap(),
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) SELECT 1"
        );
    }

    #[test]
    fn loaded_statistics_distinguish_null_empty_and_literal_null_and_scope_to_result() {
        let source = QueryResult {
            columns: vec![
                ColumnInfo::result("label", 0, "TEXT"),
                ColumnInfo::result("count", 1, "INTEGER"),
            ],
            rows: vec![
                RowData::new(vec![CellValue::Null, CellValue::Integer(1)]),
                RowData::new(vec![CellValue::Text(String::new()), CellValue::Integer(3)]),
                RowData::new(vec![CellValue::Text("NULL".into()), CellValue::Integer(5)]),
                RowData::new(vec![CellValue::Text("NULL".into()), CellValue::Null]),
            ],
            truncated: true,
            ..QueryResult::empty(None, 0)
        };
        let stats = loaded_column_statistics(&source);
        assert_eq!(stats.rows[0].values[2], CellValue::Integer(4));
        assert_eq!(stats.rows[0].values[3], CellValue::Integer(1));
        assert_eq!(stats.rows[0].values[4], CellValue::Integer(2));
        assert_eq!(stats.rows[0].values[6], CellValue::Null);
        assert_eq!(stats.rows[1].values[5], CellValue::Integer(3));
        assert_eq!(stats.rows[1].values[6], CellValue::Real(1.));
        assert_eq!(stats.rows[1].values[7], CellValue::Real(5.));
        assert_eq!(stats.rows[1].values[8], CellValue::Real(3.));
        assert!(source.truncated);
    }
    #[test]
    fn schema_drafts_keep_destructive_changes_and_identifier_newlines_commented() {
        let column = |name: &str| ColumnInfo::result(name, 0, "TEXT");
        let table = |name: &str, columns: Vec<ColumnInfo>| crate::RelationalTable {
            table: crate::TableInfo::table(name, None),
            structure: crate::TableStructure {
                columns,
                foreign_keys: Vec::new(),
                ..Default::default()
            },
        };
        let before = RelationalSchema {
            database: "db".into(),
            tables: vec![
                table("old\nSELECT 1;", vec![column("value")]),
                table("kept", vec![column("old\nSELECT 2;")]),
            ],
            details_captured: true,
            objects: Vec::new(),
            objects_captured: false,
        };
        let after = RelationalSchema {
            database: "db".into(),
            tables: vec![table("kept", vec![column("new")])],
            details_captured: true,
            objects: Vec::new(),
            objects_captured: false,
        };
        let draft = schema_migration(DatabaseKind::SQLite, &before, &after).unwrap();
        let statements = crate::split_sql_statements(&draft.sql);
        assert_eq!(statements.len(), 1);
        assert!(statements[0].starts_with("ALTER TABLE"));
        assert!(statements[0].contains("ADD COLUMN"));
        assert_eq!(draft.changes.len(), 3);
        assert_eq!(draft.warnings.len(), 2);
    }

    #[test]
    fn schema_draft_creates_referenced_tables_before_adding_foreign_keys() {
        let mut key = ColumnInfo::result("id", 0, "INTEGER");
        key.primary_key = true;
        key.nullable = false;
        let schema = RelationalSchema {
            database: "db".into(),
            tables: vec![
                crate::RelationalTable {
                    table: crate::TableInfo::table("child", None),
                    structure: crate::TableStructure {
                        columns: vec![key.clone(), ColumnInfo::result("parent_id", 1, "INTEGER")],
                        foreign_keys: vec![crate::ForeignKeyInfo {
                            constraint_name: Some("parent_fk".into()),
                            columns: vec!["parent_id".into()],
                            referenced_schema: None,
                            referenced_table: "parent".into(),
                            referenced_columns: vec!["id".into()],
                            on_update: None,
                            on_delete: None,
                        }],
                        ..Default::default()
                    },
                },
                crate::RelationalTable {
                    table: crate::TableInfo::table("parent", None),
                    structure: crate::TableStructure {
                        columns: vec![key],
                        foreign_keys: Vec::new(),
                        ..Default::default()
                    },
                },
            ],
            ..Default::default()
        };
        for kind in [DatabaseKind::PostgreSQL, DatabaseKind::MySQL] {
            let before = RelationalSchema {
                database: "db".into(),
                tables: Vec::new(),
                ..Default::default()
            };
            let draft = schema_migration(kind, &before, &schema).unwrap();
            let statements = crate::split_sql_statements(&draft.sql);
            assert_eq!(statements.len(), 3);
            assert!(statements[0].starts_with("CREATE TABLE"));
            assert!(statements[1].starts_with("CREATE TABLE"));
            assert!(statements[2].starts_with("ALTER TABLE"));
            assert!(statements[2].contains("FOREIGN KEY"));
        }
    }
}
