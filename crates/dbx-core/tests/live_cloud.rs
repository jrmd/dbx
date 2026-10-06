//! Opt-in real provider checks. Credentials stay in process environment.
use dbx_core::*;

async fn sqlite_cloud_contract(kind: DatabaseKind, variable: &str) {
    let url = std::env::var(variable).expect("Set the provider URL for a disposable test database");
    let engine = DatabaseEngine::connect(ConnectionConfig::new(kind, url))
        .await
        .unwrap();
    engine.execute_sql("CREATE TABLE dbx_cloud_audit (id INTEGER PRIMARY KEY, value TEXT DEFAULT 'default', document TEXT)").await.unwrap();
    engine
        .execute_sql("CREATE UNIQUE INDEX dbx_cloud_audit_value ON dbx_cloud_audit(value)")
        .await
        .unwrap();
    let dangerous_text = "quote '; DROP TABLE dbx_cloud_audit; --";
    engine
        .execute(&SqlStatement::new(
            "INSERT INTO dbx_cloud_audit(id,value,document) VALUES (?,?,?)",
            vec![
                CellValue::Integer(1),
                CellValue::Text(dangerous_text.into()),
                CellValue::Text("{\"checked\":true}".into()),
            ],
        ))
        .await
        .unwrap();
    let result = engine
        .query_statement(
            &SqlStatement::new(
                "SELECT value FROM dbx_cloud_audit WHERE id=?",
                vec![CellValue::Integer(1)],
            ),
            QueryOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.rows[0].values[0],
        CellValue::Text(dangerous_text.into())
    );
    let table = TableRef::new("dbx_cloud_audit");
    let structure = engine.table_structure(&table).await.unwrap();
    assert!(
        structure
            .columns
            .iter()
            .any(|column| column.name == "value" && column.default_value.is_some())
    );
    assert!(
        structure
            .indexes
            .iter()
            .any(|index| index.name == "dbx_cloud_audit_value" && index.unique)
    );
    let request = UpdateRequest::for_primary_key(
        table.clone(),
        vec![("value".into(), CellValue::Text("persisted".into()))],
        vec![("id".into(), CellValue::Integer(1))],
    );
    engine
        .update_checked(
            &request,
            &[("value".into(), CellValue::Text(dangerous_text.into()))],
        )
        .await
        .unwrap();
    assert!(matches!(
        engine
            .update_checked(
                &request,
                &[("value".into(), CellValue::Text(dangerous_text.into()))]
            )
            .await,
        Err(DbxError::Conflict)
    ));
    let result = engine
        .query("SELECT value FROM dbx_cloud_audit", QueryOptions::default())
        .await
        .unwrap();
    assert_eq!(
        result.rows[0].values[0],
        CellValue::Text("persisted".into())
    );
    engine.drop_table(&table).await.unwrap();
    assert!(
        !engine
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|table| table.name == "dbx_cloud_audit")
    );
}

#[tokio::test]
#[ignore = "Requires a disposable live D1 database"]
async fn d1_live_bound_values_metadata_and_conflict_checked_persistence() {
    sqlite_cloud_contract(DatabaseKind::CloudflareD1, "DBX_TEST_D1_URL").await;
}

#[tokio::test]
#[ignore = "Requires a disposable live Turso database"]
async fn turso_live_bound_values_metadata_and_conflict_checked_persistence() {
    sqlite_cloud_contract(DatabaseKind::Turso, "DBX_TEST_TURSO_URL").await;
}
