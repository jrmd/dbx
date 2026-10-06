use dbx_core::{
    CellValue, ColumnInfo, ConnectionConfig, CreateColumn, CreateTableRequest, DatabaseEngine,
    DatabaseKind, Filter, FilterOperator, InsertRequest, Order, OrderDirection, Page, QueryOptions,
    ReferentialAction, Result, RowData, TableRef, UpdateRequest,
};

const TABLE_NAME: &str = "dbx_integration_rows";
const FOREIGN_KEY_PARENT_TABLE: &str = "dbx_integration_fk_parent";
const FOREIGN_KEY_CHILD_TABLE: &str = "dbx_integration_fk_child";

#[tokio::test]
#[ignore = "run scripts/test-transports.py for disposable databases and SSH server"]
async fn socket_and_ssh_connections_integration() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::var("DBX_TEST_TRANSPORT_DIRECTORY").expect("transport fixtures required"),
    );
    let ssh = dbx_core::SshConfig {
        host: "127.0.0.1".into(),
        port: std::env::var("DBX_TEST_SSH_PORT").unwrap().parse().unwrap(),
        username: std::env::var("DBX_TEST_SSH_USER").unwrap(),
        identity_file: Some(root.join("identity")),
        jump_host: None,
    };
    for (kind, socket) in [
        (DatabaseKind::PostgreSQL, root.join("sockets/pg")),
        (DatabaseKind::MySQL, root.join("sockets/mysql.sock")),
        (DatabaseKind::Redis, root.join("sockets/redis.sock")),
    ] {
        for mode in ["socket", "ssh-tcp", "ssh-socket"] {
            let database = if kind == DatabaseKind::Redis {
                "0"
            } else {
                "dbx_test"
            };
            let userinfo = if kind == DatabaseKind::Redis {
                ""
            } else {
                "dbx_test:dbx_test_password@"
            };
            let address_port = if mode == "ssh-tcp" {
                let name = match kind {
                    DatabaseKind::PostgreSQL => "POSTGRES",
                    DatabaseKind::MySQL => "MYSQL",
                    _ => "REDIS",
                };
                std::env::var(format!("DBX_TEST_TRANSPORT_{name}_PORT"))
                    .unwrap()
                    .parse::<u16>()
                    .unwrap()
            } else {
                match kind {
                    DatabaseKind::PostgreSQL => 5432,
                    DatabaseKind::MySQL => 3306,
                    _ => 6379,
                }
            };
            let mut config = ConnectionConfig::new(
                kind,
                format!(
                    "{}://{userinfo}127.0.0.1:{address_port}/{database}",
                    kind.scheme()
                ),
            );
            if mode != "ssh-tcp" {
                config.socket = Some(socket.clone());
            }
            if mode != "socket" {
                config.ssh = Some(ssh.clone());
            }
            let engine = DatabaseEngine::connect(config).await?;
            let result = engine
                .query(
                    if kind == DatabaseKind::Redis {
                        "PING"
                    } else {
                        "SELECT 42"
                    },
                    QueryOptions::default(),
                )
                .await?;
            assert_eq!(result.rows.len(), 1, "{kind} via {mode}");
            if kind != DatabaseKind::Redis {
                assert_eq!(result.rows[0].values[0], CellValue::Integer(42));
                engine.use_database(database).await?;
                engine.query("SELECT 43", QueryOptions::default()).await?;
            } else {
                engine.use_database("1").await?;
                assert_eq!(engine.current_database().await?, "1");
            }
            eprintln!("verified {kind} via {mode}");
        }
    }
    let mut untrusted = ConnectionConfig::new(
        DatabaseKind::Redis,
        format!(
            "redis://127.0.0.1:{}/0",
            std::env::var("DBX_TEST_TRANSPORT_REDIS_PORT").unwrap()
        ),
    );
    untrusted.ssh = Some(dbx_core::SshConfig {
        host: "localhost".into(),
        ..ssh
    });
    let error = DatabaseEngine::connect(untrusted)
        .await
        .expect_err("unknown host must be rejected");
    assert!(
        error.to_string().contains("Host key verification failed"),
        "{error}"
    );
    Ok(())
}

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

