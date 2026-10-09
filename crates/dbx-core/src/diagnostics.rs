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
    let details = before.details_captured && after.details_captured;
    let mut draft = MigrationDraft { sql: "-- Migration from captured schema to current schema. Review before running.\n-- Scope: columns, defaults, primary keys, check constraints, indexes and foreign keys. Views, triggers, sequences and routines are compared when captured; grants are not captured.\n".into(), changes: Vec::new(), warnings: Vec::new() };
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
    fn schema_drafts_cover_defaults_indexes_and_checks_only_for_detailed_baselines() {
        let mut code = ColumnInfo::result("code", 1, "text");
        code.nullable = false;
        let index = |name: &str, unique: bool| crate::IndexInfo {
            name: name.into(),
            columns: vec!["code".into()],
            unique,
            primary: false,
            method: Some("btree".into()),
            predicate: None,
            definition: Some(format!(
                "CREATE {}INDEX {name} ON public.items USING btree (code)",
                if unique { "UNIQUE " } else { "" }
            )),
        };
        let schema = |code: ColumnInfo, indexes, checks, details_captured| RelationalSchema {
            database: "db".into(),
            tables: vec![crate::RelationalTable {
                table: crate::TableInfo::table("items", Some("public".into())),
                structure: crate::TableStructure {
                    columns: vec![code],
                    indexes,
                    checks,
                    ..Default::default()
                },
            }],
            details_captured,
            objects: Vec::new(),
            objects_captured: false,
        };
        let mut defaulted = code.clone();
        defaulted.default_value = Some("'new'::text".into());
        let check = crate::CheckConstraintInfo {
            name: Some("code_length".into()),
            expression: "length(code) > 0".into(),
        };
        let before = schema(code.clone(), vec![index("items_old", false)], vec![], true);
        let after = schema(
            defaulted.clone(),
            vec![index("items_code", true)],
            vec![check.clone()],
            true,
        );
        let draft = schema_migration(DatabaseKind::PostgreSQL, &before, &after).unwrap();
        assert!(draft.sql.contains(
            "ALTER TABLE \"public\".\"items\" ALTER COLUMN \"code\" SET DEFAULT 'new'::text;"
        ));
        assert!(draft.sql.contains("DROP INDEX \"public\".\"items_old\";"));
        assert!(draft.sql.contains(
            "CREATE UNIQUE INDEX IF NOT EXISTS items_code ON public.items USING btree (code);"
        ));
        assert!(draft.sql.contains(
            "ALTER TABLE \"public\".\"items\" ADD CONSTRAINT \"code_length\" CHECK (length(code) > 0);"
        ));
        assert!(!draft.sql.contains("MANUAL"), "{}", draft.sql);

        // A baseline from an earlier version has no defaults or indexes.
        let legacy = schema(code, vec![], vec![], false);
        let draft = schema_migration(DatabaseKind::PostgreSQL, &legacy, &after).unwrap();
        assert!(draft.changes.is_empty(), "{:?}", draft.changes);
        assert_eq!(draft.warnings.len(), 1);
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
