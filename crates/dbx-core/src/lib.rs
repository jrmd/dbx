//! Shared database access primitives for DBX.
//!
//! The crate deliberately keeps the UI out of the connection layer.  A
//! [`DatabaseEngine`] owns one connection pool (or one Redis connection
//! manager) and exposes database-agnostic metadata, query, and mutation
//! operations for the GPUI client.

mod cloud_auth;
pub use cloud_auth::{CloudAuthentication, cloud_database_password};
mod schema_objects;
pub use model::{SchemaObject, SchemaObjectKind};
mod designer;
pub use designer::{TableAlteration, draft_table_alteration};
mod connectors;
mod console;
mod diagnostics;
mod engine;
mod error;
mod model;
mod protected;
mod redis_catalog;
mod redis_engine;
mod sql;
mod sqlx_engine;
mod transfer;
pub use diagnostics::{
    MigrationDraft, Monitor, compare_execution_plans, execution_plan_query, format_execution_plan,
    monitor_query, schema_migration,
};
mod transport;

pub use transport::SshConfig;

pub use console::{QueryCancellation, QuerySession, ScriptResult, StatementResult};
pub use engine::{DatabaseEngine, Engine, QueryOptions};
pub use error::{DbxError, Result};
pub use model::{
    CellValue, CheckConstraintInfo, ColumnInfo, ConnectionConfig, CreateColumn, CreateTableRequest,
    DatabaseKind, EntityKind, ExecResult, Filter, FilterOperator, ForeignKeyInfo, IndexInfo,
    InsertRequest, MutationValue, Order, OrderDirection, Page, QueryResult, ReferentialAction,
    RelationalSchema, RelationalTable, RowChange, RowData, TableInfo, TableRef, TableStructure,
    UpdateRequest,
};
pub use redis_catalog::{RedisCommand, RedisCommandArgument, RedisCommandCatalog};
pub use redis_engine::RedisEngine;
pub use sql::{
    SqlStatement, build_count, build_create_table, build_delete, build_delete_with_columns,
    build_drop_table, build_insert, build_insert_with_columns, build_multi_row_insert,
    build_row_estimate, build_select, build_select_with_columns, build_truncate_table,
    build_update, build_update_with_columns, quote_identifier, validate_sql_expression,
};
pub use sqlx_engine::SqlxEngine;
pub use transfer::checked_split_sql_for;
pub use transfer::{
    DatabaseExportRequest, DatabaseExportSummary, DelimitedReader, DumpFormat, ExportSummary,
    FileFormat, ImportReport, TransferControl, detect_file_format, export_database, export_table,
    import_database, import_file, render_row_change, render_sql_indexes, render_sql_insert,
    render_sql_schema, split_sql_statements, with_transfer_control,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_debug_redacts_credentials() {
        let config = ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            "postgres://secret:password@example.test/app",
        );
        let debug = format!("{config:?}");
        assert!(!debug.contains("password"));
        assert!(debug.contains("<redacted>@example.test"));
    }

    #[test]
    fn sql_builder_parameterizes_values_and_quotes_identifiers() {
        let request = InsertRequest {
            table: TableRef::in_schema("public", "user\"events"),
            columns: vec!["display name".into(), "count".into()],
            values: vec![
                CellValue::Text("O'Reilly".into()).into(),
                CellValue::Integer(3).into(),
            ],
        };
        let statement = build_insert(DatabaseKind::PostgreSQL, &request).unwrap();
        assert_eq!(
            statement.sql,
            "INSERT INTO \"public\".\"user\"\"events\" (\"display name\", \"count\") VALUES ($1, $2)"
        );
        assert_eq!(
            statement.params,
            vec![CellValue::Text("O'Reilly".into()), CellValue::Integer(3)]
        );
        assert!(!statement.sql.contains("O'Reilly"));
    }

    #[test]
    fn mutation_builders_support_full_rows_and_typed_values() {
        let request = InsertRequest::from_row(
            TableRef::in_schema("public", "events"),
            vec![
                ("id".into(), CellValue::Integer(7)),
                ("enabled".into(), CellValue::Boolean(true)),
                ("note".into(), CellValue::Null),
                (
                    "payload".into(),
                    CellValue::Json(serde_json::json!({ "ok": true })),
                ),
            ],
        );
        let insert = build_insert(DatabaseKind::PostgreSQL, &request).unwrap();
        assert_eq!(
            insert.sql,
            "INSERT INTO \"public\".\"events\" (\"id\", \"enabled\", \"note\", \"payload\") VALUES ($1, $2, $3, $4)"
        );
        assert_eq!(
            insert.params,
            vec![
                CellValue::Integer(7),
                CellValue::Boolean(true),
                CellValue::Null,
                CellValue::Json(serde_json::json!({ "ok": true })),
            ]
        );

        let update = UpdateRequest::for_primary_key(
            TableRef::new("events"),
            vec![
                ("enabled".into(), CellValue::Boolean(false)),
                ("note".into(), CellValue::Null),
            ],
            vec![("id".into(), CellValue::Integer(7))],
        );
        let statement = build_update(DatabaseKind::MySQL, &update).unwrap();
        assert_eq!(
            statement.sql,
            "UPDATE `events` SET `enabled` = ?, `note` = ? WHERE `id` = ?"
        );
        assert_eq!(
            statement.params,
            vec![
                CellValue::Boolean(false),
                CellValue::Null,
                CellValue::Integer(7)
            ]
        );
    }

    #[test]
    fn mutation_builders_interleave_parameters_and_sql_expressions() {
        let request = InsertRequest::new_with_mutation_values(
            TableRef::new("events"),
            vec![
                "id".into(),
                "name".into(),
                "created_at".into(),
                "enabled".into(),
            ],
            vec![
                MutationValue::expression("uuidv7()"),
                MutationValue::parameter(CellValue::Text("launch".into())),
                MutationValue::expression("NOW()"),
                MutationValue::parameter(CellValue::Boolean(true)),
            ],
        );

        for (kind, expected_sql) in [
            (
                DatabaseKind::PostgreSQL,
                "INSERT INTO \"events\" (\"id\", \"name\", \"created_at\", \"enabled\") VALUES (uuidv7(), $1, NOW(), $2)",
            ),
            (
                DatabaseKind::MySQL,
                "INSERT INTO `events` (`id`, `name`, `created_at`, `enabled`) VALUES (uuidv7(), ?, NOW(), ?)",
            ),
            (
                DatabaseKind::SQLite,
                "INSERT INTO \"events\" (\"id\", \"name\", \"created_at\", \"enabled\") VALUES (uuidv7(), ?, NOW(), ?)",
            ),
        ] {
            let statement = build_insert(kind, &request).unwrap();
            assert_eq!(statement.sql, expected_sql);
            assert_eq!(
                statement.params,
                vec![CellValue::Text("launch".into()), CellValue::Boolean(true)]
            );
        }

        let update = UpdateRequest::new_with_mutation_values(
            TableRef::new("events"),
            vec![
                ("created_at".into(), MutationValue::expression("NOW()")),
                (
                    "name".into(),
                    MutationValue::parameter(CellValue::Text("launch".into())),
                ),
            ],
            vec![Filter::new(
                "id",
                FilterOperator::Equals,
                Some(CellValue::Integer(7)),
            )],
        );
        let statement = build_update(DatabaseKind::PostgreSQL, &update).unwrap();
        assert_eq!(
            statement.sql,
            "UPDATE \"events\" SET \"created_at\" = NOW(), \"name\" = $1 WHERE \"id\" = $2"
        );
        assert_eq!(
            statement.params,
            vec![CellValue::Text("launch".into()), CellValue::Integer(7)]
        );
    }

    #[test]
    fn mutation_expressions_reject_statement_separators_and_comments() {
        for expression in [
            "",
            "  ",
            "NOW(); DELETE FROM events",
            "NOW() -- comment",
            "/* comment */ NOW()",
            "NOW()\n",
            "NOW()#comment",
            "NOW()\0",
        ] {
            assert!(
                validate_sql_expression(expression).is_err(),
                "{expression:?}"
            );
        }
        assert_eq!(validate_sql_expression("  uuidv7()  ").unwrap(), "uuidv7()");

        let request = InsertRequest::new_with_mutation_values(
            TableRef::new("events"),
            vec!["id".into()],
            vec![MutationValue::expression("uuidv7(); SELECT 1")],
        );
        assert!(build_insert(DatabaseKind::PostgreSQL, &request).is_err());
    }

    fn typed_column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            enum_values: Vec::new(),
            nullable: true,
            ordinal: 1,
            primary_key: false,
            default_value: None,
        }
    }

    #[test]
    fn postgres_mutations_cast_text_and_null_parameters_to_the_column_type() {
        let columns = vec![
            typed_column("seen_at", "timestamp without time zone"),
            typed_column("flags", "bit(3)"),
            typed_column("tags", "text[]"),
            typed_column("count", "integer"),
            typed_column("odd", "x; DROP TABLE t"),
        ];
        let request = UpdateRequest::for_primary_key(
            TableRef::new("events"),
            vec![
                ("seen_at".into(), CellValue::Null),
                ("flags".into(), CellValue::Text("101".into())),
                ("tags".into(), CellValue::Text("{a,b}".into())),
                ("count".into(), CellValue::Integer(3)),
                ("odd".into(), CellValue::Text("x".into())),
            ],
            vec![("id".into(), CellValue::Integer(7))],
        );
        assert_eq!(
            build_update_with_columns(DatabaseKind::PostgreSQL, &request, &columns)
                .unwrap()
                .sql,
            "UPDATE \"events\" SET \"seen_at\" = CAST($1 AS timestamp without time zone), \"flags\" = CAST($2 AS bit(3)), \"tags\" = CAST($3 AS text[]), \"count\" = $4, \"odd\" = $5 WHERE \"id\" = $6"
        );

        let insert = InsertRequest::new(
            TableRef::new("events"),
            vec!["seen_at".into(), "count".into()],
            vec![
                CellValue::Text("2025-01-02 03:04:05".into()),
                CellValue::Integer(1),
            ],
        );
        assert_eq!(
            build_insert_with_columns(DatabaseKind::PostgreSQL, &insert, &columns)
                .unwrap()
                .sql,
            "INSERT INTO \"events\" (\"seen_at\", \"count\") VALUES (CAST($1 AS timestamp without time zone), $2)"
        );
    }

    #[test]
    fn mysql_bit_text_is_written_as_a_number() {
        let columns = vec![
            typed_column("mask", "bit(8)"),
            typed_column("name", "varchar(20)"),
        ];
        let request = UpdateRequest::for_primary_key(
            TableRef::new("flags"),
            vec![
                ("mask".into(), CellValue::Text("15".into())),
                ("name".into(), CellValue::Text("x".into())),
            ],
            vec![("id".into(), CellValue::Integer(1))],
        );
        assert_eq!(
            build_update_with_columns(DatabaseKind::MySQL, &request, &columns)
                .unwrap()
                .sql,
            "UPDATE `flags` SET `mask` = CAST(? AS UNSIGNED), `name` = ? WHERE `id` = ?"
        );
    }

    #[test]
    fn postgres_enum_updates_cast_text_parameters_to_the_enum_type() {
        let request = UpdateRequest::for_primary_key(
            TableRef::in_schema("public", "orders"),
            vec![("status".into(), CellValue::Text("paid".into()))],
            vec![("id".into(), CellValue::Integer(7))],
        );
        let columns = vec![ColumnInfo {
            name: "status".into(),
            data_type: "public.order_status".into(),
            enum_values: vec!["pending".into(), "paid".into()],
            nullable: false,
            ordinal: 1,
            primary_key: false,
            default_value: None,
        }];

        let statement =
            build_update_with_columns(DatabaseKind::PostgreSQL, &request, &columns).unwrap();

        assert_eq!(
            statement.sql,
            "UPDATE \"public\".\"orders\" SET \"status\" = CAST($1 AS public.order_status) WHERE \"id\" = $2"
        );
        assert_eq!(
            statement.params,
            request
                .assignments
                .iter()
                .filter_map(|(_, value)| match value {
                    MutationValue::Parameter(value) => Some(value.clone()),
                    MutationValue::Expression(_) => None,
                })
                .chain(
                    request
                        .filters
                        .iter()
                        .map(|filter| filter.value.clone().unwrap())
                )
                .collect::<Vec<_>>()
        );

        let expression_request = UpdateRequest::for_primary_key_with_mutation_values(
            TableRef::in_schema("public", "orders"),
            vec![("status".into(), MutationValue::expression("next_status()"))],
            vec![("id".into(), CellValue::Integer(7))],
        );
        let statement =
            build_update_with_columns(DatabaseKind::PostgreSQL, &expression_request, &columns)
                .unwrap();
        assert_eq!(
            statement.sql,
            "UPDATE \"public\".\"orders\" SET \"status\" = next_status() WHERE \"id\" = $1"
        );
        assert_eq!(statement.params, vec![CellValue::Integer(7)]);
    }

    #[test]
    fn insert_builder_supports_rows_that_use_only_database_defaults() {
        let request = InsertRequest::from_row(TableRef::new("defaults"), Vec::new());
        assert_eq!(
            build_insert(DatabaseKind::PostgreSQL, &request)
                .unwrap()
                .sql,
            "INSERT INTO \"defaults\" DEFAULT VALUES"
        );
        assert_eq!(
            build_insert(DatabaseKind::SQLite, &request).unwrap().sql,
            "INSERT INTO \"defaults\" DEFAULT VALUES"
        );
        assert_eq!(
            build_insert(DatabaseKind::MySQL, &request).unwrap().sql,
            "INSERT INTO `defaults` () VALUES ()"
        );
    }

    #[test]
    fn filter_builder_handles_null_and_like_operators() {
        let statement = build_select(
            DatabaseKind::SQLite,
            &TableRef::new("items"),
            &[],
            &[
                Filter::new(
                    "name",
                    FilterOperator::Contains,
                    Some(CellValue::Text("50%_!".into())),
                ),
                Filter::new("deleted_at", FilterOperator::IsNull, None),
            ],
            &[Order {
                column: "name".into(),
                direction: OrderDirection::Ascending,
            }],
            Some(Page {
                limit: 25,
                offset: 50,
            }),
        )
        .unwrap();
        assert_eq!(
            statement.sql,
            "SELECT * FROM \"items\" WHERE \"name\" LIKE ? ESCAPE '!' AND \"deleted_at\" IS NULL ORDER BY \"name\" ASC LIMIT ? OFFSET ?"
        );
        assert_eq!(
            statement.params,
            vec![
                CellValue::Text("%50!%!_!!%".into()),
                CellValue::Unsigned(25),
                CellValue::Unsigned(50)
            ]
        );
    }

    #[test]
    fn mutation_builders_require_a_guard_filter() {
        let error = build_delete(DatabaseKind::SQLite, &TableRef::new("items"), &[]).unwrap_err();
        assert!(error.to_string().contains("requires at least one filter"));
        let error = build_update(
            DatabaseKind::SQLite,
            &UpdateRequest {
                table: TableRef::new("items"),
                assignments: vec![("name".into(), CellValue::Text("new".into()).into())],
                filters: Vec::new(),
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("primary-key equality predicates")
        );
        let error = build_update(
            DatabaseKind::SQLite,
            &UpdateRequest {
                table: TableRef::new("items"),
                assignments: vec![("name".into(), CellValue::Text("new".into()).into())],
                filters: vec![Filter::new(
                    "name",
                    FilterOperator::Contains,
                    Some(CellValue::Text("old".into())),
                )],
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("primary-key equality predicates")
        );
    }

    #[test]
    fn postgres_filters_cast_text_parameters_to_non_text_column_types() {
        let columns = vec![
            ColumnInfo {
                name: "id".into(),
                data_type: "uuid".into(),
                enum_values: Vec::new(),
                nullable: false,
                ordinal: 1,
                primary_key: true,
                default_value: None,
            },
            ColumnInfo {
                name: "name".into(),
                data_type: "character varying(8)".into(),
                enum_values: Vec::new(),
                nullable: true,
                ordinal: 2,
                primary_key: false,
                default_value: None,
            },
            ColumnInfo {
                name: "seen_at".into(),
                data_type: "timestamp with time zone".into(),
                enum_values: Vec::new(),
                nullable: true,
                ordinal: 3,
                primary_key: false,
                default_value: None,
            },
        ];
        let text = |value: &str| Some(CellValue::Text(value.into()));
        let statement = build_select_with_columns(
            DatabaseKind::PostgreSQL,
            &TableRef::new("users"),
            &[],
            &[
                Filter::new("id", FilterOperator::Equals, text("7f1d")),
                Filter::new("name", FilterOperator::NotEquals, text("longer than eight")),
                Filter::new("seen_at", FilterOperator::GreaterThan, text("2026-01-01")),
                Filter::new("id", FilterOperator::StartsWith, text("7f")),
                Filter::new("name", FilterOperator::Contains, text("a")),
            ],
            &[],
            None,
            &columns,
        )
        .unwrap();
        assert_eq!(
            statement.sql,
            "SELECT * FROM \"users\" WHERE \"id\" = CAST($1 AS uuid) AND \"name\" <> $2 AND \"seen_at\" > CAST($3 AS timestamp with time zone) AND CAST(\"id\" AS text) LIKE $4 ESCAPE '!' AND \"name\" LIKE $5 ESCAPE '!'"
        );

        let delete = build_delete_with_columns(
            DatabaseKind::PostgreSQL,
            &TableRef::new("users"),
            &[Filter::new("id", FilterOperator::Equals, text("7f1d"))],
            &columns,
        )
        .unwrap();
        assert_eq!(
            delete.sql,
            "DELETE FROM \"users\" WHERE \"id\" = CAST($1 AS uuid)"
        );

        let update = build_update_with_columns(
            DatabaseKind::PostgreSQL,
            &UpdateRequest {
                table: TableRef::new("users"),
                assignments: vec![("name".into(), CellValue::Text("new".into()).into())],
                filters: vec![Filter::new("id", FilterOperator::Equals, text("7f1d"))],
            },
            &columns,
        )
        .unwrap();
        assert_eq!(
            update.sql,
            "UPDATE \"users\" SET \"name\" = CAST($1 AS character varying(8)) WHERE \"id\" = CAST($2 AS uuid)"
        );
    }

    #[test]
    fn equality_filters_parameterize_null_as_is_null() {
        let statement = build_select(
            DatabaseKind::SQLite,
            &TableRef::new("items"),
            &[],
            &[Filter::new(
                "deleted_at",
                FilterOperator::Equals,
                Some(CellValue::Null),
            )],
            &[],
            None,
        )
        .unwrap();
        assert_eq!(
            statement.sql,
            "SELECT * FROM \"items\" WHERE \"deleted_at\" IS NULL"
        );
        assert!(statement.params.is_empty());
    }

    #[test]
    fn table_action_builders_quote_schema_and_name_per_dialect() {
        let table = TableRef::in_schema("tenant", "audit\"events");

        let postgres_truncate = build_truncate_table(DatabaseKind::PostgreSQL, &table).unwrap();
        assert_eq!(
            postgres_truncate.sql,
            "TRUNCATE TABLE \"tenant\".\"audit\"\"events\""
        );
        assert!(postgres_truncate.params.is_empty());
        assert_eq!(
            build_drop_table(DatabaseKind::PostgreSQL, &table)
                .unwrap()
                .sql,
            "DROP TABLE \"tenant\".\"audit\"\"events\""
        );

        assert_eq!(
            build_truncate_table(DatabaseKind::MySQL, &table)
                .unwrap()
                .sql,
            "TRUNCATE TABLE `tenant`.`audit\"events`"
        );
        assert_eq!(
            build_drop_table(DatabaseKind::MySQL, &table).unwrap().sql,
            "DROP TABLE `tenant`.`audit\"events`"
        );

        assert_eq!(
            build_truncate_table(DatabaseKind::SQLite, &table)
                .unwrap()
                .sql,
            "DELETE FROM \"tenant\".\"audit\"\"events\""
        );
        assert_eq!(
            build_drop_table(DatabaseKind::SQLite, &table).unwrap().sql,
            "DROP TABLE \"tenant\".\"audit\"\"events\""
        );
    }

    #[test]
    fn table_action_builders_reject_redis() {
        let table = TableRef::new("keys");
        let truncate = build_truncate_table(DatabaseKind::Redis, &table).unwrap_err();
        assert!(matches!(
            truncate,
            DbxError::Unsupported {
                operation,
                kind: DatabaseKind::Redis
            } if operation == "truncate_table"
        ));
        let drop = build_drop_table(DatabaseKind::Redis, &table).unwrap_err();
        assert!(matches!(
            drop,
            DbxError::Unsupported {
                operation,
                kind: DatabaseKind::Redis
            } if operation == "drop_table"
        ));
    }

    #[test]
    fn create_table_builder_rejects_statement_injection() {
        let error = build_create_table(
            DatabaseKind::SQLite,
            &CreateTableRequest {
                table: TableRef::new("items"),
                columns: vec![CreateColumn {
                    name: "name".into(),
                    data_type: "TEXT; DROP TABLE items".into(),
                    nullable: false,
                    primary_key: false,
                    default_expression: None,
                }],
                if_not_exists: true,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid column type"));
    }

    #[test]
    fn bulk_insert_builder_numbers_placeholders_across_rows() {
        let rows = vec![
            vec![CellValue::Integer(1), CellValue::Text("a".into())],
            vec![CellValue::Null, CellValue::Text("b".into())],
        ];
        let statement = build_multi_row_insert(
            DatabaseKind::PostgreSQL,
            &TableRef::new("items"),
            &["id".into(), "name".into()],
            &rows,
        )
        .unwrap();
        assert_eq!(
            statement.sql,
            "INSERT INTO \"items\" (\"id\", \"name\") VALUES ($1, $2), ($3, $4)"
        );
        assert_eq!(statement.params.len(), 4);
        let error = build_multi_row_insert(
            DatabaseKind::SQLite,
            &TableRef::new("items"),
            &["id".into()],
            &[vec![CellValue::Integer(1), CellValue::Null]],
        )
        .unwrap_err();
        assert!(error.to_string().contains("match the column count"));
    }

    #[test]
    fn models_round_trip_through_json() {
        let value = CellValue::Json(serde_json::json!({ "ok": true }));
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: CellValue = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, value);
    }

    #[tokio::test]
    async fn sqlite_engine_supports_schema_queries_and_edits_without_a_server() {
        let config =
            ConnectionConfig::new(DatabaseKind::SQLite, "sqlite::memory:").with_max_connections(1);
        let engine = DatabaseEngine::connect(config).await.unwrap();
        engine
            .execute_sql(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, active INTEGER)",
            )
            .await
            .unwrap();
        let empty = engine
            .query_table(
                &TableRef::new("people"),
                &[],
                &[],
                &[],
                None,
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(empty.rows.len(), 0);
        assert_eq!(empty.columns.len(), 3);
        engine
            .insert(&InsertRequest::from_row(
                TableRef::new("people"),
                vec![
                    ("id".into(), CellValue::Integer(1)),
                    ("name".into(), CellValue::Text("Ada".into())),
                    ("active".into(), CellValue::Boolean(true)),
                ],
            ))
            .await
            .unwrap();
        let tables = engine.list_tables().await.unwrap();
        assert!(tables.iter().any(|table| table.name == "people"));
        let columns = engine
            .describe_table(&TableRef::new("people"))
            .await
            .unwrap();
        assert_eq!(columns.len(), 3);
        assert!(
            columns
                .iter()
                .any(|column| column.name == "id" && column.primary_key)
        );
        let result = engine
            .query_table(
                &TableRef::new("people"),
                &[],
                &[Filter::new(
                    "name",
                    FilterOperator::Equals,
                    Some(CellValue::Text("Ada".into())),
                )],
                &[],
                None,
                QueryOptions { max_rows: Some(10) },
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].values[1], CellValue::Text("Ada".into()));

        let updated = engine
            .update(&UpdateRequest::for_primary_key(
                TableRef::new("people"),
                vec![
                    ("name".into(), CellValue::Text("Ada Lovelace".into())),
                    ("active".into(), CellValue::Null),
                ],
                vec![("id".into(), CellValue::Integer(1))],
            ))
            .await
            .unwrap();
        assert_eq!(updated.rows_affected, 1);
        let updated_row = engine
            .query_table(
                &TableRef::new("people"),
                &[],
                &[Filter::new(
                    "id",
                    FilterOperator::Equals,
                    Some(CellValue::Integer(1)),
                )],
                &[],
                None,
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            updated_row.rows[0].values,
            vec![
                CellValue::Integer(1),
                CellValue::Text("Ada Lovelace".into()),
                CellValue::Null,
            ]
        );

        let error = engine
            .update(&UpdateRequest {
                table: TableRef::new("people"),
                assignments: vec![("name".into(), CellValue::Text("unsafe".into()).into())],
                filters: vec![Filter::new(
                    "name",
                    FilterOperator::Equals,
                    Some(CellValue::Text("Ada Lovelace".into())),
                )],
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("every primary-key column"));

        let truncated = engine
            .truncate_table(&TableRef::new("people"))
            .await
            .unwrap();
        assert_eq!(truncated.rows_affected, 1);
        assert!(
            engine
                .query_table(
                    &TableRef::new("people"),
                    &[],
                    &[],
                    &[],
                    None,
                    QueryOptions::default(),
                )
                .await
                .unwrap()
                .rows
                .is_empty()
        );

        engine.drop_table(&TableRef::new("people")).await.unwrap();
        assert!(
            !engine
                .list_tables()
                .await
                .unwrap()
                .iter()
                .any(|table| table.name == "people")
        );
    }

    #[tokio::test]
    async fn raw_sql_query_reports_outcomes_metadata_and_truncation() {
        let engine = DatabaseEngine::connect(
            ConnectionConfig::new(DatabaseKind::SQLite, "sqlite::memory:").with_max_connections(1),
        )
        .await
        .unwrap();

        let ddl = engine
            .query(
                "CREATE TABLE query_outcomes (id INTEGER PRIMARY KEY, name TEXT)",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(ddl.rows_affected, Some(0));
        assert!(!ddl.truncated);

        let dml = engine
            .query(
                "INSERT INTO query_outcomes (name) VALUES ('Ada')",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(dml.rows_affected, Some(1));

        let computed = engine
            .query("SELECT sqlite_version()", QueryOptions::default())
            .await
            .unwrap();
        assert_eq!(computed.rows.len(), 1);
        assert_eq!(computed.columns.len(), 1);
        assert_eq!(computed.columns[0].data_type, "TEXT");
        assert_eq!(computed.rows_affected, None);

        let empty = engine
            .query(
                "SELECT id, name FROM query_outcomes WHERE 1 = 0",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(empty.rows_affected, None);
        assert_eq!(
            empty
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["id", "name"]
        );

        engine
            .query(
                "INSERT INTO query_outcomes (name) VALUES ('Grace'), ('Linus')",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        let bounded = engine
            .query(
                "SELECT id FROM query_outcomes ORDER BY id",
                QueryOptions { max_rows: Some(2) },
            )
            .await
            .unwrap();
        assert_eq!(bounded.rows.len(), 2);
        assert_eq!(bounded.rows_affected, None);
        assert!(bounded.truncated);
    }

    #[tokio::test]
    async fn raw_sql_multi_statement_success_and_failure_have_documented_sqlite_semantics() {
        let engine = DatabaseEngine::connect(
            ConnectionConfig::new(DatabaseKind::SQLite, "sqlite::memory:").with_max_connections(1),
        )
        .await
        .unwrap();

        let success = engine
            .query(
                "CREATE TABLE script_outcomes (value TEXT); \
                 INSERT INTO script_outcomes VALUES ('first'); \
                 INSERT INTO script_outcomes VALUES ('second')",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(success.rows_affected, Some(2));

        // SQLite raw scripts execute each statement in autocommit mode unless
        // callers provide explicit BEGIN/COMMIT. A later failure therefore
        // leaves earlier successful statements durable.
        let error = engine
            .query(
                "INSERT INTO script_outcomes VALUES ('durable'); \
                 INSERT INTO missing_table VALUES ('fails')",
                QueryOptions::default(),
            )
            .await;
        assert!(error.is_err());
        let rows = engine
            .query(
                "SELECT value FROM script_outcomes WHERE value = 'durable'",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(rows.rows.len(), 1);
    }

    #[tokio::test]
    async fn raw_sql_mixed_select_and_dml_preserves_both_outcomes() {
        let engine = DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .query(
                "CREATE TABLE mixed_outcomes (value TEXT)",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        engine
            .query(
                "INSERT INTO mixed_outcomes VALUES ('seed')",
                QueryOptions::default(),
            )
            .await
            .unwrap();

        let result = engine
            .query(
                "SELECT 'before'; INSERT INTO mixed_outcomes VALUES ('after')",
                QueryOptions::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows_affected, Some(1));

        let empty_result = engine
            .query(
                "select value from mixed_outcomes where 0; insert into mixed_outcomes values ('after empty')",
                QueryOptions::default(),
            )
            .await
            .unwrap();

        assert!(empty_result.rows.is_empty());
        assert_eq!(empty_result.columns[0].name, "value");
        assert_eq!(empty_result.rows_affected, Some(1));

        let cte_insert = engine
            .query(
                "WITH seed(value) AS (VALUES ('from cte')) INSERT INTO mixed_outcomes SELECT value FROM seed",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert!(cte_insert.rows.is_empty());
        assert_eq!(cte_insert.rows_affected, Some(1));
    }

    #[tokio::test]
    async fn sqlite_browse_preserves_mixed_numeric_storage_classes() {
        let engine = DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql("CREATE TABLE budgets (id INTEGER PRIMARY KEY, amount NUMERIC)")
            .await
            .unwrap();
        engine
            .execute_sql(
                "INSERT INTO budgets VALUES (1, 125.5), (2, 251), (3, NULL), (4, 'pending')",
            )
            .await
            .unwrap();
        let result = engine
            .query_table(
                &TableRef::new("budgets"),
                &[],
                &[],
                &[],
                None,
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            result
                .rows
                .iter()
                .map(|row| row.values[1].clone())
                .collect::<Vec<_>>(),
            vec![
                CellValue::Real(125.5),
                CellValue::Integer(251),
                CellValue::Null,
                CellValue::Text("pending".into())
            ],
        );
        assert_ne!(result.columns[1].data_type, "NULL");
    }

    #[tokio::test]
    async fn sqlite_reports_databases_and_rejects_switching() {
        let engine = DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        assert_eq!(engine.current_database().await.unwrap(), "main");
        let databases = engine.list_databases().await.unwrap();
        assert!(databases.iter().any(|name| name == "main"));
        let error = engine.use_database("other").await.unwrap_err();
        assert!(matches!(
            error,
            DbxError::Unsupported { operation, kind } if operation == "use_database" && kind == DatabaseKind::SQLite
        ));
        // A rejected switch must leave the connection usable.
        assert_eq!(engine.current_database().await.unwrap(), "main");
    }

    #[tokio::test]
    async fn sqlite_table_structure_includes_composite_foreign_keys() {
        let engine = DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql(
                "CREATE TABLE projects (id INTEGER, account_id INTEGER, UNIQUE (id, account_id))",
            )
            .await
            .unwrap();
        engine
            .execute_sql(
                "CREATE TABLE tasks (project_id INTEGER, account_id INTEGER, FOREIGN KEY (project_id, account_id) REFERENCES projects (id, account_id) ON UPDATE CASCADE ON DELETE SET NULL)",
            )
            .await
            .unwrap();

        let structure = engine
            .table_structure(&TableRef::new("tasks"))
            .await
            .unwrap();
        assert_eq!(structure.columns.len(), 2);
        assert_eq!(structure.foreign_keys.len(), 1);
        assert_eq!(
            structure.foreign_keys[0],
            ForeignKeyInfo {
                constraint_name: None,
                columns: vec!["project_id".into(), "account_id".into()],
                referenced_schema: None,
                referenced_table: "projects".into(),
                referenced_columns: vec!["id".into(), "account_id".into()],
                on_update: Some(ReferentialAction::Cascade),
                on_delete: Some(ReferentialAction::SetNull),
            }
        );
    }

    #[tokio::test]
    async fn sqlite_relational_schema_is_ordered_and_includes_foreign_keys() {
        let engine = DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql("CREATE TABLE zebra (id INTEGER PRIMARY KEY)")
            .await
            .unwrap();
        engine
            .execute_sql(
                "CREATE TABLE alpha (id INTEGER PRIMARY KEY, zebra_id INTEGER REFERENCES zebra(id))",
            )
            .await
            .unwrap();

        let schema = engine.relational_schema().await.unwrap();
        assert_eq!(schema.database, "main");
        assert_eq!(
            schema
                .tables
                .iter()
                .map(|table| table.table.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zebra"]
        );
        assert_eq!(schema.tables[0].structure.foreign_keys.len(), 1);
        assert_eq!(
            schema.tables[0].structure.foreign_keys[0].referenced_table,
            "zebra"
        );
    }
}
