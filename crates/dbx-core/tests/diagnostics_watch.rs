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
