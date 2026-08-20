use dbx_core::{
    CellValue, ColumnInfo, ConnectionConfig, CreateColumn, CreateTableRequest, DatabaseEngine,
    DatabaseKind, Filter, FilterOperator, InsertRequest, Order, OrderDirection, Page, QueryOptions,
    ReferentialAction, Result, RowData, TableRef, UpdateRequest,
};

const TABLE_NAME: &str = "dbx_integration_rows";
const FOREIGN_KEY_PARENT_TABLE: &str = "dbx_integration_fk_parent";
const FOREIGN_KEY_CHILD_TABLE: &str = "dbx_integration_fk_child";

#[tokio::test]
#[ignore = "requires the disposable integration databases"]
async fn postgresql_crud_integration() -> Result<()> {
    run_sql_scenario(DatabaseKind::PostgreSQL, "DBX_TEST_POSTGRES_URL").await
}

#[tokio::test]
#[ignore = "requires the disposable integration databases"]
async fn mysql_crud_integration() -> Result<()> {
    run_sql_scenario(DatabaseKind::MySQL, "DBX_TEST_MYSQL_URL").await
}

#[tokio::test]
#[ignore = "requires the disposable integration databases"]
async fn sqlite_file_crud_integration() -> Result<()> {
    run_sql_scenario(DatabaseKind::SQLite, "DBX_TEST_SQLITE_URL").await
}

#[tokio::test]
#[ignore = "requires the disposable integration databases"]
async fn redis_scan_type_ttl_and_commands_integration() -> Result<()> {
    let Some(url) = integration_url("DBX_TEST_REDIS_URL") else {
        return Ok(());
    };

    let engine = DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::Redis, url)).await?;
    let tables = engine.list_tables().await?;
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "keys");

    let prefix = format!("dbx:integration:{}:", std::process::id());
    let string_key = format!("{prefix}string");
    let hash_key = format!("{prefix}hash");

    // A process-specific prefix keeps this test safe when DBX_TEST_REDIS_URL
    // points at a shared development Redis rather than the compose service.
    let _ = engine
        .execute_sql(&format!("DEL {string_key} {hash_key}"))
        .await;
    let set = engine
        .execute_sql(&format!("SET {string_key} hello"))
        .await?;
    assert_eq!(set.rows_affected, 1);
    engine
        .execute_sql(&format!("EXPIRE {string_key} 60"))
        .await?;
    engine
        .execute_sql(&format!("HSET {hash_key} field value"))
        .await?;

    // Raw commands remain available in the Redis console.
    let get = engine
        .query(&format!("GET {string_key}"), QueryOptions::default())
        .await?;
    assert_eq!(get.rows.len(), 1);
    assert_eq!(get.rows[0].values, vec![CellValue::Text("hello".into())]);

    let hash = engine
        .query(&format!("HGETALL {hash_key}"), QueryOptions::default())
        .await?;
    assert_eq!(hash.rows.len(), 1);
    assert_eq!(hash.rows[0].values[0], CellValue::Text("field".into()));
    assert_eq!(hash.rows[0].values[1], CellValue::Text("value".into()));

    let scan = engine
        .query(
            &format!("SCAN 0 MATCH {prefix}* COUNT 100"),
            QueryOptions {
                max_rows: Some(100),
            },
        )
        .await?;
    assert_eq!(column_names(&scan.columns), ["key", "type", "ttl"]);
    assert!(scan.rows.len() >= 2, "SCAN should return both test keys");

    let string_row = find_row(&scan.rows, &string_key).expect("SET key should be in SCAN");
    assert_eq!(string_row.values[1], CellValue::Text("string".into()));
    assert!(
        integer_value(&string_row.values[2]) > 0,
        "SET key should have a TTL"
    );

    let hash_row = find_row(&scan.rows, &hash_key).expect("HSET key should be in SCAN");
    assert_eq!(hash_row.values[1], CellValue::Text("hash".into()));
    assert_eq!(
        integer_value(&hash_row.values[2]),
        -1,
        "hash key should be persistent"
    );

    let _ = engine
        .execute_sql(&format!("DEL {string_key} {hash_key}"))
        .await;
    Ok(())
}

