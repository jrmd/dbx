//! Reusable per-field drafts for row inspection, update, and insert flows.
//!
//! [`FieldRow`] owns the GPUI entities needed to edit one column.  The
//! GPUI-independent [`FieldDraft`] and the extraction functions below keep
//! type conversion, null/default handling, and change detection testable
//! without a window or application context.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use dbx_core::{CellValue, ColumnInfo};
use gpui::{App, AppContext, Context, Entity, Window};

use crate::editor::TextEditor;

/// A stable identity for one field in a row draft.
pub type FieldId = u64;

/// Descriptive alias used by callers that name the identity after the draft
/// rather than the rendered field row.
pub type FieldDraftId = FieldId;

static NEXT_FIELD_ID: AtomicU64 = AtomicU64::new(1);

fn next_field_id() -> FieldId {
    NEXT_FIELD_ID.fetch_add(1, Ordering::Relaxed)
}

/// The state of a field's current value.
///
/// `Default` is meaningful for inserts: the field is omitted from the insert
/// request and the database supplies its default expression.  The core
/// mutation model has no SQL-`DEFAULT` value variant, so using `Default` for
/// an update is rejected by [`changed_fields`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FieldValueState {
    #[default]
    Value,
    Null,
    Default,
}

/// Short alias for call sites that use `FieldState` terminology.
pub type FieldState = FieldValueState;

impl FieldValueState {
    pub const fn is_null(self) -> bool {
        matches!(self, Self::Null)
    }

    pub const fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }
}

/// The scalar kinds supported by the field editor's text parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldValueKind {
    Boolean,
    Integer,
    Unsigned,
    Real,
    Bytes,
    Json,
    Text,
}

impl FieldValueKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Unsigned => "unsigned integer",
            Self::Real => "real number",
            Self::Bytes => "hex bytes",
            Self::Json => "JSON",
            Self::Text => "text",
        }
    }
}

impl fmt::Display for FieldValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// Classify a [`ColumnInfo`] for typed field parsing.
pub fn field_value_kind(column: &ColumnInfo) -> FieldValueKind {
    let data_type = column.data_type.trim().to_ascii_lowercase();

    if data_type.contains("json") {
        FieldValueKind::Json
    } else if data_type.contains("blob")
        || data_type.contains("bytea")
        || data_type.contains("binary")
    {
        FieldValueKind::Bytes
    } else if data_type.contains("bool")
        || data_type == "bit"
        || data_type.starts_with("bit(")
        || data_type.starts_with("tinyint(1")
    {
        FieldValueKind::Boolean
    } else if data_type.contains("unsigned")
        || data_type.starts_with("uint")
        || data_type.starts_with("ubigint")
    {
        FieldValueKind::Unsigned
    } else if data_type.contains("int")
        || data_type.contains("serial")
        || data_type.starts_with("sint")
    {
        FieldValueKind::Integer
    } else if data_type.contains("real")
        || data_type.contains("double")
        || data_type.contains("float")
        || data_type.contains("decimal")
        || data_type.contains("numeric")
        || data_type == "number"
    {
        FieldValueKind::Real
    } else {
        FieldValueKind::Text
    }
}

/// A typed value parsing failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldValueError {
    pub expected: FieldValueKind,
    pub input: String,
    pub reason: String,
}

