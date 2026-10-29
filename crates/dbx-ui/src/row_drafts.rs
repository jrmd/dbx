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

use crate::binary_values::parse_hex_bytes;
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
    /// Exact numerics are validated as numbers but sent as text so values
    /// wider than an `f64` keep every digit.
    Decimal,
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
            Self::Decimal => "decimal number",
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
///
/// Engines spell types differently (`int unsigned`, `tinyint(1)`,
/// `double precision`, `character varying(20)`), so this matches the base
/// type name rather than substrings: `interval` and `point` are not integers.
/// Enums, bit strings, dates, and unknown types are edited as text; the SQL
/// builder casts text to the column type where the database needs it.
pub fn field_value_kind(column: &ColumnInfo) -> FieldValueKind {
    if !column.enum_values.is_empty() {
        return FieldValueKind::Text;
    }
    let data_type = column.data_type.trim().to_ascii_lowercase();
    if data_type.ends_with("[]") {
        return FieldValueKind::Text;
    }
    let mut words = data_type
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty());
    let base = words.next().unwrap_or_default();
    let unsigned = base == "unsigned" || words.any(|word| word == "unsigned");

    match base {
        "json" | "jsonb" => FieldValueKind::Json,
        "bytea" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary" => {
            FieldValueKind::Bytes
        }
        "bool" | "boolean" => FieldValueKind::Boolean,
        "tinyint" if data_type.starts_with("tinyint(1)") => FieldValueKind::Boolean,
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
        | "int8" | "serial" | "smallserial" | "bigserial" | "serial2" | "serial4" | "serial8"
        | "unsigned" | "year"
            if unsigned =>
        {
            FieldValueKind::Unsigned
        }
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
        | "int8" | "serial" | "smallserial" | "bigserial" | "serial2" | "serial4" | "serial8"
        | "year" => FieldValueKind::Integer,
        "real" | "double" | "float" | "float4" | "float8" => FieldValueKind::Real,
        "decimal" | "dec" | "numeric" | "number" | "fixed" => FieldValueKind::Decimal,
        _ => FieldValueKind::Text,
    }
}

/// Longer VARCHAR/CHAR values open the popover editor instead of an inline
/// cell edit. 200 characters is roughly two wrapped lines in a 100px grid
/// column at the default text size, past which an inline field can't show the
/// value or its caret context; a typical name, URL or code stays below it.
pub const LARGE_TEXT_CHARS: usize = 200;

/// Where a value is edited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditSurface {
    /// The ordinary single-line cell (or sidebar) field.
    Inline,
    /// The popover editor, with JSON line numbers and Format.
    Json,
    /// The popover editor for long or multi-line text.
    Text,
}

impl EditSurface {
    pub fn is_popover(self) -> bool {
        self != Self::Inline
    }
}

/// Whether a column type holds unbounded text (TEXT, MEDIUMTEXT, CLOB, ...)
/// rather than a bounded VARCHAR/CHAR. Arrays of text stay inline.
pub fn is_text_like_type(data_type: &str) -> bool {
    let data_type = data_type.trim().to_ascii_lowercase();
    if data_type.ends_with("[]") {
        return false;
    }
    let base = data_type
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    matches!(
        base,
        "text" | "tinytext" | "mediumtext" | "longtext" | "ntext" | "citext" | "clob" | "nclob"
    ) || data_type.contains("large object")
}