async fn run_sql_scenario(kind: DatabaseKind, variable: &str) -> Result<()> {
    let Some(url) = integration_url(variable) else {
        return Ok(());
    };

    let engine = DatabaseEngine::connect(
        ConnectionConfig::new(kind, url)
            .with_max_connections(2)
            .with_connect_timeout_ms(10_000),
    )
    .await?;
    let table = table_ref(kind);

    // Make reruns deterministic without touching any table outside this
    // fixed integration-test name.
    let _ = engine.drop_table(&table).await;
    let before = engine.list_tables().await?;
    assert!(!before.iter().any(|item| item.name == TABLE_NAME));

    let created = engine
        .create_table(&CreateTableRequest {
            table: table.clone(),
            columns: vec![
                CreateColumn {
                    name: "id".into(),
                    data_type: "INTEGER".into(),
                    nullable: false,
                    primary_key: true,
                    default_expression: None,
                },
                CreateColumn {
                    name: "name".into(),
                    data_type: "TEXT".into(),
                    nullable: false,
                    primary_key: false,
                    default_expression: None,
                },
                CreateColumn {
                    name: "score".into(),
                    data_type: "INTEGER".into(),
                    nullable: false,
                    primary_key: false,
                    default_expression: None,
                },
                CreateColumn {
                    name: "note".into(),
                    data_type: "TEXT".into(),
                    nullable: true,
                    primary_key: false,
                    default_expression: None,
                },
            ],
            if_not_exists: false,
        })
        .await?;
    assert_eq!(created.rows_affected, 0);

    let tables = engine.list_tables().await?;
    let discovered = tables
        .iter()
        .find(|item| item.name == TABLE_NAME)
        .expect("created table should be discoverable");
    if kind == DatabaseKind::PostgreSQL {
        assert_eq!(discovered.schema.as_deref(), Some("public"));
    }

    let columns = engine.describe_table(&table).await?;
    assert_eq!(column_names(&columns), ["id", "name", "score", "note"]);
    assert!(columns[0].primary_key);
    assert!(!columns[0].nullable);
    assert!(!columns[1].nullable);

    assert_foreign_key_structure(&engine, kind).await?;

    for (id, name, score) in [(1_i64, "Ada", 10_i64), (2, "Grace", 20), (3, "Linus", 30)] {
        let result = engine
            .insert(&InsertRequest::from_row(
                table.clone(),
                vec![
                    ("id".into(), CellValue::Integer(id)),
                    ("name".into(), CellValue::Text(name.into())),
                    ("score".into(), CellValue::Integer(score)),
                    ("note".into(), CellValue::Null),
                ],
            ))
            .await?;
        assert_eq!(result.rows_affected, 1);
    }

    let all = engine
        .query_table(
            &table,
            &[],
            &[],
            &[Order {
                column: "id".into(),
                direction: OrderDirection::Ascending,
            }],
            Some(Page {
                limit: 10,
                offset: 0,
            }),
            QueryOptions::default(),
        )
        .await?;
    assert_eq!(all.rows.len(), 3);
    assert_eq!(integer_value(&all.rows[0].values[0]), 1);
    assert_eq!(all.rows[0].values[1], CellValue::Text("Ada".into()));

    // This exercises the GUI-style LIKE filter path, including parameter
    // binding and dialect-specific placeholders.
    let filtered = engine
        .query_table(
            &table,
            &[],
            &[Filter::new(
                "name",
                FilterOperator::Contains,
                Some(CellValue::Text("ra".into())),
            )],
            &[],
            None,
            QueryOptions::default(),
        )
        .await?;
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0].values[1], CellValue::Text("Grace".into()));

    let raw = engine
        .query(
            &format!("SELECT COUNT(*) AS count FROM {}", qualified_table(kind)),
            QueryOptions::default(),
        )
        .await?;
    assert_eq!(integer_value(&raw.rows[0].values[0]), 3);

    let updated = engine
        .update(&UpdateRequest::for_primary_key(
            table.clone(),
            vec![
                ("score".into(), CellValue::Integer(99)),
                ("name".into(), CellValue::Text("Grace Hopper".into())),
                ("note".into(), CellValue::Null),
            ],
            vec![("id".into(), CellValue::Integer(2))],
        ))
        .await?;
    assert_eq!(updated.rows_affected, 1);
    let row = engine
        .query_table(
            &table,
            &[],
            &[Filter::new(
                "id",
                FilterOperator::Equals,
                Some(CellValue::Integer(2)),
            )],
            &[],
            None,
            QueryOptions::default(),
        )
        .await?;
    assert_eq!(integer_value(&row.rows[0].values[2]), 99);
    assert_eq!(
        row.rows[0].values[1],
        CellValue::Text("Grace Hopper".into())
    );
    assert_eq!(row.rows[0].values[3], CellValue::Null);

    let deleted = engine
        .delete(
            &table,
            &[Filter::new(
                "id",
                FilterOperator::Equals,
                Some(CellValue::Integer(3)),
            )],
        )
        .await?;
    assert_eq!(deleted.rows_affected, 1);
    let remaining = engine
        .query_table(&table, &[], &[], &[], None, QueryOptions::default())
        .await?;
    assert_eq!(remaining.rows.len(), 2);

    engine.truncate_table(&table).await?;
    let empty = engine
        .query_table(&table, &[], &[], &[], None, QueryOptions::default())
        .await?;
    assert!(empty.rows.is_empty());

    engine.drop_table(&table).await?;
    let after_drop = engine.list_tables().await?;
    assert!(!after_drop.iter().any(|item| item.name == TABLE_NAME));
    Ok(())
}