impl FieldValueError {
    fn new(expected: FieldValueKind, input: &str, reason: impl Into<String>) -> Self {
        Self {
            expected,
            input: input.to_owned(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for FieldValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "expected {}, got {:?}: {}",
            self.expected, self.input, self.reason
        )
    }
}

impl std::error::Error for FieldValueError {}

/// Parse editor text according to the database type reported by `column`.
///
/// SQL NULL is selected through [`FieldValueState::Null`], keeping the literal
/// text `NULL` available for text columns and JSON `null` available for JSON.
pub fn parse_field_value(column: &ColumnInfo, text: &str) -> Result<CellValue, FieldValueError> {
    let kind = field_value_kind(column);
    let trimmed = text.trim();
    match kind {
        FieldValueKind::Boolean => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" | "yes" | "y" | "on" => Ok(CellValue::Boolean(true)),
            "false" | "f" | "0" | "no" | "n" | "off" => Ok(CellValue::Boolean(false)),
            _ => Err(FieldValueError::new(kind, text, "use true or false")),
        },
        FieldValueKind::Integer => trimmed
            .parse::<i64>()
            .map(CellValue::Integer)
            .map_err(|_| FieldValueError::new(kind, text, "use a signed base-10 integer")),
        FieldValueKind::Unsigned => trimmed
            .parse::<u64>()
            .map(CellValue::Unsigned)
            .map_err(|_| FieldValueError::new(kind, text, "use a non-negative base-10 integer")),
        FieldValueKind::Real => match trimmed.parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(CellValue::Real(number)),
            Ok(_) => Err(FieldValueError::new(
                kind,
                text,
                "use a finite decimal number",
            )),
            Err(_) => Err(FieldValueError::new(kind, text, "use a decimal number")),
        },
        FieldValueKind::Bytes => parse_hex_bytes(trimmed)
            .map(CellValue::Bytes)
            .map_err(|reason| FieldValueError::new(kind, text, reason)),
        FieldValueKind::Json => serde_json::from_str(trimmed)
            .map(CellValue::Json)
            .map_err(|error| FieldValueError::new(kind, text, format!("invalid JSON ({error})"))),
        FieldValueKind::Text => Ok(CellValue::Text(text.to_owned())),
    }
}

fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, &'static str> {
    let digits = text.strip_prefix("0x").unwrap_or(text);
    if !digits.len().is_multiple_of(2) {
        return Err("use an even number of hexadecimal digits, optionally prefixed with 0x");
    }
    digits
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_| "use hexadecimal digits only")?;
            u8::from_str_radix(pair, 16).map_err(|_| "use hexadecimal digits only")
        })
        .collect()
}

/// Alias matching the core model's `CellValue` terminology.
pub fn parse_cell_value_for_column(
    column: &ColumnInfo,
    text: &str,
) -> Result<CellValue, FieldValueError> {
    parse_field_value(column, text)
}

/// A pure field draft, suitable for validation and extraction tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldDraft {
    pub id: FieldId,
    pub column: String,
    /// The value before editing. `None` means this is an insert field with no
    /// existing value; `Some(CellValue::Null)` is an existing SQL NULL.
    pub original: Option<CellValue>,
    /// Text currently shown by the field editor. It is ignored for Null and
    /// Default states, but retained so switching back to Value is lossless.
    pub current: String,
    pub state: FieldValueState,
    /// Optional metadata for a known database default. Table introspection
    /// does not expose this on every engine, so `Default` still means "omit
    /// this column" when this is `None`.
    pub default_expression: Option<String>,
    /// Read-only fields can remain in an inspector model without being sent
    /// as assignments or insert values.
    pub editable: bool,
}

impl FieldDraft {
    pub fn new(
        id: FieldId,
        column: impl Into<String>,
        original: Option<CellValue>,
        current: impl Into<String>,
        state: FieldValueState,
        default_expression: Option<String>,
    ) -> Self {
        Self {
            id,
            column: column.into(),
            original,
            current: current.into(),
            state,
            default_expression,
            editable: true,
        }
    }

    pub fn with_new_id(
        column: impl Into<String>,
        original: Option<CellValue>,
        current: impl Into<String>,
        state: FieldValueState,
        default_expression: Option<String>,
    ) -> Self {
        Self::new(
            next_field_id(),
            column,
            original,
            current,
            state,
            default_expression,
        )
    }

    pub fn selected_column(&self) -> &str {
        &self.column
    }

