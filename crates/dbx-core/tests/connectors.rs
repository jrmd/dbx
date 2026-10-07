use dbx_core::*;

#[tokio::test]
#[ignore = "requires disposable ClickHouse on port 58123"]
async fn clickhouse_live_sql_metadata_filters_limits_and_database_switching() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::ClickHouse,
        std::env::var("DBX_TEST_CLICKHOUSE_URL").unwrap_or_else(|_| {
            "clickhouse://dbx_test:dbx_test_password@127.0.0.1:58123/dbx_test".into()
        }),
    ))
    .await
    .unwrap();
    assert_eq!(engine.current_database().await.unwrap(), "dbx_test");
    engine
        .execute_sql("DROP TABLE IF EXISTS dbx_qa_items")
        .await
        .unwrap();
    engine.execute_sql("CREATE TABLE dbx_qa_items (id UInt64, name String, optional Nullable(String), amount Decimal(20, 4), nested Array(UInt8)) ENGINE = MergeTree ORDER BY id").await.unwrap();
    engine.execute_sql("INSERT INTO dbx_qa_items VALUES (18446744073709551615, 'O\'\'Reilly %_\\\\ end', NULL, 1234567890123456.1234, [1, 2]), (2, 'other', 'present', 2, [])").await.unwrap();
    let table = TableRef::new("dbx_qa_items");
    assert!(
        engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == table.name)
    );
    let columns = engine.describe_table(&table).await.unwrap();
    assert_eq!(columns.len(), 5);
    assert!(columns[2].nullable);
    assert!(!columns.iter().any(|c| c.primary_key));
    let result = engine
        .query_table(
            &table,
            &[],
            &[Filter::new(
                "id",
                FilterOperator::Equals,
                Some(CellValue::Unsigned(u64::MAX)),
            )],
            &[],
            None,
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].values[0], CellValue::Unsigned(u64::MAX));
    assert_eq!(result.rows[0].values[2], CellValue::Null);
    assert_eq!(
        result.rows[0].values[3],
        CellValue::Text("1234567890123456.1234".into())
    );
    assert_eq!(
        result.rows[0].values[4],
        CellValue::Json(serde_json::json!([1, 2]))
    );
    let name = result.rows[0].values[1].clone();
    assert_eq!(
        engine
            .query_table(
                &table,
                &[],
                &[Filter::new("name", FilterOperator::Equals, Some(name))],
                &[],
                None,
                QueryOptions::default()
            )
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
    assert_eq!(
        engine
            .query_table(
                &table,
                &[],
                &[Filter::new(
                    "name",
                    FilterOperator::Contains,
                    Some(CellValue::Text("%_\\".into()))
                )],
                &[],
                None,
                QueryOptions::default()
            )
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
    let result = engine
        .query(
            "SELECT number FROM numbers(100)",
            QueryOptions { max_rows: Some(2) },
        )
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 2);
    assert!(result.truncated);
    let result = engine
        .query(
            "SELECT id, name FROM dbx_qa_items WHERE 0",
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.columns.len(), 2);
    assert!(result.rows.is_empty());
    let result = engine
        .query(
            "SELECT 1 AS duplicate, 1 AS duplicate",
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.columns.len(), 2);
    assert_eq!(result.rows[0].values.len(), 2);
    assert!(
        engine
            .query("SELECT missing_column", QueryOptions::default())
            .await
            .is_err()
    );
    let directory = tempfile::tempdir().unwrap();
    let csv = directory.path().join("items.csv");
    assert_eq!(
        export_table(&engine, &table, &csv)
            .await
            .unwrap()
            .rows_exported,
        2
    );
    assert!(
        std::fs::read_to_string(&csv)
            .unwrap()
            .contains("18446744073709551615")
    );
    assert!(
        export_table(&engine, &table, &directory.path().join("items.sql"))
            .await
            .is_err()
    );
    assert!(import_file(&engine, Some(&table), &csv).await.is_err());
    assert!(
        engine
            .insert(&InsertRequest::from_row(
                table.clone(),
                vec![("id".into(), CellValue::Integer(3))]
            ))
            .await
            .is_err()
    );
    assert!(
        engine
            .delete(
                &table,
                &[Filter::new(
                    "id",
                    FilterOperator::Equals,
                    Some(CellValue::Integer(2))
                )]
            )
            .await
            .is_err()
    );
    assert!(
        engine
            .list_databases()
            .await
            .unwrap()
            .iter()
            .any(|d| d == "default")
    );
    engine.use_database("default").await.unwrap();
    assert_eq!(
        engine
            .query("SELECT currentDatabase()", QueryOptions::default())
            .await
            .unwrap()
            .rows[0]
            .values[0],
        CellValue::Text("default".into())
    );
    assert!(engine.use_database("dbx_missing_database").await.is_err());
    assert_eq!(engine.current_database().await.unwrap(), "default");
    engine.use_database("dbx_test").await.unwrap();
    engine.truncate_table(&table).await.unwrap();
    engine.drop_table(&table).await.unwrap();
}

