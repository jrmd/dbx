use crate::{
    ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, EntityKind, QueryOptions,
    QueryResult, Result, TableInfo, TableRef,
};
use async_trait::async_trait;
use rdkafka::{
    ClientConfig, Message, Offset, TopicPartitionList,
    consumer::{BaseConsumer, Consumer},
    error::RDKafkaErrorCode,
    producer::{FutureProducer, FutureRecord},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) struct KafkaEngine {
    config: ClientConfig,
    address: String,
    timeout: Duration,
}
fn error(error: rdkafka::error::KafkaError) -> DbxError {
    DbxError::Driver(format!(
        "Kafka operation failed ({:?}); check broker access, authentication and topic permissions",
        error.rdkafka_error_code()
    ))
}
fn worker_error(_: tokio::task::JoinError) -> DbxError {
    DbxError::Driver("Kafka worker failed".into())
}
impl KafkaEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        let url = url::Url::parse(&config.url)
            .map_err(|_| DbxError::InvalidConfig("Invalid Kafka URL".into()))?;
        let host = url
            .host_str()
            .ok_or_else(|| DbxError::InvalidConfig("Kafka broker is required".into()))?;
        let address = format!("{host}:{}", url.port().unwrap_or(9092));
        let mut client = ClientConfig::new();
        client
            .set("bootstrap.servers", &address)
            .set("group.id", "dbx-browser")
            .set("enable.auto.commit", "false")
            .set("enable.auto.offset.store", "false");
        if !url.username().is_empty() {
            client
                .set("security.protocol", "SASL_SSL")
                .set("sasl.mechanism", "PLAIN");
        } else if url.password().is_some() {
            return Err(DbxError::InvalidConfig(
                "Kafka SASL requires a username as well as a password".into(),
            ));
        }
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "brokers" => {
                    client.set("bootstrap.servers", value.as_ref());
                }
                "security.protocol"
                | "sasl.mechanism"
                | "ssl.ca.location"
                | "ssl.certificate.location"
                | "ssl.key.location" => {
                    client.set(key.as_ref(), value.as_ref());
                }
                _ => {
                    return Err(DbxError::InvalidConfig(format!(
                        "Unsupported Kafka option: {key}"
                    )));
                }
            }
        }
        if !url.username().is_empty() {
            client.set("sasl.username", super::decode(url.username())?);
        }
        if let Some(password) = url.password() {
            client.set("sasl.password", super::decode(password)?);
        }
        client.set("socket.timeout.ms", config.connect_timeout_ms.to_string());
        client.set("message.timeout.ms", config.connect_timeout_ms.to_string());
        let engine = Self {
            config: client,
            address,
            timeout: Duration::from_millis(config.connect_timeout_ms),
        };
        engine.list_tables().await?;
        Ok(engine)
    }
}
#[async_trait]
impl Engine for KafkaEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Kafka
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.address.clone())
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let config = self.config.clone();
        let timeout = self.timeout;
        tokio::task::spawn_blocking(move || {
            let consumer: BaseConsumer = config.create().map_err(error)?;
            let metadata = consumer.fetch_metadata(None, timeout).map_err(error)?;
            let mut tables: Vec<_> = metadata
                .topics()
                .iter()
                .filter(|topic| topic.error().is_none())
                .map(|topic| TableInfo {
                    name: topic.name().into(),
                    schema: None,
                    kind: EntityKind::Collection,
                })
                .collect();
            tables.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(tables)
        })
        .await
        .map_err(worker_error)?
    }
    async fn describe_table(&self, _table: &TableRef) -> Result<Vec<ColumnInfo>> {
        Ok([
            ("partition", "integer"),
            ("offset", "integer"),
            ("timestamp", "integer"),
            ("key", "bytes"),
            ("value", "bytes"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (n, t))| super::column(n, t, i))
        .collect())
    }
    async fn query(&self, command: &str, options: QueryOptions) -> Result<QueryResult> {
        let request:Value=serde_json::from_str(command).map_err(|_|DbxError::Parse("Kafka queries use JSON: {\"action\":\"topics\"} or {\"action\":\"consume\",\"topic\":\"events\"}".into()))?;
        if request["action"] == "topics" {
            let started = Instant::now();
            return Ok(super::json_rows(
                self.list_tables()
                    .await?
                    .into_iter()
                    .map(|t| json!({"topic":t.name}))
                    .collect(),
                options,
                started,
            ));
        }
        if request["action"] == "produce" {
            let started = Instant::now();
            let topic = request["topic"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| DbxError::Parse("Kafka topic is required".into()))?;
            let producer: FutureProducer = self.config.create().map_err(error)?;
            let payload = request["value"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| request["value"].to_string());
            let mut record = FutureRecord::<str, str>::to(topic).payload(&payload);
            if let Some(key) = request["key"].as_str() {
                record = record.key(key);
            }
            if let Some(partition) = request["partition"].as_i64() {
                record = record.partition(
                    i32::try_from(partition)
                        .map_err(|_| DbxError::Parse("Invalid partition".into()))?,
                );
            }
            let delivery = tokio::time::timeout(self.timeout, producer.send(record, self.timeout))
                .await
                .map_err(|_| {
                    DbxError::Query("Kafka delivery timed out; delivery status is unknown".into())
                })?
                .map_err(|(e, _)| error(e))?;
            let mut result = super::json_rows(
                vec![json!({"partition":delivery.partition,"offset":delivery.offset})],
                options,
                started,
            );
            result.rows_affected = Some(1);
            return Ok(result);
        }
        let config = self.config.clone();
        let timeout = self.timeout;
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let topic = request["topic"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| DbxError::Parse("Kafka topic is required".into()))?;
            match request["action"].as_str() {
                Some("consume") => {}
                _ => {
                    return Err(DbxError::Parse(
                        "Kafka action must be topics or consume".into(),
                    ));
                }
            }
            let consumer: BaseConsumer = config.create().map_err(error)?;
            let metadata = consumer
                .fetch_metadata(Some(topic), timeout)
                .map_err(|failure| {
                    DbxError::Driver(format!(
                        "Kafka topic metadata failed ({:?})",
                        failure.rdkafka_error_code()
                    ))
                })?;
            let topic_metadata = metadata
                .topics()
                .iter()
                .find(|t| t.name() == topic && t.error().is_none())
                .ok_or_else(|| DbxError::Query("Kafka topic is unavailable".into()))?;
            let offset = request["offset"].as_i64().unwrap_or(0);
            if offset < 0 {
                return Err(DbxError::Parse("Kafka offset must be nonnegative".into()));
            }
            let mut assignments = TopicPartitionList::new();
            let mut ends = std::collections::HashMap::new();
            for partition in topic_metadata.partitions() {
                if let Some(wanted) = request["partition"].as_i64()
                    && i64::from(partition.id()) != wanted
                {
                    continue;
                }
                let (low, high) = consumer
                    .fetch_watermarks(topic, partition.id(), timeout)
                    .map_err(|failure| {
                        DbxError::Driver(format!(
                            "Kafka partition watermarks failed ({:?})",
                            failure.rdkafka_error_code()
                        ))
                    })?;
                let start = offset.max(low);
                if start < high {
                    assignments
                        .add_partition_offset(topic, partition.id(), Offset::Offset(start))
                        .map_err(error)?;
                    ends.insert(partition.id(), high);
                }
            }
            consumer.assign(&assignments).map_err(error)?;
            let cap = crate::engine::row_limit(options)
                .unwrap_or(10_000)
                .min(request["limit"].as_u64().unwrap_or(100) as usize)
                .max(1);
            let mut values = Vec::new();
            let deadline = Instant::now() + timeout;
            while !ends.is_empty() && values.len() <= cap && Instant::now() < deadline {
                if let Some(message) = consumer.poll(Duration::from_millis(100)) {
                    // A bootstrap address may fail while another advertised
                    // broker connects. librdkafka retries these global events.
                    let message = match message {
                        Ok(message) => message,
                        Err(failure)
                            if matches!(
                                failure.rdkafka_error_code(),
                                Some(
                                    RDKafkaErrorCode::BrokerTransportFailure
                                        | RDKafkaErrorCode::AllBrokersDown
                                )
                            ) =>
                        {
                            continue;
                        }
                        Err(failure) => return Err(error(failure)),
                    };
                    let bytes = |data: Option<&[u8]>| match data {
                        None => crate::CellValue::Null,
                        Some(bytes) => match std::str::from_utf8(bytes) {
                            Ok(text) => crate::CellValue::Text(text.into()),
                            Err(_) => crate::CellValue::Bytes(bytes.to_vec()),
                        },
                    };
                    values.push(crate::RowData::new(vec![
                        crate::CellValue::Integer(message.partition().into()),
                        crate::CellValue::Integer(message.offset()),
                        message
                            .timestamp()
                            .to_millis()
                            .map(crate::CellValue::Integer)
                            .unwrap_or(crate::CellValue::Null),
                        bytes(message.key()),
                        bytes(message.payload()),
                    ]));
                    if ends
                        .get(&message.partition())
                        .is_some_and(|end| message.offset() + 1 >= *end)
                    {
                        ends.remove(&message.partition());
                    }
                }
            }
            if values.is_empty() && !ends.is_empty() {
                return Err(DbxError::Query(
                    "Kafka consumption timed out before receiving messages".into(),
                ));
            }
            let truncated = values.len() > cap;
            values.truncate(cap);
            let columns = [
                ("partition", "integer"),
                ("offset", "integer"),
                ("timestamp", "integer"),
                ("key", "bytes"),
                ("value", "bytes"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (n, t))| super::column(n, t, i))
            .collect();
            let mut result = crate::engine::query_result(columns, values, None, truncated, started);
            result.truncated |= !ends.is_empty();
            Ok(result)
        })
        .await
        .map_err(worker_error)?
    }
}
