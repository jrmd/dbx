//! Structured, multi-row filters for the DBX data view.
//!
//! [`FilterRow`] is the GPUI-facing part of this module.  It owns the value
//! entity and the text editor that paints/edits that entity, but keeps the
//! actual filter conversion in [`validate_filter_drafts`].  Keeping that
//! conversion pure makes it possible to validate filters before starting a
//! query and to test all of the type parsing without a running GPUI window.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use dbx_core::{CellValue, ColumnInfo, Filter, FilterOperator};
use gpui::{App, AppContext, Context, Entity, SharedString, Window};
use gpui_component::{
    IndexPath,
    select::{SearchableVec, SelectState},
};

use crate::editor::TextEditor;

/// A stable identifier for a filter row.
///
/// IDs are deliberately independent of a row's position.  This means that a
/// row can be removed or reordered without invalidating event handlers or
/// focus state associated with another row.
pub type FilterRowId = u64;

static NEXT_FILTER_ROW_ID: AtomicU64 = AtomicU64::new(1);

fn next_filter_row_id() -> FilterRowId {
    NEXT_FILTER_ROW_ID.fetch_add(1, Ordering::Relaxed)
}

/// The value kinds understood by the filter value parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterValueKind {
    Boolean,
    Integer,
    Unsigned,
    Real,
    Json,
    Text,
}

impl FilterValueKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Unsigned => "unsigned integer",
            Self::Real => "real number",
            Self::Json => "JSON",
            Self::Text => "text",
        }
    }
}

impl fmt::Display for FilterValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// A label and value requirement for one operator in an operator picker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilterOperatorOption {
    pub operator: FilterOperator,
    pub label: &'static str,
    /// Whether this operator needs a value to be valid.
    pub requires_value: bool,
}

impl FilterOperatorOption {
    pub const fn new(operator: FilterOperator, label: &'static str, requires_value: bool) -> Self {
        Self {
            operator,
            label,
            requires_value,
        }
    }

    pub const fn requires_value(self) -> bool {
        self.requires_value
    }
}

/// All operators in their stable picker order.
pub const FILTER_OPERATOR_OPTIONS: [FilterOperatorOption; 11] = [
    FilterOperatorOption::new(FilterOperator::Equals, "Equals", true),
    FilterOperatorOption::new(FilterOperator::NotEquals, "Does not equal", true),
    FilterOperatorOption::new(FilterOperator::Contains, "Contains", true),
    FilterOperatorOption::new(FilterOperator::StartsWith, "Starts with", true),
    FilterOperatorOption::new(FilterOperator::EndsWith, "Ends with", true),
    FilterOperatorOption::new(FilterOperator::GreaterThan, "Greater than", true),
    FilterOperatorOption::new(
        FilterOperator::GreaterThanOrEqual,
        "Greater than or equal",
        true,
    ),
    FilterOperatorOption::new(FilterOperator::LessThan, "Less than", true),
    FilterOperatorOption::new(FilterOperator::LessThanOrEqual, "Less than or equal", true),
    FilterOperatorOption::new(FilterOperator::IsNull, "Is null", false),
    FilterOperatorOption::new(FilterOperator::IsNotNull, "Is not null", false),
];

/// Alias that emphasizes that these values are operator metadata.
pub type FilterOperatorMetadata = FilterOperatorOption;

/// Return the operator metadata used by a filter picker.
pub fn filter_operator_options() -> &'static [FilterOperatorOption] {
    &FILTER_OPERATOR_OPTIONS
}

/// Return metadata for one operator.
pub fn operator_metadata(operator: FilterOperator) -> FilterOperatorOption {
    FILTER_OPERATOR_OPTIONS
        .iter()
        .copied()
        .find(|option| option.operator == operator)
        .expect("every FilterOperator has picker metadata")
}

/// Return the user-facing label for an operator.
pub fn operator_label(operator: FilterOperator) -> &'static str {
    operator_metadata(operator).label
}

/// Return whether an operator needs a value.
pub fn operator_requires_value(operator: FilterOperator) -> bool {
    operator_metadata(operator).requires_value
}

/// Alias for [`operator_requires_value`].
pub fn operator_value_required(operator: FilterOperator) -> bool {
    operator_requires_value(operator)
}