#[test]
fn provider_urls_and_sql_dialects_are_explicit() {
    for kind in DatabaseKind::ALL {
        let config = ConnectionConfig::new(kind, kind.default_url());
        assert!(config.validate().is_ok(), "{kind}");
        assert_eq!(
            serde_json::from_str::<DatabaseKind>(&serde_json::to_string(&kind).unwrap()).unwrap(),
            kind
        );
    }
    assert!(
        ConnectionConfig::new(
            DatabaseKind::MongoDB,
            "mongodb+srv://user:secret@example.test/app"
        )
        .validate()
        .is_ok()
    );
    assert!(
        ConnectionConfig::new(DatabaseKind::Turso, "https://:secret@example.test")
            .validate()
            .is_ok()
    );
    for kind in [
        DatabaseKind::CloudflareD1,
        DatabaseKind::Turso,
        DatabaseKind::BigQuery,
    ] {
        let config = ConnectionConfig::new(
            kind,
            format!("{}://:vault-secret@example.test/database", kind.scheme()),
        );
        assert!(!format!("{config:?}").contains("vault-secret"));
        let config = ConnectionConfig::new(
            kind,
            format!("{}://example.test/database?token=secret", kind.scheme()),
        );
        assert!(config.validate().is_err());
    }
    let table = TableRef::new("items");
    let statement = build_select(
        DatabaseKind::CockroachDB,
        &table,
        &[],
        &[Filter::new(
            "name",
            FilterOperator::Equals,
            Some(CellValue::Text("secret".into())),
        )],
        &[],
        Some(Page {
            limit: 10,
            offset: 0,
        }),
    )
    .unwrap();
    assert!(statement.sql.contains("$1"));
    assert!(!statement.sql.contains("secret"));
    for kind in [DatabaseKind::Turso, DatabaseKind::CloudflareD1] {
        assert_eq!(
            build_truncate_table(kind, &table).unwrap().sql,
            "DELETE FROM \"items\""
        );
    }
    assert_eq!(
        quote_identifier(DatabaseKind::BigQuery, "project.dataset.items").unwrap(),
        "`project.dataset.items`"
    );
}

#[tokio::test]
async fn duckdb_browses_filters_and_mutates_without_sqlite_substitution() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::DuckDB,
        "duckdb::memory:",
    ))
    .await
    .unwrap();
    engine
        .execute_sql("CREATE TABLE items (id BIGINT PRIMARY KEY, name VARCHAR, data BLOB)")
        .await
        .unwrap();
    engine
        .insert(&InsertRequest {
            table: TableRef::new("items"),
            columns: vec!["id".into(), "name".into(), "data".into()],
            values: vec![
                CellValue::Integer(1).into(),
                CellValue::Text("O'Reilly".into()).into(),
                CellValue::Bytes(vec![0, 255]).into(),
            ],
        })
        .await
        .unwrap();
    let columns = engine
        .describe_table(&TableRef::new("items"))
        .await
        .unwrap();
    assert!(columns[0].primary_key);
    let result = engine
        .query_table(
            &TableRef::new("items"),
            &[],
            &[Filter::new(
                "id",
                FilterOperator::Equals,
                Some(CellValue::Integer(1)),
            )],
            &[],
            Some(Page {
                limit: 10,
                offset: 0,
            }),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0].values[1], CellValue::Text("O'Reilly".into()));
    assert_eq!(result.rows[0].values[2], CellValue::Bytes(vec![0, 255]));
    let result = engine
        .query("SELECT * FROM items WHERE false", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(result.columns.len(), 3);
    assert!(result.rows.is_empty());
    let result = engine
        .query("SELECT * FROM range(5)", QueryOptions { max_rows: Some(2) })
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 2);
    assert!(result.truncated);
    assert!(
        engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == "items")
    );
    engine
        .execute_sql(
            "CREATE TABLE children (id BIGINT PRIMARY KEY, item_id BIGINT REFERENCES items(id))",
        )
        .await
        .unwrap();
    let structure = engine
        .table_structure(&TableRef::new("children"))
        .await
        .unwrap();
    assert_eq!(structure.foreign_keys[0].columns, vec!["item_id"]);
    assert_eq!(structure.foreign_keys[0].referenced_table, "items");
    assert!(
        engine
            .execute_sql("DELETE FROM items; DROP TABLE children")
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .query("SELECT count(*) FROM items", QueryOptions::default())
            .await
            .unwrap()
            .rows[0]
            .values[0],
        CellValue::Integer(1)
    );
}

