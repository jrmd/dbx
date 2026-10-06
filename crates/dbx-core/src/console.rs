//! Query documents own SQL sessions. An interrupted connection is discarded,
//! never returned to a pool with an unknown transaction or pending response.
use crate::sqlx_engine::{
    SqlxPool, bind_mysql_query, bind_postgres_query, bind_sqlite_query, decode_mysql_row,
    decode_postgres_row, decode_sqlite_row, refine_dynamic_column_types, result_columns,
};
use crate::{
    CellValue, DatabaseEngine, DatabaseKind, DbxError, QueryOptions, QueryResult, Result, RowData,
    SqlStatement,
};
use futures_util::TryStreamExt;
use sqlx::{Connection, Either, Executor, Row};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Notify};

const RESULT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct QueryCancellation {
    flag: Arc<AtomicBool>,
    wake: Arc<Notify>,
}
impl QueryCancellation {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }
    pub(crate) async fn cancelled(&self) {
        while !self.flag.load(Ordering::SeqCst) {
            self.wake.notified().await;
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatementResult {
    pub statement: String,
    pub result: QueryResult,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct ScriptResult {
    pub statements: Vec<StatementResult>,
    pub in_transaction: bool,
}

enum SqlConnection {
    Postgres(sqlx::PgConnection),
    MySql(sqlx::MySqlConnection),
    SQLite(sqlx::SqliteConnection),
    Memory(sqlx::pool::PoolConnection<sqlx::Sqlite>),
    MemoryTransaction(sqlx::Transaction<'static, sqlx::Sqlite>),
}
enum CancelTarget {
    Postgres(Arc<sqlx::postgres::PgConnectOptions>, i32),
    MySql(Arc<sqlx::mysql::MySqlConnectOptions>, u64),
    Local,
}
impl CancelTarget {
    async fn cancel(&self) -> bool {
        let result: Result<bool> = async {
            match self {
                Self::Postgres(options, id) => {
                    let mut control = sqlx::PgConnection::connect_with(options).await?;
                    let stopped: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
                        .bind(id)
                        .fetch_one(&mut control)
                        .await?;
                    Ok(stopped)
                }
                Self::MySql(options, id) => {
                    let mut control = sqlx::MySqlConnection::connect_with(options).await?;
                    sqlx::query(&format!("KILL QUERY {id}"))
                        .execute(&mut control)
                        .await?;
                    Ok(true)
                }
                Self::Local => Ok(false),
            }
        }
        .await;
        result.unwrap_or(false)
    }
}

struct SessionState {
    connection: Option<SqlConnection>,
    in_transaction: bool,
    explicit_transaction: bool,
    savepoints: Vec<String>,
}
impl SessionState {
    fn reset(&mut self) {
        self.connection.take();
        self.in_transaction = false;
        self.explicit_transaction = false;
        self.savepoints.clear();
    }
}
pub struct QuerySession {
    engine: Arc<DatabaseEngine>,
    state: Mutex<SessionState>,
}
impl QuerySession {
    pub fn new(engine: Arc<DatabaseEngine>) -> Self {
        Self {
            engine,
            state: Mutex::new(SessionState {
                connection: None,
                in_transaction: false,
                explicit_transaction: false,
                savepoints: Vec::new(),
            }),
        }
    }
    /// A timeout/cancel terminates this run and discards its SQL session. Writes
    /// that already committed remain committed; callers must show that caveat.
    pub async fn run(
        &self,
        sql: &str,
        options: QueryOptions,
        timeout: Duration,
        cancellation: QueryCancellation,
    ) -> Result<ScriptResult> {
        let deadline = Instant::now() + timeout;
        let mut state = tokio::select! {
            state = tokio::time::timeout(timeout, self.state.lock()) => state.map_err(|_| DbxError::Interrupted("Query session was busy until the timeout".into()))?,
            _ = cancellation.cancelled() => return Err(DbxError::Interrupted("Query cancelled while waiting for its session".into())),
        };
        let prepare = async {
            Ok::<_, DbxError>(if let DatabaseEngine::Sql(engine) = self.engine.as_ref() {
                if state.connection.is_none() {
                    state.connection = Some(open_connection(engine.pool_snapshot().await).await?);
                    if engine.is_read_only() {
                        let protection = match engine.kind().dialect() {
                            DatabaseKind::MySQL => "SET SESSION TRANSACTION READ ONLY",
                            DatabaseKind::SQLite => "PRAGMA query_only=ON",
                            _ => "SET default_transaction_read_only=on",
                        };
                        state
                            .connection
                            .as_mut()
                            .unwrap()
                            .query(
                                &SqlStatement::new(protection, Vec::new()),
                                QueryOptions::default(),
                            )
                            .await?;
                    }
                }
                cancel_target(
                    state.connection.as_mut().unwrap(),
                    engine.pool_snapshot().await,
                )
                .await?
            } else {
                CancelTarget::Local
            })
        };
        let target = tokio::select! {
            result = tokio::time::timeout(timeout, prepare) => match result {
                Ok(Ok(target)) => target,
                Ok(Err(error)) => { state.reset(); return Err(error); },
                Err(_) => { state.reset(); return Err(DbxError::Interrupted("Query session initialization timed out".into())); }
            },
            _ = cancellation.cancelled() => { state.reset(); return Err(DbxError::Interrupted("Query session initialization cancelled".into())); }
        };
        if let Some(connection) = state.connection.as_mut() {
            match connection {
                SqlConnection::SQLite(connection) => {
                    install_progress(connection, cancellation.flag.clone(), deadline).await?
                }
                SqlConnection::Memory(connection) => {
                    install_progress(connection, cancellation.flag.clone(), deadline).await?
                }
                SqlConnection::MemoryTransaction(connection) => {
                    install_progress(connection, cancellation.flag.clone(), deadline).await?
                }
                _ => {}
            }
        }
        let outcome = {
            let work = execute_script(
                &self.engine,
                &mut state,
                sql,
                options,
                &cancellation,
                deadline,
            );
            tokio::pin!(work);
            tokio::select! {
                result = &mut work => Some(result),
                _ = cancellation.cancelled() => None,
                _ = tokio::time::sleep(deadline.saturating_duration_since(Instant::now())) => { cancellation.cancel(); None },
            }
        };
        let outcome = if cancellation.flag.load(Ordering::SeqCst) || Instant::now() >= deadline {
            None
        } else {
            outcome
        };
        if let Some(mut result) = outcome {
            match state.connection.as_mut() {
                Some(SqlConnection::SQLite(connection)) => {
                    connection.lock_handle().await?.remove_progress_handler()
                }
                Some(SqlConnection::Memory(connection)) => {
                    connection.lock_handle().await?.remove_progress_handler()
                }
                Some(SqlConnection::MemoryTransaction(connection)) => {
                    connection.lock_handle().await?.remove_progress_handler()
                }
                _ => {}
            }
            if result.is_err() {
                state.reset();
            }
            if let Ok(script) = &mut result
                && script
                    .statements
                    .iter()
                    .any(|statement| statement.result.truncated)
            {
                let _ = tokio::time::timeout(Duration::from_secs(5), target.cancel()).await;
                state.reset();
                script.in_transaction = false;
                if let Some(last) = script.statements.last_mut() {
                    last.error = Some("Result limit reached. The query session was closed, its open transaction rolled back, and remaining statements were skipped. Narrow the query before retrying.".into());
                }
            }
            // Release the shared in-memory SQLite handle outside transactions;
            // metadata uses that same database, unlike a new :memory: connection.
            if !state.in_transaction && matches!(state.connection, Some(SqlConnection::Memory(_))) {
                state.connection.take();
            }
            return result;
        }
        let confirmed = tokio::time::timeout(Duration::from_secs(5), target.cancel())
            .await
            .unwrap_or(false);
        // Dropping direct connections closes the socket. An in-memory pooled
        // connection is closed too, rather than recycling pending writes.
        match state.connection.as_mut() {
            Some(SqlConnection::Memory(connection)) => {
                connection.lock_handle().await?.remove_progress_handler();
            }
            Some(SqlConnection::MemoryTransaction(connection)) => {
                connection.lock_handle().await?.remove_progress_handler();
            }
            _ => {}
        }
        state.reset();
        Err(DbxError::Interrupted(format!(
            "{}; this query session was closed. Earlier committed statements remain committed. Verify the outcome of any write before retrying.",
            if confirmed {
                "Server cancellation acknowledged"
            } else if Instant::now() >= deadline {
                "Query timed out; final write outcome is unknown"
            } else {
                "Query stopped locally; final write outcome is unknown"
            }
        )))
    }
}

async fn open_connection(pool: SqlxPool) -> Result<SqlConnection> {
    Ok(match pool {
        SqlxPool::Postgres(pool) => SqlConnection::Postgres(
            sqlx::PgConnection::connect_with(&pool.connect_options()).await?,
        ),
        SqlxPool::MySql(pool) => SqlConnection::MySql(
            sqlx::MySqlConnection::connect_with(&pool.connect_options()).await?,
        ),
        SqlxPool::SQLite(pool) => {
            // Acquiring preserves the exact SQLite memory database. File-backed
            // handles can be detached safely and retain tab-local PRAGMA/temp state.
            let mut connection = pool.acquire().await?;
            let databases: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
                .fetch_all(&mut *connection)
                .await
                .unwrap_or_default();
            if databases.first().is_none_or(|(_, _, path)| path.is_empty()) {
                SqlConnection::Memory(connection)
            } else {
                SqlConnection::SQLite(connection.detach())
            }
        }
    })
}
async fn cancel_target(connection: &mut SqlConnection, pool: SqlxPool) -> Result<CancelTarget> {
    Ok(match (connection, pool) {
        (SqlConnection::Postgres(connection), SqlxPool::Postgres(pool)) => CancelTarget::Postgres(
            pool.connect_options(),
            sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(connection)
                .await?,
        ),
        (SqlConnection::MySql(connection), SqlxPool::MySql(pool)) => CancelTarget::MySql(
            pool.connect_options(),
            sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(connection)
                .await?,
        ),
        _ => CancelTarget::Local,
    })
}
async fn install_progress(
    connection: &mut sqlx::SqliteConnection,
    flag: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<()> {
    connection
        .lock_handle()
        .await?
        .set_progress_handler(1000, move || {
            !flag.load(Ordering::SeqCst) && Instant::now() < deadline
        });
    Ok(())
}

async fn execute_script(
    engine: &DatabaseEngine,
    state: &mut SessionState,
    sql: &str,
    options: QueryOptions,
    cancellation: &QueryCancellation,
    deadline: Instant,
) -> Result<ScriptResult> {
    let statements = if engine.kind().is_sql() {
        crate::transfer::checked_split_sql_for(Some(engine.kind()), sql)?
    } else {
        vec![sql.to_owned()]
    };
    let mut output = ScriptResult::default();
    let mut retained_bytes = 0usize;
    for statement in statements {
        if engine.is_read_only()
            && let Err(error) = crate::protected::ensure_query(engine.kind(), &statement)
        {
            output.statements.push(StatementResult {
                statement,
                result: QueryResult::empty(None, 0),
                error: Some(error.to_string()),
            });
            break;
        }
        let started = Instant::now();
        let keyword = crate::sqlx_engine::top_level_operation_keyword(&statement)
            .unwrap_or_default()
            .to_ascii_uppercase();
        let words = crate::protected::sql_words_for(Some(engine.kind()), &statement);
        if keyword == "SET" && words.iter().any(|word| word == "AUTOCOMMIT") {
            output.statements.push(StatementResult { statement, result: QueryResult::empty(None, 0), error: Some("Use BEGIN, COMMIT and ROLLBACK instead of changing autocommit for a query document".into()) });
            break;
        }
        if !matches!(engine, DatabaseEngine::Sql(_))
            && matches!(
                keyword.as_str(),
                "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "SAVEPOINT" | "RELEASE"
            )
        {
            output.statements.push(StatementResult {
                statement,
                result: QueryResult::empty(None, 0),
                error: Some(
                    DbxError::Unsupported {
                        operation: "tab-owned interactive transactions".into(),
                        kind: engine.kind(),
                    }
                    .to_string(),
                ),
            });
            break;
        }
        if state.connection.is_none()
            && let DatabaseEngine::Sql(engine) = engine
        {
            state.connection = Some(open_connection(engine.pool_snapshot().await).await?);
        }
        match state.connection.as_mut() {
            Some(SqlConnection::SQLite(connection)) => {
                install_progress(connection, cancellation.flag.clone(), deadline).await?
            }
            Some(SqlConnection::Memory(connection)) => {
                install_progress(connection, cancellation.flag.clone(), deadline).await?
            }
            Some(SqlConnection::MemoryTransaction(connection)) => {
                install_progress(connection, cancellation.flag.clone(), deadline).await?
            }
            _ => {}
        }
        let memory_begin =
            keyword == "BEGIN" && matches!(state.connection, Some(SqlConnection::Memory(_)));
        let rollback_to = keyword == "ROLLBACK" && words.iter().any(|word| word == "TO");
        let memory_end = matches!(keyword.as_str(), "COMMIT" | "END" | "ROLLBACK")
            && !rollback_to
            && (words.len() == 1 || (words.len() == 2 && words[1] == "TRANSACTION"));
        let memory_savepoint =
            keyword == "SAVEPOINT" && matches!(state.connection, Some(SqlConnection::Memory(_)));
        if memory_savepoint && let Some(SqlConnection::Memory(connection)) = state.connection.take()
        {
            state.connection = Some(SqlConnection::MemoryTransaction(
                sqlx::Transaction::begin(connection, None).await?,
            ));
        }
        let outcome = if memory_begin {
            if let Some(SqlConnection::Memory(connection)) = state.connection.take() {
                state.connection = Some(SqlConnection::MemoryTransaction(
                    sqlx::Transaction::begin(connection, Some(statement.clone().into())).await?,
                ));
            }
            Ok(QueryResult::empty(Some(0), 0))
        } else if memory_end
            && matches!(state.connection, Some(SqlConnection::MemoryTransaction(_)))
        {
            if let Some(SqlConnection::MemoryTransaction(mut transaction)) = state.connection.take()
            {
                transaction.lock_handle().await?.remove_progress_handler();
                if keyword == "ROLLBACK" {
                    transaction.rollback().await?;
                } else {
                    transaction.commit().await?;
                }
            }
            Ok(QueryResult::empty(Some(0), 0))
        } else {
            match state.connection.as_mut() {
                Some(connection) => {
                    connection
                        .query(&SqlStatement::new(&statement, Vec::new()), options)
                        .await
                }
                None => engine.query(&statement, options).await,
            }
        };
        match outcome {
            Ok(mut result) => {
                let size = statement
                    .len()
                    .saturating_add(
                        result
                            .columns
                            .iter()
                            .map(|column| column.name.len() + column.data_type.len())
                            .sum::<usize>(),
                    )
                    .saturating_add(
                        result
                            .rows
                            .iter()
                            .map(|row| row.values.iter().map(cell_bytes).sum::<usize>())
                            .sum::<usize>(),
                    );
                retained_bytes = retained_bytes.saturating_add(size);
                if retained_bytes > RESULT_BYTES {
                    result.rows.clear();
                    result.rows.shrink_to_fit();
                    result.truncated = true;
                }
                let chain = words.windows(2).any(|words| words == ["AND", "CHAIN"]);
                match keyword.as_str() {
                    "BEGIN" | "START" => {
                        state.in_transaction = true;
                        state.explicit_transaction = true;
                        state.savepoints.clear();
                    }
                    "COMMIT" | "END" => {
                        state.in_transaction = chain;
                        state.explicit_transaction = chain;
                        state.savepoints.clear();
                    }
                    "ROLLBACK" if !rollback_to => {
                        state.in_transaction = chain;
                        state.explicit_transaction = chain;
                        state.savepoints.clear();
                    }
                    "SAVEPOINT"
                        if state.in_transaction || engine.kind() == DatabaseKind::SQLite =>
                    {
                        state.in_transaction = true;
                        state
                            .savepoints
                            .push(control_name(&statement, "SAVEPOINT", engine.kind()));
                    }
                    "RELEASE" => {
                        let name = control_name(&statement, "RELEASE", engine.kind());
                        if let Some(index) =
                            state.savepoints.iter().rposition(|saved| saved == &name)
                        {
                            state.savepoints.truncate(index);
                        }
                        if state.savepoints.is_empty() && !state.explicit_transaction {
                            if matches!(
                                &state.connection,
                                Some(SqlConnection::MemoryTransaction(_))
                            ) && let Some(SqlConnection::MemoryTransaction(mut transaction)) =
                                state.connection.take()
                            {
                                transaction.lock_handle().await?.remove_progress_handler();
                                transaction.commit().await?;
                            }
                            state.in_transaction = false;
                        }
                    }
                    "ROLLBACK" if rollback_to => {
                        let name = control_name(&statement, "TO", engine.kind());
                        if let Some(index) =
                            state.savepoints.iter().rposition(|saved| saved == &name)
                        {
                            state.savepoints.truncate(index + 1);
                        }
                    }
                    "CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "LOCK" | "UNLOCK"
                        if engine.kind() == DatabaseKind::MySQL
                            && !words.iter().any(|word| word == "TEMPORARY") =>
                    {
                        state.in_transaction = false;
                        state.explicit_transaction = false;
                        state.savepoints.clear();
                    }
                    _ => {}
                }
                output.statements.push(StatementResult {
                    statement,
                    result,
                    error: None,
                });
                if output
                    .statements
                    .last()
                    .is_some_and(|statement| statement.result.truncated)
                {
                    break;
                }
            }
            Err(error) => {
                if memory_savepoint {
                    state.reset();
                }
                output.statements.push(StatementResult {
                    statement,
                    result: QueryResult::empty(None, started.elapsed().as_millis() as u64),
                    error: Some(error.to_string()),
                });
                break;
            }
        }
    }
    output.in_transaction = state.in_transaction;
    Ok(output)
}

fn control_name(statement: &str, operation: &str, kind: DatabaseKind) -> String {
    let upper = statement.to_ascii_uppercase();
    let position = upper.find(operation).unwrap_or(0) + operation.len();
    let mut name = statement.get(position..).unwrap_or_default().trim();
    if name
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SAVEPOINT"))
    {
        name = name[9..].trim();
    }
    let mut output = String::new();
    let mut characters = name.chars().peekable();
    if let Some(quote @ ('"' | '`' | '\'' | '[')) = characters.peek().copied() {
        characters.next();
        let end = if quote == '[' { ']' } else { quote };
        while let Some(character) = characters.next() {
            if character == end {
                if characters.peek() == Some(&end) {
                    characters.next();
                    output.push(end);
                } else {
                    break;
                }
            } else {
                output.push(character);
            }
        }
        if kind == DatabaseKind::PostgreSQL {
            return output;
        }
    } else {
        output = characters
            .take_while(|character| !character.is_whitespace())
            .collect();
    }
    output.to_ascii_lowercase()
}

impl SqlConnection {
    async fn query(
        &mut self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        let mut output = QueryResult::empty(None, 0);
        let mut bytes = 0usize;
        // A raw stream retains statement outcomes, including zero-row DDL/DML.
        // Bound statements are used by snapshot exports and atomic imports.
        macro_rules! fetch {
            ($connection:expr, $bind:ident, $decode:ident) => {{
                #[allow(deprecated)]
                let mut events = if statement.params.is_empty() {
                    sqlx::raw_sql(&statement.sql).fetch_many(&mut *$connection)
                } else {
                    $bind(statement).fetch_many(&mut *$connection)
                };
                while let Some(event) = events.try_next().await? {
                    match event {
                        Either::Left(result) => {
                            if output.columns.is_empty()
                                && !crate::sqlx_engine::statement_likely_returns_rows(
                                    &statement.sql,
                                )
                            {
                                output.rows_affected = Some(result.rows_affected());
                            }
                        }
                        Either::Right(row) => {
                            if output.columns.is_empty() {
                                output.columns = result_columns(row.columns());
                            }
                            let values = $decode(&row)?;
                            bytes =
                                bytes.saturating_add(values.iter().map(cell_bytes).sum::<usize>());
                            if options
                                .max_rows
                                .is_some_and(|limit| output.rows.len() >= limit)
                                || bytes > RESULT_BYTES
                            {
                                output.truncated = true;
                                // Ending the stream stops fetching; this connection
                                // is not allowed back into the metadata pool.
                                break;
                            }
                            refine_dynamic_column_types(&mut output.columns, &values);
                            output.rows.push(RowData::new(values));
                        }
                    }
                }
            }};
        }
        match self {
            Self::Postgres(connection) => {
                fetch!(connection, bind_postgres_query, decode_postgres_row)
            }
            Self::MySql(connection) => fetch!(connection, bind_mysql_query, decode_mysql_row),
            Self::SQLite(connection) => fetch!(connection, bind_sqlite_query, decode_sqlite_row),
            Self::Memory(connection) => {
                fetch!(&mut **connection, bind_sqlite_query, decode_sqlite_row)
            }
            Self::MemoryTransaction(connection) => {
                fetch!(&mut **connection, bind_sqlite_query, decode_sqlite_row)
            }
        }
        if output.columns.is_empty()
            && crate::sqlx_engine::statement_likely_returns_rows(&statement.sql)
        {
            macro_rules! describe {
                ($connection:expr) => {{
                    output.columns = $connection
                        .describe(&statement.sql)
                        .await
                        .map(|description| result_columns(description.columns()))
                        .unwrap_or_default();
                }};
            }
            match self {
                Self::Postgres(connection) => describe!(connection),
                Self::MySql(connection) => describe!(connection),
                Self::SQLite(connection) => describe!(connection),
                Self::Memory(connection) => describe!(&mut **connection),
                Self::MemoryTransaction(connection) => describe!(&mut **connection),
            }
        }
        output.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(output)
    }
}
fn cell_bytes(value: &CellValue) -> usize {
    match value {
        CellValue::Text(value) => value.len(),
        CellValue::Bytes(value) => value.len(),
        CellValue::Json(value) => value.to_string().len(),
        _ => 16,
    }
}

/// Owns one transactional transfer connection. Dropping before commit closes
/// it, so an aborted import cannot return an open transaction to a pool.
pub(crate) struct SqlTransaction {
    connection: Option<SqlConnection>,
}
impl SqlTransaction {
    pub(crate) async fn begin(engine: &DatabaseEngine, read_only: bool) -> Result<Self> {
        if !read_only {
            engine.ensure_writable()?;
        }
        let DatabaseEngine::Sql(engine) = engine else {
            return Err(DbxError::Unsupported {
                operation: "atomic transfer transaction".into(),
                kind: engine.kind(),
            });
        };
        let mut connection = open_connection(engine.pool_snapshot().await).await?;
        if let SqlConnection::Memory(connection) = connection {
            let transaction = sqlx::Transaction::begin(connection, None).await?;
            return Ok(Self {
                connection: Some(SqlConnection::MemoryTransaction(transaction)),
            });
        }
        let begin = if read_only {
            match engine.kind().dialect() {
                DatabaseKind::PostgreSQL => "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
                DatabaseKind::MySQL => {
                    "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ; START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY"
                }
                _ => "BEGIN",
            }
        } else {
            "BEGIN"
        };
        // Control scripts have no rowsets; this helper intentionally runs them
        // together on the same connection.
        connection
            .query(
                &SqlStatement::new(begin, Vec::new()),
                QueryOptions::default(),
            )
            .await?;
        Ok(Self {
            connection: Some(connection),
        })
    }
    pub(crate) async fn query(&mut self, statement: &SqlStatement) -> Result<QueryResult> {
        let result = self
            .connection
            .as_mut()
            .unwrap()
            .query(statement, QueryOptions { max_rows: None })
            .await?;
        if result.truncated {
            return Err(DbxError::Query(
                "transfer page exceeds the 64 MiB result budget; reduce the size of large values"
                    .into(),
            ));
        }
        Ok(result)
    }
    pub(crate) async fn commit(mut self) -> Result<()> {
        if matches!(self.connection, Some(SqlConnection::MemoryTransaction(_))) {
            if let Some(SqlConnection::MemoryTransaction(transaction)) = self.connection.take() {
                transaction.commit().await?;
            }
            return Ok(());
        }
        self.query(&SqlStatement::new("COMMIT", Vec::new())).await?;
        self.connection.take();
        Ok(())
    }
}
