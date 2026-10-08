use dbx_core::*;
use std::{sync::Arc, time::Duration};

async fn database() -> (tempfile::TempDir, Arc<DatabaseEngine>) {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("db.sqlite").display()
    );
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::SQLite, url))
            .await
            .unwrap(),
    );
    (directory, engine)
}

#[tokio::test]
async fn full_query_export_streams_past_grid_limits_and_refuses_writes() {
    let (directory, engine) = database().await;
    let path = directory.path().join("all.jsonl");
    let sql = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<20001) SELECT x, NULL AS empty FROM n";
    let report = export_query(&engine, sql, &path, QueryExportFormat::JsonLines)
        .await
        .unwrap();
    assert_eq!(report, 20001);
    let output = std::fs::read_to_string(&path).unwrap();
    assert_eq!(output.lines().count(), 20002);
    assert!(output.contains("20001"));
    let previous = output.clone();
    assert!(
        export_query(
            &engine,
            "CREATE TABLE forbidden(id int)",
            &path,
            QueryExportFormat::Csv
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), previous);
    assert!(
        export_query(&engine, "SELECT 1; SELECT 2", &path, QueryExportFormat::Csv)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn preview_mapping_preserves_nulls_defaults_and_rolls_back_bad_rows() {
    use dbx_core::data_import::{ImportData, diff_data, import_data, snapshot_data};
    let (directory, engine) = database().await;
    engine
        .execute_sql(
            "CREATE TABLE target(id INTEGER PRIMARY KEY, value TEXT, flag INTEGER DEFAULT 7)",
        )
        .await
        .unwrap();
    let table = TableRef::new("target");
    let path = directory.path().join("rows.csv");
    std::fs::write(&path, "key,text\n1,\n2,\"\"\n").unwrap();
    let data = ImportData::read(&path).unwrap();
    assert_eq!(data.rows[0][1], CellValue::Null);
    assert_eq!(data.rows[1][1], CellValue::Text(String::new()));
    let columns = engine.describe_table(&table).await.unwrap();
    let database = engine.current_database().await.unwrap();
    let mapping = vec![Some("id".into()), Some("value".into())];
    std::fs::write(&path, "key,text\n99,changed after preview\n").unwrap();
    assert_eq!(
        import_data(&engine, &table, &data, &mapping, &database, &columns)
            .await
            .unwrap(),
        2
    );
    let snapshot = snapshot_data(&engine, &table).await.unwrap();
    assert_eq!(snapshot.rows.len(), 2);
    assert!(
        snapshot
            .rows
            .iter()
            .all(|row| row[2] == CellValue::Integer(7))
    );
    let bad = ImportData {
        headers: data.headers.clone(),
        rows: vec![
            vec![CellValue::Integer(3), CellValue::Text("temporary".into())],
            vec![CellValue::Integer(1), CellValue::Text("duplicate".into())],
        ],
    };
    let error = import_data(&engine, &table, &bad, &mapping, &database, &columns)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Row 2"));
    let after = snapshot_data(&engine, &table).await.unwrap();
    assert_eq!(after.rows.len(), 2);
    assert_eq!(
        diff_data(&snapshot, &after, &["id".into()]).unwrap().equal,
        2
    );
    engine
        .execute_sql("ALTER TABLE target ADD COLUMN extra TEXT")
        .await
        .unwrap();
    assert!(
        import_data(&engine, &table, &data, &mapping, &database, &columns)
            .await
            .unwrap_err()
            .to_string()
            .contains("schema changed")
    );
}

#[tokio::test]
async fn json_preview_and_append_preserve_decimal_large_integer_and_nested_digits() {
    use dbx_core::data_import::{ImportData, import_data};
    let (directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE exact_numbers(amount TEXT, huge TEXT, nested TEXT)")
        .await
        .unwrap();
    let table = TableRef::new("exact_numbers");
    let columns = engine.describe_table(&table).await.unwrap();
    let database = engine.current_database().await.unwrap();
    for extension in ["json", "jsonl"] {
        let path = directory.path().join(format!("numbers.{extension}"));
        let object = r#"{"amount":1234567890.1234567890123456789,"huge":184467440737095516160,"nested":{"value":0.1234567890123456789}}"#;
        std::fs::write(
            &path,
            if extension == "json" {
                format!("[{object}]")
            } else {
                object.into()
            },
        )
        .unwrap();
        let data = ImportData::read(&path).unwrap();
        let mapping = data.default_mapping(&columns);
        import_data(&engine, &table, &data, &mapping, &database, &columns)
            .await
            .unwrap();
    }
    let rows = engine
        .query("SELECT * FROM exact_numbers", QueryOptions::default())
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(
            row.values[0],
            CellValue::Text("1234567890.1234567890123456789".into())
        );
        assert_eq!(
            row.values[1],
            CellValue::Text("184467440737095516160".into())
        );
        assert_eq!(
            row.values[2].to_string(),
            r#"{"value":0.1234567890123456789}"#
        );
    }
}

#[tokio::test]
async fn mcp_pairing_authenticates_scopes_and_bounds_reads_then_revokes() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    engine.execute_sql("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<101) INSERT INTO items SELECT x, 'hello' FROM n").await.unwrap();
    let pairing = mcp::Pairing::start(engine.clone()).await.unwrap();
    let recipe = pairing.recipe(std::path::Path::new("dbx"));
    let token = recipe["mcpServers"]["dbx"]["env"]["DBX_MCP_TOKEN"]
        .as_str()
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let request = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}});
    assert_eq!(
        client
            .post(&pairing.url)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(&pairing.url)
            .bearer_auth(token)
            .header("Origin", "http://untrusted.test")
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let init: serde_json::Value = client
        .post(&pairing.url)
        .bearer_auth(token)
        .json(&request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    let table = engine
        .list_tables()
        .await
        .unwrap()
        .into_iter()
        .find(|table| table.name == "items")
        .unwrap();
    let mut arguments = serde_json::json!({"table":"items","limit":100});
    if let Some(schema) = table.schema {
        arguments["schema"] = schema.into();
    }
    let request = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"dbx_read_rows","arguments":arguments}});
    let read: serde_json::Value = client
        .post(&pairing.url)
        .bearer_auth(token)
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(read["result"]["isError"], false, "{read}");
    assert_eq!(
        read["result"]["structuredContent"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    assert_eq!(read["result"]["structuredContent"]["truncated"], true);
    let mut invalid = request.clone();
    invalid["params"]["arguments"]["sql"] = "DROP TABLE items".into();
    let denied: serde_json::Value = client
        .post(&pairing.url)
        .bearer_auth(token)
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&invalid)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(denied["error"]["code"], -32602);
    let discover = serde_json::json!({"jsonrpc":"2.0","id":3,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}});
    let discovery: serde_json::Value = client
        .post(&pairing.url)
        .bearer_auth(token)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "server/discover")
        .json(&discover)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        discovery["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "DBX"
    );
    assert!(!pairing.activity().join("\n").contains("hello"));
    let url = pairing.url.clone();
    drop(pairing);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let revoked = client.post(url).json(&request).send().await;
    assert!(revoked.is_err() || revoked.unwrap().status() == reqwest::StatusCode::GONE);
}