#[tokio::test]
#[ignore = "requires disposable MongoDB on port 57017"]
async fn mongo_live_commands_and_collection_browsing() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::MongoDB,
        std::env::var("DBX_TEST_MONGO_URL")
            .unwrap_or_else(|_| "mongodb://localhost:57017/dbx_qa".into()),
    ))
    .await
    .unwrap();
    engine.query(r#"{"insert":"items","documents":[{"name":"hello","nested":{"ok":true}},{"name":"world"}]}"#,QueryOptions::default()).await.unwrap();
    assert!(
        engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == "items")
    );
    let result = engine
        .query_table(
            &TableRef::new("items"),
            &[],
            &[],
            &[],
            Some(Page {
                limit: 1,
                offset: 1,
            }),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert!(result.columns.iter().any(|c| c.name == "_id"));
    let result = engine
        .query("{\"find\":\"items\"}", QueryOptions { max_rows: Some(1) })
        .await
        .unwrap();
    assert!(result.truncated);
}

#[tokio::test]
#[ignore = "requires disposable CockroachDB on port 56257"]
async fn cockroach_live_postgres_driver_and_metadata() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::CockroachDB,
        std::env::var("DBX_TEST_COCKROACH_URL")
            .unwrap_or_else(|_| "postgres://root@localhost:56257/defaultdb?sslmode=disable".into()),
    ))
    .await
    .unwrap();
    engine
        .execute_sql("CREATE TABLE IF NOT EXISTS dbx_qa_items (id INT PRIMARY KEY, name STRING)")
        .await
        .unwrap();
    engine
        .execute_sql("UPSERT INTO dbx_qa_items VALUES (1, 'hello')")
        .await
        .unwrap();
    assert!(
        engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == "dbx_qa_items")
    );
    let columns = engine
        .describe_table(&TableRef {
            schema: Some("public".into()),
            name: "dbx_qa_items".into(),
        })
        .await
        .unwrap();
    assert!(columns[0].primary_key);
    assert!(!engine.relational_schema().await.unwrap().tables.is_empty());

    engine
        .execute_sql("DROP TABLE IF EXISTS dbx_qa_details")
        .await
        .unwrap();
    engine
        .execute_sql("CREATE TABLE dbx_qa_details (id INT PRIMARY KEY, code STRING NOT NULL DEFAULT 'new', qty INT CHECK (qty >= 0), INDEX dbx_qa_qty (qty))")
        .await
        .unwrap();
    let structure = engine
        .table_structure(&TableRef {
            schema: Some("public".into()),
            name: "dbx_qa_details".into(),
        })
        .await
        .unwrap();
    let code = structure
        .columns
        .iter()
        .find(|column| column.name == "code")
        .unwrap();
    assert!(
        code.default_value
            .as_deref()
            .unwrap_or_default()
            .contains("new")
    );
    assert!(
        structure
            .indexes
            .iter()
            .any(|index| index.name == "dbx_qa_qty" && index.columns == ["qty"] && !index.unique)
    );
    assert!(structure.indexes.iter().any(|index| index.primary));
    assert_eq!(structure.checks.len(), 1, "{:?}", structure.checks);
    assert_eq!(
        engine
            .query("SELECT name FROM dbx_qa_items", QueryOptions::default())
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
}

#[tokio::test]
#[ignore = "requires disposable Elasticsearch on port 59200"]
async fn elastic_live_requests_and_index_browsing() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::Elasticsearch,
        std::env::var("DBX_TEST_ELASTICSEARCH_URL")
            .unwrap_or_else(|_| "http://localhost:59200".into()),
    ))
    .await
    .unwrap();
    engine
        .query(
            "PUT /dbx-qa/_doc/1?refresh=true\n{\"name\":\"hello\"}",
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert!(
        engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == "dbx-qa")
    );
    let result = engine
        .query_table(
            &TableRef::new("dbx-qa"),
            &[],
            &[],
            &[],
            None,
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert!(result.columns.iter().any(|c| c.name == "name"));
}

