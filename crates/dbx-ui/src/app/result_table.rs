use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use dbx_core::{
    CellValue, ColumnInfo, FilterOperator, ForeignKeyInfo, MutationValue, Order, OrderDirection,
    QueryResult, TableInfo,
};
use gpui::{
    App, Context, Div, FontWeight, IntoElement, Pixels, SharedString, Stateful, Window, div,
    prelude::*, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    table::{Column as DataColumn, ColumnSort, TableDelegate, TableState},
};

use super::{DbxApp, SecondaryTabId, SessionId};
use crate::editor::TextEditor;
use gpui::{
    ClickEvent, ClipboardItem, DragMoveEvent, Entity, EntityId, MouseButton, MouseDownEvent,
    MouseUpEvent, Render, WeakEntity,
};
use gpui_component::table::TableEvent;

use crate::workspace::TableLayout;
use gpui_component::menu::{PopupMenu, PopupMenuItem};

use crate::diagram::display_type;
use crate::row_drafts::{FieldValueKind, field_value_kind};
use crate::theme::{Icon, icon, theme};

#[derive(Clone, Copy)]
enum RowAction {
    Inspect,
    Edit,
    Delete,
    Duplicate,
    Revert,
}

/// A data tab's staged changeset as the grid shows it.
#[derive(Clone, Default)]
pub(super) struct GridChanges {
    /// Staged values for loaded rows, keyed by (row, data column).
    pub(super) edits: HashMap<(usize, usize), MutationValue>,
    /// New rows, shown after the loaded rows. `None` keeps the column default.
    pub(super) inserts: Vec<Vec<Option<MutationValue>>>,
    pub(super) deletes: BTreeSet<usize>,
    pub(super) marked: BTreeSet<usize>,
}

const ROW_NUMBER_COLUMN_KEY: &str = "__dbx_row_number";
const MIN_COLUMN_WIDTH: f32 = 60.;
const MAX_COLUMN_WIDTH: f32 = 1_200.;
const RESIZE_HANDLE_WIDTH: f32 = 7.;
/// gpui-component's header sort button: a 12px icon with 2px padding.
const SORT_ICON_WIDTH: f32 = 16.;
/// The right padding the table adds to headers of zero-padding columns.
const HEADER_TRAILING_PADDING: f32 = 8.;

/// The drag payload for a header resize: the owning table and grid column.
#[derive(Clone, Copy)]
struct ColumnResize(EntityId, usize);

impl Render for ColumnResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

fn finish_resize(
    table: &mut TableState<ResultTableDelegate>,
    _: &MouseUpEvent,
    _: &mut Window,
    cx: &mut Context<TableState<ResultTableDelegate>>,
) {
    if table.delegate_mut().resizing.take().is_some() {
        emit_widths(table, cx);
        cx.notify();
    }
}

/// Report widths the way the table's own resize does, so owners remember them.
fn emit_widths(
    table: &mut TableState<ResultTableDelegate>,
    cx: &mut Context<TableState<ResultTableDelegate>>,
) {
    let widths = table
        .delegate()
        .columns
        .iter()
        .map(|column| column.width)
        .collect();
    cx.emit(TableEvent::ColumnWidthsChanged(widths));
}

/// Result columns in display order: pinned columns first, then the saved
/// order, then any remaining columns in result order. Hidden columns are
/// left out, though at least one column always stays visible.
fn display_order(columns: &[ColumnInfo], layout: Option<&TableLayout>) -> Vec<usize> {
    let Some(layout) = layout else {
        return (0..columns.len()).collect();
    };
    let position = |name: &String| columns.iter().position(|column| column.name == *name);
    let mut order = Vec::with_capacity(columns.len());
    for index in layout
        .pinned
        .iter()
        .chain(&layout.order)
        .filter_map(position)
        .chain(0..columns.len())
    {
        if !order.contains(&index) && !layout.hidden.contains(&columns[index].name) {
            order.push(index);
        }
    }
    if order.is_empty() && !columns.is_empty() {
        order.push(0);
    }
    order
}
const AUTO_WIDTH_SAMPLE_ROWS: usize = 200;
/// Longest text handed to a grid cell. The column ellipsizes visually; this only
/// keeps huge values from being shaped or copied every frame.
const CELL_TEXT_LIMIT: usize = 300;

/// Single-line display text for a cell, capped at `CELL_TEXT_LIMIT` characters.
fn cell_display_text(value: &CellValue) -> String {
    let full;
    let text = match value {
        CellValue::Text(text) => text.as_str(),
        CellValue::Bytes(bytes) => {
            let shown = &bytes[..bytes.len().min(CELL_TEXT_LIMIT / 2)];
            full = format!(
                "{}{}",
                CellValue::Bytes(shown.to_vec()),
                if shown.len() < bytes.len() { "…" } else { "" }
            );
            full.as_str()
        }
        value => {
            full = value.to_string();
            full.as_str()
        }
    };
    match text.char_indices().nth(CELL_TEXT_LIMIT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// Order cells for a local header sort: NULLs first, numbers numerically,
/// then everything else by its display text.
fn compare_cells(left: Option<&CellValue>, right: Option<&CellValue>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn number(value: &CellValue) -> Option<f64> {
        match value {
            CellValue::Integer(value) => Some(*value as f64),
            CellValue::Unsigned(value) => Some(*value as f64),
            CellValue::Real(value) => Some(*value),
            CellValue::Text(text) => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite()),
            _ => None,
        }
    }
    let left = left.unwrap_or(&CellValue::Null);
    let right = right.unwrap_or(&CellValue::Null);
    match (left, right) {
        (CellValue::Null, CellValue::Null) => Ordering::Equal,
        (CellValue::Null, _) => Ordering::Less,
        (_, CellValue::Null) => Ordering::Greater,
        (CellValue::Integer(left), CellValue::Integer(right)) => left.cmp(right),
        (CellValue::Unsigned(left), CellValue::Unsigned(right)) => left.cmp(right),
        (CellValue::Boolean(left), CellValue::Boolean(right)) => left.cmp(right),
        (CellValue::Bytes(left), CellValue::Bytes(right)) => left.cmp(right),
        _ => match (number(left), number(right)) {
            (Some(left), Some(right)) => left.total_cmp(&right),
            _ => plain_cell_text(left).cmp(&plain_cell_text(right)),
        },
    }
}

/// Shared, virtualized backing model for both table browsing and ad-hoc query results.
///
/// `QueryResult` stays owned by the session/tab through an `Arc`, while DataTable only
/// asks this delegate to render cells that are currently visible.
pub(super) struct ResultTableDelegate {
    result: Option<Arc<QueryResult>>,
    columns: Vec<DataColumn>,
    /// Per data column: numeric values are right-aligned so digits line up.
    numeric: Vec<bool>,
    /// Lazily filled display text, row-major over data columns, so scrolling
    /// never re-formats or re-copies values that were already rendered.
    cell_text: Vec<Option<SharedString>>,
    foreign_keys: Vec<ForeignKeyInfo>,
    row_actions: Option<(WeakEntity<DbxApp>, SessionId, SecondaryTabId, bool)>,
    sorting: ResultSorting,
    /// Unsorted rows behind a locally sorted result, restored when the sort
    /// returns to its default state.
    unsorted: Option<Arc<QueryResult>>,
    /// The active header sort as (data column, direction).
    sort: Option<(usize, OrderDirection)>,
    /// The order a server-sorted data tab requested for its next result.
    server_order: Option<Order>,
    /// The data tab's staged changeset and Shift/Cmd-marked rows.
    changes: GridChanges,
    /// Absolute number of the first loaded row, so page two starts at 1,001.
    row_offset: usize,
    /// Result column shown at each display position after the row number.
    /// Pinned columns come first; hidden columns are absent.
    order: Vec<usize>,
    /// The result column under the last right-click, for column actions.
    context_column: Option<usize>,
    /// The column being resized by the grab handle, and its starting width.
    resizing: Option<usize>,
    /// The cell currently being edited inline, as (row, data column, editor).
    editing: Option<(usize, usize, Entity<TextEditor>)>,
}

/// How a header click orders rows.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ResultSorting {
    Disabled,
    /// Reorder the rows already held by the grid.
    Local,
    /// Ask the owning data tab to reload with `ORDER BY`.
    Server,
}

