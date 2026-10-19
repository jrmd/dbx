//! Validated document and key edits. Raw query execution stays available;
//! structured editing adds identity and optimistic-concurrency checks.
use crate::{CellValue, DatabaseEngine, DatabaseKind, DbxError, QueryOptions, Result};
use serde_json::{Value, json};

/// Build a replacement command that cannot change MongoDB's immutable _id.
/// The full original document participates in the filter to avoid lost edits.
pub fn mongo_replacement(
    collection: &str,
    original: &Value,
    replacement: &Value,
) -> Result<String> {
    let old = original
        .as_object()
        .ok_or_else(|| DbxError::Parse("The original document must be a JSON object".into()))?;
    let new = replacement
        .as_object()
        .ok_or_else(|| DbxError::Parse("The replacement must be a JSON object".into()))?;
    let id = old
        .get("_id")
        .ok_or_else(|| DbxError::Parse("The selected document has no _id".into()))?;
    if new.get("_id") != Some(id) {
        return Err(DbxError::Parse(
            "_id is immutable; retain its original Extended JSON value".into(),
        ));
    }
    Ok(json!({"update":collection,"updates":[{"q":{"_id":id,"$expr":{"$eq":["$$ROOT",{"$literal":original}]}},"u":replacement,"multi":false,"upsert":false}],"ordered":true}).to_string())
}

impl DatabaseEngine {
    pub async fn replace_mongo_document(
        &self,
        collection: &str,
        original: &Value,
        replacement: &Value,
    ) -> Result<()> {
        if self.kind() != DatabaseKind::MongoDB {
            return Err(DbxError::Parse("This editor requires MongoDB".into()));
        }
        if self.is_read_only() {
            return Err(DbxError::Query(
                "Protected connection: writes are disabled".into(),
            ));
        }
        let result = self
            .query(
                &mongo_replacement(collection, original, replacement)?,
                QueryOptions::default(),
            )
            .await?;
        let number = |name: &str| {
            result
                .columns
                .iter()
                .position(|column| column.name == name)
                .and_then(|index| result.rows.first()?.values.get(index))
                .and_then(|value| match value {
                    CellValue::Integer(n) => Some(*n),
                    CellValue::Unsigned(n) => i64::try_from(*n).ok(),
                    CellValue::Real(n) => Some(*n as i64),
                    _ => None,
                })
        };
        if let Some(index) = result.columns.iter().position(|column| column.name == "writeErrors")
            && result.rows.first().and_then(|row| row.values.get(index)).is_some_and(|value| matches!(value, CellValue::Json(Value::Array(errors)) if !errors.is_empty())) {
            return Err(DbxError::Query("MongoDB rejected the document replacement".into()));
        }
        if number("n") != Some(1) {
            return Err(DbxError::Query(
                "The document changed or was removed. Reload it before editing again.".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mongo_edit_preserves_extended_id_and_checks_full_original() {
        let original =
            json!({"_id":{"$oid":"507f1f77bcf86cd799439011"},"balance":{"$numberDecimal":"12.34"}});
        let mut edited = original.clone();
        edited["balance"] = json!({"$numberDecimal":"22.34"});
        let command: Value = serde_json::from_str(
            &mongo_replacement("odd\"collection", &original, &edited).unwrap(),
        )
        .unwrap();
        assert_eq!(
            command["updates"][0]["q"]["$expr"]["$eq"][1]["$literal"],
            original
        );
        assert_eq!(command["updates"][0]["upsert"], false);
        edited["_id"] = json!("other");
        assert!(mongo_replacement("x", &original, &edited).is_err());
        assert!(mongo_replacement("x", &original, &json!([])).is_err());
    }
}