    pub fn is_changed(&self, value: &CellValue) -> bool {
        self.original.as_ref() != Some(value)
    }
}

impl Default for FieldDraft {
    fn default() -> Self {
        Self::new(0, "", None, "", FieldValueState::Value, None)
    }
}

/// One editable column in a row inspector or insert form.
pub struct FieldRow {
    pub id: FieldId,
    pub column: ColumnInfo,
    pub original: Option<CellValue>,
    pub value: Entity<String>,
    pub editor: Entity<TextEditor>,
    pub state: FieldValueState,
    pub default_expression: Option<String>,
    pub editable: bool,
}

/// Alias emphasizing that a field row is an entity-backed draft.
pub type FieldDraftRow = FieldRow;

impl FieldRow {
    /// Construct a row field. `original = None` is the insert case.
    pub fn new<T: 'static>(
        column: ColumnInfo,
        original: Option<CellValue>,
        default_expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let state = match original.as_ref() {
            Some(CellValue::Null) => FieldValueState::Null,
            Some(_) => FieldValueState::Value,
            None if default_expression.is_some() => FieldValueState::Default,
            None => FieldValueState::Value,
        };
        let initial_text = original
            .as_ref()
            .filter(|value| !matches!(value, CellValue::Null))
            .map(ToString::to_string)
            .unwrap_or_default();
        Self::with_state(
            column,
            original,
            initial_text,
            state,
            default_expression,
            window,
            cx,
        )
    }

    /// Construct an insert field explicitly. Insert fields start omitted so
    /// generated columns and database defaults are never overwritten merely
    /// because the form was opened.
    pub fn new_insert<T: 'static>(
        column: ColumnInfo,
        default_expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        Self::with_state(
            column,
            None,
            String::new(),
            FieldValueState::Default,
            default_expression,
            window,
            cx,
        )
    }

    /// Construct an update field explicitly from its original value.
    pub fn new_update<T: 'static>(
        column: ColumnInfo,
        original: CellValue,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        Self::new(column, Some(original), None, window, cx)
    }

    /// Construct a row field with an explicitly selected state and text.
    pub fn with_state<T: 'static>(
        column: ColumnInfo,
        original: Option<CellValue>,
        initial_text: impl Into<String>,
        state: FieldValueState,
        default_expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let value = cx.new(|_| initial_text.into());
        let editor = cx.new(|editor_cx| TextEditor::new(value.clone(), false, window, editor_cx));
        Self {
            id: next_field_id(),
            column,
            original,
            value,
            editor,
            state,
            default_expression,
            editable: true,
        }
    }

    pub fn selected_column(&self) -> &str {
        &self.column.name
    }

    pub fn column_name(&self) -> &str {
        self.selected_column()
    }

    pub fn field_id(&self) -> FieldId {
        self.id
    }

    pub fn set_editable(&mut self, editable: bool) {
        self.editable = editable;
    }

    pub fn set_state(&mut self, state: FieldValueState) {
        self.state = state;
    }

    pub fn set_null(&mut self) {
        self.state = FieldValueState::Null;
    }

    pub fn set_default(&mut self) {
        self.state = FieldValueState::Default;
    }

    pub fn set_value(&mut self) {
        self.state = FieldValueState::Value;
    }

    /// Snapshot this entity-backed field into pure data.
    pub fn draft(&self, cx: &App) -> FieldDraft {
        FieldDraft {
            id: self.id,
            column: self.column.name.clone(),
            original: self.original.clone(),
            current: self.value.read(cx).clone(),
            state: self.state,
            default_expression: self.default_expression.clone(),
            editable: self.editable,
        }
    }
}

/// Ordered collection of fields for one row or one insert form.
pub struct RowDraftModel {
    fields: Vec<FieldRow>,
}

/// Alias emphasizing that this model is the row inspector's field collection.
pub type RowFieldsModel = RowDraftModel;