impl Default for ResultTableDelegate {
    fn default() -> Self {
        Self {
            result: None,
            columns: vec![Self::row_number_column()],
            numeric: Vec::new(),
            cell_text: Vec::new(),
            foreign_keys: Vec::new(),
            row_actions: None,
            sorting: ResultSorting::Disabled,
            unsorted: None,
            sort: None,
            server_order: None,
            changes: GridChanges::default(),
            row_offset: 0,
            order: Vec::new(),
            context_column: None,
            resizing: None,
            editing: None,
        }
    }
}

impl ResultTableDelegate {
    pub(super) fn with_row_actions(
        app: WeakEntity<DbxApp>,
        session: SessionId,
        tab: SecondaryTabId,
        is_data: bool,
    ) -> Self {
        Self {
            row_actions: Some((app, session, tab, is_data)),
            sorting: if is_data {
                ResultSorting::Disabled
            } else {
                ResultSorting::Local
            },
            ..Self::default()
        }
    }

    /// Enable or disable server-side header sorting for a data tab and record
    /// the order the next result was loaded with.
    pub(super) fn set_server_sort(&mut self, enabled: bool, order: Option<&Order>) {
        self.sorting = if enabled {
            ResultSorting::Server
        } else {
            ResultSorting::Disabled
        };
        self.server_order = order.cloned();
    }

    pub(super) fn set_cell_edits(
        &mut self,
        changes: GridChanges,
        editing: Option<(usize, usize, Entity<TextEditor>)>,
    ) {
        self.changes = changes;
        self.editing = editing;
    }

    pub(super) fn set_row_offset(&mut self, row_offset: usize) {
        self.row_offset = row_offset;
    }

    /// The result column a grid column shows. Column 0 is the row number.
    pub(super) fn result_column(&self, col_ix: usize) -> Option<usize> {
        self.order.get(col_ix.checked_sub(1)?).copied()
    }

    /// The grid column showing a result column, if it is visible.
    pub(super) fn grid_column(&self, column: usize) -> Option<usize> {
        self.order
            .iter()
            .position(|shown| *shown == column)
            .map(|index| index + 1)
    }

    /// Result columns in display order.
    pub(super) fn display_order(&self) -> &[usize] {
        &self.order
    }

