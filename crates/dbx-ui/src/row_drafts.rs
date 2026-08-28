//! Reusable per-field drafts for row inspection, update, and insert flows.
//!
//! [`FieldRow`] owns the GPUI entities needed to edit one column.  The
//! GPUI-independent [`FieldDraft`] and the extraction functions below keep
//! type conversion, null/default handling, and change detection testable
//! without a window or application context.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use dbx_core::{CellValue, ColumnInfo, MutationValue, validate_sql_expression};
use gpui::{App, AppContext, Context, Entity, SharedString, Window};
use gpui_component::{
    IndexPath,
    select::{SearchableVec, SelectState},
};

use crate::editor::{EditorLanguage, TextEditor};

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
/// request and the database supplies its default expression. `Sql` is an
/// explicit escape hatch for one validated database expression; ordinary
/// values remain parameterized. Using `Default` for an update is rejected by
/// [`changed_fields`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FieldValueState {
    #[default]
    Value,
    Sql,
    Null,
    Default,
}

/// Short alias for call sites that use `FieldState` terminology.
pub type FieldState = FieldValueState;

impl FieldValueState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Value => "Value",
            Self::Sql => "SQL",
            Self::Null => "NULL",
            Self::Default => "Default",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Value" => Some(Self::Value),
            "SQL" => Some(Self::Sql),
            "NULL" => Some(Self::Null),
            "Default" => Some(Self::Default),
            _ => None,
        }
    }

    pub const fn is_null(self) -> bool {
        matches!(self, Self::Null)
    }

    pub const fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }

    pub const fn is_sql(self) -> bool {
        matches!(self, Self::Sql)
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
        .as_chunks::<2>()
        .0
        .iter()
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
    /// A single-line SQL editor shares the canonical value entity with the
    /// typed editor. Switching modes keeps the user's text losslessly while
    /// changing how it is highlighted and submitted.
    pub sql_editor: Entity<TextEditor>,
    /// A native select for database enum columns. The text editor remains
    /// available as the canonical value entity for validation and mutation.
    pub enum_selector: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    /// Boolean columns use a native true/false selector instead of accepting
    /// a loose collection of textual aliases in the primary editing path.
    pub boolean_selector: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
    /// One compact selector replaces the repeated Value / NULL / Default
    /// button cluster when a column supports more than one value state.
    pub state_selector: Option<Entity<SelectState<SearchableVec<SharedString>>>>,
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
            .map(field_editor_text)
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
        let initial_text = initial_text.into();
        let is_insert = original.is_none();
        let value_kind = field_value_kind(&column);
        let value = cx.new(|_| initial_text.clone());
        let editor = cx.new(|editor_cx| {
            if value_kind == FieldValueKind::Json {
                TextEditor::new_json(value.clone(), window, editor_cx)
            } else {
                TextEditor::new(value.clone(), false, window, editor_cx)
            }
        });
        let sql_editor = cx.new(|editor_cx| {
            TextEditor::new_with_language(
                value.clone(),
                false,
                EditorLanguage::Sql,
                window,
                editor_cx,
            )
        });
        let enum_selector = if column.enum_values.is_empty() {
            None
        } else {
            let items = SearchableVec::new(
                column
                    .enum_values
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            );
            let selected_index = column
                .enum_values
                .iter()
                .position(|option| option == &initial_text);
            Some(cx.new(|select_cx| {
                SelectState::new(items, selected_index.map(IndexPath::new), window, select_cx)
            }))
        };
        let boolean_selector = if value_kind == FieldValueKind::Boolean {
            let options = ["true", "false"];
            let selected_index = options
                .iter()
                .position(|option| option.eq_ignore_ascii_case(initial_text.trim()));
            let items = SearchableVec::new(
                options
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            );
            Some(cx.new(|select_cx| {
                SelectState::new(items, selected_index.map(IndexPath::new), window, select_cx)
            }))
        } else {
            None
        };
        let state_options = field_state_options(is_insert, column.nullable);
        let state_selector = if state_options.is_empty() {
            None
        } else {
            let selected_index = state_options.iter().position(|option| *option == state);
            let items = SearchableVec::new(
                state_options
                    .into_iter()
                    .map(|option| SharedString::from(option.label()))
                    .collect::<Vec<_>>(),
            );
            Some(cx.new(|select_cx| {
                SelectState::new(items, selected_index.map(IndexPath::new), window, select_cx)
            }))
        };
        Self {
            id: next_field_id(),
            column,
            original,
            value,
            editor,
            sql_editor,
            enum_selector,
            boolean_selector,
            state_selector,
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

    pub fn value_kind(&self) -> FieldValueKind {
        field_value_kind(&self.column)
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

fn field_editor_text(value: &CellValue) -> String {
    match value {
        CellValue::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        value => value.to_string(),
    }
}

/// Valid value modes for one field. SQL is always explicit and available on
/// SQL-backed row mutations; NULL and database omission remain constrained by
/// column/mutation semantics.
pub fn field_state_options(is_insert: bool, nullable: bool) -> Vec<FieldValueState> {
    let mut options = vec![FieldValueState::Value, FieldValueState::Sql];
    if nullable {
        options.push(FieldValueState::Null);
    }
    if is_insert {
        options.push(FieldValueState::Default);
    }
    options
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
pub type FieldAssignment = (String, MutationValue);

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
    pub value: Option<MutationValue>,
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
    InvalidSqlExpression {
        field_id: FieldId,
        field_index: usize,
        column: String,
        input: String,
        reason: String,
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
            | Self::InvalidSqlExpression { field_id, .. }
            | Self::InvalidValue { field_id, .. } => *field_id,
        }
    }

    pub fn field_index(&self) -> usize {
        match self {
            Self::MissingColumn { field_index, .. }
            | Self::UnknownColumn { field_index, .. }
            | Self::NullNotAllowed { field_index, .. }
            | Self::DefaultNotSupportedForUpdate { field_index, .. }
            | Self::InvalidSqlExpression { field_index, .. }
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
            Self::InvalidSqlExpression {
                field_index,
                column,
                input,
                reason,
                ..
            } => write!(
                formatter,
                "Field {} ({:?}) has invalid SQL expression {:?}: {}",
                field_index + 1,
                column,
                input,
                reason
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
                    Some(MutationValue::Parameter(CellValue::Null))
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
                FieldValueState::Sql => {
                    let expression = validate_sql_expression(&draft.current).map_err(|error| {
                        FieldValidationError::InvalidSqlExpression {
                            field_id: draft.id,
                            field_index,
                            column: column.name.clone(),
                            input: draft.current.clone(),
                            reason: error.to_string(),
                        }
                    })?;
                    Some(MutationValue::Expression(expression.to_owned()))
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
                    Some(MutationValue::Parameter(parsed))
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
                resolved.value.and_then(|value| match &value {
                    MutationValue::Parameter(value) => (draft.original.as_ref() != Some(value))
                        .then_some((resolved.column, MutationValue::Parameter(value.clone()))),
                    MutationValue::Expression(_) => Some((resolved.column, value)),
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
    use gpui::{IntoElement, Render, TestAppContext, div};

    fn column(name: &str, data_type: &str, nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            enum_values: Vec::new(),
            nullable,
            ordinal: 0,
            primary_key: false,
        }
    }

    struct FieldRowHarness {
        field: FieldRow,
    }

    impl Render for FieldRowHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn boolean_fields_build_a_true_false_selector(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (view, cx) = cx.add_window_view(|window, cx| FieldRowHarness {
            field: FieldRow::new_update(
                column("enabled", "BOOLEAN", false),
                CellValue::Boolean(false),
                window,
                cx,
            ),
        });

        cx.update(|_, cx| {
            let field = &view.read(cx).field;
            let selector = field
                .boolean_selector
                .as_ref()
                .expect("boolean field should have a selector");
            assert_eq!(
                selector.read(cx).selected_value().map(ToString::to_string),
                Some("false".into())
            );
            assert!(field.enum_selector.is_none());
        });
    }

    #[test]
    fn field_state_labels_round_trip_for_compact_selectors() {
        for state in [
            FieldValueState::Value,
            FieldValueState::Sql,
            FieldValueState::Null,
            FieldValueState::Default,
        ] {
            assert_eq!(FieldValueState::from_label(state.label()), Some(state));
        }
        assert_eq!(FieldValueState::from_label("Unknown"), None);
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
                (
                    "name".into(),
                    MutationValue::Parameter(CellValue::Text("Grace".into()))
                ),
                ("active".into(), MutationValue::Parameter(CellValue::Null)),
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
                (
                    "name".into(),
                    MutationValue::Parameter(CellValue::Text("Ada".into()))
                ),
                ("note".into(), MutationValue::Parameter(CellValue::Null)),
            ]
        );
    }

    #[test]
    fn field_state_options_keep_sql_explicit_in_insert_and_update_modes() {
        assert_eq!(
            field_state_options(false, false),
            vec![FieldValueState::Value, FieldValueState::Sql]
        );
        assert_eq!(
            field_state_options(false, true),
            vec![
                FieldValueState::Value,
                FieldValueState::Sql,
                FieldValueState::Null,
            ]
        );
        assert_eq!(
            field_state_options(true, true),
            vec![
                FieldValueState::Value,
                FieldValueState::Sql,
                FieldValueState::Null,
                FieldValueState::Default,
            ]
        );
    }

    #[test]
    fn sql_expressions_are_extracted_without_parsing_as_column_values() {
        let columns = vec![
            column("id", "UUID", false),
            column("updated_at", "TIMESTAMPTZ", false),
        ];
        let insert = vec![
            FieldDraft::new(1, "id", None, "uuidv7()", FieldValueState::Sql, None),
            FieldDraft::new(2, "updated_at", None, " NOW() ", FieldValueState::Sql, None),
        ];
        assert_eq!(
            insert_values(&insert, &columns).unwrap(),
            vec![
                ("id".into(), MutationValue::Expression("uuidv7()".into())),
                (
                    "updated_at".into(),
                    MutationValue::Expression("NOW()".into())
                ),
            ]
        );

        let update = vec![FieldDraft::new(
            3,
            "updated_at",
            Some(CellValue::Text("2026-08-23T12:00:00Z".into())),
            "NOW()",
            FieldValueState::Sql,
            None,
        )];
        assert_eq!(
            changed_fields(&update, &columns[1..]).unwrap(),
            vec![(
                "updated_at".into(),
                MutationValue::Expression("NOW()".into())
            )]
        );
    }

    #[test]
    fn unsafe_or_empty_sql_expressions_fail_before_the_mutation_runs() {
        let columns = vec![column("id", "UUID", false)];
        for expression in ["", "uuidv7(); DROP TABLE users"] {
            let error = insert_values(
                &[FieldDraft::new(
                    1,
                    "id",
                    None,
                    expression,
                    FieldValueState::Sql,
                    None,
                )],
                &columns,
            )
            .unwrap_err();
            assert!(matches!(
                error,
                FieldValidationError::InvalidSqlExpression { .. }
            ));
        }
    }

    #[test]
    fn default_remains_insert_only() {
        let columns = vec![column("id", "INTEGER", false)];
        let error = changed_fields(
            &[FieldDraft::new(
                1,
                "id",
                Some(CellValue::Integer(1)),
                "",
                FieldValueState::Default,
                None,
            )],
            &columns,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            FieldValidationError::DefaultNotSupportedForUpdate { .. }
        ));
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
    fn json_values_open_pretty_without_becoming_false_changes() {
        let original = CellValue::Json(serde_json::json!({
            "enabled": true,
            "nested": { "count": 2 }
        }));
        let pretty = field_editor_text(&original);
        assert!(pretty.contains('\n'));

        let columns = vec![column("payload", "JSONB", false)];
        let drafts = vec![FieldDraft::new(
            1,
            "payload",
            Some(original),
            pretty,
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