#[tokio::test]
async fn dotted_identifiers_preserve_literal_components_through_browse_and_edits() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql(
            "CREATE TABLE \"invoices.v2\" (\"id.key\" INTEGER PRIMARY KEY, \"amount.net\" TEXT)",
        )
        .await
        .unwrap();
    let table = TableRef::new("invoices.v2");
    engine
        .insert(&InsertRequest::from_row(
            table.clone(),
            vec![
                ("id.key".into(), CellValue::Integer(1)),
                ("amount.net".into(), CellValue::Text("original".into())),
            ],
        ))
        .await
        .unwrap();
    let rows = engine
        .query_table(
            &table,
            &["amount.net".into()],
            &[],
            &[],
            None,
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(rows.rows[0].values[0], CellValue::Text("original".into()));
    let request = UpdateRequest::for_primary_key(
        table.clone(),
        vec![("amount.net".into(), CellValue::Text("updated".into()))],
        vec![("id.key".into(), CellValue::Integer(1))],
    );
    engine
        .update_checked(
            &request,
            &[("amount.net".into(), CellValue::Text("original".into()))],
        )
        .await
        .unwrap();
    let rows = engine
        .query_table(
            &table,
            &["amount.net".into()],
            &[],
            &[],
            None,
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(rows.rows[0].values[0], CellValue::Text("updated".into()));
    engine
        .delete_checked(
            &table,
            &request.filters,
            &[("amount.net".into(), CellValue::Text("updated".into()))],
        )
        .await
        .unwrap();
    assert!(
        engine
            .query_table(&table, &[], &[], &[], None, QueryOptions::default())
            .await
            .unwrap()
            .rows
            .is_empty()
    );
}

#[test]
fn failed_export_preserves_the_previous_file_and_removes_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("report.csv");
    std::fs::write(&path, "previous complete report").unwrap();
    let failure = atomic_export(&path, |output| {
        output.write_all(b"partial new report").unwrap();
        Err(DbxError::Io("injected write failure".into()))
    });
    assert!(failure.is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "previous complete report"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    write_atomic_export(&path, b"new complete report").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "new complete report"
    );
}

#[tokio::test]
async fn changed_and_deleted_rows_are_conflicts_without_overwriting_data() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT); ")
        .await
        .unwrap();
    engine
        .execute_sql("INSERT INTO items VALUES (1, 'original')")
        .await
        .unwrap();
    let request = UpdateRequest::for_primary_key(
        TableRef::new("items"),
        vec![("value".into(), CellValue::Text("mine".into()))],
        vec![("id".into(), CellValue::Integer(1))],
    );
    engine
        .execute_sql("UPDATE items SET value='theirs' WHERE id=1")
        .await
        .unwrap();
    let original = [("value".into(), CellValue::Text("original".into()))];
    assert!(matches!(
        engine.update_checked(&request, &original).await,
        Err(DbxError::Conflict)
    ));
    assert!(matches!(
        engine
            .delete_checked(&request.table, &request.filters, &original)
            .await,
        Err(DbxError::Conflict)
    ));
    let rows = engine
        .query("SELECT value FROM items", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(rows.rows[0].values[0], CellValue::Text("theirs".into()));
    engine.execute_sql("DELETE FROM items").await.unwrap();
    assert!(matches!(
        engine.update_checked(&request, &original).await,
        Err(DbxError::Conflict)
    ));
}

/// A query tab whose connection the server closed reconnects on its next run.
async fn assert_query_tab_recovers_from_a_lost_connection(
    engine: &Arc<DatabaseEngine>,
    kind: DatabaseKind,
) {
    let session = QuerySession::new(engine.clone());
    let run = |sql: String| {
        let session = &session;
        async move {
            session
                .run(
                    &sql,
                    QueryOptions::default(),
                    Duration::from_secs(10),
                    QueryCancellation::default(),
                )
                .await
                .unwrap()
        }
    };
    let pid = run(if kind == DatabaseKind::PostgreSQL {
        "SELECT pg_backend_pid()".into()
    } else {
        "SELECT CONNECTION_ID()".into()
    })
    .await
    .statements[0]
        .result
        .rows[0]
        .values[0]
        .to_string();
    engine
        .execute_sql(&if kind == DatabaseKind::PostgreSQL {
            format!("SELECT pg_terminate_backend({pid})")
        } else {
            format!("KILL {pid}")
        })
        .await
        .unwrap();
    // Nothing ran on the dead connection, so the tab reconnects silently.
    let recovered = run("SELECT 1".into()).await;
    assert!(
        recovered.statements[0].error.is_none(),
        "{:?}",
        recovered.statements[0].error
    );
}

/// A staged batch commits together, and one conflict rolls back every update.
async fn assert_batch_updates_are_atomic(engine: &DatabaseEngine, table: &str, current: &str) {
    let update = |value: &str, original: &str| {
        (
            UpdateRequest::for_primary_key(
                TableRef::new(table),
                vec![("value".into(), CellValue::Text(value.into()))],
                vec![("id".into(), CellValue::Integer(1))],
            ),
            vec![("value".to_owned(), CellValue::Text(original.into()))],
        )
    };
    let value = || async {
        engine
            .query(
                &format!("SELECT value FROM {table} WHERE id = 1"),
                QueryOptions::default(),
            )
            .await
            .unwrap()
            .rows[0]
            .values[0]
            .clone()
    };
    assert_eq!(
        engine
            .update_checked_batch(&[update("batched", current)])
            .await
            .unwrap(),
        1
    );
    assert_eq!(value().await, CellValue::Text("batched".into()));
    assert!(matches!(
        engine
            .update_checked_batch(&[update("first", "batched"), update("second", "stale")])
            .await,
        Err(DbxError::Conflict)
    ));
    assert_eq!(value().await, CellValue::Text("batched".into()));
}

#[tokio::test]
async fn staged_row_updates_commit_or_roll_back_together() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    engine
        .execute_sql("INSERT INTO items VALUES (1, 'original')")
        .await
        .unwrap();
    assert_batch_updates_are_atomic(&engine, "items", "original").await;
}