/// Values the row editor sends as text (dates, uuids, enums, arrays, bit
/// strings) and NULLs must land in typed columns, and every column must
/// decode back to the text the editor would show.
#[tokio::test]
#[ignore = "requires the disposable integration databases"]
async fn postgresql_typed_mutation_round_trip() -> Result<()> {
    typed_mutation_round_trip(
        DatabaseKind::PostgreSQL,
        "DBX_TEST_POSTGRES_URL",
        &[
            "DROP TABLE IF EXISTS dbx_integration_types",
            "DROP TYPE IF EXISTS dbx_integration_level",
            "CREATE TYPE dbx_integration_level AS ENUM ('low', 'high')",
            "CREATE TABLE dbx_integration_types (id integer PRIMARY KEY, price numeric(10,2), day date, seen timestamp, token uuid, level dbx_integration_level, address inet, tags integer[], flags bit(3), wait interval, cost money)",
        ],
        &[
            ("price", "4.50"),
            ("day", "2025-02-03"),
            ("seen", "2025-02-03 04:05:06"),
            ("token", "00000000-0000-0000-0000-000000000002"),
            ("level", "high"),
            ("address", "10.0.0.2"),
            ("tags", "{3,4}"),
            ("flags", "011"),
            ("wait", "2 days 01:00:00"),
            ("cost", "5.25"),
        ],
    )
    .await
}

#[tokio::test]
#[ignore = "requires the disposable integration databases"]
async fn mysql_typed_mutation_round_trip() -> Result<()> {
    typed_mutation_round_trip(
        DatabaseKind::MySQL,
        "DBX_TEST_MYSQL_URL",
        &[
            "DROP TABLE IF EXISTS dbx_integration_types",
            "CREATE TABLE dbx_integration_types (id int PRIMARY KEY, price decimal(10,2) NULL, day date NULL, seen timestamp NULL, level enum('low','high') NULL, mask bit(8) NULL)",
        ],
        &[
            ("price", "4.50"),
            ("day", "2025-02-03"),
            ("seen", "2025-02-03 04:05:06"),
            ("level", "high"),
            ("mask", "15"),
        ],
    )
    .await
}

