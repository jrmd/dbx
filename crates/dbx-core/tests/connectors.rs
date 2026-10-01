use dbx_core::*;

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
        "mongodb://localhost:57017/dbx_qa",
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
        "postgres://root@localhost:56257/defaultdb?sslmode=disable",
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
        "http://localhost:59200",
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
        "kafka://localhost:59092",
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
