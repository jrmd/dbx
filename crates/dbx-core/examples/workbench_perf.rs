//! Fixed, disposable workbench workload. Measures core paths, not GUI latency.
use dbx_core::*;
use serde_json::json;
use std::{sync::Arc, time::Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let output = std::env::args()
        .nth(1)
        .ok_or_else(|| DbxError::Parse("Usage: workbench_perf report.json".into()))?;
    let directory = tempfile::tempdir().map_err(|e| DbxError::Io(e.to_string()))?;
    let config = ConnectionConfig::new(
        DatabaseKind::SQLite,
        format!(
            "sqlite://{}?mode=rwc",
            directory.path().join("workload.sqlite").display()
        ),
    );
    let start = Instant::now();
    let engine = Arc::new(DatabaseEngine::connect(config.clone()).await?);
    let connect_ms = start.elapsed().as_millis();
    engine
        .execute_sql(
            "CREATE TABLE workload(id INTEGER PRIMARY KEY, category INTEGER, payload TEXT)",
        )
        .await?;
    engine.execute_sql("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO workload SELECT x, x%100, printf('%01024d', x) FROM n").await?;
    for index in 0..1000 {
        engine
            .execute_sql(&format!(
                "CREATE TABLE schema_{index}(id INTEGER PRIMARY KEY, value TEXT)"
            ))
            .await?;
    }
    let start = Instant::now();
    let tables = engine.list_tables().await?;
    let discover_ms = start.elapsed().as_millis();
    assert_eq!(tables.len(), 1001);
    let start = Instant::now();
    let rows = engine
        .query(
            "SELECT * FROM workload ORDER BY id LIMIT 1000 OFFSET 90000",
            QueryOptions::default(),
        )
        .await?;
    let deep_offset_ms = start.elapsed().as_millis();
    assert_eq!(rows.rows.len(), 1000);
    let start = Instant::now();
    let rows = engine
        .query(
            "SELECT * FROM workload WHERE id>90000 ORDER BY id LIMIT 1000",
            QueryOptions::default(),
        )
        .await?;
    let deep_keyset_ms = start.elapsed().as_millis();
    assert_eq!(rows.rows.len(), 1000);
    let start = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let engine = engine.clone();
        tasks.push(tokio::spawn(async move {
            engine
                .query(
                    "SELECT * FROM workload WHERE id>50000 LIMIT 1000",
                    QueryOptions::default(),
                )
                .await
        }));
    }
    for task in tasks {
        assert_eq!(
            task.await
                .map_err(|e| DbxError::Io(e.to_string()))??
                .rows
                .len(),
            1000
        );
    }
    let concurrent_ms = start.elapsed().as_millis();
    let start = Instant::now();
    let report = export_query(
        &engine,
        "SELECT * FROM workload ORDER BY id",
        &directory.path().join("all.csv"),
        QueryExportFormat::Csv,
    )
    .await?;
    let export_ms = start.elapsed().as_millis();
    assert_eq!(report, 100000);
    let start = Instant::now();
    let _reconnected = DatabaseEngine::connect(config).await?;
    let reconnect_ms = start.elapsed().as_millis();
    let peak_rss_kib = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
        });
    let report = json!({"version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"scope":"core paths; excludes GUI, cold application start and physical input","workload":{"tables":1001,"rows":100000,"payloadBytes":1024,"concurrentReaders":8},"milliseconds":{"connect":connect_ms,"schemaDiscovery":discover_ms,"deepOffset":deep_offset_ms,"deepKeyset":deep_keyset_ms,"concurrentReads":concurrent_ms,"fullQueryExport":export_ms,"reconnect":reconnect_ms},"peakRssKiB":peak_rss_kib});
    write_atomic_export(std::path::Path::new(&output), report.to_string().as_bytes())?;
    // Broad absolute budgets catch hangs/regressions, without claiming a
    // comparative advantage on variable CI hardware.
    assert!(
        connect_ms < 10_000
            && discover_ms < 10_000
            && deep_offset_ms < 10_000
            && deep_keyset_ms < 10_000
            && concurrent_ms < 30_000
            && export_ms < 60_000
            && reconnect_ms < 10_000,
        "Workload exceeded its absolute budget: {report}"
    );
    println!("{report}");
    Ok(())
}