/// The one rule for grid double-click and the row inspector: JSON values and
/// JSON-typed columns, TEXT-like columns, and any text that contains a line
/// break or exceeds [`LARGE_TEXT_CHARS`] use the popover; everything else
/// (short VARCHAR/CHAR, numbers, dates, NULL in a VARCHAR) edits inline.
/// Binary columns follow the VARCHAR rule on the text of their current
/// display format (UTF-8 text, hex, UUID, bits), so a large BLOB expands.
/// `value` is the current text (None for NULL); `json_value` says it parses
/// as a JSON document, including UTF-8 bytes.
pub fn edit_surface(data_type: &str, value: Option<&str>, json_value: bool) -> EditSurface {
    if json_value || data_type.to_ascii_lowercase().contains("json") {
        return EditSurface::Json;
    }
    if is_text_like_type(data_type) {
        return EditSurface::Text;
    }
    match value {
        Some(text)
            if !data_type.trim().to_ascii_lowercase().ends_with("[]")
                && (text.contains(['\n', '\r']) || text.chars().count() > LARGE_TEXT_CHARS) =>
        {
            EditSurface::Text
        }
        _ => EditSurface::Inline,
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
        FieldValueKind::Decimal => {
            let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
            let (mantissa, exponent) = unsigned
                .split_once(['e', 'E'])
                .map_or((unsigned, None), |(mantissa, exponent)| {
                    (mantissa, Some(exponent))
                });
            let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
            let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
            let valid = !(whole.is_empty() && fraction.is_empty())
                && digits(whole)
                && digits(fraction)
                && exponent.is_none_or(|exponent| {
                    let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
                    !exponent.is_empty() && digits(exponent)
                });
            if valid {
                Ok(CellValue::Text(trimmed.to_owned()))
            } else {
                Err(FieldValueError::new(kind, text, "use a decimal number"))
            }
        }
        FieldValueKind::Bytes => parse_hex_bytes(trimmed)
            .map(CellValue::Bytes)
            .map_err(|reason| FieldValueError::new(kind, text, reason)),
        FieldValueKind::Json => serde_json::from_str(trimmed)
            .map(CellValue::Json)
            .map_err(|error| FieldValueError::new(kind, text, format!("invalid JSON ({error})"))),
        FieldValueKind::Text => Ok(CellValue::Text(text.to_owned())),
    }
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
    pub(crate) binary_format: Option<crate::binary_values::BinaryFormat>,
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
            binary_format: None,
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
    pub(crate) binary_format: Option<crate::binary_values::BinaryFormat>,
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
                    .row_value()
                    .line_numbers()
                    .accessibility_label(column.name.clone())
            } else {
                // Text may hold line breaks, so Enter inserts one.
                TextEditor::new(
                    value.clone(),
                    value_kind == FieldValueKind::Text,
                    window,
                    editor_cx,
                )
                .row_value()
                .accessibility_label(column.name.clone())
            }
        });
        let sql_editor = cx.new(|editor_cx| {
            TextEditor::new_with_language(
                value.clone(),
                false,
                EditorLanguage::Sql(None),
                window,
                editor_cx,
            )
            .accessibility_label(format!("SQL expression for {}", column.name))
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
            let selected_index = match parse_field_value(&column, &initial_text) {
                Ok(CellValue::Boolean(true)) => Some(0),
                Ok(CellValue::Boolean(false)) => Some(1),
                _ => None,
            };
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
            binary_format: None,
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

    /// Point the enum/boolean selectors at the current text, or clear them
    /// when the text is not one of their options.
    pub fn sync_value_selectors(&self, window: &mut Window, cx: &mut App) {
        let text = self.value.read(cx).trim().to_owned();
        if let Some(selector) = &self.enum_selector {
            let index = self
                .column
                .enum_values
                .iter()
                .position(|option| *option == text);
            selector.update(cx, |selector, cx| {
                selector.set_selected_index(index.map(IndexPath::new), window, cx)
            });
        }
        if let Some(selector) = &self.boolean_selector {
            let index = match parse_field_value(&self.column, &text) {
                Ok(CellValue::Boolean(true)) => Some(0),
                Ok(CellValue::Boolean(false)) => Some(1),
                _ => None,
            };
            selector.update(cx, |selector, cx| {
                selector.set_selected_index(index.map(IndexPath::new), window, cx)
            });
        }
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
            binary_format: self.binary_format,
        }
    }
}

pub(crate) fn field_editor_text(value: &CellValue) -> String {
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
                    let parsed = draft
                        .binary_format
                        .map_or_else(
                            || parse_field_value(column, &draft.current),
                            |format| {
                                format.parse(&draft.current).map_err(|error| {
                                    FieldValueError::new(
                                        FieldValueKind::Bytes,
                                        &draft.current,
                                        error,
                                    )
                                })
                            },
                        )
                        .map_err(|error| FieldValidationError::InvalidValue {
                            field_id: draft.id,
                            field_index,
                            column: column.name.clone(),
                            expected: error.expected,
                            input: error.input,
                            reason: error.reason,
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
                    MutationValue::Parameter(value) => (!draft
                        .original
                        .as_ref()
                        .is_some_and(|original| same_value(original, value)))
                    .then_some((resolved.column, MutationValue::Parameter(value.clone()))),
                    MutationValue::Expression(_) => Some((resolved.column, value)),
                })
            })
            .collect(),
    )
}

