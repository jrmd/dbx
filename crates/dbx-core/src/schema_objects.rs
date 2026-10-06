//! Ancillary schema objects shared by explorer, dumps, and comparison.
use crate::{
    CellValue, DatabaseKind, Engine, QueryOptions, Result, SchemaObject, SchemaObjectKind,
};

pub(crate) async fn capture<E: Engine + ?Sized>(engine: &E) -> Result<Vec<SchemaObject>> {
    let kind = engine.kind();
    let sql = match kind {
        DatabaseKind::SQLite | DatabaseKind::Turso | DatabaseKind::CloudflareD1 => "SELECT type, name, NULL AS schema_name, tbl_name, sql FROM sqlite_master WHERE type='trigger' AND sql IS NOT NULL ORDER BY name".to_owned(),
        DatabaseKind::PostgreSQL => r#"SELECT 'function', p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')', n.nspname, NULL::text, pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND p.prokind IN ('f','p') UNION ALL SELECT 'trigger', t.tgname, n.nspname, c.relname, pg_get_triggerdef(t.oid) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE NOT t.tgisinternal UNION ALL SELECT 'sequence', sequencename, schemaname, NULL::text, 'CREATE SEQUENCE ' || quote_ident(schemaname) || '.' || quote_ident(sequencename) || ' AS ' || data_type::text || ' INCREMENT BY ' || increment_by || ' MINVALUE ' || min_value || ' MAXVALUE ' || max_value || ' START WITH ' || start_value || ' CACHE ' || cache_size || CASE WHEN cycle THEN ' CYCLE' ELSE ' NO CYCLE' END FROM pg_sequences WHERE schemaname NOT IN ('pg_catalog','information_schema') AND NOT EXISTS (SELECT 1 FROM pg_class sc JOIN pg_namespace sn ON sn.oid=sc.relnamespace JOIN pg_depend d ON d.objid=sc.oid AND d.classid='pg_class'::regclass WHERE sc.relname=sequencename AND sn.nspname=schemaname AND d.deptype IN ('a','i')) ORDER BY 1,3,2"#.to_owned(),
        DatabaseKind::MySQL => "SELECT ROUTINE_TYPE, ROUTINE_NAME, ROUTINE_SCHEMA, NULL AS table_name, ROUTINE_DEFINITION FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA=DATABASE() UNION ALL SELECT 'trigger', TRIGGER_NAME, TRIGGER_SCHEMA, EVENT_OBJECT_TABLE, ACTION_STATEMENT FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA=DATABASE() ORDER BY 1,3,2".to_owned(),
        DatabaseKind::SqlServer => "SELECT CASE WHEN o.type='TR' THEN 'trigger' WHEN o.type='P' THEN 'procedure' ELSE 'function' END, o.name, SCHEMA_NAME(o.schema_id), OBJECT_NAME(NULLIF(o.parent_object_id,0)), m.definition FROM sys.objects o JOIN sys.sql_modules m ON m.object_id=o.object_id WHERE o.is_ms_shipped=0 AND o.type IN ('TR','P','FN','IF','TF') UNION ALL SELECT 'sequence', name, SCHEMA_NAME(schema_id), NULL, 'CREATE SEQUENCE ' + QUOTENAME(SCHEMA_NAME(schema_id)) + '.' + QUOTENAME(name) + ' AS ' + TYPE_NAME(system_type_id) + ' START WITH ' + CONVERT(varchar(40),start_value) + ' INCREMENT BY ' + CONVERT(varchar(40),increment) + ' MINVALUE ' + CONVERT(varchar(40),minimum_value) + ' MAXVALUE ' + CONVERT(varchar(40),maximum_value) + CASE WHEN is_cycling=1 THEN ' CYCLE' ELSE ' NO CYCLE' END FROM sys.sequences ORDER BY 1,3,2".to_owned(),
        DatabaseKind::DuckDB => "SELECT 'function', function_name, schema_name, NULL AS table_name, 'CREATE MACRO ' || function_name || '(' || array_to_string(parameters, ', ') || ') AS ' || macro_definition FROM duckdb_functions() WHERE function_type='macro' AND NOT internal ORDER BY schema_name, function_name".to_owned(),
        DatabaseKind::BigQuery => {
            let database = engine.current_database().await?;
            if database.is_empty() { return Ok(Vec::new()); }
            format!("SELECT routine_type, routine_name, routine_schema, CAST(NULL AS STRING) AS table_name, ddl FROM {}.INFORMATION_SCHEMA.ROUTINES ORDER BY routine_name", crate::quote_identifier(kind, &database)?)
        }
        _ => return Ok(Vec::new()),
    };
    let rows = engine
        .query(&sql, QueryOptions { max_rows: None })
        .await?
        .rows;
    let mut objects = Vec::new();
    for row in rows {
        let text = |index| {
            row.values.get(index).and_then(|value| {
                if let CellValue::Null = value {
                    None
                } else {
                    Some(value.to_string())
                }
            })
        };
        let category = text(0).unwrap_or_default().to_ascii_lowercase();
        let object_kind = match category.as_str() {
            "trigger" => SchemaObjectKind::Trigger,
            "sequence" => SchemaObjectKind::Sequence,
            "procedure" => SchemaObjectKind::Procedure,
            _ => SchemaObjectKind::Function,
        };
        let name = text(1).unwrap_or_default();
        let schema = text(2);
        let table = text(3);
        let mut definition = text(4);
        if kind == DatabaseKind::MySQL {
            let keyword = match object_kind {
                SchemaObjectKind::Trigger => "TRIGGER",
                SchemaObjectKind::Procedure => "PROCEDURE",
                _ => "FUNCTION",
            };
            let qualified = crate::sql::quote_table(
                kind,
                &crate::TableRef {
                    schema: schema.clone(),
                    name: name.clone(),
                },
            )?;
            let result = engine
                .query(
                    &format!("SHOW CREATE {keyword} {qualified}"),
                    QueryOptions::default(),
                )
                .await?;
            let index = result.columns.iter().position(|column| {
                column
                    .name
                    .eq_ignore_ascii_case(&format!("Create {keyword}"))
            });
            definition = index
                .and_then(|index| result.rows.first()?.values.get(index))
                .map(ToString::to_string);
        }
        objects.push(SchemaObject {
            kind: object_kind,
            name,
            schema,
            table,
            definition,
        });
    }
    Ok(objects)
}