/// Alias emphasizing that callers can use this as a field-draft model.
pub type FieldDraftModel = RowDraftModel;

impl Default for RowDraftModel {
    fn default() -> Self {
        Self::new()
    }
}

impl RowDraftModel {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn from_fields(fields: Vec<FieldRow>) -> Self {
        Self { fields }
    }

    pub fn fields(&self) -> &[FieldRow] {
        &self.fields
    }

    pub fn fields_mut(&mut self) -> &mut [FieldRow] {
        &mut self.fields
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn push(&mut self, field: FieldRow) -> FieldId {
        let id = field.id;
        self.fields.push(field);
        id
    }

    pub fn add_field<T: 'static>(
        &mut self,
        column: ColumnInfo,
        original: Option<CellValue>,
        default_expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> FieldId {
        let field = FieldRow::new(column, original, default_expression, window, cx);
        self.push(field)
    }

    pub fn add_insert_field<T: 'static>(
        &mut self,
        column: ColumnInfo,
        default_expression: Option<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> FieldId {
        let field = FieldRow::new_insert(column, default_expression, window, cx);
        self.push(field)
    }

    pub fn remove(&mut self, id: FieldId) -> Option<FieldRow> {
        let index = self.fields.iter().position(|field| field.id == id)?;
        Some(self.fields.remove(index))
    }

    pub fn move_field(&mut self, id: FieldId, new_index: usize) -> bool {
        let Some(old_index) = self.fields.iter().position(|field| field.id == id) else {
            return false;
        };
        let field = self.fields.remove(old_index);
        self.fields.insert(new_index.min(self.fields.len()), field);
        true
    }

    pub fn drafts(&self, cx: &App) -> Vec<FieldDraft> {
        self.fields.iter().map(|field| field.draft(cx)).collect()
    }

    pub fn changed_fields(&self, cx: &App) -> Result<Vec<FieldAssignment>, FieldValidationError> {
        changed_fields(&self.drafts(cx), &self.columns())
    }

    pub fn changed_assignments(
        &self,
        cx: &App,
    ) -> Result<Vec<FieldAssignment>, FieldValidationError> {
        self.changed_fields(cx)
    }

    pub fn insert_values(&self, cx: &App) -> Result<Vec<FieldAssignment>, FieldValidationError> {
        insert_values(&self.drafts(cx), &self.columns())
    }

    pub fn insert_assignments(
        &self,
        cx: &App,
    ) -> Result<Vec<FieldAssignment>, FieldValidationError> {
        self.insert_values(cx)
    }

    fn columns(&self) -> Vec<ColumnInfo> {
        self.fields
            .iter()
            .map(|field| field.column.clone())
            .collect()
    }
}

/// A column/value pair ready for `UpdateRequest.assignments` or an
/// `InsertRequest`'s `columns`/`values` vectors.
pub type FieldAssignment = (String, CellValue);

/// Whether pure fields are being resolved for an update or an insert.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldDraftMode {
    Update,
    Insert,
}

/// A resolved field value, retaining identity and state for callers that
/// need more than the tuple-shaped extraction helpers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedField {
    pub id: FieldId,
    pub column: String,
    pub value: Option<CellValue>,
    pub state: FieldValueState,
}

/// Why one field could not be validated or extracted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldValidationError {
    MissingColumn {
        field_id: FieldId,
        field_index: usize,
    },
    UnknownColumn {
        field_id: FieldId,
        field_index: usize,
        column: String,
    },
    NullNotAllowed {
        field_id: FieldId,
        field_index: usize,
        column: String,
    },
    DefaultNotSupportedForUpdate {
        field_id: FieldId,
        field_index: usize,
        column: String,
    },
    InvalidValue {
        field_id: FieldId,
        field_index: usize,
        column: String,
        expected: FieldValueKind,
        input: String,
        reason: String,
    },
}