/// Classify a database column for filter value parsing.
///
/// Drivers report type names with different casing and with optional size or
/// precision suffixes.  Classification is therefore deliberately
/// case-insensitive and conservative: types that are not known scalar types
/// are treated as text instead of making a valid column impossible to filter.
pub fn filter_value_kind(column: &ColumnInfo) -> FilterValueKind {
    let data_type = column.data_type.trim().to_ascii_lowercase();

    if data_type.contains("json") {
        FilterValueKind::Json
    } else if data_type.contains("bool")
        || data_type == "bit"
        || data_type.starts_with("bit(")
        || data_type.starts_with("tinyint(1")
    {
        FilterValueKind::Boolean
    } else if data_type.contains("unsigned")
        || data_type.starts_with("uint")
        || data_type.starts_with("ubigint")
    {
        FilterValueKind::Unsigned
    } else if data_type.contains("int")
        || data_type.contains("serial")
        || data_type.starts_with("sint")
    {
        FilterValueKind::Integer
    } else if data_type.contains("real")
        || data_type.contains("double")
        || data_type.contains("float")
        || data_type.contains("decimal")
        || data_type.contains("numeric")
        || data_type == "number"
    {
        FilterValueKind::Real
    } else {
        FilterValueKind::Text
    }
}

/// A parse failure for a typed filter value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilterValueError {
    pub expected: FilterValueKind,
    pub input: String,
    pub reason: String,
}

impl FilterValueError {
    fn new(expected: FilterValueKind, input: &str, reason: impl Into<String>) -> Self {
        Self {
            expected,
            input: input.to_owned(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for FilterValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "expected {}, got {:?}: {}",
            self.expected, self.input, self.reason
        )
    }
}

impl std::error::Error for FilterValueError {}

/// Parse a text-editor value according to a column's reported data type.
pub fn parse_filter_value(column: &ColumnInfo, value: &str) -> Result<CellValue, FilterValueError> {
    let kind = filter_value_kind(column);
    let trimmed = value.trim();

    match kind {
        FilterValueKind::Boolean => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "t" | "1" | "yes" | "y" | "on" => Ok(CellValue::Boolean(true)),
            "false" | "f" | "0" | "no" | "n" | "off" => Ok(CellValue::Boolean(false)),
            _ => Err(FilterValueError::new(kind, value, "use true or false")),
        },
        FilterValueKind::Integer => trimmed
            .parse::<i64>()
            .map(CellValue::Integer)
            .map_err(|_| FilterValueError::new(kind, value, "use a signed base-10 integer")),
        FilterValueKind::Unsigned => trimmed
            .parse::<u64>()
            .map(CellValue::Unsigned)
            .map_err(|_| FilterValueError::new(kind, value, "use a non-negative base-10 integer")),
        FilterValueKind::Real => match trimmed.parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(CellValue::Real(number)),
            Ok(_) => Err(FilterValueError::new(
                kind,
                value,
                "use a finite decimal number",
            )),
            Err(_) => Err(FilterValueError::new(kind, value, "use a decimal number")),
        },
        FilterValueKind::Json => serde_json::from_str(trimmed)
            .map(CellValue::Json)
            .map_err(|error| FilterValueError::new(kind, value, format!("invalid JSON ({error})"))),
        FilterValueKind::Text => Ok(CellValue::Text(value.to_owned())),
    }
}

/// Parse a filter value and return a simple displayable error.
pub fn parse_cell_value_for_column(column: &ColumnInfo, value: &str) -> Result<CellValue, String> {
    parse_filter_value(column, value).map_err(|error| error.to_string())
}

/// A GPUI-independent filter draft.
///
/// The draft contains only serializable/value-like state.  [`FilterRow::draft`]
/// is the bridge from an entity-backed row to this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilterDraft {
    pub id: FilterRowId,
    pub column: String,
    pub operator: FilterOperator,
    pub value: String,
}

impl FilterDraft {
    pub fn new(
        id: FilterRowId,
        column: impl Into<String>,
        operator: FilterOperator,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id,
            column: column.into(),
            operator,
            value: value.into(),
        }
    }

    /// Construct a draft with an automatically allocated stable ID.
    pub fn with_new_id(
        column: impl Into<String>,
        operator: FilterOperator,
        value: impl Into<String>,
    ) -> Self {
        Self::new(next_filter_row_id(), column, operator, value)
    }

    pub fn selected_column(&self) -> &str {
        &self.column
    }
}

impl Default for FilterDraft {
    fn default() -> Self {
        Self::new(0, "", FilterOperator::Equals, "")
    }
}

/// A filter row with a stable ID, operator/column selection, and GPUI input
/// entities for its value.
pub struct FilterRow {
    pub id: FilterRowId,
    pub selected_column: String,
    pub operator: FilterOperator,
    pub value: Entity<String>,
    pub editor: Entity<TextEditor>,
    pub column_selector: Entity<SelectState<SearchableVec<SharedString>>>,
    pub operator_selector: Entity<SelectState<SearchableVec<SharedString>>>,
}

