use dbx_core::{
    CellValue, ConnectionConfig, DatabaseEngine, DatabaseKind, QueryCancellation, QueryOptions,
    QuerySession, format_execution_plan,
};
use std::{sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server via DBX_TEST_POSTGRES_URL"]
async fn postgres_analysis_and_watch_use_isolated_read_only_transactions() -> dbx_core::Result<()> {
    let url = std::env::var("DBX_TEST_POSTGRES_URL").expect("DBX_TEST_POSTGRES_URL is required");
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::PostgreSQL, url)).await?,
    );
    let error = engine
        .query("SELECT dbx_missing_column", QueryOptions::default())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("column \"dbx_missing_column\" does not exist"));
    assert!(
        error.contains("Position: 8"),
        "server position must survive decoding: {error}"
    );
    assert!(error.contains("Database code: 42703"));
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let table = format!("dbx_watch_{suffix}");
    let function = format!("dbx_watch_write_{suffix}");
    engine
        .execute_sql(&format!("CREATE TABLE {table}(id INTEGER)"))
        .await?;
    engine
        .execute_sql(&format!("INSERT INTO {table} VALUES(1)"))
        .await?;
    engine.execute_sql(&format!("CREATE FUNCTION {function}() RETURNS INTEGER LANGUAGE plpgsql AS $$ BEGIN INSERT INTO {table} VALUES(99); RETURN 99; END $$")).await?;
    let tab = QuerySession::new(engine.clone());
    let tab_run = tab
        .run(
            &format!("BEGIN; INSERT INTO {table} VALUES(2)"),
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert!(tab_run.in_transaction);
    assert!(
        tab_run
            .statements
            .iter()
            .all(|statement| statement.error.is_none())
    );

    let plan = QuerySession::explain_analyze(
        engine.clone(),
        &format!("SELECT * FROM {table} -- trailing comment"),
        QueryOptions::default(),
        Duration::from_secs(10),
        QueryCancellation::default(),
    )
    .await?;
    let formatted = format_execution_plan(DatabaseKind::PostgreSQL, &plan);
    assert!(
        formatted
            .columns
            .iter()
            .any(|column| column.name == "Actual time (ms)")
    );
    assert!(
        formatted
            .rows
            .iter()
            .any(|row| row.values.get(4) == Some(&CellValue::Text("1".into())))
    );

    let sample = QuerySession::read_only_query(
        engine.clone(),
        &format!("SELECT count(*) FROM {table}"),
        QueryOptions::default(),
        Duration::from_secs(10),
        QueryCancellation::default(),
    )
    .await?;
    assert_eq!(
        sample.rows[0].values[0].to_string(),
        "1",
        "watch must not share the tab's uncommitted row"
    );
    let original = tab
        .run(
            &format!("SELECT count(*) FROM {table}"),
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert!(original.in_transaction);
    assert_eq!(
        original.statements[0].result.rows[0].values[0].to_string(),
        "2",
        "analysis and watch must preserve the original transaction"
    );

    for analyze in [false, true] {
        let sql = format!("SELECT {function}()");
        let result = if analyze {
            QuerySession::explain_analyze(
                engine.clone(),
                &sql,
                QueryOptions::default(),
                Duration::from_secs(10),
                QueryCancellation::default(),
            )
            .await
        } else {
            QuerySession::read_only_query(
                engine.clone(),
                &sql,
                QueryOptions::default(),
                Duration::from_secs(10),
                QueryCancellation::default(),
            )
            .await
        };
        assert!(
            result
                .unwrap_err()
                .to_string()
                .to_lowercase()
                .contains("read-only")
        );
    }
    let check = engine
        .query(
            &format!("SELECT count(*) FROM {table}"),
            QueryOptions::default(),
        )
        .await?;
    assert_eq!(
        check.rows[0].values[0].to_string(),
        "1",
        "errors must never commit function writes"
    );

    let cancellation = QueryCancellation::default();
    let pending = tokio::spawn(QuerySession::read_only_query(
        engine.clone(),
        "SELECT pg_sleep(30)",
        QueryOptions::default(),
        Duration::from_secs(30),
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();
    let cancelled = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("watch cancellation must complete")
        .expect("watch task joins");
    assert!(cancelled.is_err());
    let original = tab
        .run(
            &format!("SELECT count(*) FROM {table}; ROLLBACK"),
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert_eq!(
        original.statements[0].result.rows[0].values[0].to_string(),
        "2"
    );
    assert!(!original.in_transaction);
    engine
        .execute_sql(&format!("DROP FUNCTION {function}()"))
        .await?;
    engine.execute_sql(&format!("DROP TABLE {table}")).await?;
    Ok(())
}

async fn isolated_sample(
    engine: Arc<DatabaseEngine>,
    sql: &str,
) -> dbx_core::Result<dbx_core::QueryResult> {
    QuerySession::read_only_query(
        engine,
        sql,
        QueryOptions::default(),
        Duration::from_secs(10),
        QueryCancellation::default(),
    )
    .await
}

#[tokio::test]
async fn sqlite_watch_keeps_tab_transaction_and_cancels_without_losing_data() -> dbx_core::Result<()>
{
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("watch.sqlite");
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            format!("sqlite://{}?mode=rwc", path.display()),
        ))
        .await?,
    );
    engine
        .execute_sql("CREATE TABLE samples(id INTEGER, label TEXT, payload BLOB)")
        .await?;
    engine.execute_sql("INSERT INTO samples VALUES(9007199254740993, 'z', NULL), (9007199254740994, NULL, NULL)").await?;
    let tab = QuerySession::new(engine.clone());
    let run = tab
        .run(
            "BEGIN; INSERT INTO samples VALUES(3, 'pending', NULL)",
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert!(run.in_transaction);
    let sample = isolated_sample(
        engine.clone(),
        "SELECT COUNT(*) FROM samples -- trailing comment",
    )
    .await?;
    assert_eq!(sample.rows[0].values[0], CellValue::Integer(2));
    let columns = engine
        .describe_table(&dbx_core::TableRef::new("samples"))
        .await?;
    let sql = dbx_core::table_column_statistics_query(
        DatabaseKind::SQLite,
        &dbx_core::TableRef::new("samples"),
        &columns,
    )?;
    let raw = isolated_sample(engine.clone(), &sql).await?;
    let stats = dbx_core::format_table_column_statistics(DatabaseKind::SQLite, &columns, &raw)?;
    assert_eq!(
        stats.rows[0].values[5],
        CellValue::Integer(9007199254740993)
    );
    assert_eq!(
        stats.rows[0].values[6],
        CellValue::Integer(9007199254740994)
    );
    assert_eq!(stats.rows[1].values[3], CellValue::Integer(1));
    assert!(stats.rows[2].values[7].to_string().contains("unsupported"));
    let cancellation = QueryCancellation::default();
    let pending = tokio::spawn(QuerySession::read_only_query(
        engine.clone(),
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n",
        QueryOptions::default(),
        Duration::from_secs(30),
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let timeout = QuerySession::read_only_query(
        engine.clone(),
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n",
        QueryOptions::default(),
        Duration::from_millis(30),
        QueryCancellation::default(),
    )
    .await;
    assert!(timeout.is_err());
    let original = tab
        .run(
            "SELECT COUNT(*) FROM samples; ROLLBACK",
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert_eq!(
        original.statements[0].result.rows[0].values[0],
        CellValue::Integer(3)
    );
    // A readonly PRAGMA must never leak into the writable pool or tab.
    engine
        .execute_sql("INSERT INTO samples VALUES(4, 'after cancel', NULL)")
        .await?;
    assert_eq!(
        isolated_sample(engine, "SELECT COUNT(*) FROM samples")
            .await?
            .rows[0]
            .values[0],
        CellValue::Integer(3)
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_private_memory_watch_is_rejected_without_touching_original() -> dbx_core::Result<()>
{
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await?,
    );
    engine
        .execute_sql("CREATE TABLE preserved(id INTEGER)")
        .await?;
    engine
        .execute_sql("INSERT INTO preserved VALUES(7)")
        .await?;
    let error = isolated_sample(engine.clone(), "SELECT * FROM preserved")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("file-backed"), "{error}");
    let result = engine
        .query("SELECT * FROM preserved", QueryOptions::default())
        .await?;
    assert_eq!(result.rows[0].values[0], CellValue::Integer(7));
    Ok(())
}

#[tokio::test]
#[ignore = "requires a disposable MySQL server via DBX_TEST_MYSQL_URL"]
async fn mysql_watch_analyze_stats_and_cancellation_are_isolated() -> dbx_core::Result<()> {
    let url = std::env::var("DBX_TEST_MYSQL_URL").expect("DBX_TEST_MYSQL_URL is required");
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::MySQL, url.clone())).await?,
    );
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let table = format!("dbx_diagnostics_{suffix}");
    let function = format!("dbx_diagnostics_write_{suffix}");
    engine
        .execute_sql(&format!(
            "CREATE TABLE {table}(id BIGINT UNSIGNED, amount DECIMAL(40,12), label VARCHAR(20))"
        ))
        .await?;
    engine.execute_sql(&format!("INSERT INTO {table} VALUES(18446744073709551614, 12345678901234567890.123456789012, 'z')")).await?;
    // Binary logging requires SUPER to create this deliberately writing
    // function. Do not weaken global server settings to arrange the fixture.
    let admin = DatabaseEngine::connect(ConnectionConfig::new(
        DatabaseKind::MySQL,
        std::env::var("DBX_TEST_MYSQL_ADMIN_URL").expect("DBX_TEST_MYSQL_ADMIN_URL is required"),
    ))
    .await?;
    admin.execute_sql(&format!("CREATE FUNCTION {function}() RETURNS INTEGER DETERMINISTIC MODIFIES SQL DATA BEGIN INSERT INTO {table}(id) VALUES(99); RETURN 99; END")).await?;
    let tab = QuerySession::new(engine.clone());
    let run = tab
        .run(
            &format!("BEGIN; INSERT INTO {table}(id) VALUES(2)"),
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert!(run.in_transaction);
    assert!(
        run.statements
            .iter()
            .all(|statement| statement.error.is_none())
    );
    assert_eq!(
        isolated_sample(engine.clone(), &format!("SELECT COUNT(*) FROM {table}"))
            .await?
            .rows[0]
            .values[0]
            .to_string(),
        "1"
    );
    let plan = QuerySession::explain_analyze(
        engine.clone(),
        &format!("SELECT * FROM {table}"),
        QueryOptions::default(),
        Duration::from_secs(10),
        QueryCancellation::default(),
    )
    .await?;
    let version = engine
        .query("SELECT VERSION()", QueryOptions::default())
        .await?;
    if !version.rows[0].values[0]
        .to_string()
        .to_ascii_lowercase()
        .contains("mariadb")
    {
        assert!(plan.rows[0].values[0].to_string().contains("actual time"));
    }
    let tree = format_execution_plan(DatabaseKind::MySQL, &plan);
    assert_eq!(tree.columns[0].name, "Depth");
    assert!(tree.rows.iter().any(|row| row.values[4] != CellValue::Null));
    for analyze in [false, true] {
        let sql = format!("SELECT {function}()");
        let result = if analyze {
            QuerySession::explain_analyze(
                engine.clone(),
                &sql,
                QueryOptions::default(),
                Duration::from_secs(10),
                QueryCancellation::default(),
            )
            .await
        } else {
            isolated_sample(engine.clone(), &sql).await
        };
        let error = result.unwrap_err().to_string().to_ascii_lowercase();
        assert!(
            error.contains("read only") || error.contains("read-only"),
            "{error}"
        );
    }
    let columns = engine
        .describe_table(&dbx_core::TableRef::new(&table))
        .await?;
    let sql = dbx_core::table_column_statistics_query(
        DatabaseKind::MySQL,
        &dbx_core::TableRef::new(&table),
        &columns,
    )?;
    let stats = dbx_core::format_table_column_statistics(
        DatabaseKind::MySQL,
        &columns,
        &isolated_sample(engine.clone(), &sql).await?,
    )?;
    assert_eq!(
        stats.rows[0].values[5],
        CellValue::Unsigned(18446744073709551614)
    );
    assert_eq!(
        stats.rows[1].values[5].to_string(),
        "12345678901234567890.123456789012"
    );
    let cancellation = QueryCancellation::default();
    let pending = tokio::spawn(QuerySession::read_only_query(
        engine.clone(),
        "SELECT SLEEP(30)",
        QueryOptions::default(),
        Duration::from_secs(30),
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    // A protected native profile must accept our exact readonly START syntax.
    let mut config = ConnectionConfig::new(DatabaseKind::MySQL, url);
    config.read_only = true;
    let protected = Arc::new(DatabaseEngine::connect(config).await?);
    assert_eq!(
        isolated_sample(protected.clone(), "SELECT 7").await?.rows[0].values[0].to_string(),
        "7"
    );
    QuerySession::explain_analyze(
        protected,
        &format!("SELECT * FROM {table}"),
        QueryOptions::default(),
        Duration::from_secs(10),
        QueryCancellation::default(),
    )
    .await?;
    let original = tab
        .run(
            &format!("SELECT COUNT(*) FROM {table}; ROLLBACK"),
            QueryOptions::default(),
            Duration::from_secs(10),
            QueryCancellation::default(),
        )
        .await?;
    assert_eq!(
        original.statements[0].result.rows[0].values[0].to_string(),
        "2"
    );
    admin
        .execute_sql(&format!("DROP FUNCTION {function}"))
        .await?;
    engine.execute_sql(&format!("DROP TABLE {table}")).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a disposable CockroachDB server via DBX_TEST_COCKROACH_URL"]
async fn cockroach_watch_analyze_and_cancellation_use_native_session() -> dbx_core::Result<()> {
    let url = std::env::var("DBX_TEST_COCKROACH_URL").expect("DBX_TEST_COCKROACH_URL is required");
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::CockroachDB, url)).await?,
    );
    assert_eq!(
        isolated_sample(engine.clone(), "SELECT 7").await?.rows[0].values[0].to_string(),
        "7"
    );
    let plan = QuerySession::explain_analyze(
        engine.clone(),
        "SELECT 7",
        QueryOptions::default(),
        Duration::from_secs(10),
        QueryCancellation::default(),
    )
    .await?;
    assert!(
        plan.rows
            .iter()
            .any(|row| row.values[0].to_string().contains("actual row count"))
    );
    let tree = format_execution_plan(DatabaseKind::CockroachDB, &plan);
    assert_eq!(tree.columns[0].name, "Depth");
    assert!(tree.rows.iter().any(|row| row.values[4] != CellValue::Null));
    let sequence = format!("dbx_watch_seq_{}", std::process::id());
    engine
        .execute_sql(&format!("CREATE SEQUENCE {sequence}"))
        .await?;
    for analyze in [false, true] {
        let sql = format!("SELECT nextval('{sequence}')");
        let result = if analyze {
            QuerySession::explain_analyze(
                engine.clone(),
                &sql,
                QueryOptions::default(),
                Duration::from_secs(10),
                QueryCancellation::default(),
            )
            .await
        } else {
            isolated_sample(engine.clone(), &sql).await
        };
        assert!(result.unwrap_err().to_string().contains("read-only"));
    }
    assert_eq!(
        engine
            .query(
                &format!("SELECT nextval('{sequence}')"),
                QueryOptions::default()
            )
            .await?
            .rows[0]
            .values[0]
            .to_string(),
        "1"
    );
    engine
        .execute_sql(&format!("DROP SEQUENCE {sequence}"))
        .await?;
    let cancellation = QueryCancellation::default();
    let pending = tokio::spawn(QuerySession::read_only_query(
        engine.clone(),
        "SELECT pg_sleep(30)",
        QueryOptions::default(),
        Duration::from_secs(30),
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Server cancellation acknowledged"),
        "{error}"
    );
    let running = engine
        .query(
            "SELECT count(*) FROM [SHOW CLUSTER QUERIES] WHERE query = 'SELECT pg_sleep(30)'",
            QueryOptions::default(),
        )
        .await?;
    assert_eq!(running.rows[0].values[0].to_string(), "0");
    Ok(())
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL server via DBX_TEST_POSTGRES_URL"]
async fn postgres_full_table_statistics_preserve_decimal_and_large_integer_extrema()
-> dbx_core::Result<()> {
    let engine = Arc::new(
        DatabaseEngine::connect(ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            std::env::var("DBX_TEST_POSTGRES_URL").expect("DBX_TEST_POSTGRES_URL is required"),
        ))
        .await?,
    );
    let table = format!("dbx_stats_{}", std::process::id());
    engine.execute_sql(&format!("CREATE TABLE {table}(id BIGINT, amount NUMERIC(40,12), flag BOOLEAN, doc JSONB, ids INTEGER[])")).await?;
    engine.execute_sql(&format!("INSERT INTO {table} VALUES(9007199254740993, 12345678901234567890.123456789012, true, '{{}}', ARRAY[1]), (9007199254740994, NULL, NULL, NULL, NULL)")).await?;
    let columns = engine
        .describe_table(&dbx_core::TableRef::new(&table))
        .await?;
    let sql = dbx_core::table_column_statistics_query(
        DatabaseKind::PostgreSQL,
        &dbx_core::TableRef::new(&table),
        &columns,
    )?;
    let stats = dbx_core::format_table_column_statistics(
        DatabaseKind::PostgreSQL,
        &columns,
        &isolated_sample(engine.clone(), &sql).await?,
    )?;
    assert_eq!(
        stats.rows[0].values[5],
        CellValue::Integer(9007199254740993)
    );
    assert_eq!(
        stats.rows[0].values[6],
        CellValue::Integer(9007199254740994)
    );
    assert_eq!(
        stats.rows[1].values[5].to_string(),
        "12345678901234567890.123456789012"
    );
    assert_eq!(stats.rows[2].values[4].to_string(), "1");
    assert_eq!(stats.rows[2].values[5], CellValue::Null);
    assert!(stats.rows[3].values[7].to_string().contains("unsupported"));
    assert!(stats.rows[4].values[7].to_string().contains("unsupported"));
    engine.execute_sql(&format!("DROP TABLE {table}")).await?;
    Ok(())
}
