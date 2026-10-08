//! Reviewed, bounded data imports. The plan owns the exact previewed values;
//! changing the source file after preview cannot change the submitted import.
use crate::{CellValue, ColumnInfo, DatabaseEngine, DbxError, Result, TableRef};
use std::{collections::HashSet, io::Read, path::Path};

pub const IMPORT_PREVIEW_BYTES: usize = 64 * 1024 * 1024;
pub const IMPORT_PREVIEW_ROWS: usize = 100_000;

#[derive(Clone, Debug)]
pub struct ImportData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

impl ImportData {
    pub fn read(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(io_error)?
            .take(IMPORT_PREVIEW_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > IMPORT_PREVIEW_BYTES {
            return Err(DbxError::Parse("Previewed imports are limited to 64 MiB; split the file or use the streaming SQL import".into()));
        }
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let data = match extension.as_str() {
            "csv" | "tsv" => {
                let mut reader = crate::transfer::DelimitedReader::new(&bytes, if extension == "tsv" { b'\t' } else { b',' });
                let headers: Vec<String> = reader.next_record().map_err(io_error)?.ok_or_else(|| DbxError::Parse("Missing header row".into()))?
                    .into_iter().map(|v| v.unwrap_or_default()).collect();
                Self { headers: headers.clone(), rows: Vec::new() }.validate()?;
                let mut rows = Vec::new();
                let mut budget = 0usize;
                while let Some(row) = reader.next_record().map_err(io_error)? {
                    let row: Vec<_> = row.into_iter().map(|v| v.map(CellValue::Text).unwrap_or(CellValue::Null)).collect();
                    if row.len() != headers.len() { return Err(DbxError::Parse(format!("Row {} has the wrong number of values", rows.len() + 1))); }
                    count_preview(&row, &mut budget)?;
                    rows.push(row);
                    check_rows(rows.len())?;
                }
                Self { headers, rows }
            }
            "json" | "jsonl" | "ndjson" => {
                let mut data = Self { headers: Vec::new(), rows: Vec::new() };
                let mut budget = 0usize;
                if extension == "json" {
                    struct Rows<'a>(&'a mut ImportData, &'a mut usize);
                    impl<'de> serde::de::Visitor<'de> for Rows<'_> {
                        type Value = ();
                        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("an array of row objects") }
                        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
                            while let Some(value) = seq.next_element::<serde_json::Value>()? {
                                self.0.push_object(value, self.1).map_err(serde::de::Error::custom)?;
                            }
                            Ok(())
                        }
                    }
                    let mut reader = serde_json::Deserializer::from_slice(&bytes);
                    serde::de::Deserializer::deserialize_seq(&mut reader, Rows(&mut data, &mut budget)).map_err(|e| DbxError::Parse(e.to_string()))?;
                    reader.end().map_err(|e| DbxError::Parse(e.to_string()))?;
                } else {
                    let text = std::str::from_utf8(&bytes).map_err(|e| DbxError::Parse(e.to_string()))?;
                    for (row, line) in text.lines().filter(|line| !line.trim().is_empty()).enumerate() {
                        let value = serde_json::from_str(line).map_err(|e| DbxError::Parse(format!("JSONL row {}: {e}", row + 1)))?;
                        data.push_object(value, &mut budget)?;
                    }
                }
                data
            }
            _ => return Err(DbxError::Parse("Preview supports CSV, TSV, JSON arrays and JSONL. Use SQL import for SQL or gzip dumps.".into())),
        };
        data.validate()?;
        Ok(data)
    }
    pub fn validate(&self) -> Result<()> {
        check_rows(self.rows.len())?;
        let mut names = HashSet::new();
        if self.headers.is_empty()
            || self.headers.len() > 1000
            || self
                .headers
                .iter()
                .any(|name| name.is_empty() || !names.insert(name))
        {
            return Err(DbxError::Parse(
                "Headers must be nonempty, unique names (at most 1000 columns)".into(),
            ));
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.len() != self.headers.len() {
                return Err(DbxError::Parse(format!(
                    "Row {} has {} values for {} headers",
                    index + 1,
                    row.len(),
                    self.headers.len()
                )));
            }
        }
        let mut budget = 0usize;
        for row in &self.rows {
            count_preview(row, &mut budget)?;
        }
        Ok(())
    }
    fn push_object(&mut self, value: serde_json::Value, budget: &mut usize) -> Result<()> {
        check_rows(self.rows.len() + 1)?;
        let object = value.as_object().ok_or_else(|| {
            DbxError::Parse(format!("Row {} is not an object", self.rows.len() + 1))
        })?;
        if self.rows.is_empty() {
            self.headers = object.keys().cloned().collect();
            self.validate()?;
        }
        if object.keys().any(|name| !self.headers.contains(name)) {
            return Err(DbxError::Parse(format!(
                "Row {} has fields absent from the first object",
                self.rows.len() + 1
            )));
        }
        let row = self
            .headers
            .iter()
            .map(|name| json_cell(object.get(name).unwrap_or(&serde_json::Value::Null)))
            .collect::<Vec<_>>();
        count_preview(&row, budget)?;
        self.rows.push(row);
        Ok(())
    }
    /// One optional destination name per input column; None omits a field so
    /// the destination's own default applies. No value conversion is guessed.
    pub fn default_mapping(&self, columns: &[ColumnInfo]) -> Vec<Option<String>> {
        self.headers
            .iter()
            .map(|name| {
                columns
                    .iter()
                    .find(|column| column.name == *name)
                    .map(|c| c.name.clone())
            })
            .collect()
    }
}