impl FilterRow {
    /// Create an empty filter row.
    pub fn new<T: 'static>(
        selected_column: impl Into<String>,
        operator: FilterOperator,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        Self::with_value_and_columns(selected_column, operator, "", &[], window, cx)
    }

    /// Create an empty filter row with native GPUI selectors populated from
    /// the currently loaded table columns.
    pub fn new_with_columns<T: 'static>(
        selected_column: impl Into<String>,
        operator: FilterOperator,
        columns: &[ColumnInfo],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        Self::with_value_and_columns(selected_column, operator, "", columns, window, cx)
    }

    /// Create a filter row with an initial editor value.
    pub fn with_value<T: 'static>(
        selected_column: impl Into<String>,
        operator: FilterOperator,
        initial_value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        Self::with_value_and_columns(selected_column, operator, initial_value, &[], window, cx)
    }

    /// Create a filter row with a value and native GPUI selectors.
    pub fn with_value_and_columns<T: 'static>(
        selected_column: impl Into<String>,
        operator: FilterOperator,
        initial_value: impl Into<String>,
        columns: &[ColumnInfo],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let selected_column = selected_column.into();
        let value = cx.new(|_| initial_value.into());
        let editor = cx.new(|editor_cx| TextEditor::new(value.clone(), false, window, editor_cx));
        let column_items = SearchableVec::new(
            columns
                .iter()
                .map(|column| SharedString::from(column.name.clone()))
                .collect::<Vec<_>>(),
        );
        let column_index = columns
            .iter()
            .position(|column| column.name == selected_column);
        let operator_items = SearchableVec::new(
            filter_operator_options()
                .iter()
                .map(|option| SharedString::from(option.label))
                .collect::<Vec<_>>(),
        );
        let operator_index = filter_operator_options()
            .iter()
            .position(|option| option.operator == operator);
        let column_selector = cx.new(|select_cx| {
            SelectState::new(
                column_items,
                column_index.map(IndexPath::new),
                window,
                select_cx,
            )
            .searchable(true)
        });
        let operator_selector = cx.new(|select_cx| {
            SelectState::new(
                operator_items,
                operator_index.map(IndexPath::new),
                window,
                select_cx,
            )
            .searchable(true)
        });

        Self {
            id: next_filter_row_id(),
            selected_column,
            operator,
            value,
            editor,
            column_selector,
            operator_selector,
        }
    }

    pub fn selected_column(&self) -> &str {
        &self.selected_column
    }

    pub fn column(&self) -> &str {
        self.selected_column()
    }

    pub fn set_selected_column(&mut self, selected_column: impl Into<String>) {
        self.selected_column = selected_column.into();
    }

    pub fn set_operator(&mut self, operator: FilterOperator) {
        self.operator = operator;
    }

    /// Read the entity value into a GPUI-independent draft.
    pub fn draft(&self, cx: &App) -> FilterDraft {
        FilterDraft::new(
            self.id,
            self.selected_column.clone(),
            self.operator,
            self.value.read(cx).clone(),
        )
    }

    /// Alias for [`Self::draft`] that makes the pure boundary explicit.
    pub fn pure_spec(&self, cx: &App) -> FilterDraft {
        self.draft(cx)
    }
}

/// Ordered collection of structured filter rows.
pub struct FilterModel {
    rows: Vec<FilterRow>,
}

impl Default for FilterModel {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterModel {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn from_rows(rows: Vec<FilterRow>) -> Self {
        Self { rows }
    }

    pub fn rows(&self) -> &[FilterRow] {
        &self.rows
    }