#[tokio::test]
#[ignore = "requires disposable Kafka on port 59092 with dbx-qa topic"]
async fn kafka_live_acknowledged_produce_and_noncommitting_consume() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::Kafka,
        std::env::var("DBX_TEST_KAFKA_URL").unwrap_or_else(|_| "kafka://localhost:59092".into()),
    ))
    .await
    .unwrap();
    let result = engine
        .query(
            r#"{"action":"produce","topic":"dbx-qa","key":"hello","value":"world"}"#,
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows_affected, Some(1));
    assert!(
        engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|t| t.name == "dbx-qa")
    );
    let result = engine
        .query_table(
            &TableRef::new("dbx-qa"),
            &[],
            &[],
            &[],
            None,
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert!(!result.rows.is_empty());
}

#[tokio::test]
#[ignore = "requires disposable SQL Server on port 51433"]
async fn sqlserver_live_sql_metadata_browsing_and_checked_edits() {
    let engine = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::SqlServer,
        std::env::var("DBX_TEST_SQLSERVER_URL").unwrap_or_else(|_| "sqlserver://sa:Dbx_test_Passw0rd@127.0.0.1:51433/master?trust_server_certificate=true".into()),
    ))
    .await
    .unwrap();
    engine
        .execute_sql("IF DB_ID(N'dbx_test') IS NULL CREATE DATABASE dbx_test")
        .await
        .unwrap();
    engine.use_database("dbx_test").await.unwrap();
    assert_eq!(engine.current_database().await.unwrap(), "dbx_test");
    assert!(
        engine
            .list_databases()
            .await
            .unwrap()
            .contains(&"dbx_test".to_owned())
    );
    for statement in [
        "IF OBJECT_ID(N'dbo.dbx_items_view', N'V') IS NOT NULL DROP VIEW dbo.dbx_items_view",
        "IF OBJECT_ID(N'dbo.dbx_items', N'U') IS NOT NULL DROP TABLE dbo.dbx_items",
        "IF OBJECT_ID(N'dbo.dbx_owners', N'U') IS NOT NULL DROP TABLE dbo.dbx_owners",
        "CREATE TABLE dbo.dbx_owners (id INT PRIMARY KEY)",
        "CREATE TABLE dbo.dbx_items (id INT IDENTITY(1,1) PRIMARY KEY, code NVARCHAR(20) NOT NULL DEFAULT N'new', qty INT NOT NULL CONSTRAINT dbx_qty CHECK (qty >= 0), price DECIMAL(10,2) NULL, seen DATETIME2 NULL, owner_id INT NULL CONSTRAINT dbx_owner_fk REFERENCES dbo.dbx_owners (id) ON DELETE SET NULL)",
        "CREATE UNIQUE INDEX dbx_items_code ON dbo.dbx_items (code)",
        "CREATE VIEW dbo.dbx_items_view AS SELECT id, code FROM dbo.dbx_items",
        "INSERT INTO dbo.dbx_owners (id) VALUES (1)",
    ] {
        engine.execute_sql(statement).await.unwrap();
    }
    let table = TableRef {
        schema: Some("dbo".into()),
        name: "dbx_items".into(),
    };
    let inserted = engine
        .insert(&dbx_core::InsertRequest::from_row(
            table.clone(),
            vec![
                ("code".into(), CellValue::Text("a[1]".into())),
                ("qty".into(), CellValue::Integer(3)),
                ("price".into(), CellValue::Text("12.50".into())),
                ("owner_id".into(), CellValue::Integer(1)),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(inserted.rows_affected, 1);
    engine
        .execute_sql(
            "INSERT INTO dbo.dbx_items (code, qty, seen) VALUES (N'b', 5, '2026-01-02T03:04:05')",
        )
        .await
        .unwrap();
    assert_eq!(engine.count_rows(&table, &[], None).await.unwrap(), 2);
    assert_eq!(engine.estimate_rows(&table).await.unwrap(), Some(2));

    let tables = engine.list_tables().await.unwrap();
    assert!(
        tables
            .iter()
            .any(|t| t.name == "dbx_items" && t.schema.as_deref() == Some("dbo"))
    );
    assert!(
        tables
            .iter()
            .any(|t| t.name == "dbx_items_view" && t.kind == EntityKind::View)
    );

    let structure = engine.table_structure(&table).await.unwrap();
    let column = |name: &str| structure.columns.iter().find(|c| c.name == name).unwrap();
    assert!(column("id").primary_key);
    assert_eq!(column("code").data_type, "nvarchar(20)");
    assert_eq!(column("price").data_type, "decimal(10,2)");
    assert!(
        column("code")
            .default_value
            .as_deref()
            .unwrap()
            .contains("new")
    );
    assert!(!column("qty").nullable);
    assert_eq!(structure.foreign_keys.len(), 1);
    assert_eq!(structure.foreign_keys[0].referenced_table, "dbx_owners");
    assert_eq!(
        structure.foreign_keys[0].on_delete,
        Some(dbx_core::ReferentialAction::SetNull)
    );
    assert!(
        structure
            .indexes
            .iter()
            .any(|i| i.name == "dbx_items_code" && i.unique && i.columns == ["code"])
    );
    assert!(structure.indexes.iter().any(|i| i.primary));
    assert_eq!(structure.checks.len(), 1);
    assert!(structure.checks[0].expression.contains("qty"));
    let view = engine
        .table_structure(&TableRef {
            schema: Some("dbo".into()),
            name: "dbx_items_view".into(),
        })
        .await
        .unwrap();
    assert!(view.definition.unwrap().contains("SELECT"));

    // Filters escape LIKE character classes; pages need OFFSET/FETCH.
    let filtered = engine
        .query_table(
            &table,
            &[],
            &[dbx_core::Filter {
                column: "code".into(),
                operator: dbx_core::FilterOperator::Contains,
                value: Some(CellValue::Text("[1]".into())),
            }],
            &[dbx_core::Order {
                column: "id".into(),
                direction: dbx_core::OrderDirection::Descending,
            }],
            Some(dbx_core::Page {
                limit: 10,
                offset: 0,
            }),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(filtered.rows.len(), 1);
    let code_index = filtered
        .columns
        .iter()
        .position(|c| c.name == "code")
        .unwrap();
    let price_index = filtered
        .columns
        .iter()
        .position(|c| c.name == "price")
        .unwrap();
    assert_eq!(
        filtered.rows[0].values[code_index],
        CellValue::Text("a[1]".into())
    );
    assert_eq!(
        filtered.rows[0].values[price_index],
        CellValue::Text("12.50".into())
    );
    let second_page = engine
        .query_table(
            &table,
            &[],
            &[],
            &[dbx_core::Order {
                column: "id".into(),
                direction: dbx_core::OrderDirection::Ascending,
            }],
            Some(dbx_core::Page {
                limit: 1,
                offset: 1,
            }),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(second_page.rows.len(), 1);
    let seen_index = second_page
        .columns
        .iter()
        .position(|c| c.name == "seen")
        .unwrap();
    assert_eq!(
        second_page.rows[0].values[seen_index],
        CellValue::Text("2026-01-02 03:04:05".into())
    );

    for monitor in [Monitor::Sessions, Monitor::Locks] {
        let sql = monitor_query(DatabaseKind::SqlServer, monitor).unwrap();
        engine.query(&sql, QueryOptions::default()).await.unwrap();
    }

    // Checked edits detect concurrent changes.
    let request = dbx_core::UpdateRequest::for_primary_key(
        table.clone(),
        vec![("qty".into(), CellValue::Integer(9))],
        vec![("id".into(), CellValue::Integer(1))],
    );
    engine
        .update_checked(&request, &[("qty".into(), CellValue::Integer(3))])
        .await
        .unwrap();
    assert!(matches!(
        engine
            .update_checked(&request, &[("qty".into(), CellValue::Integer(3))])
            .await,
        Err(dbx_core::DbxError::Conflict)
    ));

    // T-SQL scripts: GO batches, #temp tables and module bodies.
    let session = dbx_core::QuerySession::new(std::sync::Arc::new(engine));
    let script = session
        .run(
            "SELECT 1 AS one INTO #scratch;\nSELECT one FROM #scratch\nGO\nCREATE OR ALTER PROCEDURE dbo.dbx_proc AS\nBEGIN\n  SET NOCOUNT ON;\n  SELECT 42 AS answer;\nEND\nGO\nEXEC dbo.dbx_proc",
            QueryOptions::default(),
            std::time::Duration::from_secs(30),
            dbx_core::QueryCancellation::default(),
        )
        .await
        .unwrap();
    for statement in &script.statements {
        assert!(
            statement.error.is_none(),
            "{}: {:?}",
            statement.statement,
            statement.error
        );
    }
    assert_eq!(
        script.statements[1].result.rows[0].values[0],
        CellValue::Integer(1)
    );
    assert_eq!(
        script.statements.last().unwrap().result.rows[0].values[0],
        CellValue::Integer(42)
    );
}