impl FieldValidationError {
    pub fn field_id(&self) -> FieldId {
        match self {
            Self::MissingColumn { field_id, .. }
            | Self::UnknownColumn { field_id, .. }
            | Self::NullNotAllowed { field_id, .. }
            | Self::DefaultNotSupportedForUpdate { field_id, .. }
            | Self::InvalidValue { field_id, .. } => *field_id,
        }
    }

    pub fn field_index(&self) -> usize {
        match self {
            Self::MissingColumn { field_index, .. }
            | Self::UnknownColumn { field_index, .. }
            | Self::NullNotAllowed { field_index, .. }
            | Self::DefaultNotSupportedForUpdate { field_index, .. }
            | Self::InvalidValue { field_index, .. } => *field_index,
        }
    }
}

impl fmt::Display for FieldValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingColumn { field_index, .. } => {
                write!(
                    formatter,
                    "Field {} has no column selected",
                    field_index + 1
                )
            }
            Self::UnknownColumn {
                field_index,
                column,
                ..
            } => write!(
                formatter,
                "Field {} references unknown column {:?}",
                field_index + 1,
                column
            ),
            Self::NullNotAllowed {
                field_index,
                column,
                ..
            } => write!(
                formatter,
                "Field {} ({:?}) cannot be NULL because the column is not nullable",
                field_index + 1,
                column
            ),
            Self::DefaultNotSupportedForUpdate {
                field_index,
                column,
                ..
            } => write!(
                formatter,
                "Field {} ({:?}) cannot use DEFAULT during an update",
                field_index + 1,
                column
            ),
            Self::InvalidValue {
                field_index,
                column,
                expected,
                input,
                reason,
                ..
            } => write!(
                formatter,
                "Field {} ({:?}) has invalid {} value {:?}: {}",
                field_index + 1,
                column,
                expected,
                input,
                reason
            ),
        }
    }
}

impl std::error::Error for FieldValidationError {}

fn resolve_field_drafts(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
    mode: FieldDraftMode,
) -> Result<Vec<ResolvedField>, FieldValidationError> {
    drafts
        .iter()
        .enumerate()
        .map(|(field_index, draft)| {
            let column_name = draft.column.trim();
            if column_name.is_empty() {
                return Err(FieldValidationError::MissingColumn {
                    field_id: draft.id,
                    field_index,
                });
            }
            let column = columns
                .iter()
                .find(|column| column.name == column_name)
                .ok_or_else(|| FieldValidationError::UnknownColumn {
                    field_id: draft.id,
                    field_index,
                    column: column_name.to_owned(),
                })?;

            if !draft.editable {
                return Ok(ResolvedField {
                    id: draft.id,
                    column: column.name.clone(),
                    value: None,
                    state: draft.state,
                });
            }

            let value = match draft.state {
                FieldValueState::Null => {
                    if !column.nullable {
                        return Err(FieldValidationError::NullNotAllowed {
                            field_id: draft.id,
                            field_index,
                            column: column.name.clone(),
                        });
                    }
                    Some(CellValue::Null)
                }
                FieldValueState::Default => {
                    if mode == FieldDraftMode::Update {
                        return Err(FieldValidationError::DefaultNotSupportedForUpdate {
                            field_id: draft.id,
                            field_index,
                            column: column.name.clone(),
                        });
                    }
                    None
                }
                FieldValueState::Value => {
                    let parsed = parse_field_value(column, &draft.current).map_err(|error| {
                        FieldValidationError::InvalidValue {
                            field_id: draft.id,
                            field_index,
                            column: column.name.clone(),
                            expected: error.expected,
                            input: error.input,
                            reason: error.reason,
                        }
                    })?;
                    if matches!(parsed, CellValue::Null) && !column.nullable {
                        return Err(FieldValidationError::NullNotAllowed {
                            field_id: draft.id,
                            field_index,
                            column: column.name.clone(),
                        });
                    }
                    Some(parsed)
                }
            };

            Ok(ResolvedField {
                id: draft.id,
                column: column.name.clone(),
                value,
                state: draft.state,
            })
        })
        .collect()
}