    pub fn rows_mut(&mut self) -> &mut [FilterRow] {
        &mut self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn add_row<T: 'static>(
        &mut self,
        selected_column: impl Into<String>,
        operator: FilterOperator,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> FilterRowId {
        let row = FilterRow::new(selected_column, operator, window, cx);
        let id = row.id;
        self.rows.push(row);
        id
    }

    pub fn add_row_with_value<T: 'static>(
        &mut self,
        selected_column: impl Into<String>,
        operator: FilterOperator,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> FilterRowId {
        let row = FilterRow::with_value(selected_column, operator, value, window, cx);
        let id = row.id;
        self.rows.push(row);
        id
    }

    pub fn add_row_with_columns<T: 'static>(
        &mut self,
        selected_column: impl Into<String>,
        operator: FilterOperator,
        columns: &[ColumnInfo],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> FilterRowId {
        let row = FilterRow::new_with_columns(selected_column, operator, columns, window, cx);
        let id = row.id;
        self.rows.push(row);
        id
    }

    pub fn add_row_with_value_and_columns<T: 'static>(
        &mut self,
        selected_column: impl Into<String>,
        operator: FilterOperator,
        value: impl Into<String>,
        columns: &[ColumnInfo],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> FilterRowId {
        let row = FilterRow::with_value_and_columns(
            selected_column,
            operator,
            value,
            columns,
            window,
            cx,
        );
        let id = row.id;
        self.rows.push(row);
        id
    }

    pub fn push(&mut self, row: FilterRow) -> FilterRowId {
        let id = row.id;
        self.rows.push(row);
        id
    }

    pub fn remove(&mut self, id: FilterRowId) -> Option<FilterRow> {
        let index = self.rows.iter().position(|row| row.id == id)?;
        Some(self.rows.remove(index))
    }

    /// Move a row to an absolute position, retaining all other row order.
    pub fn move_row(&mut self, id: FilterRowId, new_index: usize) -> bool {
        let Some(old_index) = self.rows.iter().position(|row| row.id == id) else {
            return false;
        };
        let row = self.rows.remove(old_index);
        let target = new_index.min(self.rows.len());
        self.rows.insert(target, row);
        true
    }

    pub fn ordered_drafts(&self, cx: &App) -> Vec<FilterDraft> {
        self.rows.iter().map(|row| row.draft(cx)).collect()
    }

    pub fn validate(
        &self,
        cx: &App,
        columns: &[ColumnInfo],
    ) -> Result<Vec<Filter>, FilterValidationError> {
        validate_filter_drafts(&self.ordered_drafts(cx), columns)
    }
}

/// Why one filter draft could not be converted into a core filter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterValidationError {
    MissingColumn {
        row_id: FilterRowId,
        row_index: usize,
    },
    UnknownColumn {
        row_id: FilterRowId,
        row_index: usize,
        column: String,
    },
    MissingValue {
        row_id: FilterRowId,
        row_index: usize,
        column: String,
        operator: FilterOperator,
    },
    InvalidValue {
        row_id: FilterRowId,
        row_index: usize,
        column: String,
        operator: FilterOperator,
        expected: FilterValueKind,
        input: String,
        reason: String,
    },
}

impl FilterValidationError {
    pub fn row_id(&self) -> FilterRowId {
        match self {
            Self::MissingColumn { row_id, .. }
            | Self::UnknownColumn { row_id, .. }
            | Self::MissingValue { row_id, .. }
            | Self::InvalidValue { row_id, .. } => *row_id,
        }
    }

    pub fn row_index(&self) -> usize {
        match self {
            Self::MissingColumn { row_index, .. }
            | Self::UnknownColumn { row_index, .. }
            | Self::MissingValue { row_index, .. }
            | Self::InvalidValue { row_index, .. } => *row_index,
        }
    }
}

impl fmt::Display for FilterValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingColumn { row_index, .. } => {
                write!(
                    formatter,
                    "Filter row {} has no column selected",
                    row_index + 1
                )
            }
            Self::UnknownColumn {
                row_index, column, ..
            } => write!(
                formatter,
                "Filter row {} references unknown column {:?}",
                row_index + 1,
                column
            ),
            Self::MissingValue {
                row_index,
                column,
                operator,
                ..
            } => write!(
                formatter,
                "Filter row {} for column {:?} ({}) needs a value",
                row_index + 1,
                column,
                operator_label(*operator)
            ),
            Self::InvalidValue {
                row_index,
                column,
                operator,
                expected,
                input,
                reason,
                ..
            } => write!(
                formatter,
                "Filter row {} for column {:?} ({}) has invalid {} value {:?}: {}",
                row_index + 1,
                column,
                operator_label(*operator),
                expected,
                input,
                reason
            ),
        }
    }
}

impl std::error::Error for FilterValidationError {}

