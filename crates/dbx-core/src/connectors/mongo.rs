use crate::{
    ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, EntityKind, QueryOptions,
    QueryResult, Result, TableInfo, TableRef,
};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use mongodb::{
    Client,
    bson::{Document, doc},
    options::ClientOptions,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

pub(super) struct MongoEngine {
    client: Client,
    database: Arc<RwLock<String>>,
}

fn error(error: mongodb::error::Error) -> DbxError {
    // Driver errors can include the original URI and server replies. Do not
    // expose either credentials or arbitrary server text in diagnostics.
    let detail = match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => format!("server code {}", command.code),
        _ => "check the connection, permissions and command".into(),
    };
    DbxError::Driver(format!("MongoDB operation failed ({detail})"))
}

impl MongoEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let mut options = tokio::time::timeout(timeout, ClientOptions::parse(&config.url))
            .await
            .map_err(|_| DbxError::Connection("MongoDB connection timed out".into()))?
            .map_err(|_| DbxError::InvalidConfig("Invalid MongoDB connection URI".into()))?;
        options.server_selection_timeout = Some(timeout);
        options.connect_timeout = Some(timeout);
        options.max_pool_size = Some(config.max_connections);
        let database = options
            .default_database
            .clone()
            .unwrap_or_else(|| "test".into());
        let client = Client::with_options(options).map_err(error)?;
        client
            .database(&database)
            .run_command(doc! {"ping": 1})
            .await
            .map_err(error)?;
        Ok(Self {
            client,
            database: Arc::new(RwLock::new(database)),
        })
    }
}

#[async_trait]
impl Engine for MongoEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MongoDB
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let db = self.database.read().await.clone();
        Ok(self
            .client
            .database(&db)
            .list_collection_names()
            .await
            .map_err(error)?
            .into_iter()
            .map(|name| TableInfo {
                name,
                schema: None,
                kind: EntityKind::Collection,
            })
            .collect())
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.client.list_database_names().await.map_err(error)
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.database.read().await.clone())
    }
    async fn use_database(&self, name: &str) -> Result<()> {
        if name.is_empty() || name.contains(['/', '\0']) {
            return Err(DbxError::InvalidConfig(
                "Invalid MongoDB database name".into(),
            ));
        }
        self.client
            .database(name)
            .run_command(doc! {"ping":1})
            .await
            .map_err(error)?;
        *self.database.write().await = name.into();
        Ok(())
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        Ok(self
            .query(
                &serde_json::json!({"find":table.name,"limit":1}).to_string(),
                QueryOptions { max_rows: Some(1) },
            )
            .await?
            .columns)
    }
    async fn query(&self, command: &str, options: QueryOptions) -> Result<QueryResult> {
        let started = Instant::now();
        let document = parse_command(command)?;
        let database = self.database.read().await.clone();
        let db = self.client.database(&database);
        let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
        let values = if document.contains_key("find")
            || document.contains_key("aggregate")
            || document.contains_key("listCollections")
        {
            let mut cursor = db.run_cursor_command(document).await.map_err(error)?;
            let mut values = Vec::new();
            while let Some(document) = cursor.try_next().await.map_err(error)? {
                values.push(
                    serde_json::to_value(document)
                        .map_err(|_| DbxError::Decode("Invalid BSON document".into()))?,
                );
                if values.len() > limit {
                    break;
                }
            }
            values
        } else {
            let reply = db.run_command(document).await.map_err(error)?;
            vec![
                serde_json::to_value(reply)
                    .map_err(|_| DbxError::Decode("Invalid BSON reply".into()))?,
            ]
        };
        Ok(super::json_rows(values, options, started))
    }
}

fn parse_command(command: &str) -> Result<Document> {
    // Convert Extended JSON explicitly. Deserializing directly into BSON through
    // deserialize_any would expose serde_json's private precise-number wrapper.
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(command)
        .ok()
        .and_then(|object| Document::try_from(object).ok())
        .ok_or_else(|| {
            DbxError::Parse(
                "Enter a MongoDB JSON command, e.g. {\"find\":\"users\",\"filter\":{}}".into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_numbers_remain_bson_numbers_with_precise_json_enabled() {
        let command = parse_command(r#"{"find":"users","limit":100,"filter":{"balance":{"$numberDecimal":"1234567890.1234567890123456789"}}}"#).unwrap();
        assert_eq!(command.get_i32("limit").unwrap(), 100);
        assert!(matches!(
            command.get_document("filter").unwrap().get("balance"),
            Some(mongodb::bson::Bson::Decimal128(_))
        ));
        assert!(parse_command("[]").is_err());
    }
}
