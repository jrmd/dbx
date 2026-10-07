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