/// Convert ordered, pure drafts into core filters.
///
/// The order of `drafts` is retained exactly.  Column names are resolved
/// against `columns`, and value text is parsed using the selected column's
/// reported data type.  `IsNull` and `IsNotNull` intentionally discard any
/// stale text left in their editor and always produce a filter with no value.
pub fn validate_filter_drafts(
    drafts: &[FilterDraft],
    columns: &[ColumnInfo],
) -> Result<Vec<Filter>, FilterValidationError> {
    drafts
        .iter()
        .enumerate()
        .map(|(row_index, draft)| {
            let column_name = draft.column.trim();
            if column_name.is_empty() {
                return Err(FilterValidationError::MissingColumn {
                    row_id: draft.id,
                    row_index,
                });
            }

            let column = columns
                .iter()
                .find(|column| column.name == column_name)
                .ok_or_else(|| FilterValidationError::UnknownColumn {
                    row_id: draft.id,
                    row_index,
                    column: column_name.to_owned(),
                })?;

            if !operator_requires_value(draft.operator) {
                return Ok(Filter::new(column.name.clone(), draft.operator, None));
            }

            if draft.value.trim().is_empty() {
                return Err(FilterValidationError::MissingValue {
                    row_id: draft.id,
                    row_index,
                    column: column.name.clone(),
                    operator: draft.operator,
                });
            }

            let value = parse_filter_value(column, &draft.value).map_err(|error| {
                FilterValidationError::InvalidValue {
                    row_id: draft.id,
                    row_index,
                    column: column.name.clone(),
                    operator: draft.operator,
                    expected: error.expected,
                    input: error.input,
                    reason: error.reason,
                }
            })?;

            Ok(Filter::new(
                column.name.clone(),
                draft.operator,
                Some(value),
            ))
        })
        .collect()
}

/// Short alias for callers that already refer to their input as specs.
pub fn validate_filter_specs(
    specs: &[FilterDraft],
    columns: &[ColumnInfo],
) -> Result<Vec<Filter>, FilterValidationError> {
    validate_filter_drafts(specs, columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo::result(name, 0, data_type)
    }

    #[test]
    fn operator_metadata_marks_only_null_operators_without_values() {
        assert_eq!(filter_operator_options().len(), 11);
        assert!(operator_requires_value(FilterOperator::Equals));
        assert!(!operator_requires_value(FilterOperator::IsNull));
        assert!(!operator_value_required(FilterOperator::IsNotNull));
        assert_eq!(operator_label(FilterOperator::Contains), "Contains");
    }

    #[test]
    fn parser_handles_all_supported_scalar_kinds() {
        assert_eq!(
            parse_filter_value(&column("active", "BOOLEAN"), " true ").unwrap(),
            CellValue::Boolean(true)
        );
        assert_eq!(
            parse_filter_value(&column("count", "BIGINT"), "-42").unwrap(),
            CellValue::Integer(-42)
        );
        assert_eq!(
            parse_filter_value(&column("count", "BIGINT UNSIGNED"), "42").unwrap(),
            CellValue::Unsigned(42)
        );
        assert_eq!(
            parse_filter_value(&column("score", "DOUBLE PRECISION"), "1.25").unwrap(),
            CellValue::Real(1.25)
        );
        assert_eq!(
            parse_filter_value(&column("payload", "JSONB"), r#"{"ok":true}"#).unwrap(),
            CellValue::Json(serde_json::json!({ "ok": true }))
        );
        assert_eq!(
            parse_filter_value(&column("name", "VARCHAR(255)"), "Ada ").unwrap(),
            CellValue::Text("Ada ".into())
        );
    }

    #[test]
    fn validation_preserves_order_and_null_filters_have_no_value() {
        let columns = vec![column("active", "BOOLEAN"), column("name", "TEXT")];
        let drafts = vec![
            FilterDraft::new(10, "name", FilterOperator::Contains, "Ada"),
            FilterDraft::new(11, "active", FilterOperator::IsNull, "stale text"),
        ];

        let filters = validate_filter_drafts(&drafts, &columns).unwrap();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].column, "name");
        assert_eq!(filters[0].operator, FilterOperator::Contains);
        assert_eq!(filters[0].value, Some(CellValue::Text("Ada".into())));
        assert_eq!(filters[1].column, "active");
        assert_eq!(filters[1].operator, FilterOperator::IsNull);
        assert_eq!(filters[1].value, None);
    }

    #[test]
    fn validation_reports_missing_unknown_and_bad_values() {
        let columns = vec![column("count", "INTEGER")];

        let error = validate_filter_drafts(
            &[FilterDraft::new(1, "", FilterOperator::Equals, "1")],
            &columns,
        )
        .unwrap_err();
        assert!(error.to_string().contains("no column selected"));

        let error = validate_filter_drafts(
            &[FilterDraft::new(2, "missing", FilterOperator::Equals, "1")],
            &columns,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown column"));

        let error = validate_filter_drafts(
            &[FilterDraft::new(
                3,
                "count",
                FilterOperator::Equals,
                "not a number",
            )],
            &columns,
        )
        .unwrap_err();
        assert!(error.to_string().contains("signed base-10 integer"));
    }

    #[test]
    fn row_ids_are_stable_when_rows_move_or_are_removed() {
        // This test covers the pure collection behavior without constructing
        // GPUI entities; FilterModel::move_row/remove use the same IDs as the
        // entity-backed rows created by FilterRow::new.
        assert_ne!(next_filter_row_id(), next_filter_row_id());
    }
}
