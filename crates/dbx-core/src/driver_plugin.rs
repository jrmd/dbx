//! Explicitly registered external database drivers, protocol version 1.
//! One bounded JSON request/response per process. Credentials travel only on
//! stdin; executable paths and arguments are fixed by a trusted manifest.
use crate::{
    ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, ExecResult, QueryOptions,
    QueryResult, Result, SqlStatement, TableInfo, TableRef, TableStructure,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const DRIVER_PROTOCOL_VERSION: u32 = 1;
pub const DRIVER_MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DriverManifest {
    pub protocol: u32,
    pub id: String,
    pub name: String,
    /// SQL dialect or document/key-value UI this driver is compatible with.
    pub family: DatabaseKind,
    pub executable: PathBuf,
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Accepted URL schemes. Allows new community database names/schemes.
    pub schemes: Vec<String>,
}
impl DriverManifest {
    pub fn read(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(64 * 1024 + 1).read_to_end(&mut bytes))
            .map_err(|_| {
                DbxError::InvalidConfig("Cannot read registered driver manifest".into())
            })?;
        if bytes.len() > 64 * 1024 {
            return Err(DbxError::InvalidConfig(
                "Driver manifest exceeds 64 KiB".into(),
            ));
        }
        let manifest: Self = serde_json::from_slice(&bytes)
            .map_err(|_| DbxError::InvalidConfig("Invalid driver manifest JSON".into()))?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn validate(&self) -> Result<()> {
        if self.protocol != DRIVER_PROTOCOL_VERSION
            || self.id.is_empty()
            || self.id.len() > 80
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !(self.family.is_sql() || self.family == DatabaseKind::MongoDB)
            || self.name.trim().is_empty()
            || self.name.len() > 160
            || !self.executable.is_absolute()
            || self.arguments.len() > 32
            || self
                .arguments
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            || self.schemes.is_empty()
            || self.schemes.iter().any(|scheme| {
                scheme.is_empty()
                    || !scheme
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
            })
        {
            return Err(DbxError::InvalidConfig(
                "Invalid driver protocol, id, name, executable, arguments or URL schemes".into(),
            ));
        }
        Ok(())
    }
}
pub(crate) fn validate_config(config: &ConnectionConfig) -> Result<DriverManifest> {
    let manifest = DriverManifest::read(
        config
            .driver_manifest
            .as_deref()
            .ok_or_else(|| DbxError::InvalidConfig("Missing driver manifest".into()))?,
    )?;
    if manifest.family != config.kind {
        return Err(DbxError::InvalidConfig(
            "Driver compatibility family does not match connection".into(),
        ));
    }
    let url = url::Url::parse(&config.url)
        .map_err(|_| DbxError::InvalidConfig("Invalid driver connection URL".into()))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(DbxError::InvalidConfig("Driver URLs cannot contain query parameters or fragments. Store credentials in the URL password field so the vault can protect them.".into()));
    }
    if !manifest.schemes.iter().any(|scheme| scheme == url.scheme())
        || config.max_connections == 0
        || config.connect_timeout_ms == 0
        || config.connect_timeout_ms > 300_000
    {
        return Err(DbxError::InvalidConfig(
            "URL scheme or connection limits do not match this driver".into(),
        ));
    }
    if config.ssh.is_some() || config.socket.is_some() || config.cloud_auth.is_some() {
        return Err(DbxError::InvalidConfig("Driver plugins currently use connection URLs; native socket, SSH and cloud-auth settings are unavailable".into()));
    }
    Ok(manifest)
}