    /// Filter, copy, hide and pin actions for the right-clicked cell's column.
    fn column_menu(
        &self,
        mut menu: PopupMenu,
        app: &WeakEntity<DbxApp>,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row_ix: usize,
        column: usize,
    ) -> PopupMenu {
        let Some(result) = self.result.as_deref() else {
            return menu;
        };
        let (Some(info), Some(value)) =
            (result.columns.get(column), self.cell_value(row_ix, column))
        else {
            return menu;
        };
        let name = info.name.clone();
        let null = *value == CellValue::Null;
        let copy = plain_cell_text(value);
        menu = menu.item(PopupMenuItem::new("Copy cell").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
        }));
        let filters: &[(&str, FilterOperator)] = if null {
            &[
                ("Filter: is NULL", FilterOperator::IsNull),
                ("Filter: is not NULL", FilterOperator::IsNotNull),
            ]
        } else {
            &[
                ("Filter: equals this value", FilterOperator::Equals),
                ("Filter: not equal", FilterOperator::NotEquals),
            ]
        };
        for (label, operator) in filters.iter().copied() {
            let app = app.clone();
            menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = app.update(cx, |this, cx| {
                    this.filter_by_cell_for(
                        session_id,
                        tab_id,
                        (row_ix, column),
                        operator,
                        window,
                        cx,
                    )
                });
            }));
        }
        let pinned = self
            .grid_column(column)
            .and_then(|col_ix| self.columns.get(col_ix))
            .is_some_and(|column| column.fixed.is_some());
        let pin_app = app.clone();
        let hide_app = app.clone();
        menu.separator()
            .item(
                PopupMenuItem::new(if pinned {
                    format!("Unpin {name}")
                } else {
                    format!("Pin {name}")
                })
                .on_click(move |_, _, cx| {
                    let _ = pin_app.update(cx, |this, cx| {
                        this.toggle_pin_column_for(session_id, tab_id, column, cx)
                    });
                }),
            )
            .item(
                PopupMenuItem::new(format!("Hide {name}"))
                    .disabled(self.order.len() < 2)
                    .on_click(move |_, _, cx| {
                        let _ = hide_app.update(cx, |this, cx| {
                            this.hide_column_for(session_id, tab_id, column, cx)
                        });
                    }),
            )
            .separator()
    }

    fn loaded_rows(&self) -> usize {
        self.result.as_ref().map_or(0, |result| result.rows.len())
    }

    /// The staged new row a grid row shows, if it is one.
    fn insert_row(&self, row_ix: usize) -> Option<&Vec<Option<MutationValue>>> {
        self.changes
            .inserts
            .get(row_ix.checked_sub(self.loaded_rows())?)
    }

    fn column_sort(&self, index: usize) -> Option<ColumnSort> {
        if self.sorting == ResultSorting::Disabled {
            return None;
        }
        Some(match self.sort {
            Some((sorted, OrderDirection::Ascending)) if sorted == index => ColumnSort::Ascending,
            Some((sorted, OrderDirection::Descending)) if sorted == index => ColumnSort::Descending,
            _ => ColumnSort::Default,
        })
    }

    fn sort_locally(&mut self, index: usize, direction: Option<OrderDirection>) {
        let Some(source) = self.unsorted.clone().or_else(|| self.result.clone()) else {
            return;
        };
        let Some(direction) = direction else {
            self.sort = None;
            self.unsorted = None;
            self.replace_rows(source);
            return;
        };
        let mut sorted = (*source).clone();
        sorted.rows.sort_by(|left, right| {
            let ordering = compare_cells(left.values.get(index), right.values.get(index));
            match direction {
                OrderDirection::Ascending => ordering,
                OrderDirection::Descending => ordering.reverse(),
            }
        });
        self.sort = Some((index, direction));
        self.unsorted = Some(source);
        self.replace_rows(Arc::new(sorted));
    }

    fn replace_rows(&mut self, result: Arc<QueryResult>) {
        self.cell_text = vec![None; result.rows.len() * result.columns.len()];
        self.result = Some(result);
    }

    pub(super) fn row_as_json(&self, row_ix: usize) -> Option<String> {
        let result = self.result.as_ref()?;
        let row = result.rows.get(row_ix)?;
        let mut names = std::collections::HashSet::new();
        // Query aliases may repeat. Preserve all values in the existing
        // positional envelope in that case instead of overwriting a field.
        if result
            .columns
            .iter()
            .any(|column| !names.insert(&column.name))
        {
            let single_row = QueryResult {
                columns: result.columns.clone(),
                rows: vec![row.clone()],
                rows_affected: None,
                truncated: false,
                elapsed_ms: result.elapsed_ms,
            };
            return Some(json_result(&single_row));
        }
        let mut output = Vec::new();
        output.push(b'{');
        for (index, (column, value)) in result.columns.iter().zip(&row.values).enumerate() {
            if index > 0 {
                output.push(b',');
            }
            write_json_value(&mut output, &column.name);
            output.push(b':');
            write_json_cell_value(&mut output, value);
        }
        output.push(b'}');
        String::from_utf8(output).ok()
    }

    /// Return the underlying value for a data column (not the synthetic row-number column).
    ///
    /// Keeping this at the delegate boundary means callers can add selection, copy, or export
    /// controls without reaching through the virtualized table implementation.
    pub(super) fn cell_value(&self, row_ix: usize, data_column_ix: usize) -> Option<&CellValue> {
        self.result
            .as_ref()?
            .rows
            .get(row_ix)?
            .values
            .get(data_column_ix)
    }

    /// Return a complete underlying row, preserving `NULL` values and duplicate column names.
    pub(super) fn row_values(&self, row_ix: usize) -> Option<&[CellValue]> {
        Some(self.result.as_ref()?.rows.get(row_ix)?.values.as_slice())
    }

    /// Return a data column in result order, preserving `NULL` values.
    pub(super) fn column_values(&self, data_column_ix: usize) -> Option<Vec<&CellValue>> {
        let result = self.result.as_ref()?;
        result.columns.get(data_column_ix)?;
        Some(
            result
                .rows
                .iter()
                .filter_map(|row| row.values.get(data_column_ix))
                .collect(),
        )
    }

    /// Render one cell for a plain-text clipboard target. `NULL` is intentionally visible,
    /// while an empty text value remains empty.
    pub(super) fn cell_as_plain_text(
        &self,
        row_ix: usize,
        data_column_ix: usize,
    ) -> Option<String> {
        self.cell_value(row_ix, data_column_ix).map(plain_cell_text)
    }

    /// Render a single row as TSV, using quoted empty strings and a bare `NULL` sentinel so
    /// downstream consumers can distinguish database NULL from an empty text value.
    pub(super) fn row_as_tsv(&self, row_ix: usize) -> Option<String> {
        self.row_values(row_ix).map(|row| delimited_row(row, '\t'))
    }

    /// Render one data column as a headered TSV document. The header makes a copied column
    /// useful on its own, while `NULL` and empty text retain the same representation as rows
    /// and full-result exports.
    pub(super) fn column_as_tsv(&self, data_column_ix: usize) -> Option<String> {
        let result = self.result.as_deref()?;
        let column = result.columns.get(data_column_ix)?;
        let values = self.column_values(data_column_ix)?;

        Some(delimited_column(
            column.name.as_str(),
            values.into_iter(),
            '\t',
        ))
    }

    /// Render the complete result as a headered TSV document.
    pub(super) fn matching_cells(&self, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
        let Some(result) = &self.result else {
            return Vec::new();
        };
        let order = &self.order;
        result
            .rows
            .iter()
            .enumerate()
            .flat_map(|(row_index, row)| {
                order
                    .iter()
                    .enumerate()
                    .filter_map(move |(position, column_index)| {
                        let value = row.values.get(*column_index)?;
                        (!super::find::find_matches(&value.to_string(), needle, case_sensitive)
                            .is_empty())
                        .then_some((row_index, position + 1))
                    })
            })
            .collect()
    }

    pub(super) fn result_as_insert(
        &self,
        kind: dbx_core::DatabaseKind,
        target: &dbx_core::TableRef,
    ) -> Option<String> {
        let result = self.result.as_ref()?;
        let columns = result
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        result
            .rows
            .iter()
            .map(|row| {
                dbx_core::render_sql_insert(kind, target, &columns, &row.values)
                    .map(|sql| format!("{sql};"))
            })
            .collect::<dbx_core::Result<Vec<_>>>()
            .ok()
            .map(|sql| sql.join("\n"))
    }

    pub(super) fn result_as_tsv(&self) -> Option<String> {
        self.result
            .as_deref()
            .map(|result| delimited_result(result, '\t'))
    }

    /// Render the complete result as a headered RFC 4180-compatible CSV document.
    pub(super) fn result_as_csv(&self) -> Option<String> {
        self.result
            .as_deref()
            .map(|result| delimited_result(result, ','))
    }

    /// Render a lossless JSON result envelope.
    ///
    /// A columns-plus-rows shape preserves duplicate SQL aliases and keeps JSON `null` distinct
    /// from an empty string, unlike a name-keyed object per row.
    pub(super) fn result_as_json(&self) -> Option<String> {
        self.result.as_deref().map(json_result)
    }

    fn row_number_column() -> DataColumn {
        DataColumn::new(ROW_NUMBER_COLUMN_KEY, "#")
            .width(44.)
            .fixed_left()
            .resizable(false)
            .movable(false)
            .selectable(false)
            .min_width(44.)
            .max_width(44.)
            .p_0()
    }

    fn data_column_key(index: usize, column: &ColumnInfo) -> String {
        // Query results may legally contain duplicate column names, so the ordinal is
        // part of the key. Humanity has already made SQL aliases difficult enough.
        format!("column:{index}:{}", column.name)
    }

    fn auto_width(result: &QueryResult, column_index: usize, column: &ColumnInfo) -> Pixels {
        let header_chars = format!("{}  {}", column.name, column.data_type)
            .chars()
            .count();
        let value_chars = result
            .rows
            .iter()
            .take(AUTO_WIDTH_SAMPLE_ROWS)
            .filter_map(|row| row.values.get(column_index))
            .map(|value| cell_display_text(value).chars().count())
            .max()
            .unwrap_or_default();

        // This is an initial width, not a prison sentence. The user can resize it.
        px(((header_chars.max(value_chars) as f32 * 7.0) + 20.0).clamp(80.0, 420.0))
    }

    pub(super) fn set_result(
        &mut self,
        result: Option<Arc<QueryResult>>,
        remembered_widths: &HashMap<String, Pixels>,
        layout: Option<&TableLayout>,
        foreign_keys: &[ForeignKeyInfo],
        tables: &[TableInfo],
    ) {
        let mut columns = vec![Self::row_number_column()];
        self.unsorted = None;
        self.sort = match self.sorting {
            ResultSorting::Server => self.server_order.as_ref().and_then(|order| {
                let index = result
                    .as_deref()?
                    .columns
                    .iter()
                    .position(|column| column.name == order.column)?;
                Some((index, order.direction))
            }),
            _ => None,
        };

        self.order = result
            .as_deref()
            .map(|result| display_order(&result.columns, layout))
            .unwrap_or_default();
        let pinned = layout.map_or(0, |layout| {
            self.order
                .iter()
                .take_while(|index| {
                    result
                        .as_deref()
                        .is_some_and(|result| layout.pinned.contains(&result.columns[**index].name))
                })
                .count()
        });
        if let Some(result) = result.as_deref() {
            columns.extend(self.order.iter().enumerate().map(|(position, &index)| {
                let column = &result.columns[index];
                let key = Self::data_column_key(index, column);
                let width = remembered_widths
                    .get(&key)
                    .copied()
                    .or_else(|| {
                        layout
                            .and_then(|layout| layout.widths.get(&column.name))
                            .map(|width| px(*width))
                    })
                    .unwrap_or_else(|| Self::auto_width(result, index, column));

                let mut data_column =
                    DataColumn::new(key, format!("{}  {}", column.name, column.data_type))
                        .width(width)
                        .resizable(true)
                        .movable(position >= pinned)
                        .min_width(MIN_COLUMN_WIDTH)
                        .max_width(MAX_COLUMN_WIDTH)
                        .p_0();
                if position < pinned {
                    data_column = data_column.fixed_left();
                }
                data_column.sort = self.column_sort(index);
                data_column
            }));
        }

        self.numeric = result
            .as_deref()
            .map(|result| {
                result
                    .columns
                    .iter()
                    .map(|column| {
                        matches!(
                            field_value_kind(column),
                            FieldValueKind::Integer
                                | FieldValueKind::Unsigned
                                | FieldValueKind::Real
                                | FieldValueKind::Decimal
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.cell_text = result.as_deref().map_or_else(Vec::new, |result| {
            vec![None; result.rows.len() * result.columns.len()]
        });
        self.result = result;
        self.columns = columns;
        self.foreign_keys = foreign_keys
            .iter()
            .filter(|foreign_key| foreign_key_target_table(tables, foreign_key).is_some())
            .cloned()
            .collect();
    }

    /// Remembered widths by column key, from grid widths in display order.
    pub(super) fn widths_by_key(&self, widths: &[Pixels]) -> HashMap<String, Pixels> {
        let mut remembered = HashMap::new();

        if let Some(width) = widths.first().copied() {
            remembered.insert(ROW_NUMBER_COLUMN_KEY.to_owned(), width);
        }

        if let Some(result) = self.result.as_deref() {
            for (position, &index) in self.order.iter().enumerate() {
                if let Some(width) = widths.get(position + 1).copied() {
                    remembered.insert(Self::data_column_key(index, &result.columns[index]), width);
                }
            }
        }

        remembered
    }

    /// Current widths by column name, for a persisted table layout.
    pub(super) fn widths_by_name(&self) -> Vec<(String, f32)> {
        let Some(result) = self.result.as_deref() else {
            return Vec::new();
        };
        self.order
            .iter()
            .zip(self.columns.iter().skip(1))
            .map(|(&index, column)| (result.columns[index].name.clone(), f32::from(column.width)))
            .collect()
    }

    /// Set one grid column's width, clamped to the column limits.
    fn set_column_width(&mut self, col_ix: usize, width: Pixels) -> bool {
        let Some(column) = self.columns.get_mut(col_ix) else {
            return false;
        };
        let width = width.clamp(px(MIN_COLUMN_WIDTH), px(MAX_COLUMN_WIDTH));
        if column.width == width {
            return false;
        }
        column.width = width;
        true
    }

    /// The width that fits a column's header and every loaded value.
    fn fitted_width(&self, col_ix: usize) -> Option<Pixels> {
        let result = self.result.as_deref()?;
        let index = self.result_column(col_ix)?;
        let column = result.columns.get(index)?;
        let header = format!("{}  {}", column.name, display_type(&column.data_type))
            .chars()
            .count() as f32
            * 7.0
            + 28.0;
        let values = result
            .rows
            .iter()
            .filter_map(|row| row.values.get(index))
            .map(|value| cell_display_text(value).chars().count())
            .max()
            .unwrap_or_default() as f32
            * 7.0
            + 20.0;
        Some(px(header.max(values)))
    }

    fn foreign_key_for_cell(&self, row_ix: usize, col_ix: usize) -> Option<ForeignKeyInfo> {
        if col_ix == 0 {
            return None;
        }
        let result = self.result.as_ref()?;
        let row = result.rows.get(row_ix)?;
        let column = result.columns.get(self.result_column(col_ix)?)?;

        self.foreign_keys
            .iter()
            .find(|foreign_key| {
                foreign_key.columns.first() == Some(&column.name)
                    && foreign_key.columns.iter().all(|local_column| {
                        let Some(index) = result
                            .columns
                            .iter()
                            .position(|result_column| result_column.name == *local_column)
                        else {
                            return false;
                        };
                        row.values
                            .get(index)
                            .is_some_and(|value| !matches!(value, CellValue::Null))
                    })
            })
            .cloned()
    }
}

const NULL_SENTINEL: &str = "NULL";

/// Grid text for a staged value: the literal, or the SQL expression as typed.
fn staged_text(value: &MutationValue) -> String {
    match value {
        MutationValue::Parameter(value) => cell_display_text(value),
        MutationValue::Expression(expression) => expression.clone(),
    }
}

fn plain_cell_text(value: &CellValue) -> String {
    value.to_string()
}

fn delimited_row(values: &[CellValue], delimiter: char) -> String {
    let mut output = String::new();
    append_delimited_row(&mut output, values, delimiter);
    output
}

fn delimited_result(result: &QueryResult, delimiter: char) -> String {
    let mut output = String::new();
    append_delimited_text_row(
        &mut output,
        result.columns.iter().map(|column| column.name.as_str()),
        delimiter,
    );
    for row in &result.rows {
        output.push('\n');
        append_delimited_row(&mut output, &row.values, delimiter);
    }
    output
}

fn delimited_column<'a>(
    header: &str,
    values: impl Iterator<Item = &'a CellValue>,
    delimiter: char,
) -> String {
    let mut output = String::new();
    append_quoted_delimited_text(&mut output, header, delimiter, false);
    for value in values {
        output.push('\n');
        append_delimited_cell(&mut output, value, delimiter);
    }
    output
}

fn append_delimited_row(output: &mut String, values: &[CellValue], delimiter: char) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(delimiter);
        }
        append_delimited_cell(output, value, delimiter);
    }
}