/// Resolve all drafts for an explicit update/insert mode.
pub fn resolve_field_drafts_for_mode(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
    mode: FieldDraftMode,
) -> Result<Vec<ResolvedField>, FieldValidationError> {
    resolve_field_drafts(drafts, columns, mode)
}

/// Validate the current values without choosing an extraction mode.
///
/// This uses insert semantics so a valid `Default` state is accepted and
/// represented as an omitted (`None`) resolved value.
pub fn validate_field_drafts(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
) -> Result<(), FieldValidationError> {
    resolve_field_drafts(drafts, columns, FieldDraftMode::Insert).map(|_| ())
}

/// Extract only editable fields whose typed value differs from `original`.
pub fn changed_fields(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
) -> Result<Vec<FieldAssignment>, FieldValidationError> {
    Ok(
        resolve_field_drafts(drafts, columns, FieldDraftMode::Update)?
            .into_iter()
            .zip(drafts)
            .filter_map(|(resolved, draft)| {
                let unchanged_text = match (&draft.original, draft.state) {
                    (Some(CellValue::Null), FieldValueState::Null) => true,
                    (Some(original), FieldValueState::Value) => {
                        draft.current == original.to_string()
                    }
                    _ => false,
                };
                if unchanged_text {
                    return None;
                }
                resolved.value.and_then(|value| {
                    (draft.original.as_ref() != Some(&value)).then_some((resolved.column, value))
                })
            })
            .collect(),
    )
}

/// Extract values for an insert, omitting fields explicitly marked Default.
pub fn insert_values(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
) -> Result<Vec<FieldAssignment>, FieldValidationError> {
    resolve_field_drafts(drafts, columns, FieldDraftMode::Insert).map(|resolved| {
        resolved
            .into_iter()
            .filter_map(|field| field.value.map(|value| (field.column, value)))
            .collect()
    })
}

/// More explicit aliases for call sites that prefer extraction verbs.
pub fn extract_changed_fields(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
) -> Result<Vec<FieldAssignment>, FieldValidationError> {
    changed_fields(drafts, columns)
}