/// Protocol v1 provides bounded query and normalized catalog operations, not
/// native transaction, backup, streaming, SSH, or staged edit guarantees.
pub fn capabilities(family: DatabaseKind, protected: bool) -> crate::Capabilities {
    crate::Capabilities {
        sql: family.is_sql(),
        row_edits: false,
        atomic_row_changes: false,
        independent_query_sessions: false,
        structured_filters: family.is_sql(),
        streaming_query_export: false,
        atomic_file_import: false,
        native_backup: false,
        ssh: false,
        protected,
    }
}
#[cfg(unix)]
struct ProcessGroup(u32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

pub(crate) struct PluginEngine {
    manifest: DriverManifest,
    config: ConnectionConfig,
    database: tokio::sync::RwLock<Option<String>>,
}
impl PluginEngine {
    pub(crate) async fn connect(config: ConnectionConfig) -> Result<Self> {
        let manifest = validate_config(&config)?;
        let engine = Self {
            manifest,
            config,
            database: tokio::sync::RwLock::new(None),
        };
        let database: String = engine.call("connect", json!({})).await?;
        *engine.database.write().await = Some(database);
        Ok(engine)
    }
    fn validate_query(&self, sql: &str) -> Result<()> {
        if self.manifest.family.is_sql() {
            for statement in crate::checked_split_sql_for(Some(self.manifest.family), sql)? {
                let keyword = crate::sqlx_engine::top_level_operation_keyword(&statement)
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                if [
                    "BEGIN",
                    "START",
                    "COMMIT",
                    "END",
                    "ROLLBACK",
                    "SAVEPOINT",
                    "RELEASE",
                ]
                .contains(&keyword.as_str())
                {
                    return Err(DbxError::Query(
                        "Driver protocol v1 does not support persistent transaction statements"
                            .into(),
                    ));
                }
            }
        }
        if self.config.read_only {
            crate::protected::ensure_query(self.kind(), sql)?;
        }
        Ok(())
    }
    async fn call<T: DeserializeOwned>(&self, operation: &str, parameters: Value) -> Result<T> {
        let request = json!({"protocol":DRIVER_PROTOCOL_VERSION,"operation":operation,"connection":{"url":self.config.url,"read_only":self.config.read_only,"database":self.database.read().await.clone()},"parameters":parameters});
        let bytes = serde_json::to_vec(&request)
            .map_err(|_| DbxError::Parse("Cannot encode driver request".into()))?;
        if bytes.len() as u64 > DRIVER_MAX_RESPONSE_BYTES {
            return Err(DbxError::Parse("Driver request exceeds 16 MiB".into()));
        }
        let task = async {
            let mut command = tokio::process::Command::new(&self.manifest.executable);
            command
                .args(&self.manifest.arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(unix)]
            command.process_group(0);
            let mut child = command.spawn().map_err(|_| {
                DbxError::Connection("Cannot start the registered driver executable".into())
            })?;
            #[cfg(unix)]
            let _group = ProcessGroup(
                child
                    .id()
                    .ok_or_else(|| DbxError::Driver("Driver process did not start".into()))?,
            );
            let mut input = child
                .stdin
                .take()
                .ok_or_else(|| DbxError::Driver("Driver stdin unavailable".into()))?;
            let output = child
                .stdout
                .take()
                .ok_or_else(|| DbxError::Driver("Driver stdout unavailable".into()))?;
            // Read and write concurrently so a child cannot deadlock by
            // responding before consuming the complete request.
            let write = async {
                input.write_all(&bytes).await?;
                input.shutdown().await?;
                drop(input);
                Ok::<(), std::io::Error>(())
            };
            let read = async {
                let mut bytes = Vec::new();
                output
                    .take(DRIVER_MAX_RESPONSE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                Ok::<_, std::io::Error>(bytes)
            };
            let (_, bytes) = tokio::try_join!(write, read)
                .map_err(|_| DbxError::Driver("Driver transport failed".into()))?;
            if bytes.len() as u64 > DRIVER_MAX_RESPONSE_BYTES {
                return Err(DbxError::Driver("Driver response exceeds 16 MiB".into()));
            }
            if !child
                .wait()
                .await
                .map_err(|_| DbxError::Driver("Driver process failed".into()))?
                .success()
            {
                return Err(DbxError::Driver(
                    "Driver process exited unsuccessfully".into(),
                ));
            }
            let response: Value = serde_json::from_slice(&bytes)
                .map_err(|_| DbxError::Decode("Driver returned invalid JSON".into()))?;
            if response["protocol"] != DRIVER_PROTOCOL_VERSION || response["ok"] != true {
                return Err(DbxError::Driver(
                    "The driver rejected this operation. Consult the driver's local diagnostics."
                        .into(),
                ));
            }
            serde_json::from_value(response["result"].clone())
                .map_err(|_| DbxError::Decode("Driver returned an invalid result shape".into()))
        };
        tokio::time::timeout(Duration::from_millis(self.config.connect_timeout_ms), task)
            .await
            .map_err(|_| DbxError::Connection("Driver operation timed out".into()))?
    }
}
#[async_trait]
impl Engine for PluginEngine {
    fn capabilities(&self) -> crate::Capabilities {
        capabilities(self.manifest.family, self.config.read_only)
    }
    fn kind(&self) -> DatabaseKind {
        self.manifest.family
    }
    fn is_read_only(&self) -> bool {
        self.config.read_only
    }
    async fn schema_objects(&self) -> Result<Vec<crate::SchemaObject>> {
        Ok(Vec::new())
    }
    async fn relational_schema(&self) -> Result<crate::RelationalSchema> {
        let mut tables = Vec::new();
        for table in self.list_tables().await? {
            let structure = self
                .table_structure(&TableRef {
                    name: table.name.clone(),
                    schema: table.schema.clone(),
                })
                .await?;
            tables.push(crate::RelationalTable { table, structure });
        }
        Ok(crate::RelationalSchema {
            database: self.current_database().await?,
            tables,
            objects: Vec::new(),
            objects_captured: false,
            details_captured: true,
        })
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        self.call("list_tables", json!({})).await
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.call("list_databases", json!({})).await
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.database.read().await.clone().unwrap_or_default())
    }
    async fn use_database(&self, name: &str) -> Result<()> {
        self.call::<()>("use_database", json!({"name":name}))
            .await?;
        *self.database.write().await = Some(name.into());
        Ok(())
    }
    async fn document_for_edit(&self, table: &TableRef, identity: &Value) -> Result<Value> {
        self.call(
            "document_for_edit",
            json!({"table":table,"identity":identity}),
        )
        .await
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        self.call("describe_table", json!({"table":table})).await
    }
    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        self.call("table_structure", json!({"table":table})).await
    }
    async fn query(&self, query: &str, options: QueryOptions) -> Result<QueryResult> {
        self.validate_query(query)?;
        let result = self
            .call("query", json!({"query":query,"options":options}))
            .await?;
        validate_result(result, options)
    }
    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        self.validate_query(&statement.sql)?;
        let result = self.call(
            "query_statement",
            json!({"statement":{"sql":statement.sql,"params":statement.params},"options":options}),
        ).await?;
        validate_result(result, options)
    }
    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        if self.config.read_only {
            return Err(DbxError::Query(
                "Protected connection: writes are disabled".into(),
            ));
        }
        self.validate_query(&statement.sql)?;
        self.call(
            "execute",
            json!({"statement":{"sql":statement.sql,"params":statement.params}}),
        )
        .await
    }
}