#[tokio::test]
async fn mixed_changesets_commit_or_roll_back_together() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT); \
             INSERT INTO items VALUES (1, 'one'), (2, 'two');",
        )
        .await
        .unwrap();
    let table = TableRef::new("items");
    let rows = || async {
        engine
            .query(
                "SELECT id, value FROM items ORDER BY id",
                QueryOptions::default(),
            )
            .await
            .unwrap()
            .rows
            .into_iter()
            .map(|row| row.values)
            .collect::<Vec<_>>()
    };
    let delete = |id: i64, value: &str| RowChange::Delete {
        table: table.clone(),
        filters: vec![Filter::new(
            "id",
            FilterOperator::Equals,
            Some(CellValue::Integer(id)),
        )],
        originals: vec![("value".into(), CellValue::Text(value.into()))],
    };
    let update = |value: &str, original: &str| RowChange::Update {
        request: UpdateRequest::for_primary_key(
            table.clone(),
            vec![("value".into(), CellValue::Text(value.into()))],
            vec![("id".into(), CellValue::Integer(1))],
        ),
        originals: vec![("value".into(), CellValue::Text(original.into()))],
    };
    let insert = RowChange::Insert(InsertRequest::from_row(
        table.clone(),
        vec![("value".into(), CellValue::Text("three".into()))],
    ));
    let columns = engine.describe_table(&table).await.unwrap();

    // A stale delete rejects the whole set, including the valid update and insert.
    assert!(matches!(
        engine
            .apply_row_changes(
                &[update("uno", "one"), delete(2, "stale"), insert.clone()],
                Some((&table, &columns)),
            )
            .await,
        Err(DbxError::Conflict)
    ));
    assert_eq!(
        rows().await,
        vec![
            vec![CellValue::Integer(1), CellValue::Text("one".into())],
            vec![CellValue::Integer(2), CellValue::Text("two".into())],
        ]
    );

    assert_eq!(
        engine
            .apply_row_changes(
                &[update("uno", "one"), delete(2, "two"), insert.clone()],
                None
            )
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        rows().await,
        vec![
            vec![CellValue::Integer(1), CellValue::Text("uno".into())],
            vec![CellValue::Integer(2), CellValue::Text("three".into())],
        ]
    );
    assert_eq!(
        render_row_change(DatabaseKind::SQLite, &delete(2, "two")).unwrap(),
        "DELETE FROM \"items\" WHERE \"id\" = 2;"
    );
    assert_eq!(
        render_row_change(DatabaseKind::SQLite, &insert).unwrap(),
        "INSERT INTO \"items\" (\"value\") VALUES ('three');"
    );
}