/// Whether a parsed field value is the stored value. Drivers do not always
/// decode a column as the kind the editor parses it to (a YEAR arrives
/// unsigned, a SQLite boolean as an integer), so numbers compare by value.
fn same_value(original: &CellValue, value: &CellValue) -> bool {
    fn integer(value: &CellValue) -> Option<i128> {
        match value {
            CellValue::Boolean(value) => Some(i128::from(*value)),
            CellValue::Integer(value) => Some(i128::from(*value)),
            CellValue::Unsigned(value) => Some(i128::from(*value)),
            _ => None,
        }
    }
    original == value || integer(original).is_some_and(|original| Some(original) == integer(value))
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
    #[test]
    fn edit_surface_classifies_columns_and_values() {
        use super::{EditSurface::*, LARGE_TEXT_CHARS, edit_surface};
        let long = "x".repeat(LARGE_TEXT_CHARS + 1);
        let limit = "x".repeat(LARGE_TEXT_CHARS);
        // TEXT-like types always use the popover, even empty or NULL.
        for text_type in [
            "TEXT",
            "text",
            "MEDIUMTEXT",
            "LONGTEXT",
            "TINYTEXT",
            "citext",
            "CLOB",
            "NCLOB",
            "character large object",
        ] {
            assert_eq!(
                edit_surface(text_type, Some("hi"), false),
                Text,
                "{text_type}"
            );
            assert_eq!(edit_surface(text_type, None, false), Text, "{text_type}");
        }
        // Short VARCHAR/CHAR stay inline; NULL too.
        for short in ["VARCHAR(255)", "character varying", "char(3)", "varchar"] {
            assert_eq!(edit_surface(short, Some("hello"), false), Inline, "{short}");
            assert_eq!(edit_surface(short, Some(&limit), false), Inline, "{short}");
            assert_eq!(edit_surface(short, None, false), Inline, "{short}");
            // ... unless the value is large or has a line break.
            assert_eq!(edit_surface(short, Some(&long), false), Text, "{short}");
            assert_eq!(edit_surface(short, Some("a\nb"), false), Text, "{short}");
            assert_eq!(edit_surface(short, Some("a\rb"), false), Text, "{short}");
        }
        // JSON by type or by value.
        assert_eq!(edit_surface("jsonb", None, false), Json);
        assert_eq!(edit_surface("JSON", Some("{}"), true), Json);
        assert_eq!(edit_surface("TEXT", Some("{\"a\":1}"), true), Json);
        // Numbers, arrays and binary never move to the popover on length alone.
        assert_eq!(edit_surface("integer", Some("42"), false), Inline);
        assert_eq!(edit_surface("text[]", Some(&long), false), Inline);
        // Binary columns expand when their displayed text is large.
        for blob in ["BLOB", "LONGBLOB", "bytea", "VARBINARY(512)", "binary(16)"] {
            assert_eq!(
                edit_surface(blob, Some("deadbeef"), false),
                Inline,
                "{blob}"
            );
            assert_eq!(edit_surface(blob, Some(&long), false), Text, "{blob}");
            assert_eq!(edit_surface(blob, Some("a\nb"), false), Text, "{blob}");
            assert_eq!(edit_surface(blob, Some("{\"a\":1}"), true), Json, "{blob}");
            assert_eq!(edit_surface(blob, None, false), Inline, "{blob}");
        }
        assert!(!is_text_like_type("varchar(10)"));
    }

    use super::*;

    fn column(name: &str, data_type: &str, nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            enum_values: Vec::new(),
            nullable,
            ordinal: 0,
            primary_key: false,
            default_value: None,
        }
    }

    #[test]
    fn decimals_keep_every_digit_and_reject_non_numbers() {
        let price = column("price", "numeric(30,10)", false);
        assert_eq!(
            parse_field_value(&price, " 12345678901234567890.0123456789 ").unwrap(),
            CellValue::Text("12345678901234567890.0123456789".into())
        );
        for valid in ["-1", "+.5", "3.", "1e-3", "2E+10"] {
            assert!(parse_field_value(&price, valid).is_ok(), "{valid}");
        }
        for invalid in ["", ".", "1.2.3", "1e", "abc", "0x10"] {
            assert!(parse_field_value(&price, invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn untouched_rows_with_driver_typed_values_have_no_changes() {
        // Drivers decode these columns as a different kind from the one the
        // editor parses to; an unrelated save must not rewrite or reject them.
        let columns = vec![
            column("flag", "tinyint(1)", false),
            column("released", "year", false),
            column("mask", "bit(8)", false),
            column("price", "decimal(10,2)", false),
            column("sqlite_flag", "BOOLEAN", false),
        ];
        let drafts = [
            ("flag", CellValue::Boolean(true), "true"),
            ("released", CellValue::Unsigned(2024), "2024"),
            ("mask", CellValue::Unsigned(15), "15"),
            ("price", CellValue::Text("4.50".into()), "4.50"),
            ("sqlite_flag", CellValue::Integer(1), "1"),
        ]
        .into_iter()
        .map(|(name, original, text)| {
            FieldDraft::with_new_id(name, Some(original), text, FieldValueState::Value, None)
        })
        .collect::<Vec<_>>();

        assert_eq!(changed_fields(&drafts, &columns).unwrap(), Vec::new());
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
    fn uuid_binary_drafts_preserve_bytes_and_parse_changes() {
        use crate::binary_values::BinaryFormat;
        let uuid = "6ccd780c-baba-1026-9564-5b8c656024db";
        let original = BinaryFormat::Uuid.parse(uuid).unwrap();
        let columns = vec![column("payload", "BLOB", false)];
        let mut draft = FieldDraft::new(
            1,
            "payload",
            Some(original.clone()),
            uuid,
            FieldValueState::Value,
            None,
        );
        draft.binary_format = Some(BinaryFormat::Uuid);
        assert!(
            changed_fields(&[draft.clone()], &columns)
                .unwrap()
                .is_empty()
        );
        draft.current = "00000000-0000-0000-0000-000000000000".into();
        assert_eq!(
            changed_fields(&[draft], &columns).unwrap()[0].1,
            MutationValue::Parameter(CellValue::Bytes(vec![0; 16]))
        );
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
}