pub async fn import_data(
    engine: &DatabaseEngine,
    table: &TableRef,
    data: &ImportData,
    mapping: &[Option<String>],
    expected_database: &str,
    expected_columns: &[ColumnInfo],
) -> Result<u64> {
    data.validate()?;
    if engine.is_read_only() {
        return Err(DbxError::Query(
            "Protected connections cannot import data".into(),
        ));
    }
    if engine.current_database().await? != expected_database
        || engine.describe_table(table).await? != expected_columns
    {
        return Err(DbxError::Query(
            "The target database or schema changed after preview; preview again".into(),
        ));
    }
    if mapping.len() != data.headers.len() {
        return Err(DbxError::Parse(
            "Mapping width differs from the input".into(),
        ));
    }
    let mut used = HashSet::new();
    let mut mapped = Vec::new();
    for (position, name) in mapping.iter().enumerate() {
        if let Some(name) = name {
            if !expected_columns.iter().any(|c| c.name == *name) || !used.insert(name.clone()) {
                return Err(DbxError::Parse(format!(
                    "Unknown or duplicate destination column: {name}"
                )));
            }
            mapped.push((position, name.clone()));
        }
    }
    if mapped.is_empty() {
        return Err(DbxError::Parse("Map at least one input column".into()));
    }
    if engine.kind() == crate::DatabaseKind::MySQL
        && !crate::transfer::mysql_table_transactional(engine, table).await?
    {
        return Err(DbxError::Query(
            "Atomic imports require an InnoDB destination".into(),
        ));
    }
    let mut transaction = crate::console::SqlTransaction::begin(engine, false).await?;
    let names = mapped
        .iter()
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    for (index, row) in data.rows.iter().enumerate() {
        let values = mapped
            .iter()
            .map(|(position, _)| row[*position].clone())
            .collect::<Vec<_>>();
        let statement = crate::sql::build_multi_row_insert_with_columns(
            engine.kind(),
            table,
            &names,
            &[values],
            expected_columns,
        )
        .map_err(|e| DbxError::Parse(format!("Row {}: {e}", index + 1)))?;
        transaction.query(&statement).await.map_err(|e| {
            DbxError::Query(format!(
                "Row {} failed; the entire import is rolled back: {e}",
                index + 1
            ))
        })?;
        crate::transfer::report_transfer(1, 0);
    }
    transaction.commit().await?;
    Ok(data.rows.len() as u64)
}

/// Capture one consistent source snapshot for previewed cross-connection copy.
/// The same explicit memory/row budgets as file previews apply.
pub async fn snapshot_data(engine: &DatabaseEngine, table: &TableRef) -> Result<ImportData> {
    let columns = engine.describe_table(table).await?;
    let headers = columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    let statement =
        crate::build_select_with_columns(engine.kind(), table, &headers, &[], &[], None, &columns)?;
    let mut transaction = crate::console::SqlTransaction::begin(engine, true).await?;
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    transaction
        .stream_rows(&statement.sql, |_: &[crate::ColumnInfo], values| {
            if let Some(values) = values {
                count_preview(&values, &mut bytes)?;
                if bytes > IMPORT_PREVIEW_BYTES {
                    return Err(DbxError::Parse(
                        "Copy preview exceeds 64 MiB; use table export for a larger transfer"
                            .into(),
                    ));
                }
                rows.push(values);
                check_rows(rows.len())?;
            }
            Ok(())
        })
        .await?;
    transaction.commit().await?;
    Ok(ImportData { headers, rows })
}