/// SQLite exposes CHECK expressions only in its original CREATE statement.
/// Keep byte ranges in the source and skip strings, identifiers and comments.
pub(crate) fn sqlite_checks(source: &str) -> Vec<crate::CheckConstraintInfo> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        match bytes[index] {
            b'\'' | b'"' | b'`' | b'[' => {
                let closing = if bytes[index] == b'[' {
                    b']'
                } else {
                    bytes[index]
                };
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == closing {
                        index += 1;
                        if bytes.get(index) != Some(&closing) {
                            break;
                        }
                    }
                    index += 1;
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                tokens.push((start, index));
            }
            b'(' | b')' => {
                index += 1;
                tokens.push((start, index));
            }
            _ => index += 1,
        }
    }
    let mut checks = Vec::new();
    for (index, &(start, end)) in tokens.iter().enumerate() {
        if !source[start..end].eq_ignore_ascii_case("CHECK") {
            continue;
        }
        let Some(&(open, open_end)) = tokens.get(index + 1) else {
            continue;
        };
        if &source[open..open_end] != "(" {
            continue;
        }
        let mut depth = 1;
        for &(start, end) in &tokens[index + 2..] {
            match &source[start..end] {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        checks.push(crate::CheckConstraintInfo {
                            name: None,
                            expression: source[open_end..start].trim().to_owned(),
                        });
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    checks
}

#[cfg(test)]
mod tests {
    #[test]
    fn sqlite_check_parser_preserves_nested_expressions_and_skips_quoted_keywords() {
        let checks = super::sqlite_checks(
            "CREATE TABLE t (\"CHECK\" TEXT DEFAULT 'CHECK(x)', x INT CHECK (x > 0 AND length(')') > 0), CHECK ((x + 1) < 10)) -- CHECK(no)\n",
        );
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].expression, "x > 0 AND length(')') > 0");
        assert_eq!(checks[1].expression, "(x + 1) < 10");
    }
}