fn append_delimited_cell(output: &mut String, value: &CellValue, delimiter: char) {
    match value {
        CellValue::Null => output.push_str(NULL_SENTINEL),
        value => append_quoted_delimited_text(output, &plain_cell_text(value), delimiter, true),
    }
}

fn append_delimited_text_row<'a>(
    output: &mut String,
    values: impl Iterator<Item = &'a str>,
    delimiter: char,
) {
    for (index, value) in values.enumerate() {
        if index > 0 {
            output.push(delimiter);
        }
        append_quoted_delimited_text(output, value, delimiter, false);
    }
}

fn append_quoted_delimited_text(
    output: &mut String,
    value: &str,
    delimiter: char,
    protect_null_sentinel: bool,
) {
    let needs_quotes = value.is_empty()
        || (protect_null_sentinel && value == NULL_SENTINEL)
        || value.contains(delimiter)
        || value.contains('"')
        || value.contains('\r')
        || value.contains('\n');
    if !needs_quotes {
        output.push_str(value);
        return;
    }

    output.push('"');
    for character in value.chars() {
        if character == '"' {
            output.push('"');
        }
        output.push(character);
    }
    output.push('"');
}

fn json_result(result: &QueryResult) -> String {
    // Serialize directly into the final buffer. Building a serde_json::Value
    // tree first duplicates every text/JSON cell until the final string is
    // produced, which is a large and avoidable peak for result exports.
    let mut output = Vec::new();
    output.extend_from_slice(br#"{"columns":["#);
    for (index, column) in result.columns.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        output.extend_from_slice(br#"{"data_type":"#);
        write_json_value(&mut output, &column.data_type);
        output.extend_from_slice(br#","name":"#);
        write_json_value(&mut output, &column.name);
        output.push(b'}');
    }
    output.extend_from_slice(br#"],"rows":["#);
    for (row_index, row) in result.rows.iter().enumerate() {
        if row_index > 0 {
            output.push(b',');
        }
        output.push(b'[');
        for (value_index, value) in row.values.iter().enumerate() {
            if value_index > 0 {
                output.push(b',');
            }
            write_json_cell_value(&mut output, value);
        }
        output.push(b']');
    }
    output.extend_from_slice(b"]}");
    String::from_utf8(output).expect("serde_json always writes UTF-8")
}

fn write_json_value<T: serde::Serialize>(output: &mut Vec<u8>, value: &T) {
    serde_json::to_writer(output, value).expect("writing JSON to Vec cannot fail");
}

fn write_json_cell_value(output: &mut Vec<u8>, value: &CellValue) {
    match value {
        CellValue::Null => output.extend_from_slice(b"null"),
        CellValue::Boolean(value) => write_json_value(output, value),
        CellValue::Integer(value) => write_json_value(output, value),
        CellValue::Unsigned(value) => write_json_value(output, value),
        CellValue::Real(value) => {
            if let Some(number) = serde_json::Number::from_f64(*value) {
                write_json_value(output, &number);
            } else {
                // JSON has no NaN or infinity; keeping their text avoids
                // silently turning a real database value into a NULL export.
                write_json_value(output, &value.to_string());
            }
        }
        CellValue::Text(value) => write_json_value(output, value),
        CellValue::Bytes(_) => {
            let text = plain_cell_text(value);
            write_json_value(output, &text);
        }
        CellValue::Json(value) => write_json_value(output, value),
    }
}

impl TableDelegate for ResultTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.loaded_rows() + self.changes.inserts.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> DataColumn {
        self.columns[col_ix].clone()
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        let (Some(from), Some(to)) = (col_ix.checked_sub(1), to_ix.checked_sub(1)) else {
            return;
        };
        if from >= self.order.len() || to >= self.order.len() {
            return;
        }
        let index = self.order.remove(from);
        self.order.insert(to, index);
        let column = self.columns.remove(col_ix);
        self.columns.insert(to_ix, column);
    }

    fn render_header(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id("dbx-result-header")
            .bg(theme().panel_raised)
            .border_color(theme().border_strong)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let index = self.result_column(col_ix);
        let column = index.and_then(|index| self.result.as_ref()?.columns.get(index));
        let numeric = index
            .and_then(|index| self.numeric.get(index).copied())
            .unwrap_or(false);
        let cell = div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(8.))
            .overflow_hidden()
            .when(numeric, |cell| cell.justify_end());
        let Some(column) = column else {
            return cell
                .text_size(px(10.))
                .text_color(theme().text_muted)
                .child(self.columns[col_ix].name.clone());
        };
        let resizing = self.resizing == Some(col_ix);
        let table = cx.entity().entity_id();
        // The table draws a sort icon and its cell padding after this header,
        // so reach past them to sit the handle on the column's real edge.
        let trailing = HEADER_TRAILING_PADDING
            + if self.sorting == ResultSorting::Disabled {
                0.
            } else {
                SORT_ICON_WIDTH
            };
        let content = cell
            .when(column.primary_key, |cell| {
                cell.child(
                    div()
                        .flex_none()
                        .text_size(px(8.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme().warning)
                        .child("PK"),
                )
            })
            .child(
                div()
                    .flex_shrink(1.)
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme().text)
                    .child(column.name.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(9.))
                    .text_color(theme().text_muted)
                    .child(display_type(&column.data_type)),
            );
        // The content clips its labels; the handle sits outside that clip.
        div()
            .size_full()
            .relative()
            .child(content)
            // A generous grab zone on the right edge. The table's own handle is
            // a 2px sliver that is nearly impossible to find.
            .child(
                div()
                    .id(("dbx-column-resize", col_ix))
                    .group("dbx-column-resize")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-trailing))
                    .w(px(RESIZE_HANDLE_WIDTH))
                    .flex()
                    .justify_end()
                    .cursor_col_resize()
                    .occlude()
                    .child(
                        div()
                            .h_full()
                            .w(px(2.))
                            .when(resizing, |line| line.bg(theme().accent))
                            .when(!resizing, |line| {
                                line.group_hover("dbx-column-resize", |line| {
                                    line.bg(theme().border_strong)
                                })
                            }),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |table, event: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        if event.click_count() == 2 {
                            let delegate = table.delegate_mut();
                            if let Some(width) = delegate.fitted_width(col_ix)
                                && delegate.set_column_width(col_ix, width)
                            {
                                table.refresh(cx);
                                emit_widths(table, cx);
                            }
                        }
                    }))
                    .on_drag(ColumnResize(table, col_ix), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| *drag)
                    })
                    .on_mouse_up(MouseButton::Left, cx.listener(finish_resize))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(finish_resize)),
            )
            .on_drag_move(
                cx.listener(move |table, event: &DragMoveEvent<ColumnResize>, _, cx| {
                    let ColumnResize(owner, dragged) = *event.drag(cx);
                    if owner != cx.entity().entity_id() || dragged != col_ix {
                        return;
                    }
                    let width = event.event.position.x - event.bounds.left();
                    let delegate = table.delegate_mut();
                    delegate.resizing = Some(col_ix);
                    if delegate.set_column_width(col_ix, width) {
                        table.refresh(cx);
                    }
                    cx.notify();
                }),
            )
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let base = if row_ix.is_multiple_of(2) {
            theme().canvas
        } else {
            theme().grid_alternate
        };
        let background = if self.changes.marked.contains(&row_ix) && self.changes.marked.len() > 1 {
            theme().accent.alpha(0.14)
        } else if self.changes.deletes.contains(&row_ix) {
            theme().danger.alpha(0.12)
        } else if self.insert_row(row_ix).is_some() {
            theme().success.alpha(0.12)
        } else {
            base
        };
        let row = div()
            .id(("dbx-result-row", row_ix))
            .debug_selector(move || format!("dbx-result-row-{row_ix}"))
            .border_color(theme().border)
            .bg(background);
        let Some((app, session_id, tab_id, true)) = self.row_actions.clone() else {
            return row;
        };
        // Cell clicks select through the table; this only reads the modifiers
        // so Shift extends and Cmd/Ctrl toggles a multi-row mark.
        row.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            let (extend, toggle) = (event.modifiers.shift, event.modifiers.secondary());
            let app = app.clone();
            cx.defer(move |cx| {
                let _ = app.update(cx, |this, cx| {
                    this.mark_row_for(session_id, tab_id, row_ix, extend, toggle, cx);
                });
            });
        })
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(index) = self.result_column(col_ix) else {
            return;
        };
        let direction = match sort {
            ColumnSort::Ascending => Some(OrderDirection::Ascending),
            ColumnSort::Descending => Some(OrderDirection::Descending),
            ColumnSort::Default => None,
        };
        match self.sorting {
            ResultSorting::Disabled => {}
            ResultSorting::Local => {
                self.sort_locally(index, direction);
                for (column_ix, column) in self.columns.iter_mut().enumerate().skip(1) {
                    column.sort = Some(if column_ix == col_ix {
                        sort
                    } else {
                        ColumnSort::Default
                    });
                }
                cx.notify();
            }
            ResultSorting::Server => {
                let Some((app, session_id, tab_id, _)) = self.row_actions.clone() else {
                    return;
                };
                let Some(column) = self
                    .result
                    .as_ref()
                    .and_then(|result| result.columns.get(index))
                else {
                    return;
                };
                let order = direction.map(|direction| Order {
                    column: column.name.clone(),
                    direction,
                });
                // The app reloads through this grid, which is still borrowed here.
                cx.defer(move |cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.set_table_sort_for(session_id, tab_id, order, cx);
                    });
                });
            }
        }
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let mut menu = menu;
        for (label, text) in [
            ("Copy as JSON", self.row_as_json(row_ix)),
            ("Copy as TSV", self.row_as_tsv(row_ix)),
        ] {
            menu = menu.item(PopupMenuItem::new(label).disabled(text.is_none()).on_click(
                move |_, _, cx| {
                    if let Some(text) = &text {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    }
                },
            ));
        }
        let Some((app, session_id, tab_id, is_data)) = self.row_actions.clone() else {
            return menu;
        };
        if !is_data {
            return menu;
        }
        let sql = app.upgrade().and_then(|app| {
            let owner = app.read(cx);
            let kind = owner.session(session_id)?.kind;
            if !kind.is_sql() {
                return None;
            }
            let table = &owner.data_tab(session_id, tab_id)?.table;
            let result = self.result.as_ref()?;
            let row = result.rows.get(row_ix)?;
            let columns = result
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            dbx_core::render_sql_insert(kind, table, &columns, &row.values)
                .ok()
                .map(|sql| format!("{sql};"))
        });
        menu = menu
            .item(
                PopupMenuItem::new("Copy as SQL")
                    .disabled(sql.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(sql) = &sql {
                            cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()));
                        }
                    }),
            )
            .separator();
        if let Some(column) = self.context_column
            && row_ix < self.loaded_rows()
        {
            menu = self.column_menu(menu, &app, session_id, tab_id, row_ix, column);
        }
        let clicked_row = self
            .result
            .as_ref()
            .and_then(|result| result.rows.get(row_ix))
            .cloned();
        let can_edit = app.upgrade().is_some_and(|app| {
            let owner = app.read(cx);
            owner.editable_table_for(session_id, tab_id).is_some()
                && owner
                    .data_tab(session_id, tab_id)
                    .is_some_and(|data| !data.busy && data.row_draft.is_none())
        });
        let is_new = self.insert_row(row_ix).is_some();
        let has_changes = is_new
            || self.changes.deletes.contains(&row_ix)
            || self.changes.edits.keys().any(|(row, _)| *row == row_ix);
        let marked = self.changes.marked.len().max(1);
        let delete_label: SharedString = if self.changes.deletes.contains(&row_ix) {
            "Restore row".into()
        } else if marked > 1 && self.changes.marked.contains(&row_ix) {
            format!("Delete {marked} rows").into()
        } else {
            "Delete row".into()
        };
        for (label, action, enabled) in [
            ("Inspect row".into(), RowAction::Inspect, true),
            ("Edit row".into(), RowAction::Edit, can_edit),
            ("Duplicate row".into(), RowAction::Duplicate, can_edit),
            (delete_label, RowAction::Delete, can_edit),
            (
                if is_new {
                    "Remove new row"
                } else {
                    "Revert changes"
                }
                .into(),
                RowAction::Revert,
                can_edit && has_changes,
            ),
        ] {
            let app = app.clone();
            let clicked_row = clicked_row.clone();
            menu = menu.item(PopupMenuItem::new(label).disabled(!enabled).on_click(
                move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        if this
                            .data_tab(session_id, tab_id)
                            .and_then(|data| data.result.as_ref()?.rows.get(row_ix))
                            != clicked_row.as_ref()
                        {
                            return;
                        }
                        // Bulk delete keeps the marked rows; other actions
                        // apply to the clicked row alone.
                        if matches!(action, RowAction::Delete) {
                            this.delete_rows_for(session_id, tab_id, Some(row_ix), cx);
                            return;
                        }
                        this.select_row_for(session_id, tab_id, row_ix, cx);
                        // A pending draft may reject selection. Never apply an
                        // action to a different row than the one right-clicked.
                        if this.data_tab(session_id, tab_id).is_none_or(|data| {
                            data.selected_row != Some(row_ix) || data.row_draft.is_some()
                        }) {
                            return;
                        }
                        match action {
                            RowAction::Edit => {
                                this.begin_edit_selected_for(session_id, tab_id, window, cx)
                            }
                            RowAction::Duplicate => {
                                this.duplicate_row_for(session_id, tab_id, row_ix, cx)
                            }
                            RowAction::Revert => {
                                this.revert_row_for(session_id, tab_id, row_ix, cx)
                            }
                            RowAction::Delete | RowAction::Inspect => {}
                        }
                    });
                },
            ));
        }
        menu
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if let Some((_, _, editor)) = self.editing.as_ref().filter(|(row, column, _)| {
            *row == row_ix && Some(*column) == self.result_column(col_ix)
        }) {
            let focus = editor.read(cx).focus_handle();
            return div()
                .size_full()
                .p(px(2.))
                .child(
                    crate::editor::input_with_key_context(
                        editor.clone(),
                        focus,
                        false,
                        super::cell_edits::CELL_EDITOR_CONTEXT,
                    )
                    .h_full()
                    .py(px(0.))
                    .px(px(6.))
                    .text_size(px(11.)),
                )
                .into_any_element();
        }
        let deleted = self.changes.deletes.contains(&row_ix);
        let data_column = self.result_column(col_ix);
        // The table opens its row menu on right mouse down; note the column
        // first so the menu can offer column actions.
        // With cell selection on, the table's own cell handler clears the
        // right-clicked row, so its row menu would build empty. Record the row
        // here, before that handler, and keep it from running.
        let remember_column = cx.listener(move |table, _: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            table.delegate_mut().context_column = data_column;
            table.set_right_clicked_row(Some(row_ix), cx);
        });
        if let Some(insert) = self.insert_row(row_ix) {
            let text = match data_column {
                None => Some("+".to_owned()),
                Some(column) => insert
                    .get(column)
                    .cloned()
                    .flatten()
                    .map(|value| staged_text(&value)),
            };
            let muted = text.is_none() || col_ix == 0;
            return div()
                .size_full()
                .flex()
                .items_center()
                .px(px(8.))
                .text_size(px(11.))
                .text_color(if muted {
                    theme().text_muted
                } else {
                    theme().text
                })
                .when(muted && col_ix > 0, |cell| cell.italic())
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(text.unwrap_or_else(|| "DEFAULT".into())),
                )
                .on_mouse_down(MouseButton::Right, remember_column)
                .into_any_element();
        }
        let staged = data_column
            .and_then(|column| self.changes.edits.get(&(row_ix, column)))
            .cloned();
        if let Some(value) = staged.filter(|_| !deleted) {
            let muted =
                !matches!(&value, MutationValue::Parameter(value) if *value != CellValue::Null);
            return div()
                .size_full()
                .flex()
                .items_center()
                .px(px(8.))
                .bg(theme().warning.alpha(0.16))
                .text_size(px(11.))
                .text_color(if muted {
                    theme().text_muted
                } else {
                    theme().text
                })
                .when(muted, |cell| cell.italic())
                .child(div().min_w_0().truncate().child(staged_text(&value)))
                .on_mouse_down(MouseButton::Right, remember_column)
                .into_any_element();
        }
        let mut null = false;
        let (text, text_color): (SharedString, _) = if let Some(data_column) = data_column {
            let column_count = self
                .result
                .as_ref()
                .map_or(0, |result| result.columns.len());
            let color = match self.cell_value(row_ix, data_column) {
                None => None,
                Some(CellValue::Null) => {
                    null = true;
                    Some(theme().text_muted.alpha(0.7))
                }
                Some(CellValue::Boolean(true)) => Some(theme().success),
                Some(CellValue::Boolean(false)) => Some(theme().text_muted),
                Some(_) => Some(theme().text),
            };
            match color {
                None => ("—".into(), theme().text_muted),
                Some(color) => {
                    let slot = row_ix * column_count + data_column;
                    let text = match self.cell_text.get(slot).cloned().flatten() {
                        Some(text) => text,
                        None => {
                            let text: SharedString = self
                                .cell_value(row_ix, data_column)
                                .map(cell_display_text)
                                .unwrap_or_default()
                                .into();
                            if let Some(cached) = self.cell_text.get_mut(slot) {
                                *cached = Some(text.clone());
                            }
                            text
                        }
                    };
                    (text, color)
                }
            }
        } else {
            (
                (self.row_offset + row_ix + 1).to_string().into(),
                theme().text_muted,
            )
        };
        let foreign_key = self.foreign_key_for_cell(row_ix, col_ix);
        let numeric = data_column
            .and_then(|index| self.numeric.get(index).copied())
            .unwrap_or(false);

        let mut cell = div()
            .size_full()
            .on_mouse_down(MouseButton::Right, remember_column)
            .flex()
            .items_center()
            .px(px(8.))
            .whitespace_nowrap()
            .truncate()
            .text_size(px(11.))
            .text_color(if deleted {
                theme().text_muted
            } else {
                text_color
            })
            .when(null, |cell| cell.italic())
            .when(deleted && col_ix > 0, |cell| cell.line_through())
            .when(numeric && foreign_key.is_none(), |cell| cell.justify_end());
        if foreign_key.is_some() {
            cell = cell
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .when(numeric, |value| value.justify_end())
                        .child(div().min_w_0().truncate().child(text)),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "foreign-key-link-{row_ix}-{col_ix}"
                    )))
                    .with_size(gpui_component::Size::XSmall)
                    .compact()
                    .ghost()
                    .tooltip("Open referenced row")
                    .text_color(theme().accent)
                    .child(icon(Icon::ArrowRight, theme().accent))
                    .on_click(cx.listener(move |table, _, window, cx| {
                        cx.stop_propagation();
                        let Some((app, session_id, tab_id, true)) =
                            table.delegate().row_actions.clone()
                        else {
                            return;
                        };
                        // The app reloads through this grid, which is still borrowed here.
                        window.defer(cx, move |window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.navigate_to_foreign_key_row_for(
                                    session_id, tab_id, row_ix, col_ix, window, cx,
                                );
                            });
                        });
                    })),
                );
        } else {
            cell = cell.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(numeric, |value| value.text_right())
                    .child(text),
            );
        }
        cell.into_any_element()
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme().text_muted)
            .child("No rows returned")
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        if col_ix == 0 {
            return (self.row_offset + row_ix + 1).to_string();
        }

        self.result
            .as_ref()
            .and_then(|result| result.rows.get(row_ix))
            .and_then(|row| row.values.get(self.result_column(col_ix)?))
            .map(ToString::to_string)
            .unwrap_or_default()
    }
}