pub fn extract_insert_values(
    drafts: &[FieldDraft],
    columns: &[ColumnInfo],
) -> Result<Vec<FieldAssignment>, FieldValidationError> {
    insert_values(drafts, columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, data_type: &str, nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            nullable,
            ordinal: 0,
            primary_key: false,
        }
    }

    #[test]
    fn parser_handles_supported_column_types() {
        assert_eq!(
            parse_field_value(&column("enabled", "BOOLEAN", false), "true").unwrap(),
            CellValue::Boolean(true)
        );
        assert_eq!(
            parse_field_value(&column("count", "BIGINT", false), "-4").unwrap(),
            CellValue::Integer(-4)
        );
        assert_eq!(
            parse_field_value(&column("count", "BIGINT UNSIGNED", false), "4").unwrap(),
            CellValue::Unsigned(4)
        );
        assert_eq!(
            parse_field_value(&column("ratio", "DOUBLE PRECISION", false), "1.5").unwrap(),
            CellValue::Real(1.5)
        );
        assert_eq!(
            parse_field_value(&column("body", "JSONB", true), r#"{"ok":true}"#).unwrap(),
            CellValue::Json(serde_json::json!({ "ok": true }))
        );
        assert_eq!(
            parse_field_value(&column("payload", "BYTEA", false), "0xdead").unwrap(),
            CellValue::Bytes(vec![0xde, 0xad])
        );
        assert_eq!(
            parse_field_value(&column("name", "TEXT", false), "Ada ").unwrap(),
            CellValue::Text("Ada ".into())
        );
        assert_eq!(
            parse_field_value(&column("body", "JSONB", true), "null").unwrap(),
            CellValue::Json(serde_json::Value::Null)
        );
        assert_eq!(
            parse_field_value(&column("name", "TEXT", true), "NULL").unwrap(),
            CellValue::Text("NULL".into())
        );
    }

    #[test]
    fn changed_fields_are_typed_ordered_and_ignore_unchanged_values() {
        let columns = vec![
            column("id", "INTEGER", false),
            column("name", "TEXT", false),
            column("active", "BOOLEAN", true),
        ];
        let drafts = vec![
            FieldDraft::new(
                1,
                "id",
                Some(CellValue::Integer(7)),
                "7",
                FieldValueState::Value,
                None,
            ),
            FieldDraft::new(
                2,
                "name",
                Some(CellValue::Text("Ada".into())),
                "Grace",
                FieldValueState::Value,
                None,
            ),
            FieldDraft::new(
                3,
                "active",
                Some(CellValue::Boolean(true)),
                "",
                FieldValueState::Null,
                None,
            ),
        ];

        assert_eq!(
            changed_fields(&drafts, &columns).unwrap(),
            vec![
                ("name".into(), CellValue::Text("Grace".into())),
                ("active".into(), CellValue::Null),
            ]
        );
    }

    #[test]
    fn insert_values_omit_defaults_and_keep_explicit_nulls() {
        let columns = vec![
            column("id", "INTEGER", false),
            column("name", "TEXT", false),
            column("note", "TEXT", true),
        ];
        let drafts = vec![
            FieldDraft::new(
                1,
                "id",
                None,
                "",
                FieldValueState::Default,
                Some("nextval('items_id_seq')".into()),
            ),
            FieldDraft::new(2, "name", None, "Ada", FieldValueState::Value, None),
            FieldDraft::new(3, "note", None, "", FieldValueState::Null, None),
        ];

        assert_eq!(
            insert_values(&drafts, &columns).unwrap(),
            vec![
                ("name".into(), CellValue::Text("Ada".into())),
                ("note".into(), CellValue::Null),
            ]
        );
    }

    #[test]
    fn insert_can_omit_columns_without_introspected_default_metadata() {
        let columns = vec![
            column("id", "INTEGER", false),
            column("name", "TEXT", false),
        ];
        let drafts = vec![
            FieldDraft::new(1, "id", None, "", FieldValueState::Default, None),
            FieldDraft::new(2, "name", None, "", FieldValueState::Default, None),
        ];

        assert!(insert_values(&drafts, &columns).unwrap().is_empty());
    }

    #[test]
    fn unchanged_binary_display_is_not_rewritten_as_text() {
        let columns = vec![column("payload", "BLOB", false)];
        let drafts = vec![FieldDraft::new(
            1,
            "payload",
            Some(CellValue::Bytes(vec![0xde, 0xad])),
            "0xdead",
            FieldValueState::Value,
            None,
        )];

        assert!(changed_fields(&drafts, &columns).unwrap().is_empty());
    }

    #[test]
    fn invalid_values_and_nullability_errors_are_clear() {
        let columns = vec![column("id", "INTEGER", false), column("name", "TEXT", true)];
        let error = changed_fields(
            &[FieldDraft::new(
                7,
                "id",
                Some(CellValue::Integer(1)),
                "nope",
                FieldValueState::Value,
                None,
            )],
            &columns,
        )
        .unwrap_err();
        assert!(error.to_string().contains("signed base-10 integer"));

        let error = insert_values(
            &[FieldDraft::new(
                8,
                "id",
                None,
                "",
                FieldValueState::Null,
                None,
            )],
            &columns,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot be NULL"));
    }

    #[test]
    fn stable_ids_are_not_positions() {
        let first = FieldDraft::with_new_id("one", None, "", FieldValueState::Value, None);
        let second = FieldDraft::with_new_id("two", None, "", FieldValueState::Value, None);
        assert_ne!(first.id, second.id);
    }
}
