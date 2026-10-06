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
        _ => return Err(DbxError::Unsupported { operation: "server session/lock monitoring".into(), kind }),
    })
}
pub fn execution_plan_query(kind: DatabaseKind, sql: &str) -> Result<String> {
    let statements = crate::transfer::checked_split_sql_for(Some(kind), sql)?;
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
pub fn format_execution_plan(kind: DatabaseKind, result: &QueryResult) -> QueryResult {
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
    output.columns = [
        "Depth",
        "Operation",
        "Estimated rows",
        "Estimated cost",
        "Detail",
    ]
    .iter()
    .enumerate()
    .map(|(ordinal, name)| ColumnInfo::result(*name, ordinal, "TEXT"))
    .collect();
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
    let mut draft = MigrationDraft { sql: "-- Migration from captured schema to current schema. Review before running.\n-- Scope: columns, primary keys and foreign keys. Defaults, indexes, triggers, grants and views are not captured.\n".into(), changes: Vec::new(), warnings: Vec::new() };
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
                    let statement = format!(
                        "ALTER TABLE {quoted} ADD COLUMN {quoted_column} {}{};",
                        crate::transfer::safe_schema_type(&column.data_type)?,
                        if column.nullable { "" } else { " NOT NULL" }
                    );
                    if column.primary_key || !column.nullable {
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
    fn postgres_plans_are_flattened_and_compared() {
        let mut raw = QueryResult::empty(None, 1);
        raw.rows.push(RowData::new(vec![CellValue::Json(serde_json::json!([{"Plan":{"Node Type":"Nested Loop","Plan Rows":2,"Total Cost":10,"Plans":[{"Node Type":"Index Scan","Index Name":"idx","Plan Rows":2}]}}]))]));
        let before = format_execution_plan(DatabaseKind::PostgreSQL, &raw);
        assert_eq!(before.rows.len(), 2);
        assert_eq!(before.rows[1].values[0], CellValue::Integer(1));
        let mut after = before.clone();
        after.rows.pop();
        assert_eq!(
            compare_execution_plans(&before, &after).rows[1].values[1],
            CellValue::Text("Removed".into())
        );
    }
    #[test]
    fn schema_drafts_keep_destructive_changes_and_identifier_newlines_commented() {
        let column = |name: &str| ColumnInfo::result(name, 0, "TEXT");
        let table = |name: &str, columns: Vec<ColumnInfo>| crate::RelationalTable {
            table: crate::TableInfo::table(name, None),
            structure: crate::TableStructure {
                columns,
                foreign_keys: Vec::new(),
            },
        };
        let before = RelationalSchema {
            database: "db".into(),
            tables: vec![
                table("old\nSELECT 1;", vec![column("value")]),
                table("kept", vec![column("old\nSELECT 2;")]),
            ],
        };
        let after = RelationalSchema {
            database: "db".into(),
            tables: vec![table("kept", vec![column("new")])],
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
                    },
                },
                crate::RelationalTable {
                    table: crate::TableInfo::table("parent", None),
                    structure: crate::TableStructure {
                        columns: vec![key],
                        foreign_keys: Vec::new(),
                    },
                },
            ],
        };
        for kind in [DatabaseKind::PostgreSQL, DatabaseKind::MySQL] {
            let before = RelationalSchema {
                database: "db".into(),
                tables: Vec::new(),
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