pub(super) fn foreign_key_target_table(
    tables: &[TableInfo],
    foreign_key: &ForeignKeyInfo,
) -> Option<TableInfo> {
    tables
        .iter()
        .find(|table| {
            table.name == foreign_key.referenced_table
                && match foreign_key.referenced_schema.as_deref() {
                    Some(schema) => table.schema.as_deref() == Some(schema),
                    None => true,
                }
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::RowData;

    fn export_result() -> QueryResult {
        QueryResult {
            columns: vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("note", 1, "TEXT"),
                ColumnInfo::result("note", 2, "TEXT"),
            ],
            rows: vec![
                RowData::new(vec![
                    CellValue::Integer(7),
                    CellValue::Null,
                    CellValue::Text(String::new()),
                ]),
                RowData::new(vec![
                    CellValue::Integer(8),
                    CellValue::Text("comma, tab\t quote\" newline\n".into()),
                    CellValue::Text("NULL".into()),
                ]),
            ],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn layouts_pin_reorder_and_hide_columns_by_name() {
        let columns = ["id", "name", "email", "created"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| ColumnInfo::result(name, index, "TEXT"))
            .collect::<Vec<_>>();
        let layout = TableLayout {
            pinned: vec!["email".into(), "missing".into()],
            order: vec!["created".into()],
            hidden: ["name".to_owned()].into(),
            ..Default::default()
        };
        assert_eq!(display_order(&columns, Some(&layout)), vec![2, 3, 0]);
        assert_eq!(display_order(&columns, None), vec![0, 1, 2, 3]);
        let everything_hidden = TableLayout {
            hidden: columns.iter().map(|column| column.name.clone()).collect(),
            ..Default::default()
        };
        assert_eq!(display_order(&columns, Some(&everything_hidden)), vec![0]);

        let mut delegate = ResultTableDelegate::default();
        let result = QueryResult {
            columns,
            rows: Vec::new(),
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            Some(&layout),
            &[],
            &[],
        );
        // Grid column 1 is the pinned email column.
        assert_eq!(delegate.result_column(1), Some(2));
        assert_eq!(delegate.grid_column(0), Some(3));
        assert_eq!(delegate.grid_column(1), None);
        assert!(delegate.columns[1].fixed.is_some());
        assert!(delegate.set_column_width(1, px(5_000.)));
        assert_eq!(
            delegate.widths_by_name()[0],
            ("email".to_owned(), MAX_COLUMN_WIDTH)
        );
    }

    #[test]
    fn local_sort_orders_numbers_and_nulls_then_restores_original_rows() {
        let mut delegate = ResultTableDelegate {
            sorting: ResultSorting::Local,
            ..ResultTableDelegate::default()
        };
        let result = QueryResult {
            columns: vec![ColumnInfo::result("n", 0, "TEXT")],
            rows: ["10", "9", "", "b"]
                .into_iter()
                .map(|value| {
                    RowData::new(vec![if value.is_empty() {
                        CellValue::Null
                    } else {
                        CellValue::Text(value.into())
                    }])
                })
                .chain([RowData::new(vec![CellValue::Integer(2)])])
                .collect(),
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };
        delegate.set_result(Some(Arc::new(result)), &HashMap::new(), None, &[], &[]);
        let column = |delegate: &ResultTableDelegate| {
            delegate
                .column_values(0)
                .unwrap()
                .into_iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };
        let original = column(&delegate);

        delegate.sort_locally(0, Some(OrderDirection::Ascending));
        assert_eq!(column(&delegate), ["NULL", "2", "9", "10", "b"]);
        delegate.sort_locally(0, Some(OrderDirection::Descending));
        assert_eq!(column(&delegate), ["b", "10", "9", "2", "NULL"]);
        delegate.sort_locally(0, None);
        assert_eq!(column(&delegate), original);
        assert_eq!(delegate.column_sort(0), Some(ColumnSort::Default));
    }

    #[test]
    fn server_sort_marks_the_requested_column() {
        let mut delegate = delegate_with_export_result();
        delegate.set_server_sort(
            true,
            Some(&Order {
                column: "id".into(),
                direction: OrderDirection::Descending,
            }),
        );
        delegate.set_result(
            Some(Arc::new(export_result())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        assert_eq!(delegate.column_sort(0), Some(ColumnSort::Descending));
        assert_eq!(delegate.column_sort(1), Some(ColumnSort::Default));
        delegate.set_server_sort(false, None);
        delegate.set_result(
            Some(Arc::new(export_result())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        assert_eq!(delegate.column_sort(0), None);
    }

    fn delegate_with_export_result() -> ResultTableDelegate {
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(export_result())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate
    }

    #[test]
    fn foreign_key_target_resolves_the_referenced_schema() {
        let tables = vec![
            TableInfo::table("users", Some("analytics".into())),
            TableInfo::table("users", Some("public".into())),
        ];
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("events_user_id_fkey".into()),
            columns: vec!["user_id".into()],
            referenced_schema: Some("analytics".into()),
            referenced_table: "users".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };

        assert_eq!(
            foreign_key_target_table(&tables, &foreign_key),
            Some(TableInfo::table("users", Some("analytics".into())))
        );
    }

    #[test]
    fn foreign_key_target_is_unavailable_when_the_table_is_not_listed() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: None,
            columns: vec!["owner_id".into()],
            referenced_schema: None,
            referenced_table: "owners".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };

        assert_eq!(foreign_key_target_table(&[], &foreign_key), None);
    }

    #[test]
    fn result_grid_marks_populated_foreign_key_cells_as_navigable() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("orders_customer_id_fkey".into()),
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };
        let tables = vec![TableInfo::table("customers", Some("public".into()))];
        let result = QueryResult {
            columns: vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("customer_id", 1, "INTEGER"),
            ],
            rows: vec![RowData::new(vec![
                CellValue::Integer(1),
                CellValue::Integer(42),
            ])],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            None,
            &[foreign_key],
            &tables,
        );

        assert!(delegate.foreign_key_for_cell(0, 2).is_some());
    }

    #[test]
    fn result_grid_hides_foreign_key_action_for_null_values() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: None,
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };
        let tables = vec![TableInfo::table("customers", Some("public".into()))];
        let result = QueryResult {
            columns: vec![ColumnInfo::result("customer_id", 0, "INTEGER")],
            rows: vec![RowData::new(vec![CellValue::Null])],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            None,
            &[foreign_key],
            &tables,
        );

        assert!(delegate.foreign_key_for_cell(0, 1).is_none());
    }

    #[test]
    fn result_accessors_retain_database_nulls_and_data_column_order() {
        let delegate = delegate_with_export_result();

        assert_eq!(delegate.cell_value(0, 1), Some(&CellValue::Null));
        assert_eq!(delegate.cell_as_plain_text(0, 1).as_deref(), Some("NULL"));
        assert_eq!(delegate.cell_as_plain_text(0, 2).as_deref(), Some(""));
        assert_eq!(delegate.row_values(1).unwrap()[0], CellValue::Integer(8));
        assert_eq!(
            delegate.column_values(0).unwrap(),
            vec![&CellValue::Integer(7), &CellValue::Integer(8)]
        );
        assert!(delegate.cell_value(8, 0).is_none());
        assert!(delegate.column_values(8).is_none());
    }

    #[test]
    fn delimited_exports_escape_controls_and_preserve_null_vs_empty_text() {
        let delegate = delegate_with_export_result();

        assert_eq!(delegate.row_as_tsv(0).as_deref(), Some("7\tNULL\t\"\""));
        assert_eq!(
            delegate.result_as_csv().as_deref(),
            Some("id,note,note\n7,NULL,\"\"\n8,\"comma, tab\t quote\"\" newline\n\",\"NULL\"")
        );
        assert_eq!(
            delegate.result_as_tsv().as_deref(),
            Some(
                "id\tnote\tnote\n7\tNULL\t\"\"\n8\t\"comma, tab\t quote\"\" newline\n\"\t\"NULL\""
            )
        );
    }

    #[test]
    fn column_tsv_export_includes_its_header_and_preserves_null_vs_empty_text() {
        let delegate = delegate_with_export_result();

        assert_eq!(
            delegate.column_as_tsv(1).as_deref(),
            Some("note\nNULL\n\"comma, tab\t quote\"\" newline\n\"")
        );
        assert_eq!(
            delegate.column_as_tsv(2).as_deref(),
            Some("note\n\"\"\n\"NULL\"")
        );
    }

    #[test]
    fn column_tsv_export_returns_none_without_a_result_or_for_an_invalid_column() {
        let empty_delegate = ResultTableDelegate::default();
        assert!(empty_delegate.column_as_tsv(0).is_none());

        let delegate = delegate_with_export_result();
        assert!(delegate.column_as_tsv(8).is_none());
    }

    #[test]
    fn json_export_is_positional_to_preserve_duplicate_aliases_and_nulls() {
        let delegate = delegate_with_export_result();
        let exported: serde_json::Value =
            serde_json::from_str(&delegate.result_as_json().unwrap()).unwrap();

        assert_eq!(exported["columns"][1]["name"], "note");
        assert_eq!(exported["columns"][2]["name"], "note");
        assert!(exported["rows"][0][1].is_null());
        assert_eq!(exported["rows"][0][2], "");
        assert_eq!(exported["rows"][1][2], "NULL");
    }

    struct RowMenuHarness {
        grid: gpui::Entity<TableState<ResultTableDelegate>>,
    }
    impl gpui::Render for RowMenuHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui_component::table::DataTable::new(&self.grid).with_size(px(30.))
        }
    }

    #[gpui::test]
    fn right_click_copies_the_clicked_row_instead_of_the_selected_row(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (_, cx) = cx.add_window_view(|window, cx| RowMenuHarness {
            grid: cx.new(|cx| {
                let mut grid = TableState::new(delegate_with_export_result(), window, cx);
                grid.set_selected_row(0, cx);
                grid
            }),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let row = cx
            .debug_bounds("dbx-result-row-1")
            .expect("second row must be visible");
        cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("down enter");
        let copied = cx
            .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .expect("row menu must write the clipboard");
        let value: serde_json::Value = serde_json::from_str(&copied).unwrap();
        assert_eq!(value["rows"].as_array().unwrap().len(), 1);
        assert_eq!(value["rows"][0][0], 8);
    }

    #[test]
    fn row_json_preserves_typed_values_and_full_text() {
        let text = "line\nquoted\"".repeat(100);
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(QueryResult {
                columns: vec![
                    ColumnInfo::result("id", 0, "INTEGER"),
                    ColumnInfo::result("note", 1, "TEXT"),
                    ColumnInfo::result("empty", 2, "TEXT"),
                    ColumnInfo::result("missing", 3, "TEXT"),
                    ColumnInfo::result("payload", 4, "JSON"),
                ],
                rows: vec![RowData::new(vec![
                    CellValue::Integer(7),
                    CellValue::Text(text.clone()),
                    CellValue::Text(String::new()),
                    CellValue::Null,
                    CellValue::Json(serde_json::json!({"ok": true})),
                ])],
                rows_affected: None,
                truncated: false,
                elapsed_ms: 0,
            })),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        let value: serde_json::Value =
            serde_json::from_str(&delegate.row_as_json(0).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"id":7,"note":text,"empty":"","missing":null,"payload":{"ok":true}})
        );
        assert!(delegate.row_as_json(1).is_none());
    }

    #[test]
    fn row_json_keeps_duplicate_query_aliases_and_only_the_clicked_row() {
        let delegate = delegate_with_export_result();
        let value: serde_json::Value =
            serde_json::from_str(&delegate.row_as_json(1).unwrap()).unwrap();
        assert_eq!(value["columns"][1]["name"], "note");
        assert_eq!(value["columns"][2]["name"], "note");
        assert_eq!(value["rows"].as_array().unwrap().len(), 1);
        assert_eq!(value["rows"][0][0], 8);
        assert_eq!(value["rows"][0][2], "NULL");
    }

    #[test]
    fn empty_result_exports_its_headers_instead_of_disappearing() {
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(QueryResult {
                columns: vec![ColumnInfo::result("id", 0, "INTEGER")],
                rows: Vec::new(),
                rows_affected: None,
                truncated: false,
                elapsed_ms: 0,
            })),
            &HashMap::new(),
            None,
            &[],
            &[],
        );

        assert_eq!(delegate.result_as_csv().as_deref(), Some("id"));
        assert_eq!(delegate.result_as_tsv().as_deref(), Some("id"));
    }
}