#[tokio::test]
async fn query_documents_keep_transactions_and_separate_result_shapes() {
    let (_directory, engine) = database().await;
    let session = QuerySession::new(engine.clone());
    let run = |sql: &'static str| {
        session.run(
            sql,
            QueryOptions::default(),
            Duration::from_secs(5),
            QueryCancellation::default(),
        )
    };
    let results = run("SELECT 1 AS id; SELECT 'different' AS label")
        .await
        .unwrap();
    assert_eq!(results.statements.len(), 2);
    assert_eq!(results.statements[0].result.columns[0].name, "id");
    assert_eq!(results.statements[1].result.columns[0].name, "label");
    assert!(run("BEGIN").await.unwrap().in_transaction);
    assert!(
        run("CREATE TABLE pending (id INTEGER)")
            .await
            .unwrap()
            .in_transaction
    );
    assert!(!run("ROLLBACK").await.unwrap().in_transaction);
    assert!(
        engine
            .query("SELECT * FROM pending", QueryOptions::default())
            .await
            .is_err()
    );
    run("CREATE TEMP TABLE tab_local (id INTEGER)")
        .await
        .unwrap();
    assert!(
        run("SELECT * FROM tab_local").await.unwrap().statements[0]
            .error
            .is_none()
    );
    assert!(
        engine
            .query("SELECT * FROM tab_local", QueryOptions::default())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelling_sqlite_stops_work_and_allows_a_new_query() {
    let (_directory, engine) = database().await;
    let session = Arc::new(QuerySession::new(engine));
    let cancellation = QueryCancellation::default();
    let task = tokio::spawn({
        let session = session.clone();
        let cancellation = cancellation.clone();
        async move {
            session.run("WITH RECURSIVE t(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM t) SELECT sum(n) FROM t", QueryOptions::default(), Duration::from_secs(30), cancellation).await
        }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err() || result.unwrap().statements[0].error.is_some());
    let next = session
        .run(
            "SELECT 42",
            QueryOptions::default(),
            Duration::from_secs(2),
            QueryCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        next.statements[0].result.rows[0].values[0],
        CellValue::Integer(42)
    );
}

#[tokio::test]
async fn late_csv_failure_rolls_back_earlier_batches() {
    let (directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    let file = directory.path().join("bad.csv");
    let mut csv = "id,value\n".to_owned();
    for id in 0..600 {
        csv.push_str(&format!("{id},good\n"));
    }
    csv.push_str("601,bad,extra\n");
    std::fs::write(&file, csv).unwrap();
    assert!(
        import_file(&engine, Some(&TableRef::new("items")), &file)
            .await
            .is_err()
    );
    let result = engine
        .query("SELECT count(*) FROM items", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(result.rows[0].values[0], CellValue::Integer(0));
}

#[tokio::test]
async fn protected_profiles_block_writes_and_keep_metadata_readable() {
    let (directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    let url = format!("sqlite://{}", directory.path().join("db.sqlite").display());
    let protected = Arc::new(
        DatabaseEngine::connect(
            ConnectionConfig::new(DatabaseKind::SQLite, url).with_read_only(true),
        )
        .await
        .unwrap(),
    );
    assert!(protected.is_read_only());
    assert!(!protected.list_tables().await.unwrap().is_empty());
    assert!(
        protected
            .query(
                "INSERT INTO items VALUES (1, 'bad')",
                QueryOptions::default()
            )
            .await
            .is_err()
    );
    assert!(protected.execute_sql("DELETE FROM items").await.is_err());
    let session = QuerySession::new(protected);
    let result = session
        .run(
            "CREATE TABLE forbidden (id INTEGER)",
            QueryOptions::default(),
            Duration::from_secs(5),
            QueryCancellation::default(),
        )
        .await;
    assert!(result.unwrap().statements[0].error.is_some());
}

#[tokio::test]
async fn closing_an_in_memory_transaction_rolls_back_without_destroying_the_database() {
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap(),
    );
    engine
        .execute_sql("CREATE TABLE items(id INTEGER)")
        .await
        .unwrap();
    let session = QuerySession::new(engine.clone());
    let result = session
        .run(
            "BEGIN; INSERT INTO items VALUES (1)",
            QueryOptions::default(),
            Duration::from_secs(3),
            QueryCancellation::default(),
        )
        .await
        .unwrap();
    assert!(result.in_transaction);
    drop(session);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        engine.query("SELECT count(*) FROM items", QueryOptions::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.rows[0].values[0], CellValue::Integer(0));
}
#[tokio::test]
async fn sqlite_savepoints_track_implicit_transactions_and_release_them() {
    for memory in [false, true] {
        let (directory, file_engine) = database().await;
        let engine = if memory {
            Arc::new(
                DatabaseEngine::connect(ConnectionConfig::new(
                    DatabaseKind::SQLite,
                    "sqlite::memory:",
                ))
                .await
                .unwrap(),
            )
        } else {
            file_engine
        };
        engine
            .execute_sql("CREATE TABLE items(id INTEGER)")
            .await
            .unwrap();
        let session = QuerySession::new(engine.clone());
        let run = |sql: &'static str| {
            session.run(
                sql,
                QueryOptions::default(),
                Duration::from_secs(3),
                QueryCancellation::default(),
            )
        };
        assert!(run("SAVEPOINT \"outer point\"; INSERT INTO items VALUES(1); SAVEPOINT inner_point; INSERT INTO items VALUES(2)").await.unwrap().in_transaction);
        assert!(
            run("ROLLBACK\nTO SAVEPOINT inner_point")
                .await
                .unwrap()
                .in_transaction
        );
        assert!(run("RELEASE inner_point").await.unwrap().in_transaction);
        assert!(
            !run("RELEASE SAVEPOINT \"outer point\"")
                .await
                .unwrap()
                .in_transaction
        );
        let result = engine
            .query("SELECT * FROM items", QueryOptions::default())
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert!(
            run("BEGIN; COMMIT invalid_keyword")
                .await
                .unwrap()
                .in_transaction
        );
        assert!(!run("ROLLBACK").await.unwrap().in_transaction);
        drop(directory);
    }
}
#[tokio::test]
async fn nullable_sqlite_primary_keys_cannot_mutate_more_than_one_row() {
    let (_directory, engine) = database().await;
    engine.execute_sql("CREATE TABLE items(id TEXT PRIMARY KEY, value TEXT); INSERT INTO items VALUES(NULL,'one'), (NULL,'two')").await.unwrap();
    let request = UpdateRequest::for_primary_key(
        TableRef::new("items"),
        vec![("value".into(), CellValue::Text("changed".into()))],
        vec![("id".into(), CellValue::Null)],
    );
    assert!(engine.update(&request).await.is_err());
    let result = engine
        .query(
            "SELECT value FROM items ORDER BY value",
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0].values[0], CellValue::Text("one".into()));
}

#[tokio::test]
async fn result_limits_stop_the_script_and_roll_back_open_work() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items(id INTEGER)")
        .await
        .unwrap();
    let session = QuerySession::new(engine.clone());
    let result = session.run("BEGIN; INSERT INTO items VALUES(1); SELECT 1 UNION ALL SELECT 2; INSERT INTO items VALUES(2); COMMIT", QueryOptions { max_rows: Some(1) }, Duration::from_secs(3), QueryCancellation::default()).await.unwrap();
    assert_eq!(result.statements.len(), 3);
    assert!(result.statements[2].result.truncated);
    assert!(
        result.statements[2]
            .error
            .as_ref()
            .unwrap()
            .contains("remaining statements were skipped")
    );
    assert!(!result.in_transaction);
    let rows = engine
        .query("SELECT count(*) FROM items", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(rows.rows[0].values[0], CellValue::Integer(0));
}

#[tokio::test]
async fn sqlite_snapshot_exports_stream_and_round_trip_multiline_values() {
    let (directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    engine
        .execute_sql("INSERT INTO items VALUES(1, 'line one\nline two'), (2,NULL), (3,'')")
        .await
        .unwrap();
    let file = directory.path().join("items.csv.gz");
    let summary = export_table(&engine, &TableRef::new("items"), &file)
        .await
        .unwrap();
    assert!(summary.consistent_snapshot);
    assert_eq!(summary.rows_exported, 3);
    engine
        .execute_sql("CREATE TABLE copy(id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    let control = TransferControl::default();
    let report = with_transfer_control(
        control.clone(),
        import_file(&engine, Some(&TableRef::new("copy")), &file),
    )
    .await
    .unwrap();
    assert_eq!(report.rows_inserted, 3);
    assert_eq!(control.progress().0, 3);
    let original = engine
        .query("SELECT * FROM items ORDER BY id", QueryOptions::default())
        .await
        .unwrap();
    let copy = engine
        .query("SELECT * FROM copy ORDER BY id", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(original.rows, copy.rows);
}

#[tokio::test]
async fn cancelled_transfer_keeps_existing_destination_and_data() {
    let (directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items(id INTEGER)")
        .await
        .unwrap();
    let path = directory.path().join("items.csv");
    std::fs::write(&path, "previous completed export").unwrap();
    let control = TransferControl::default();
    control.cancel();
    assert!(
        with_transfer_control(
            control,
            export_table(&engine, &TableRef::new("items"), &path)
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "previous completed export"
    );
}
#[tokio::test]
async fn cancelling_an_import_after_a_batch_rolls_back_the_entire_file() {
    let (directory, engine) = database().await;
    engine
        .execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT)")
        .await
        .unwrap();
    let path = directory.path().join("large.csv");
    let mut input = "id,value\n".to_owned();
    for id in 0..20000 {
        input.push_str(&format!("{id},value\n"));
    }
    std::fs::write(&path, input).unwrap();
    let control = TransferControl::default();
    let task = tokio::spawn({
        let control = control.clone();
        let engine = engine.clone();
        async move {
            with_transfer_control(
                control,
                import_file(&engine, Some(&TableRef::new("items")), &path),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while control.progress().0 == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    control.cancel();
    assert!(task.await.unwrap().is_err());
    let result = engine
        .query("SELECT count(*) FROM items", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(result.rows[0].values[0], CellValue::Integer(0));
}

async fn native_safety(kind: DatabaseKind, variable: &str, sleep: &str) {
    let url = std::env::var(variable).expect("disposable integration database URL");
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(kind, &url))
            .await
            .unwrap(),
    );
    engine
        .execute_sql("DROP TABLE IF EXISTS dbx_workbench_safety")
        .await
        .unwrap();
    engine
        .execute_sql(
            "CREATE TABLE dbx_workbench_safety(id INTEGER PRIMARY KEY, value VARCHAR(128))",
        )
        .await
        .unwrap();
    let session = QuerySession::new(engine.clone());
    let run = |sql: &'static str| {
        session.run(
            sql,
            QueryOptions::default(),
            Duration::from_secs(5),
            QueryCancellation::default(),
        )
    };
    assert!(
        run("BEGIN; INSERT INTO dbx_workbench_safety VALUES(1, 'pending')")
            .await
            .unwrap()
            .in_transaction
    );
    let outside = engine
        .query(
            "SELECT count(*) FROM dbx_workbench_safety",
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(outside.rows[0].values[0].to_string(), "0");
    assert!(!run("COMMIT").await.unwrap().in_transaction);
    assert!(run("BEGIN; COMMIT AND CHAIN").await.unwrap().in_transaction);
    assert!(run("ROLLBACK AND CHAIN").await.unwrap().in_transaction);
    assert!(!run("ROLLBACK").await.unwrap().in_transaction);
    let results =
        run("SELECT id FROM dbx_workbench_safety; SELECT value FROM dbx_workbench_safety")
            .await
            .unwrap();
    assert_eq!(results.statements[0].result.columns[0].name, "id");
    assert_eq!(results.statements[1].result.columns[0].name, "value");
    let originals = [("value".into(), CellValue::Text("pending".into()))];
    engine
        .execute_sql("UPDATE dbx_workbench_safety SET value='changed'")
        .await
        .unwrap();
    let request = UpdateRequest::for_primary_key(
        TableRef::new("dbx_workbench_safety"),
        vec![("value".into(), CellValue::Text("mine".into()))],
        vec![("id".into(), CellValue::Integer(1))],
    );
    assert!(matches!(
        engine.update_checked(&request, &originals).await,
        Err(DbxError::Conflict)
    ));
    assert_batch_updates_are_atomic(&engine, "dbx_workbench_safety", "changed").await;
    assert_query_tab_recovers_from_a_lost_connection(&engine, kind).await;
    let explained = run(if kind == DatabaseKind::PostgreSQL {
        "EXPLAIN (FORMAT JSON) SELECT * FROM dbx_workbench_safety"
    } else {
        "EXPLAIN FORMAT=JSON SELECT * FROM dbx_workbench_safety"
    })
    .await
    .unwrap();
    assert!(
        explained.statements[0].error.is_none(),
        "{:?}",
        explained.statements[0].error
    );
    assert!(
        !format_execution_plan(kind, &explained.statements[0].result)
            .rows
            .is_empty()
    );
    let protected = DatabaseEngine::connect(ConnectionConfig::new(kind, &url).with_read_only(true))
        .await
        .unwrap();
    assert!(!protected.list_tables().await.unwrap().is_empty());
    assert!(
        protected
            .query("DELETE FROM dbx_workbench_safety", QueryOptions::default())
            .await
            .is_err()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dump.csv");
    assert!(
        export_table(&engine, &request.table, &path)
            .await
            .unwrap()
            .consistent_snapshot
    );
    let sessions = engine
        .query(
            &monitor_query(kind, Monitor::Sessions).unwrap(),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert!(!sessions.columns.is_empty());
    if kind == DatabaseKind::PostgreSQL {
        let locks = engine
            .query(
                &monitor_query(kind, Monitor::Locks).unwrap(),
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert!(!locks.columns.is_empty());
        engine
            .execute_sql("DROP TABLE IF EXISTS dbx_workbench_typed")
            .await
            .unwrap();
        engine.execute_sql("CREATE TABLE dbx_workbench_typed(id UUID PRIMARY KEY, day DATE, amount NUMERIC(30,10), payload JSONB)").await.unwrap();
        engine.execute_sql("INSERT INTO dbx_workbench_typed VALUES ('68cdf6a3-6dc9-4f8c-a846-cc9bc85d338d', '2026-10-06', 12345678901234567890.1234567890, '{\"nested\":[1,true]}')").await.unwrap();
        let table = TableRef::new("dbx_workbench_typed");
        let expected = engine
            .query("SELECT * FROM dbx_workbench_typed", QueryOptions::default())
            .await
            .unwrap();
        let csv = directory.path().join("typed.csv");
        export_table(&engine, &table, &csv).await.unwrap();
        engine
            .execute_sql("DELETE FROM dbx_workbench_typed")
            .await
            .unwrap();
        assert_eq!(
            import_file(&engine, Some(&table), &csv)
                .await
                .unwrap()
                .rows_inserted,
            1
        );
        let actual = engine
            .query("SELECT * FROM dbx_workbench_typed", QueryOptions::default())
            .await
            .unwrap();
        assert_eq!(expected.rows, actual.rows);
        engine
            .execute_sql("DROP TABLE dbx_workbench_typed")
            .await
            .unwrap();
    } else {
        let ddl = directory.path().join("implicit-commit.sql");
        std::fs::write(&ddl, "INSERT INTO dbx_workbench_safety VALUES(800, 'should roll back'); CREATE TABLE dbx_workbench_forbidden(id INT)").unwrap();
        assert!(import_file(&engine, None, &ddl).await.is_err());
        let count = engine
            .query(
                "SELECT count(*) FROM dbx_workbench_safety WHERE id=800",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(count.rows[0].values[0].to_string(), "0");
        engine
            .execute_sql("DROP TABLE IF EXISTS dbx_workbench_myisam")
            .await
            .unwrap();
        engine
            .execute_sql("CREATE TABLE dbx_workbench_myisam(id INT) ENGINE=MyISAM")
            .await
            .unwrap();
        let table = TableRef::new("dbx_workbench_myisam");
        let csv = directory.path().join("myisam.csv");
        assert!(
            !export_table(&engine, &table, &csv)
                .await
                .unwrap()
                .consistent_snapshot
        );
        assert!(import_file(&engine, Some(&table), &csv).await.is_err());
        engine
            .execute_sql("DROP TABLE dbx_workbench_myisam")
            .await
            .unwrap();
    }
    let bad = directory.path().join("bad.csv");
    let mut csv = "id,value\n".to_owned();
    for id in 2..602 {
        csv.push_str(&format!("{id},valid\n"));
    }
    csv.push_str("700,invalid,extra\n");
    std::fs::write(&bad, csv).unwrap();
    assert!(
        import_file(&engine, Some(&request.table), &bad)
            .await
            .is_err()
    );
    let count = engine
        .query(
            "SELECT count(*) FROM dbx_workbench_safety",
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(count.rows[0].values[0].to_string(), "1");
    let cancellation = QueryCancellation::default();
    let cancelling_session = Arc::new(QuerySession::new(engine.clone()));
    let task = tokio::spawn({
        let session = cancelling_session.clone();
        let cancellation = cancellation.clone();
        let sleep = sleep.to_owned();
        async move {
            session
                .run(
                    &sleep,
                    QueryOptions::default(),
                    Duration::from_secs(30),
                    cancellation,
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_secs(8), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Server cancellation acknowledged"),
        "{error}"
    );
    let next = cancelling_session
        .run(
            "SELECT 42",
            QueryOptions::default(),
            Duration::from_secs(3),
            QueryCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        next.statements[0].result.rows[0].values[0].to_string(),
        "42"
    );
}
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server"]
async fn postgres_workbench_safety() {
    native_safety(
        DatabaseKind::PostgreSQL,
        "DBX_TEST_POSTGRES_URL",
        "SELECT pg_sleep(30)",
    )
    .await;
}
#[tokio::test]
#[ignore = "requires a disposable MySQL server"]
async fn mysql_workbench_safety() {
    native_safety(
        DatabaseKind::MySQL,
        "DBX_TEST_MYSQL_URL",
        "SELECT SLEEP(30)",
    )
    .await;
}

#[tokio::test]
async fn prepared_query_session_keeps_values_out_of_sql_and_preserves_transactions() {
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap(),
    );
    engine
        .execute_sql("CREATE TABLE bound_values (value TEXT)")
        .await
        .unwrap();
    let session = QuerySession::new(engine.clone());
    let injected = "'); DROP TABLE bound_values; --";
    let result = session
        .run_prepared(
            vec![
                SqlStatement::new("BEGIN", vec![]),
                SqlStatement::new(
                    "INSERT INTO bound_values VALUES (?)",
                    vec![CellValue::Text(injected.into())],
                ),
                SqlStatement::new(
                    "SELECT value FROM bound_values WHERE value=?",
                    vec![CellValue::Text(injected.into())],
                ),
                SqlStatement::new("COMMIT", vec![]),
            ],
            QueryOptions::default(),
            Duration::from_secs(5),
            QueryCancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        result
            .statements
            .iter()
            .all(|statement| statement.error.is_none())
    );
    assert_eq!(
        result.statements[2].result.rows[0].values[0],
        CellValue::Text(injected.into())
    );
    let persisted = engine
        .query("SELECT value FROM bound_values", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(
        persisted.rows[0].values[0],
        CellValue::Text(injected.into())
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL from scripts/test-integration.sh"]
async fn postgres_schema_objects_dump_restore_round_trip() {
    let url = std::env::var("DBX_TEST_POSTGRES_URL").unwrap();
    let engine = DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::PostgreSQL, url))
        .await
        .unwrap();
    let schema = format!(
        "dbx_audit_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    engine
        .execute_sql(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    engine.execute_sql(&format!("CREATE FUNCTION {schema}.twice(n integer) RETURNS integer LANGUAGE SQL AS $$ SELECT n * 2 $$")).await.unwrap();
    engine
        .execute_sql(&format!(
            "CREATE SEQUENCE {schema}.standalone START WITH 23"
        ))
        .await
        .unwrap();
    engine.execute_sql(&format!("CREATE TABLE {schema}.items (id serial PRIMARY KEY, code text UNIQUE NOT NULL, qty integer DEFAULT {schema}.twice(2) CHECK(qty>=0))")).await.unwrap();
    engine.execute_sql(&format!("CREATE FUNCTION {schema}.bump() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.qty := NEW.qty + 1; RETURN NEW; END $$")).await.unwrap();
    engine.execute_sql(&format!("CREATE TRIGGER bump BEFORE INSERT ON {schema}.items FOR EACH ROW EXECUTE FUNCTION {schema}.bump()")).await.unwrap();
    engine
        .execute_sql(&format!(
            "CREATE VIEW {schema}.items_view AS SELECT code,qty FROM {schema}.items"
        ))
        .await
        .unwrap();
    engine
        .execute_sql(&format!(
            "INSERT INTO {schema}.items(code) VALUES ('initial')"
        ))
        .await
        .unwrap();
    engine
        .execute_sql(&format!(
            "CREATE VIEW {schema}.a_dependent AS SELECT * FROM {schema}.items_view"
        ))
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    export_database(
        &engine,
        &DatabaseExportRequest {
            tables: vec![TableRef::in_schema(&schema, "items")],
            output_directory: directory.path().to_owned(),
            output_name: "objects".into(),
            format: DumpFormat::Sql,
            schema_only: false,
            gzipped: false,
        },
    )
    .await
    .unwrap();
    engine
        .execute_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    import_database(&engine, &directory.path().join("objects.sql"))
        .await
        .unwrap();
    engine
        .execute_sql(&format!(
            "INSERT INTO {schema}.items(code) VALUES ('restored')"
        ))
        .await
        .unwrap();
    let result = engine
        .query(
            &format!("SELECT qty FROM {schema}.a_dependent WHERE code='restored'"),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0].values[0], CellValue::Integer(5));
    let sequence = engine
        .query(
            &format!("SELECT nextval('{schema}.standalone')"),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(sequence.rows[0].values[0], CellValue::Integer(23));
    assert!(
        engine
            .execute_sql(&format!(
                "INSERT INTO {schema}.items(code,qty) VALUES ('bad',-2)"
            ))
            .await
            .is_err()
    );
    let structure = engine
        .table_structure(&TableRef::in_schema(&schema, "items"))
        .await
        .unwrap();
    assert_eq!(structure.checks.len(), 1);
    assert!(
        structure
            .indexes
            .iter()
            .any(|index| index.unique && !index.primary)
    );
    engine
        .execute_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
}

#[tokio::test]
async fn rows_are_counted_with_filters() {
    let (_directory, engine) = database().await;
    engine
        .execute_sql(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT); \
             INSERT INTO items VALUES (1, 'one'), (2, NULL), (3, 'three');",
        )
        .await
        .unwrap();
    let table = TableRef::new("items");
    assert_eq!(engine.count_rows(&table, &[], None).await.unwrap(), 3);
    assert_eq!(
        engine
            .count_rows(
                &table,
                &[Filter::new("value", FilterOperator::IsNull, None)],
                None
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(engine.estimate_rows(&table).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires the disposable SQL Server integration database"]
async fn sqlserver_sessions_and_changesets_are_isolated_and_atomic() -> Result<()> {
    let config = ConnectionConfig::new(
        DatabaseKind::SqlServer,
        std::env::var("DBX_TEST_SQLSERVER_URL").expect("fixture URL required"),
    );
    let engine = Arc::new(DatabaseEngine::connect(config).await?);
    let first = QuerySession::new(engine.clone());
    let second = QuerySession::new(engine.clone());
    let run = |session: Arc<QuerySession>, sql: &'static str| async move {
        session
            .run(
                sql,
                QueryOptions::default(),
                Duration::from_secs(10),
                QueryCancellation::default(),
            )
            .await
    };
    let first = Arc::new(first);
    let second = Arc::new(second);
    let one = run(first.clone(), "SELECT @@SPID").await?;
    let two = run(second.clone(), "SELECT @@SPID").await?;
    assert_ne!(
        one.statements[0].result.rows[0].values,
        two.statements[0].result.rows[0].values
    );
    let tx = run(first.clone(), "BEGIN; CREATE TABLE #dbx_private(id int)").await?;
    assert!(tx.in_transaction, "{tx:?}");
    let isolated = run(second, "SELECT @@TRANCOUNT").await?;
    assert_eq!(
        isolated.statements[0].result.rows[0].values[0],
        CellValue::Integer(0)
    );
    let rolled_back = run(first, "ROLLBACK").await?;
    assert!(!rolled_back.in_transaction);
    engine
        .execute_sql("IF OBJECT_ID('dbx_atomic_audit') IS NOT NULL DROP TABLE dbx_atomic_audit")
        .await?;
    engine
        .execute_sql("CREATE TABLE dbx_atomic_audit(id int PRIMARY KEY, value nvarchar(100))")
        .await?;
    let table = TableRef::new("dbx_atomic_audit");
    let changes = [
        RowChange::Insert(InsertRequest::from_row(
            table.clone(),
            vec![
                ("id".into(), CellValue::Integer(1)),
                ("value".into(), CellValue::Text("first".into())),
            ],
        )),
        RowChange::Insert(InsertRequest::from_row(
            table.clone(),
            vec![
                ("id".into(), CellValue::Integer(1)),
                ("value".into(), CellValue::Text("duplicate".into())),
            ],
        )),
    ];
    assert!(
        engine
            .apply_row_changes(
                &changes,
                Some((&table, &engine.describe_table(&table).await?))
            )
            .await
            .is_err()
    );
    assert_eq!(engine.count_rows(&table, &[], None).await?, 0);
    engine.execute_sql("DROP TABLE dbx_atomic_audit").await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and installed pg_dump/pg_restore"]
async fn native_postgres_backup_restores_objects_into_a_new_database() -> Result<()> {
    let config = ConnectionConfig::new(
        DatabaseKind::PostgreSQL,
        std::env::var("DBX_TEST_POSTGRES_URL").expect("fixture URL required"),
    );
    let engine = DatabaseEngine::connect(config.clone()).await?;
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("dbx_backup_{suffix}");
    let target = format!("dbx_restore_{suffix}");
    engine
        .execute_sql(&format!("CREATE DATABASE {source}"))
        .await?;
    engine
        .execute_sql(&format!("CREATE DATABASE {target}"))
        .await?;
    let result = async {
        let mut source_url = url::Url::parse(&config.url).unwrap();
        source_url.set_path(&format!("/{source}"));
        let mut source_config = config.clone();
        source_config.url = source_url.into();
        let source_engine = DatabaseEngine::connect(source_config.clone()).await?;
        for sql in [
            "CREATE TABLE parent(id serial PRIMARY KEY, name text UNIQUE CHECK(length(name)>0))",
            "CREATE TABLE child(id int PRIMARY KEY, parent_id int REFERENCES parent(id))",
            "INSERT INTO parent(name) VALUES('round trip')",
            "INSERT INTO child VALUES(1,1)",
            "CREATE VIEW names AS SELECT name FROM parent",
            "CREATE FUNCTION dbx_answer() RETURNS integer LANGUAGE SQL AS 'SELECT 42'",
        ] {
            source_engine.execute_sql(sql).await?;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.backup");
        let control = TransferControl::default();
        native_backup(source_config, &path, false, control.clone()).await?;
        assert!(!control.log().contains("dbx_test_password"));
        let mut target_url = url::Url::parse(&config.url).unwrap();
        target_url.set_path(&format!("/{target}"));
        let mut target_config = config.clone();
        target_config.url = target_url.into();
        native_backup(
            target_config.clone(),
            &path,
            true,
            TransferControl::default(),
        )
        .await?;
        let target_engine = DatabaseEngine::connect(target_config).await?;
        assert_eq!(
            target_engine
                .query("SELECT dbx_answer()", QueryOptions::default())
                .await?
                .rows[0]
                .values[0],
            CellValue::Integer(42)
        );
        assert_eq!(
            target_engine
                .count_rows(&TableRef::new("child"), &[], None)
                .await?,
            1
        );
        assert_eq!(
            target_engine
                .table_structure(&TableRef::new("child"))
                .await?
                .foreign_keys
                .len(),
            1
        );
        assert!(
            target_engine
                .query("SELECT name FROM names", QueryOptions::default())
                .await?
                .rows[0]
                .values[0]
                .to_string()
                .contains("round trip")
        );
        Ok(())
    }
    .await;
    let _ = engine
        .execute_sql(&format!("DROP DATABASE {source} WITH (FORCE)"))
        .await;
    let _ = engine
        .execute_sql(&format!("DROP DATABASE {target} WITH (FORCE)"))
        .await;
    result
}

#[tokio::test]
#[ignore = "requires disposable MySQL and native client wrappers from test-integration.sh"]
async fn native_mysql_backup_restores_objects_into_a_new_database() -> Result<()> {
    let config = ConnectionConfig::new(
        DatabaseKind::MySQL,
        std::env::var("DBX_TEST_MYSQL_ADMIN_URL").expect("fixture admin URL required"),
    );
    let engine = DatabaseEngine::connect(config.clone()).await?;
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("dbx_backup_{suffix}");
    let target = format!("dbx_restore_{suffix}");
    engine
        .execute_sql(&format!("CREATE DATABASE {source}"))
        .await?;
    engine
        .execute_sql(&format!("CREATE DATABASE {target}"))
        .await?;
    let result = async {
        let mut source_url = url::Url::parse(&config.url).unwrap(); source_url.set_path(&format!("/{source}"));
        let mut source_config = config.clone(); source_config.url = source_url.into();
        let source_engine = DatabaseEngine::connect(source_config.clone()).await?;
        for sql in ["CREATE TABLE parent(id int PRIMARY KEY, name varchar(40) UNIQUE CHECK(length(name)>0))", "CREATE TABLE child(id int PRIMARY KEY, parent_id int, FOREIGN KEY(parent_id) REFERENCES parent(id))", "INSERT INTO parent VALUES(1,'round trip')", "INSERT INTO child VALUES(1,1)", "CREATE VIEW names AS SELECT name FROM parent", "CREATE PROCEDURE dbx_answer() SELECT 42 AS answer", "CREATE TRIGGER child_guard BEFORE INSERT ON child FOR EACH ROW SET NEW.id = NEW.id"] { source_engine.execute_sql(sql).await?; }
        let directory = tempfile::tempdir().unwrap(); let path = directory.path().join("native.sql");
        native_backup(source_config, &path, false, TransferControl::default()).await?;
        let mut target_url = url::Url::parse(&config.url).unwrap(); target_url.set_path(&format!("/{target}"));
        let mut target_config = config.clone(); target_config.url = target_url.into();
        native_backup(target_config.clone(), &path, true, TransferControl::default()).await?;
        let target_engine = DatabaseEngine::connect(target_config).await?;
        assert_eq!(target_engine.query("CALL dbx_answer()", QueryOptions::default()).await?.rows[0].values[0].to_string(), "42");
        assert_eq!(target_engine.count_rows(&TableRef::new("child"), &[], None).await?, 1);
        assert_eq!(target_engine.table_structure(&TableRef::new("child")).await?.foreign_keys.len(), 1);
        assert!(target_engine.query("SELECT name FROM names", QueryOptions::default()).await?.rows[0].values[0].to_string().contains("round trip"));
        assert_eq!(target_engine.query("SELECT COUNT(*) FROM information_schema.triggers WHERE trigger_schema = DATABASE()", QueryOptions::default()).await?.rows[0].values[0].to_string(), "1");
        Ok(())
    }.await;
    let _ = engine.execute_sql(&format!("DROP DATABASE {source}")).await;
    let _ = engine.execute_sql(&format!("DROP DATABASE {target}")).await;
    result
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and MySQL"]
async fn postgres_mysql_json_import_preserves_decimal_and_large_integer_columns() -> Result<()> {
    use dbx_core::data_import::{ImportData, import_data};
    for (kind, variable) in [
        (DatabaseKind::PostgreSQL, "DBX_TEST_POSTGRES_URL"),
        (DatabaseKind::MySQL, "DBX_TEST_MYSQL_URL"),
    ] {
        let engine = DatabaseEngine::connect(ConnectionConfig::new(
            kind,
            std::env::var(variable).unwrap(),
        ))
        .await?;
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let table = TableRef::new(format!("dbx_exact_import_{suffix}"));
        let name = quote_identifier(kind, &table.name)?;
        engine
            .execute_sql(&format!(
                "CREATE TABLE {name}(amount DECIMAL(40,19), huge DECIMAL(30,0))"
            ))
            .await?;
        let result: Result<()> = async {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("numbers.json");
            std::fs::write(
                &path,
                r#"[{"amount":1234567890.1234567890123456789,"huge":184467440737095516160}]"#,
            )
            .unwrap();
            let data = ImportData::read(&path)?;
            let columns = engine.describe_table(&table).await?;
            let mapping = data.default_mapping(&columns);
            import_data(
                &engine,
                &table,
                &data,
                &mapping,
                &engine.current_database().await?,
                &columns,
            )
            .await?;
            let rows = engine
                .query(
                    &format!("SELECT amount, huge FROM {name}"),
                    QueryOptions::default(),
                )
                .await?
                .rows;
            assert_eq!(
                rows[0].values[0].to_string(),
                "1234567890.1234567890123456789",
                "{kind}"
            );
            assert_eq!(
                rows[0].values[1].to_string(),
                "184467440737095516160",
                "{kind}"
            );
            Ok(())
        }
        .await;
        let _ = engine.execute_sql(&format!("DROP TABLE {name}")).await;
        result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and MySQL"]
async fn postgres_mysql_designer_roundtrip() -> Result<()> {
    for (kind, variable) in [
        (DatabaseKind::PostgreSQL, "DBX_TEST_POSTGRES_URL"),
        (DatabaseKind::MySQL, "DBX_TEST_MYSQL_URL"),
    ] {
        let engine = Arc::new(
            DatabaseEngine::connect(ConnectionConfig::new(
                kind,
                std::env::var(variable).unwrap(),
            ))
            .await?,
        );
        let parent = TableRef::new("designer.parent");
        let child = TableRef::new("designer.child");
        let parent_sql = dbx_core::quote_identifier(kind, &parent.name)?;
        let child_sql = dbx_core::quote_identifier(kind, &child.name)?;
        engine
            .execute_sql(&format!("DROP TABLE IF EXISTS {child_sql}"))
            .await?;
        engine
            .execute_sql(&format!("DROP TABLE IF EXISTS {parent_sql}"))
            .await?;
        engine
            .execute_sql(&format!(
                "CREATE TABLE {parent_sql}(id INTEGER PRIMARY KEY)"
            ))
            .await?;
        engine
            .execute_sql(&format!(
                "CREATE TABLE {child_sql}(id INTEGER NOT NULL, parent_id INTEGER, qty INTEGER)"
            ))
            .await?;
        let result: Result<()> = async {
            for change in [
                TableAlteration::AlterColumn {
                    name: "qty".into(),
                    data_type: "BIGINT".into(),
                    nullable: false,
                    default: Some("2".into()),
                },
                TableAlteration::AddPrimaryKey {
                    name: "designer_pk".into(),
                    columns: vec!["id".into()],
                },
                TableAlteration::AddForeignKey {
                    name: "designer_fk".into(),
                    columns: vec!["parent_id".into()],
                    referenced_table: parent.clone(),
                    referenced_columns: vec!["id".into()],
                },
                TableAlteration::AddCheck {
                    name: "designer_check".into(),
                    expression: "qty >= 0".into(),
                },
            ] {
                let sql = draft_table_alteration_for(&engine, &child, &change).await?;
                let run = QuerySession::new(engine.clone())
                    .run(
                        &sql,
                        QueryOptions::default(),
                        Duration::from_secs(10),
                        QueryCancellation::default(),
                    )
                    .await?;
                assert!(
                    run.statements
                        .iter()
                        .all(|statement| statement.error.is_none()),
                    "{kind}: {:?}",
                    run.statements
                        .iter()
                        .filter_map(|statement| statement.error.clone())
                        .collect::<Vec<_>>()
                );
            }
            if kind == DatabaseKind::MySQL {
                engine.execute_sql(&format!("ALTER TABLE {child_sql} MODIFY id INTEGER NOT NULL AUTO_INCREMENT")).await?;
                let sql = draft_table_alteration_for(&engine, &child, &TableAlteration::AlterColumn { name: "id".into(), data_type: "BIGINT".into(), nullable: false, default: None }).await?;
                assert!(sql.contains("AUTO_INCREMENT")); engine.execute_sql(&sql).await?;
                engine.execute_sql(&format!("ALTER TABLE {child_sql} ADD label VARCHAR(40) COLLATE utf8mb4_bin COMMENT 'existing comment'")).await?;
                let sql = draft_table_alteration_for(&engine, &child, &TableAlteration::AlterColumn { name: "label".into(), data_type: "VARCHAR(80)".into(), nullable: true, default: None }).await?;
                assert!(sql.contains("COLLATE `utf8mb4_bin`")); assert!(sql.contains("COMMENT 'existing comment'")); engine.execute_sql(&sql).await?;
                engine.execute_sql(&format!("ALTER TABLE {child_sql} ADD computed BIGINT GENERATED ALWAYS AS (qty*2) STORED")).await?;
                assert!(draft_table_alteration_for(&engine, &child, &TableAlteration::AlterColumn { name: "computed".into(), data_type: "BIGINT".into(), nullable: true, default: None }).await.is_err());
            }
            let structure = engine.table_structure(&child).await?;
            assert_eq!(structure.foreign_keys.len(), 1);
            assert_eq!(structure.checks.len(), 1);
            assert!(
                structure
                    .columns
                    .iter()
                    .find(|column| column.name == "id")
                    .unwrap()
                    .primary_key
            );
            let qty = structure
                .columns
                .iter()
                .find(|column| column.name == "qty")
                .unwrap();
            assert!(!qty.nullable);
            assert!(qty.default_value.is_some());
            engine
                .execute_sql(&format!("INSERT INTO {parent_sql} VALUES(1)"))
                .await?;
            engine
                .execute_sql(&format!(
                    "INSERT INTO {child_sql}(id,parent_id) VALUES(1,1)"
                ))
                .await?;
            assert!(
                engine
                    .execute_sql(&format!("INSERT INTO {child_sql}(id,parent_id,qty) VALUES(2,1,-1)"))
                    .await
                    .is_err()
            );
            assert!(
                engine
                    .execute_sql(&format!("INSERT INTO {child_sql}(id,parent_id,qty) VALUES(2,999,2)"))
                    .await
                    .is_err()
            );
            Ok(())
        }
        .await;
        let _ = engine.drop_table(&child).await;
        let _ = engine.drop_table(&parent).await;
        result?;
    }
    Ok(())
}