async fn typed_mutation_round_trip(
    kind: DatabaseKind,
    variable: &str,
    setup: &[&str],
    values: &[(&str, &str)],
) -> Result<()> {
    let Some(url) = integration_url(variable) else {
        return Ok(());
    };
    let engine = DatabaseEngine::connect(ConnectionConfig::new(kind, url)).await?;
    for statement in setup {
        engine.execute_sql(statement).await?;
    }
    let table = table_ref_named(kind, "dbx_integration_types");
    let text = |value: &str| CellValue::Text(value.to_owned());
    let (mut columns, mut row) = (vec!["id".to_owned()], vec![CellValue::Integer(1)]);
    for (column, value) in values {
        columns.push((*column).to_owned());
        row.push(text(value));
    }
    engine
        .insert(&InsertRequest::new(table.clone(), columns, row))
        .await?;
    let read_back = |result: &dbx_core::QueryResult, column: &str| {
        let index = result
            .columns
            .iter()
            .position(|metadata| metadata.name == column)
            .unwrap();
        result.rows[0].values[index].to_string()
    };
    let result = engine
        .query_table(&table, &[], &[], &[], None, QueryOptions::default())
        .await?;
    for (column, value) in values {
        assert_eq!(read_back(&result, column), *value, "{column}");
    }
    // Filters send the same editor text. PostgreSQL must type it for the
    // compared column (`uuid = text` does not exist) and match non-character
    // columns against their text rendering for LIKE operators.
    if kind == DatabaseKind::PostgreSQL {
        for (column, value) in values {
            for operator in [FilterOperator::Equals, FilterOperator::Contains] {
                let filtered = engine
                    .query_table(
                        &table,
                        &[],
                        &[Filter::new(*column, operator, Some(text(value)))],
                        &[],
                        None,
                        QueryOptions::default(),
                    )
                    .await?;
                assert_eq!(filtered.rows.len(), 1, "{column} {operator:?}");
            }
        }
    }

    let nulls = values
        .iter()
        .map(|(column, _)| ((*column).to_owned(), CellValue::Null))
        .collect();
    engine
        .update(&UpdateRequest::for_primary_key(
            table.clone(),
            nulls,
            vec![("id".into(), CellValue::Integer(1))],
        ))
        .await?;
    let result = engine
        .query_table(&table, &[], &[], &[], None, QueryOptions::default())
        .await?;
    for (column, _) in values {
        assert_eq!(read_back(&result, column), "NULL", "{column}");
    }
    engine.drop_table(&table).await?;
    Ok(())
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

    assert_schema_details(&engine, kind).await?;
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

    if kind == DatabaseKind::PostgreSQL {
        assert_postgres_enum_decoding(&engine).await?;
    }

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

async fn assert_postgres_enum_decoding(engine: &DatabaseEngine) -> Result<()> {
    let table = qualified_table(DatabaseKind::PostgreSQL);
    engine
        .execute_sql("DROP TYPE IF EXISTS dbx_integration_mood")
        .await?;
    engine
        .execute_sql("CREATE TYPE dbx_integration_mood AS ENUM ('happy', 'sad', 'neutral')")
        .await?;
    let result = std::panic::AssertUnwindSafe(async {
        engine
            .execute_sql(&format!(
                "ALTER TABLE {table} ADD COLUMN mood dbx_integration_mood"
            ))
            .await?;
        let columns = engine
            .describe_table(&table_ref(DatabaseKind::PostgreSQL))
            .await?;
        let mood = columns
            .iter()
            .find(|column| column.name == "mood")
            .expect("enum column should be present in table metadata");
        // format_type qualifies the name only when it is outside search_path,
        // so the spelling is always usable in a mutation cast.
        assert_eq!(mood.data_type, "dbx_integration_mood");
        assert_eq!(mood.enum_values, ["happy", "sad", "neutral"]);
        engine
            .execute_sql(&format!("UPDATE {table} SET mood = 'happy' WHERE id = 1"))
            .await?;

        let rows = engine
            .query(
                &format!("SELECT mood FROM {table} WHERE id = 1"),
                QueryOptions::default(),
            )
            .await?;
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(
            rows.rows[0].values[0],
            CellValue::Text("happy".into()),
            "enum labels should decode as text, not an unsupported-type placeholder"
        );

        let null_rows = engine
            .query(
                &format!("SELECT mood FROM {table} WHERE id = 2"),
                QueryOptions::default(),
            )
            .await?;
        assert_eq!(null_rows.rows[0].values[0], CellValue::Null);
        Ok::<(), dbx_core::DbxError>(())
    })
    .await;
    // The table outlives this check, so its enum column must go first.
    let _ = engine
        .execute_sql(&format!("ALTER TABLE {table} DROP COLUMN mood"))
        .await;
    engine.execute_sql("DROP TYPE dbx_integration_mood").await?;
    result
}

/// Defaults, indexes, checks, and view definitions, which the structure tab,
/// SQL dumps, and schema comparison all rely on.
async fn assert_schema_details(engine: &DatabaseEngine, kind: DatabaseKind) -> Result<()> {
    let table = table_ref_named(kind, "dbx_integration_details");
    let view = qualified_table_named(kind, "dbx_integration_details_view");
    let _ = engine
        .execute_sql(&format!("DROP VIEW IF EXISTS {view}"))
        .await;
    let _ = engine.drop_table(&table).await;
    let name = qualified_table_named(kind, "dbx_integration_details");
    engine
        .execute_sql(&format!(
            "CREATE TABLE {name} (id INTEGER PRIMARY KEY, code VARCHAR(20) NOT NULL DEFAULT 'new', qty INTEGER NOT NULL DEFAULT 1, CONSTRAINT dbx_integration_qty CHECK (qty >= 0))"
        ))
        .await?;
    engine
        .execute_sql(&format!(
            "CREATE UNIQUE INDEX dbx_integration_code ON {name} (code)"
        ))
        .await?;
    engine
        .execute_sql(&format!(
            "CREATE INDEX dbx_integration_qty_code ON {name} (qty, code DESC)"
        ))
        .await?;
    engine
        .execute_sql(&format!(
            "CREATE VIEW {view} AS SELECT id, code FROM {name}"
        ))
        .await?;

    if kind == DatabaseKind::MySQL {
        engine
            .execute_sql(&format!(
                "CREATE INDEX dbx_integration_expression ON {name} ((LOWER(code)))"
            ))
            .await?;
    }
    let structure = engine.table_structure(&table).await?;
    let default = |column: &str| {
        structure
            .columns
            .iter()
            .find(|candidate| candidate.name == column)
            .and_then(|candidate| candidate.default_value.clone())
            .unwrap_or_default()
    };
    assert!(default("code").contains("new"), "{:?}", default("code"));
    assert!(default("qty").contains('1'), "{:?}", default("qty"));
    assert_eq!(
        structure
            .columns
            .iter()
            .find(|column| column.name == "id")
            .and_then(|column| column.default_value.as_ref()),
        None
    );

    if kind == DatabaseKind::MySQL {
        let expression = structure
            .indexes
            .iter()
            .find(|index| index.name == "dbx_integration_expression")
            .expect("functional index");
        assert!(
            expression
                .definition
                .as_deref()
                .unwrap()
                .to_ascii_lowercase()
                .contains("lower")
        );
        let rendered = dbx_core::render_sql_indexes(kind, &table, &structure)?;
        let sql = rendered
            .iter()
            .find(|sql| sql.contains("dbx_integration_expression"))
            .unwrap();
        engine
            .execute_sql(&format!("DROP INDEX dbx_integration_expression ON {name}"))
            .await?;
        engine.execute_sql(sql).await?;
        assert!(
            engine
                .table_structure(&table)
                .await?
                .indexes
                .iter()
                .any(|index| index.name == "dbx_integration_expression")
        );
    }
    let unique = structure
        .indexes
        .iter()
        .find(|index| index.name == "dbx_integration_code")
        .expect("unique index");
    assert!(unique.unique && !unique.primary);
    assert_eq!(unique.columns, ["code"]);
    let composite = structure
        .indexes
        .iter()
        .find(|index| index.name == "dbx_integration_qty_code")
        .expect("composite index");
    assert!(!composite.unique);
    assert_eq!(composite.columns[0], "qty");
    assert!(composite.columns[1].starts_with("code"));
    if kind != DatabaseKind::SQLite {
        assert!(structure.indexes.iter().any(|index| index.primary));
    }

    if kind == DatabaseKind::SQLite {
        let definition = structure
            .definition
            .as_deref()
            .expect("SQLite CREATE TABLE");
        assert!(definition.contains("CHECK (qty >= 0)"), "{definition}");
    } else {
        assert_eq!(structure.checks.len(), 1, "{:?}", structure.checks);
        assert_eq!(
            structure.checks[0].name.as_deref(),
            Some("dbx_integration_qty")
        );
        assert!(structure.checks[0].expression.contains("qty"));
        assert!(structure.checks[0].expression.contains(">="));
    }

    let view_structure = engine
        .table_structure(&table_ref_named(kind, "dbx_integration_details_view"))
        .await?;
    let view_definition = view_structure
        .definition
        .expect("view definition")
        .to_ascii_lowercase();
    assert!(view_definition.contains("select"), "{view_definition}");
    assert!(view_definition.contains("code"), "{view_definition}");
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

    // The bulk snapshot must agree with the per-table queries it replaces.
    let snapshot = engine.relational_schema().await?;
    for entry in &snapshot.tables {
        let table = TableRef {
            schema: entry.table.schema.clone(),
            name: entry.table.name.clone(),
        };
        assert_eq!(
            entry.structure,
            engine.table_structure(&table).await?,
            "bulk structure differs for {}",
            entry.table.name
        );
    }
    let bulk_child = snapshot
        .tables
        .iter()
        .find(|entry| entry.table.name == FOREIGN_KEY_CHILD_TABLE)
        .expect("child table in relational schema");
    assert_eq!(bulk_child.structure.foreign_keys, structure.foreign_keys);
    assert_eq!(bulk_child.structure.columns.len(), 2);

    if kind == DatabaseKind::PostgreSQL {
        // A foreign key into another schema keeps that schema's name.
        engine
            .execute_sql("DROP SCHEMA IF EXISTS dbx_integration_other CASCADE")
            .await?;
        engine
            .execute_sql("CREATE SCHEMA dbx_integration_other")
            .await?;
        engine
            .execute_sql("CREATE TABLE dbx_integration_other.owners (id INTEGER PRIMARY KEY)")
            .await?;
        engine
            .execute_sql(&format!(
                "CREATE TABLE {} (owner_id INTEGER REFERENCES dbx_integration_other.owners (id))",
                qualified_table_named(kind, "dbx_integration_fk_cross"),
            ))
            .await?;
        let cross = engine
            .table_structure(&table_ref_named(kind, "dbx_integration_fk_cross"))
            .await?;
        assert_eq!(cross.foreign_keys.len(), 1);
        let cross_key = &cross.foreign_keys[0];
        assert_eq!(
            cross_key.referenced_schema.as_deref(),
            Some("dbx_integration_other")
        );
        assert_eq!(cross_key.referenced_table, "owners");
        assert_eq!(cross_key.referenced_columns, ["id"]);
        assert_eq!(cross_key.on_delete, Some(ReferentialAction::NoAction));
        let snapshot = engine.relational_schema().await?;
        let bulk_cross = snapshot
            .tables
            .iter()
            .find(|entry| entry.table.name == "dbx_integration_fk_cross")
            .expect("cross-schema table in relational schema");
        assert_eq!(bulk_cross.structure, cross);
        assert!(snapshot.tables.iter().any(|entry| {
            entry.table.schema.as_deref() == Some("dbx_integration_other")
                && entry.table.name == "owners"
        }));
        engine
            .execute_sql(&format!(
                "DROP TABLE {}",
                qualified_table_named(kind, "dbx_integration_fk_cross")
            ))
            .await?;
        engine
            .execute_sql("DROP SCHEMA dbx_integration_other CASCADE")
            .await?;
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
        _ => unreachable!("This fixture covers PostgreSQL, MySQL and SQLite"),
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