async fn assert_foreign_key_structure(engine: &DatabaseEngine, kind: DatabaseKind) -> Result<()> {
    let parent = table_ref_named(kind, FOREIGN_KEY_PARENT_TABLE);
    let child = table_ref_named(kind, FOREIGN_KEY_CHILD_TABLE);
    let _ = engine.drop_table(&child).await;
    let _ = engine.drop_table(&parent).await;
    engine
        .execute_sql(&format!(
            "CREATE TABLE {} (id INTEGER NOT NULL, account_id INTEGER NOT NULL, PRIMARY KEY (id, account_id))",
            qualified_table_named(kind, FOREIGN_KEY_PARENT_TABLE),
        ))
        .await?;
    engine
        .execute_sql(&format!(
            "CREATE TABLE {} (project_id INTEGER, account_id INTEGER, CONSTRAINT dbx_integration_fk FOREIGN KEY (project_id, account_id) REFERENCES {} (id, account_id) ON UPDATE CASCADE ON DELETE SET NULL)",
            qualified_table_named(kind, FOREIGN_KEY_CHILD_TABLE),
            qualified_table_named(kind, FOREIGN_KEY_PARENT_TABLE),
        ))
        .await?;

    let structure = engine.table_structure(&child).await?;
    assert_eq!(structure.foreign_keys.len(), 1);
    let foreign_key = &structure.foreign_keys[0];
    assert_eq!(foreign_key.columns, ["project_id", "account_id"]);
    assert_eq!(foreign_key.referenced_table, FOREIGN_KEY_PARENT_TABLE);
    assert_eq!(foreign_key.referenced_columns, ["id", "account_id"]);
    assert_eq!(foreign_key.on_update, Some(ReferentialAction::Cascade));
    assert_eq!(foreign_key.on_delete, Some(ReferentialAction::SetNull));
    if kind == DatabaseKind::SQLite {
        assert_eq!(foreign_key.constraint_name, None);
        assert_eq!(foreign_key.referenced_schema, None);
    } else {
        assert_eq!(
            foreign_key.constraint_name.as_deref(),
            Some("dbx_integration_fk")
        );
        assert!(foreign_key.referenced_schema.is_some());
    }

    engine.drop_table(&child).await?;
    engine.drop_table(&parent).await?;
    Ok(())
}

fn integration_url(variable: &str) -> Option<String> {
    match std::env::var(variable) {
        Ok(url) if !url.trim().is_empty() => Some(url),
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            eprintln!("skipping integration test: {variable} is not set");
            None
        }
        Err(error) => panic!("unable to read {variable}: {error}"),
    }
}

fn table_ref(kind: DatabaseKind) -> TableRef {
    table_ref_named(kind, TABLE_NAME)
}

fn table_ref_named(kind: DatabaseKind, name: &str) -> TableRef {
    if kind == DatabaseKind::PostgreSQL {
        TableRef::in_schema("public", name)
    } else {
        TableRef::new(name)
    }
}

fn qualified_table(kind: DatabaseKind) -> String {
    qualified_table_named(kind, TABLE_NAME)
}

fn qualified_table_named(kind: DatabaseKind, name: &str) -> String {
    match kind {
        DatabaseKind::PostgreSQL => format!("\"public\".\"{name}\""),
        DatabaseKind::MySQL => name.to_owned(),
        DatabaseKind::SQLite => format!("\"{name}\""),
        DatabaseKind::Redis => unreachable!("Redis does not use SQL tables"),
    }
}

fn column_names(columns: &[ColumnInfo]) -> Vec<&str> {
    columns.iter().map(|column| column.name.as_str()).collect()
}

fn find_row<'a>(rows: &'a [RowData], key: &str) -> Option<&'a RowData> {
    rows.iter()
        .find(|row| matches!(row.values.first(), Some(CellValue::Text(value)) if value == key))
}

fn integer_value(value: &CellValue) -> i64 {
    match value {
        CellValue::Integer(value) => *value,
        CellValue::Unsigned(value) => *value as i64,
        CellValue::Text(value) => value.parse().expect("integer cell value"),
        other => panic!("expected integer cell value, got {other:?}"),
    }
}