// Count encoded values without allocating a second serialization of the preview.
fn count_preview(row: &[CellValue], bytes: &mut usize) -> Result<()> {
    struct Counter<'a>(&'a mut usize);
    impl std::io::Write for Counter<'_> {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            *self.0 = self.0.saturating_add(data.len());
            if *self.0 > IMPORT_PREVIEW_BYTES {
                return Err(std::io::Error::other(
                    "Typed preview exceeds 64 MiB; split the input",
                ));
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(bytes), row).map_err(|e| DbxError::Parse(e.to_string()))
}

fn json_cell(value: &serde_json::Value) -> CellValue {
    match value {
        serde_json::Value::Null => CellValue::Null,
        serde_json::Value::Bool(v) => CellValue::Boolean(*v),
        serde_json::Value::Number(v) => v
            .as_i64()
            .map(CellValue::Integer)
            .or_else(|| v.as_u64().map(CellValue::Unsigned))
            .or_else(|| v.as_f64().map(CellValue::Real))
            .unwrap_or_else(|| CellValue::Json(value.clone())),
        serde_json::Value::String(v) => CellValue::Text(v.clone()),
        _ => CellValue::Json(value.clone()),
    }
}
fn check_rows(rows: usize) -> Result<()> {
    if rows > IMPORT_PREVIEW_ROWS {
        Err(DbxError::Parse(
            "Previewed imports and copy are limited to 100,000 rows; split the data".into(),
        ))
    } else {
        Ok(())
    }
}
fn io_error(error: std::io::Error) -> DbxError {
    DbxError::Io(error.to_string())
}

#[derive(Debug, serde::Serialize)]
pub struct DataDiff {
    pub only_source: usize,
    pub only_target: usize,
    pub changed: usize,
    pub equal: usize,
}
/// Compare complete bounded snapshots by destination key. This reports counts
/// only and never generates or applies synchronization writes.
pub fn diff_data(source: &ImportData, target: &ImportData, keys: &[String]) -> Result<DataDiff> {
    source.validate()?;
    target.validate()?;
    if keys.is_empty() {
        return Err(DbxError::Parse(
            "Data comparison requires a primary key".into(),
        ));
    }
    let mut names = source.headers.clone();
    names.sort();
    let mut target_names = target.headers.clone();
    target_names.sort();
    if names != target_names {
        return Err(DbxError::Parse("Data comparison requires the same column names; map and copy first if the schemas differ".into()));
    }
    let index = |data: &ImportData| -> Result<std::collections::BTreeMap<String, String>> {
        let key_positions = keys
            .iter()
            .map(|key| {
                data.headers
                    .iter()
                    .position(|name| name == key)
                    .ok_or_else(|| DbxError::Parse(format!("Missing key column {key}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let positions = names
            .iter()
            .map(|name| data.headers.iter().position(|field| field == name).unwrap())
            .collect::<Vec<_>>();
        let mut indexed = std::collections::BTreeMap::new();
        for row in &data.rows {
            if key_positions
                .iter()
                .any(|position| matches!(row[*position], CellValue::Null))
            {
                return Err(DbxError::Parse(
                    "Comparison keys cannot contain NULL".into(),
                ));
            }
            let key = serde_json::to_string(
                &key_positions
                    .iter()
                    .map(|position| &row[*position])
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| DbxError::Parse(e.to_string()))?;
            let value = serde_json::to_string(
                &positions
                    .iter()
                    .map(|position| &row[*position])
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| DbxError::Parse(e.to_string()))?;
            if indexed.insert(key, value).is_some() {
                return Err(DbxError::Parse(
                    "Comparison key is not unique in a snapshot".into(),
                ));
            }
        }
        Ok(indexed)
    };
    let source = index(source)?;
    let target = index(target)?;
    let mut diff = DataDiff {
        only_source: 0,
        only_target: 0,
        changed: 0,
        equal: 0,
    };
    for (key, value) in &source {
        match target.get(key) {
            None => diff.only_source += 1,
            Some(other) if other == value => diff.equal += 1,
            Some(_) => diff.changed += 1,
        }
    }
    diff.only_target = target
        .keys()
        .filter(|key| !source.contains_key(*key))
        .count();
    Ok(diff)
}
