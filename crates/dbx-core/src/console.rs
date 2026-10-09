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
    External(Box<dyn crate::Engine>),
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
    /// When the connection last finished a run, for idle health checks.
    last_used: Option<Instant>,
}
impl SessionState {
    fn reset(&mut self) {
        self.connection.take();
        self.last_used = None;
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
                last_used: None,
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
        let statements = if self.engine.kind().is_sql() {
            crate::script::checked_split_sql_for(Some(self.engine.kind()), sql)?
        } else {
            vec![sql.to_owned()]
        }
        .into_iter()
        .map(|sql| SqlStatement::new(sql, Vec::new()))
        .collect();
        self.run_prepared(statements, options, timeout, cancellation)
            .await
    }

    /// Execute driver-bound statements on the same tab-owned connection.
    /// Values never become SQL text and are not included in statement history.
    pub async fn run_prepared(
        &self,
        statements: Vec<SqlStatement>,
        options: QueryOptions,
        timeout: Duration,
        cancellation: QueryCancellation,
    ) -> Result<ScriptResult> {
        let deadline = Instant::now() + timeout;
        let mut state = tokio::select! {
            state = tokio::time::timeout(timeout, self.state.lock()) => state.map_err(|_| DbxError::Interrupted("Query session was busy until the timeout".into()))?,
            _ = cancellation.cancelled() => return Err(DbxError::Interrupted("Query cancelled while waiting for its session".into())),
        };
        // A server restart, proxy timeout, or sleeping laptop silently kills
        // idle sockets. Check before reuse so a dead connection is replaced
        // rather than failing this run.
        if state
            .last_used
            .is_some_and(|used| used.elapsed() >= IDLE_HEALTH_CHECK)
            && let Some(connection) = state.connection.as_mut()
            && !connection.is_alive().await
        {
            let lost_transaction = state.in_transaction;
            state.reset();
            if lost_transaction {
                return Err(DbxError::Connection(
                    "the connection was lost while idle, and the server rolled back its open transaction; this tab reconnects on the next run".into(),
                ));
            }
        }
        // Preparing reads the backend id over the reused connection before
        // any statement runs, so a connection lost there is retried once.
        let mut retried = false;
        let target = loop {
            let reused = state.connection.is_some();
            let lost_transaction = state.in_transaction;
            let prepared = tokio::select! {
                result = tokio::time::timeout(timeout, prepare_session(&self.engine, &mut state)) => match result {
                    Ok(result) => result,
                    Err(_) => { state.reset(); return Err(DbxError::Interrupted("Query session initialization timed out".into())); }
                },
                _ = cancellation.cancelled() => { state.reset(); return Err(DbxError::Interrupted("Query session initialization cancelled".into())); }
            };
            match prepared {
                Ok(target) => break target,
                Err(DbxError::Connection(_)) if reused && !retried => {
                    state.reset();
                    if lost_transaction {
                        return Err(DbxError::Connection(
                            "the connection was lost, and the server rolled back its open transaction; this tab reconnects on the next run".into(),
                        ));
                    }
                    retried = true;
                }
                Err(error) => {
                    state.reset();
                    return Err(error);
                }
            }
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
                statements,
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
            state.last_used = Some(Instant::now());
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

/// Open or reuse the session connection and resolve how to cancel it.
async fn prepare_session(
    engine: &DatabaseEngine,
    state: &mut SessionState,
) -> Result<CancelTarget> {
    Ok::<_, DbxError>(if let DatabaseEngine::Sql(engine) = engine {
        if state.connection.is_none() {
            state.connection = Some(open_connection(engine.pool_snapshot().await?).await?);
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
            engine.pool_snapshot().await?,
        )
        .await?
    } else {
        if state.connection.is_none()
            && let DatabaseEngine::Other(engine) = engine
        {
            state.connection = engine
                .open_query_session()
                .await?
                .map(SqlConnection::External);
        }
        CancelTarget::Local
    })
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
    statements: Vec<SqlStatement>,
    options: QueryOptions,
    cancellation: &QueryCancellation,
    deadline: Instant,
) -> Result<ScriptResult> {
    let mut output = ScriptResult::default();
    let mut retained_bytes = 0usize;
    for prepared in statements {
        let statement = prepared.sql.clone();
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
        if state.connection.is_none()
            && !matches!(engine, DatabaseEngine::Sql(_))
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
            state.connection = Some(open_connection(engine.pool_snapshot().await?).await?);
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
                    let adapted = match connection {
                        SqlConnection::Postgres(postgres) if !prepared.params.is_empty() => Some(
                            crate::parameters::adapt_postgres_parameters(postgres, &prepared).await,
                        ),
                        _ => None,
                    };
                    match adapted {
                        Some(Ok(adapted)) => connection.query(&adapted, options).await,
                        Some(Err(error)) => Err(error),
                        None => connection.query(&prepared, options).await,
                    }
                }
                None => engine.query_statement(&prepared, options).await,
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
                // SQL Server supports nested and named transactions. Grammar
                // guesses cannot establish whether COMMIT actually closed one.
                if engine.kind() == DatabaseKind::SqlServer
                    && let Some(connection) = state.connection.as_mut()
                {
                    let transaction = connection
                        .query(
                            &SqlStatement::new("SELECT @@TRANCOUNT", Vec::new()),
                            QueryOptions { max_rows: Some(1) },
                        )
                        .await?;
                    let count = transaction
                        .rows
                        .first()
                        .and_then(|row| row.values.first())
                        .map(ToString::to_string)
                        .and_then(|value| value.parse::<u64>().ok())
                        .ok_or_else(|| {
                            DbxError::Query("Cannot determine SQL Server transaction state".into())
                        })?;
                    state.in_transaction = count > 0;
                    state.explicit_transaction = count > 0;
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
                let lost = matches!(error, DbxError::Connection(_));
                let message = if lost {
                    format!(
                        "{error}. The connection was lost{}; this tab reconnects on the next run.",
                        if state.in_transaction {
                            " and its open transaction was rolled back"
                        } else {
                            ""
                        }
                    )
                } else {
                    error.to_string()
                };
                if memory_savepoint || lost {
                    state.reset();
                }
                output.statements.push(StatementResult {
                    statement,
                    result: QueryResult::empty(None, started.elapsed().as_millis() as u64),
                    error: Some(message),
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

/// How long a query tab's connection may sit unused before it is pinged.
const IDLE_HEALTH_CHECK: Duration = Duration::from_secs(30);

impl SqlConnection {
    /// Ping network connections; embedded SQLite handles cannot drop.
    async fn is_alive(&mut self) -> bool {
        let ping = async {
            match self {
                Self::Postgres(connection) => connection.ping().await.is_ok(),
                Self::MySql(connection) => connection.ping().await.is_ok(),
                _ => true,
            }
        };
        tokio::time::timeout(Duration::from_secs(5), ping)
            .await
            .unwrap_or(false)
    }

    async fn query(
        &mut self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        if let Self::External(engine) = self {
            let mut statement = statement.clone();
            if engine.kind() == DatabaseKind::SqlServer
                && statement.sql.trim().eq_ignore_ascii_case("BEGIN")
            {
                statement.sql = "BEGIN TRANSACTION".into();
            }
            return engine.query_statement(&statement, options).await;
        }
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
            Self::External(_) => unreachable!("handled above"),
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
                Self::External(_) => unreachable!("handled above"),
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
        if let DatabaseEngine::Other(inner) = engine
            && engine.kind() == DatabaseKind::SqlServer
            && !read_only
        {
            let connection =
                inner
                    .open_query_session()
                    .await?
                    .ok_or_else(|| DbxError::Unsupported {
                        operation: "atomic SQL Server session".into(),
                        kind: engine.kind(),
                    })?;
            let mut transaction = Self {
                connection: Some(SqlConnection::External(connection)),
            };
            transaction
                .query(&SqlStatement::new(
                    "SET XACT_ABORT ON; BEGIN TRANSACTION",
                    Vec::new(),
                ))
                .await?;
            return Ok(transaction);
        }
        let DatabaseEngine::Sql(engine) = engine else {
            return Err(DbxError::Unsupported {
                operation: "atomic transfer transaction".into(),
                kind: engine.kind(),
            });
        };
        let mut connection = open_connection(engine.pool_snapshot().await?).await?;
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
    pub(crate) async fn stream_rows(
        &mut self,
        sql: &str,
        mut consume: impl FnMut(&[crate::ColumnInfo], Option<Vec<CellValue>>) -> Result<()> + Send,
    ) -> Result<u64> {
        let mut count = 0u64;
        macro_rules! stream {
            ($connection:expr, $decode:ident) => {{
                let columns = $connection
                    .describe(sql)
                    .await
                    .map(|description| result_columns(description.columns()))?;
                consume(&columns, None)?;
                let mut rows = sqlx::query(sql).fetch(&mut *$connection);
                while let Some(row) = rows.try_next().await? {
                    consume(&columns, Some($decode(&row)?))?;
                    count += 1;
                }
            }};
        }
        match self.connection.as_mut().expect("active transaction") {
            SqlConnection::External(engine) => {
                return Err(DbxError::Unsupported {
                    operation: "streaming query export".into(),
                    kind: engine.kind(),
                });
            }
            SqlConnection::Postgres(connection) => stream!(connection, decode_postgres_row),
            SqlConnection::MySql(connection) => stream!(connection, decode_mysql_row),
            SqlConnection::SQLite(connection) => stream!(connection, decode_sqlite_row),
            SqlConnection::Memory(connection) => stream!(&mut **connection, decode_sqlite_row),
            SqlConnection::MemoryTransaction(connection) => {
                stream!(&mut **connection, decode_sqlite_row)
            }
        }
        Ok(count)
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