fn validate_result(mut result: QueryResult, options: QueryOptions) -> Result<QueryResult> {
    if result
        .rows
        .iter()
        .any(|row| row.values.len() != result.columns.len())
    {
        return Err(DbxError::Decode(
            "Driver returned rows with inconsistent column counts".into(),
        ));
    }
    if let Some(limit) = options.max_rows
        && result.rows.len() > limit
    {
        result.rows.truncate(limit);
        result.truncated = true;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifests_require_explicit_absolute_program_and_compatible_scheme() {
        let mut manifest = DriverManifest {
            protocol: 1,
            id: "community-db".into(),
            name: "Community DB".into(),
            family: DatabaseKind::PostgreSQL,
            executable: "/opt/community/driver".into(),
            arguments: vec![],
            schemes: vec!["community".into()],
        };
        manifest.validate().unwrap();
        manifest.id = "../bad".into();
        assert!(manifest.validate().is_err());
        manifest.id = "good".into();
        manifest.executable = "relative-program".into();
        assert!(manifest.validate().is_err());
    }
    #[tokio::test]
    async fn fixture_driver_receives_secrets_only_on_stdin_and_enforces_protection() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("driver.py");
        std::fs::write(
            &script,
            r#"import json,sys
assert len(sys.argv)==1
r=json.load(sys.stdin)
assert r['connection']['url']=='community://user:secret@localhost/db'
op=r['operation']
result='db' if op=='connect' else []
json.dump({'protocol':1,'ok':True,'result':result},sys.stdout)
"#,
        )
        .unwrap();
        let python = if Path::new("/usr/bin/python3").exists() {
            PathBuf::from("/usr/bin/python3")
        } else {
            return;
        };
        let manifest = DriverManifest {
            protocol: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            family: DatabaseKind::PostgreSQL,
            executable: python,
            arguments: vec![script.display().to_string()],
            schemes: vec!["community".into()],
        };
        let path = directory.path().join("fixture.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut config = ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            "community://user:secret@localhost/db",
        )
        .with_read_only(true);
        config.driver_manifest = Some(path);
        let engine = PluginEngine::connect(config).await.unwrap();
        assert_eq!(engine.current_database().await.unwrap(), "db");
        assert!(engine.list_tables().await.unwrap().is_empty());
        assert!(
            engine
                .query("DELETE FROM t", QueryOptions::default())
                .await
                .is_err()
        );
        assert!(
            engine
                .execute(&SqlStatement::new("DROP TABLE t", vec![]))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn driver_failures_timeout_and_oversize_outputs_are_bounded() {
        if !Path::new("/usr/bin/python3").exists() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        for (name, script, timeout, expected) in [
            ("slow", "import time; time.sleep(5)", 50, "timed out"),
            (
                "large",
                "import sys; sys.stdout.write('x'*(16*1024*1024+1))",
                5000,
                "exceeds 16 MiB",
            ),
            ("invalid", "print('not JSON')", 5000, "invalid JSON"),
            (
                "secret",
                "print('{\"protocol\":1,\"ok\":false,\"error\":\"secret-password\"}')",
                5000,
                "rejected",
            ),
        ] {
            let script_path = directory.path().join(format!("{name}.py"));
            std::fs::write(&script_path, script).unwrap();
            let manifest = DriverManifest {
                protocol: 1,
                id: name.into(),
                name: name.into(),
                family: DatabaseKind::PostgreSQL,
                executable: "/usr/bin/python3".into(),
                arguments: vec![script_path.display().to_string()],
                schemes: vec!["community".into()],
            };
            let path = directory.path().join(format!("{name}.json"));
            std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let mut config = ConnectionConfig::new(
                DatabaseKind::PostgreSQL,
                "community://user:secret-password@localhost/db",
            )
            .with_connect_timeout_ms(timeout);
            config.driver_manifest = Some(path);
            let error = match PluginEngine::connect(config).await {
                Ok(_) => panic!("accepted {name}"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains(expected), "{name}: {error}");
            assert!(!error.contains("secret-password"));
        }
    }
    #[tokio::test]
    async fn example_driver_browses_queries_binds_values_and_enforces_row_caps() {
        if !Path::new("/usr/bin/python3").exists() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/drivers/sqlite-driver.py")
            .canonicalize()
            .unwrap();
        let manifest = DriverManifest {
            protocol: 1,
            id: "sqlite-example".into(),
            name: "Example SQLite".into(),
            family: DatabaseKind::SQLite,
            executable: "/usr/bin/python3".into(),
            arguments: vec![script.display().to_string()],
            schemes: vec!["example-sqlite".into()],
        };
        let path = directory.path().join("manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let database = directory.path().join("example.sqlite");
        let mut config = ConnectionConfig::new(
            DatabaseKind::SQLite,
            format!("example-sqlite://{}", database.display()),
        );
        config.driver_manifest = Some(path);
        let engine = crate::DatabaseEngine::connect(config.clone())
            .await
            .unwrap();
        assert!(
            !engine.capabilities().row_edits
                && !engine.capabilities().independent_query_sessions
                && !engine.capabilities().native_backup
        );
        assert_eq!(crate::Engine::capabilities(&engine), engine.capabilities());
        assert!(
            engine
                .query("BEGIN", QueryOptions::default())
                .await
                .is_err()
        );
        engine
            .execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .await
            .unwrap();
        engine
            .execute(&SqlStatement::new(
                "INSERT INTO items VALUES(?,?)",
                vec![
                    crate::CellValue::Integer(1),
                    crate::CellValue::Text("quoted ' text".into()),
                ],
            ))
            .await
            .unwrap();
        engine
            .execute_sql("INSERT INTO items VALUES(2,'second')")
            .await
            .unwrap();
        assert_eq!(engine.list_tables().await.unwrap()[0].name, "items");
        assert_eq!(
            engine
                .table_structure(&TableRef::new("items"))
                .await
                .unwrap()
                .columns
                .len(),
            2
        );
        let result = engine
            .query_statement(
                &SqlStatement::new(
                    "SELECT name FROM items WHERE id=?",
                    vec![crate::CellValue::Integer(1)],
                ),
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            result.rows[0].values[0],
            crate::CellValue::Text("quoted ' text".into())
        );
        let result = engine
            .query("SELECT * FROM items", QueryOptions { max_rows: Some(1) })
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert!(result.truncated);
        let schema = engine.relational_schema().await.unwrap();
        assert!(!schema.objects_captured && schema.objects.is_empty());
        config.read_only = true;
        let protected = crate::DatabaseEngine::connect(config).await.unwrap();
        assert!(protected.execute_sql("DELETE FROM items").await.is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_driver_request_terminates_descendant_work() {
        if !Path::new("/usr/bin/python3").exists() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let marker = directory.path().join("descendant-ran");
        let child_script = format!(
            "import time,pathlib;time.sleep(1.2);pathlib.Path({}).write_text('survived')",
            serde_json::to_string(&marker.display().to_string()).unwrap()
        );
        let script = format!(
            "import subprocess,pathlib,sys,time\nsubprocess.Popen(['/usr/bin/python3','-c',{}])\npathlib.Path({}).write_text('ready')\nsys.stdin.read()\ntime.sleep(60)",
            serde_json::to_string(&child_script).unwrap(),
            serde_json::to_string(&ready.display().to_string()).unwrap()
        );
        let script_path = directory.path().join("driver.py");
        std::fs::write(&script_path, script).unwrap();
        let manifest = DriverManifest {
            protocol: 1,
            id: "descendant".into(),
            name: "Descendant fixture".into(),
            family: DatabaseKind::PostgreSQL,
            executable: "/usr/bin/python3".into(),
            arguments: vec![script_path.display().to_string()],
            schemes: vec!["community".into()],
        };
        let path = directory.path().join("manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut config =
            ConnectionConfig::new(DatabaseKind::PostgreSQL, "community://localhost/db");
        config.driver_manifest = Some(path);
        let task = tokio::spawn(PluginEngine::connect(config));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(1400)).await;
        assert!(
            !marker.exists(),
            "cancellation must terminate helper processes as well as the direct driver"
        );
    }
}
