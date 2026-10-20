use gpui_component::ElementExt;
use std::rc::Rc;
mod accessibility;
pub(super) use accessibility::accessible_grid;

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

use crate::row_drafts::{FieldValueKind, field_value_kind};
use crate::theme::{ButtonAccessibility as _, Icon, icon, theme};

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

const MIN_COLUMN_WIDTH: f32 = 60.;
const MAX_COLUMN_WIDTH: f32 = 1_200.;
const RESIZE_HANDLE_WIDTH: f32 = 7.;
/// gpui-component's header sort button: a 12px icon with 2px padding.
const SORT_ICON_WIDTH: f32 = 12.;
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
pub(super) fn emit_widths(
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

fn rendered_cell_text(text: SharedString, json: bool) -> gpui::StyledText {
    if json {
        super::value_view::json_text(text)
    } else {
        gpui::StyledText::new(text)
    }
}

/// One row's sort cell with its numeric reading computed once, so an
/// `n log n` sort never re-parses text or re-formats values per comparison.
struct SortKey<'a> {
    value: &'a CellValue,
    number: Option<f64>,
    text: std::cell::OnceCell<String>,
}

impl<'a> SortKey<'a> {
    fn new(value: Option<&'a CellValue>) -> Self {
        let value = value.unwrap_or(&CellValue::Null);
        let number = match value {
            CellValue::Integer(value) => Some(*value as f64),
            CellValue::Unsigned(value) => Some(*value as f64),
            CellValue::Real(value) => Some(*value),
            CellValue::Text(text) => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite()),
            _ => None,
        };
        Self {
            value,
            number,
            text: std::cell::OnceCell::new(),
        }
    }

    fn text(&self) -> &str {
        self.text.get_or_init(|| plain_cell_text(self.value))
    }
}

/// Order cells for a local header sort: NULLs first, numbers numerically,
/// then everything else by its display text.
fn compare_cells(left: &SortKey, right: &SortKey) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left.value, right.value) {
        (CellValue::Null, CellValue::Null) => Ordering::Equal,
        (CellValue::Null, _) => Ordering::Less,
        (_, CellValue::Null) => Ordering::Greater,
        (CellValue::Integer(left), CellValue::Integer(right)) => left.cmp(right),
        (CellValue::Unsigned(left), CellValue::Unsigned(right)) => left.cmp(right),
        (CellValue::Boolean(left), CellValue::Boolean(right)) => left.cmp(right),
        (CellValue::Bytes(left), CellValue::Bytes(right)) => left.cmp(right),
        _ => match (left.number, right.number) {
            (Some(left), Some(right)) => left.total_cmp(&right),
            _ => left.text().cmp(right.text()),
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
    pub(super) binary_formats: HashMap<usize, super::value_view::BinaryFormat>,
    foreign_keys: Vec<ForeignKeyInfo>,
    row_actions: Option<(WeakEntity<DbxApp>, SessionId, SecondaryTabId, bool)>,
    /// Catalog primary keys identify table rows, but can repeat in query joins.
    is_data_result: bool,
    table_columns: Vec<ColumnInfo>,
    accessible_revision: u64,
    accessible_dataset_revision: u64,
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
    /// Built only while assistive technology is active. Table primary keys
    /// survive reloads; ambiguous results are scoped to their snapshot and page.
    /// Typed values and occurrence keys distinguish duplicates during sorting.
    accessible_rows: Vec<u64>,
    accessible_insert_keys: Vec<u64>,
    next_accessible_insert_key: u64,
    accessible_selected_cell: Option<(usize, usize)>,
    accessible_selected_row: Option<usize>,
    accessible_focused: bool,
    accessible_editable: bool,
    /// The semantic model covers loaded cells independently of visual virtualization.
    accessible_logical_tree: bool,
    accessible_header_focus: Option<SharedString>,
    accessible_header_selection: Option<(usize, usize)>,
    accessible_geometry: Rc<std::cell::RefCell<accessibility::GridGeometry>>,
    /// Result column shown at each display position.
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
            columns: Vec::new(),
            numeric: Vec::new(),
            cell_text: Vec::new(),
            binary_formats: HashMap::new(),
            foreign_keys: Vec::new(),
            row_actions: None,
            is_data_result: false,
            table_columns: Vec::new(),
            accessible_revision: 0,
            accessible_dataset_revision: 0,
            sorting: ResultSorting::Disabled,
            unsorted: None,
            sort: None,
            server_order: None,
            changes: GridChanges::default(),
            row_offset: 0,
            accessible_rows: Vec::new(),
            accessible_insert_keys: Vec::new(),
            next_accessible_insert_key: 1,
            accessible_selected_cell: None,
            accessible_selected_row: None,
            accessible_focused: false,
            accessible_editable: false,
            accessible_logical_tree: false,
            accessible_header_focus: None,
            accessible_header_selection: None,
            accessible_geometry: Rc::new(std::cell::RefCell::new(Default::default())),
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
            is_data_result: is_data,
            sorting: if is_data {
                ResultSorting::Disabled
            } else {
                ResultSorting::Local
            },
            ..Self::default()
        }
    }

    pub(super) fn grow_text_widths(&mut self, ratio: f32) {
        for column in &mut self.columns {
            column.width = (column.width * ratio).clamp(column.min_width, column.max_width);
        }
    }

    pub(super) fn scale_column_widths(&mut self, ratio: f32) {
        for column in &mut self.columns {
            column.width *= ratio;
            column.min_width *= ratio;
            column.max_width *= ratio;
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
        if changes.inserts.len() != self.changes.inserts.len() {
            let mut used = std::collections::HashSet::new();
            self.accessible_insert_keys = changes
                .inserts
                .iter()
                .map(|insert| {
                    let previous = self
                        .changes
                        .inserts
                        .iter()
                        .enumerate()
                        .find(|(index, old)| !used.contains(index) && *old == insert)
                        .map(|(index, _)| index);
                    if let Some(index) = previous {
                        used.insert(index);
                        self.accessible_insert_keys[index]
                    } else {
                        let key = self.next_accessible_insert_key;
                        self.next_accessible_insert_key += 1;
                        key
                    }
                })
                .collect();
        }
        self.changes = changes;
        self.editing = editing;
    }

    pub(super) fn set_table_columns(&mut self, columns: &[ColumnInfo]) {
        if self.table_columns != columns {
            self.table_columns = columns.to_vec();
            self.accessible_rows.clear();
            self.accessible_revision = self.accessible_revision.wrapping_add(1);
        }
    }

    pub(super) fn set_row_offset(&mut self, row_offset: usize) {
        if self.row_offset != row_offset {
            self.row_offset = row_offset;
            self.accessible_rows.clear();
            self.accessible_revision = self.accessible_revision.wrapping_add(1);
        }
    }

    /// The result column a grid column shows.
    pub(super) fn result_column(&self, col_ix: usize) -> Option<usize> {
        self.order.get(col_ix).copied()
    }

    /// The grid column showing a result column, if it is visible.
    pub(super) fn grid_column(&self, column: usize) -> Option<usize> {
        self.order.iter().position(|shown| *shown == column)
    }

    pub(super) fn binary_format(&self, column: usize) -> super::value_view::BinaryFormat {
        self.binary_formats
            .get(&column)
            .copied()
            .unwrap_or_default()
    }

    fn formatted_value(&self, column: usize, value: &CellValue) -> String {
        if matches!(value, CellValue::Bytes(_)) {
            let format = self.binary_format(column);
            let limit = if format == super::value_view::BinaryFormat::Binary {
                CELL_TEXT_LIMIT / 9
            } else {
                CELL_TEXT_LIMIT / 2
            };
            format.limited_text(value, limit)
        } else {
            cell_display_text(value)
        }
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
        (row_ix, column): (usize, usize),
        cx: &App,
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
        let rows = self.menu_rows(row_ix);
        let multiple = rows.len() > 1;
        let copy = if multiple {
            self.result.as_ref().and_then(|result| {
                let values = rows
                    .iter()
                    .map(|row| result.rows.get(*row)?.values.get(column).cloned())
                    .collect::<Option<Vec<_>>>()?;
                Some(
                    values
                        .iter()
                        .map(|value| {
                            let value = if matches!(value, CellValue::Bytes(_)) {
                                CellValue::Text(self.binary_format(column).editor_text(value))
                            } else {
                                value.clone()
                            };
                            delimited_row(std::slice::from_ref(&value), '\t')
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            })
        } else {
            Some(self.binary_format(column).editor_text(value))
        };
        menu = menu.item(
            PopupMenuItem::new(if multiple {
                format!("Copy {} cells", rows.len())
            } else {
                "Copy cell".into()
            })
            .disabled(copy.is_none())
            .on_click(move |_, _, cx| {
                if let Some(copy) = &copy {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                }
            }),
        );
        if matches!(value, CellValue::Bytes(_)) || field_value_kind(info) == FieldValueKind::Bytes {
            use super::value_view::BinaryFormat;
            for (label, format) in [
                ("View as binary", BinaryFormat::Binary),
                ("View as hex / edit hex", BinaryFormat::Hex),
                ("View as UUID", BinaryFormat::Uuid),
                ("View as UUID (MySQL swapped)", BinaryFormat::UuidSwapped),
            ] {
                let app = app.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(self.binary_format(column) == format)
                        .on_click(move |_, _, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.set_binary_format_for(session_id, tab_id, column, format, cx);
                            });
                        }),
                );
            }
        }
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
        let supports_filters = app.upgrade().is_some_and(|app| {
            app.read(cx).session(session_id).is_some_and(|session| {
                session
                    .engine
                    .as_ref()
                    .is_some_and(|engine| engine.capabilities().structured_filters)
            })
        });
        for (label, operator) in filters.iter().copied().filter(|_| supports_filters) {
            let app = app.clone();
            menu = menu.item(PopupMenuItem::new(label).disabled(multiple).on_click(
                move |_, window, cx| {
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
                },
            ));
        }
        let (can_edit, nullable) = app
            .upgrade()
            .map(|app| {
                let owner = app.read(cx);
                let data = owner.data_tab(session_id, tab_id);
                let editable = owner.editable_table_for(session_id, tab_id).is_some()
                    && data.is_some_and(|data| {
                        !data.busy
                            && data.cell_editor.is_none()
                            && data.row_draft.is_none()
                            && data.recovered_changeset.is_none()
                    });
                let nullable = data
                    .and_then(|data| data.table_columns.iter().find(|info| info.name == name))
                    .is_none_or(|info| info.nullable);
                (editable, nullable)
            })
            .unwrap_or((false, false));
        let supports_edits = app
            .upgrade()
            .is_some_and(|app| app.read(cx).driver_supports_row_edits(session_id));
        for (label, set_null) in [
            (
                format!(
                    "Set value in {}…",
                    super::counted(rows.len(), "selected row", "selected rows")
                ),
                false,
            ),
            (
                format!(
                    "Set NULL in {}",
                    super::counted(rows.len(), "selected row", "selected rows")
                ),
                true,
            ),
        ]
        .into_iter()
        .filter(|_| supports_edits)
        {
            let app = app.clone();
            let rows = rows.clone();
            let snapshot = self.result.clone();
            let inserts = self.changes.inserts.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .disabled(!can_edit || (set_null && !nullable))
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| {
                            let valid = this.data_tab(session_id, tab_id).is_some_and(|data| {
                                data.result
                                    .as_ref()
                                    .zip(snapshot.as_ref())
                                    .is_some_and(|(current, old)| Arc::ptr_eq(current, old))
                                    && data.data_grid.read(cx).delegate().changes.inserts == inserts
                            });
                            if valid {
                                this.begin_bulk_cell_edit_for(
                                    session_id,
                                    tab_id,
                                    (rows.clone(), column),
                                    set_null,
                                    window,
                                    cx,
                                );
                            }
                        });
                    }),
            );
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
        let keys = source
            .rows
            .iter()
            .map(|row| SortKey::new(row.values.get(index)))
            .collect::<Vec<_>>();
        let mut order = (0..keys.len()).collect::<Vec<_>>();
        order.sort_by(|&left, &right| {
            let ordering = compare_cells(&keys[left], &keys[right]);
            match direction {
                OrderDirection::Ascending => ordering,
                OrderDirection::Descending => ordering.reverse(),
            }
        });
        let sorted = QueryResult {
            columns: source.columns.clone(),
            rows: order.iter().map(|&row| source.rows[row].clone()).collect(),
            rows_affected: source.rows_affected,
            truncated: source.truncated,
            elapsed_ms: source.elapsed_ms,
        };
        self.sort = Some((index, direction));
        self.unsorted = Some(source);
        self.replace_rows(Arc::new(sorted));
    }

    fn replace_rows(&mut self, result: Arc<QueryResult>) {
        self.accessible_revision = self.accessible_revision.wrapping_add(1);
        self.accessible_rows.clear();
        self.cell_text = vec![None; result.rows.len() * result.columns.len()];
        self.result = Some(result);
    }

    /// Right-clicking inside a marked range acts on that range; outside it
    /// acts on the clicked row without borrowing an unrelated selection.
    fn menu_rows(&self, clicked: usize) -> Vec<usize> {
        if self.changes.marked.contains(&clicked) {
            self.changes.marked.iter().copied().collect()
        } else {
            vec![clicked]
        }
    }

    fn rows_as_json(&self, rows: &[usize]) -> Option<String> {
        let result = self.result.as_ref()?;
        if rows.is_empty() {
            return None;
        }
        let mut names = std::collections::HashSet::new();
        if result
            .columns
            .iter()
            .any(|column| !names.insert(&column.name))
        {
            let selected = QueryResult {
                columns: result.columns.clone(),
                rows: rows
                    .iter()
                    .map(|row| result.rows.get(*row).cloned())
                    .collect::<Option<Vec<_>>>()?,
                rows_affected: None,
                truncated: false,
                elapsed_ms: result.elapsed_ms,
            };
            return Some(json_result(&selected));
        }
        let objects = rows
            .iter()
            .map(|row| self.row_as_json(*row))
            .collect::<Option<Vec<_>>>()?;
        Some(if objects.len() == 1 {
            objects[0].clone()
        } else {
            format!("[{}]", objects.join(","))
        })
    }

    fn rows_as_tsv(&self, rows: &[usize]) -> Option<String> {
        if rows.is_empty() {
            return None;
        }
        Some(
            rows.iter()
                .map(|row| self.row_as_tsv(*row))
                .collect::<Option<Vec<_>>>()?
                .join("\n"),
        )
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

    fn is_json_cell(&self, row_ix: usize, col_ix: usize) -> bool {
        let Some(column) = self.result_column(col_ix) else {
            return false;
        };
        if self
            .result
            .as_ref()
            .and_then(|result| result.columns.get(column))
            .is_some_and(|column| field_value_kind(column) == FieldValueKind::Json)
        {
            return true;
        }
        let staged = self
            .insert_row(row_ix)
            .and_then(|row| row.get(column))
            .and_then(Option::as_ref)
            .or_else(|| self.changes.edits.get(&(row_ix, column)));
        match staged {
            Some(MutationValue::Parameter(value)) => matches!(value, CellValue::Json(_)),
            Some(MutationValue::Expression(_)) => false,
            None => matches!(self.cell_value(row_ix, column), Some(CellValue::Json(_))),
        }
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
        self.cell_value(row_ix, data_column_ix)
            .map(|value| self.binary_format(data_column_ix).editor_text(value))
    }

    /// Render a single row as TSV, using quoted empty strings and a bare `NULL` sentinel so
    /// downstream consumers can distinguish database NULL from an empty text value.
    pub(super) fn row_as_tsv(&self, row_ix: usize) -> Option<String> {
        let row = self.row_values(row_ix)?;
        let values = row
            .iter()
            .enumerate()
            .map(|(column, value)| {
                if matches!(value, CellValue::Bytes(_)) {
                    CellValue::Text(self.binary_format(column).editor_text(value))
                } else {
                    value.clone()
                }
            })
            .collect::<Vec<_>>();
        Some(delimited_row(&values, '\t'))
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
            values
                .into_iter()
                .map(|value| {
                    if matches!(value, CellValue::Bytes(_)) {
                        CellValue::Text(self.binary_format(data_column_ix).editor_text(value))
                    } else {
                        value.clone()
                    }
                })
                .collect::<Vec<_>>()
                .iter(),
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
                        .then_some((row_index, position))
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

    pub(super) fn result_as_markdown(&self) -> Option<String> {
        self.result.as_deref().map(crate::result_export::markdown)
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

    fn data_column_key(index: usize, column: &ColumnInfo) -> String {
        // Query results may legally contain duplicate column names, so the ordinal is
        // part of the key. Humanity has already made SQL aliases difficult enough.
        format!("column:{index}:{}", column.name)
    }

    fn auto_width(result: &QueryResult, column_index: usize, column: &ColumnInfo) -> Pixels {
        let header_chars = column.name.chars().count();
        let value_chars = result
            .rows
            .iter()
            .take(AUTO_WIDTH_SAMPLE_ROWS)
            .filter_map(|row| row.values.get(column_index))
            .map(|value| cell_display_text(value).chars().count())
            .max()
            .unwrap_or_default();

        // This is an initial width, not a prison sentence. The user can resize it.
        crate::theme::text_px((header_chars as f32 * 7.0 + 20.0).clamp(80.0, 420.0)).max(
            crate::theme::row_text_px((value_chars as f32 * 7.0 + 20.0).clamp(80.0, 420.0)),
        )
    }

    pub(super) fn set_result(
        &mut self,
        result: Option<Arc<QueryResult>>,
        remembered_widths: &HashMap<String, Pixels>,
        layout: Option<&TableLayout>,
        foreign_keys: &[ForeignKeyInfo],
        tables: &[TableInfo],
    ) {
        // Layout changes can reuse the same result; a new source snapshot
        // invalidates ambiguous row identities while table primary keys survive.
        let same_source = match (&self.result, &result) {
            (Some(before), Some(after)) => Arc::ptr_eq(before, after),
            (None, None) => true,
            _ => false,
        };
        if !same_source {
            self.accessible_dataset_revision = self.accessible_dataset_revision.wrapping_add(1);
        }
        let mut columns = Vec::new();
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

        self.binary_formats = result
            .as_ref()
            .map(|result| {
                result
                    .columns
                    .iter()
                    .enumerate()
                    .filter_map(|(index, column)| {
                        Some((index, *layout?.binary_formats.get(&column.name)?))
                    })
                    .collect()
            })
            .unwrap_or_default();
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
                        .min_width(MIN_COLUMN_WIDTH * crate::theme::ui_scale())
                        .max_width(MAX_COLUMN_WIDTH * crate::theme::ui_scale())
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
        self.accessible_revision = self.accessible_revision.wrapping_add(1);
        self.accessible_rows.clear();
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

        if let Some(result) = self.result.as_deref() {
            for (position, &index) in self.order.iter().enumerate() {
                if let Some(width) = widths.get(position).copied() {
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
            .zip(self.columns.iter())
            .map(|(&index, column)| (result.columns[index].name.clone(), f32::from(column.width)))
            .collect()
    }

    /// Set one grid column's width, clamped to the column limits.
    fn set_column_width(&mut self, col_ix: usize, width: Pixels) -> bool {
        let Some(column) = self.columns.get_mut(col_ix) else {
            return false;
        };
        let width = width.clamp(
            crate::theme::ui_px(MIN_COLUMN_WIDTH),
            crate::theme::ui_px(MAX_COLUMN_WIDTH),
        );
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
        let header = column.name.chars().count() as f32 * 7.0 + 28.0;
        let values = result
            .rows
            .iter()
            .filter_map(|row| row.values.get(index))
            .map(|value| cell_display_text(value).chars().count())
            .max()
            .unwrap_or_default() as f32
            * 7.0
            + 20.0;
        Some(crate::theme::text_px(header).max(crate::theme::row_text_px(values)))
    }

    fn foreign_key_for_cell(&self, row_ix: usize, col_ix: usize) -> Option<ForeignKeyInfo> {
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
        if col_ix >= self.order.len() || to_ix >= self.order.len() {
            return;
        }
        let index = self.order.remove(col_ix);
        self.order.insert(to_ix, index);
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
            .when(!self.accessible_logical_tree, |row| {
                row.role(gpui::Role::Row).aria_row_index(0)
            })
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
        let column = index.and_then(|index| {
            let result_column = self.result.as_ref()?.columns.get(index)?;
            if self.is_data_result {
                self.table_columns
                    .iter()
                    .find(|column| column.name == result_column.name)
                    .or(Some(result_column))
            } else {
                Some(result_column)
            }
        });
        let numeric = index
            .and_then(|index| self.numeric.get(index).copied())
            .unwrap_or(false);
        let label = column.map_or_else(String::new, |column| column.name.clone());
        let key = self.columns[col_ix].key.clone();
        let sort = index.and_then(|index| self.column_sort(index));
        let author_id = format!("dbx-grid-{}-header-{key}", cx.entity_id());
        let position = format!("Column {} of {}. ", col_ix + 1, self.columns.len());
        let sort_description = match sort {
            Some(ColumnSort::Ascending) => ". Sorted ascending",
            Some(ColumnSort::Descending) => ". Sorted descending",
            _ => "",
        };
        let geometry = self.accessible_geometry.clone();
        let geometry_key = accessibility::GridNodeKey::Header(key.clone());
        let cell = div()
            .id(SharedString::from(format!("dbx-result-header-{key}")))
            .on_prepaint(move |bounds, window, _| {
                geometry
                    .borrow_mut()
                    .record(geometry_key.clone(), bounds, window)
            })
            .when(!self.accessible_logical_tree, |cell| {
                cell.role(gpui::Role::ColumnHeader)
            })
            .aria_label(label)
            .aria_column_index(col_ix)
            .aria_row_index(0)
            .aria_description(column.map_or_else(
                || position.clone(),
                |column| {
                    format!(
                        "{position}{}{}{sort_description}",
                        column.data_type,
                        if column.primary_key {
                            ", primary key"
                        } else {
                            ""
                        }
                    )
                },
            ))
            .a11y_synthetic_children(move |builder| {
                builder.parent_node().set_author_id(author_id.clone());
                if let Some(sort) = sort {
                    let direction = match sort {
                        ColumnSort::Ascending => Some(gpui::accesskit::SortDirection::Ascending),
                        ColumnSort::Descending => Some(gpui::accesskit::SortDirection::Descending),
                        ColumnSort::Default => None,
                    };
                    if let Some(direction) = direction {
                        builder.parent_node().set_sort_direction(direction);
                    }
                }
            })
            .when(!self.accessible_logical_tree && sort.is_some(), |cell| {
                let table = cx.entity();
                let key = key.clone();
                cell.on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                    table.update(cx, |table, cx| {
                        let delegate = table.delegate_mut();
                        let Some(column) =
                            delegate.columns.iter().position(|column| column.key == key)
                        else {
                            return;
                        };
                        let next = match delegate.columns[column].sort {
                            Some(ColumnSort::Ascending) => ColumnSort::Descending,
                            Some(ColumnSort::Descending) => ColumnSort::Default,
                            _ => ColumnSort::Ascending,
                        };
                        delegate.perform_sort(column, next, window, cx);
                        table.refresh(cx);
                    });
                })
            })
            .size_full()
            .flex()
            .items_center()
            .gap(crate::theme::ui_px(5.))
            .px(crate::theme::ui_px(8.))
            .overflow_hidden()
            .when(numeric, |cell| cell.justify_end());
        let Some(column) = column else {
            return cell
                .text_size(crate::theme::text_px(10.))
                .text_color(theme().text_muted)
                .child(self.columns[col_ix].name.clone())
                .into_any_element();
        };
        let resizing = self.resizing == Some(col_ix);
        let table = cx.entity().entity_id();
        // The table draws a sort icon and its cell padding after this header,
        // so reach past them to sit the handle on the column's real edge.
        let trailing = HEADER_TRAILING_PADDING
            + if self.sorting == ResultSorting::Disabled {
                0.
            } else {
                SORT_ICON_WIDTH * crate::theme::ui_scale() + 4.
            };
        let content = cell
            .tooltip(crate::theme::tip(format!(
                "{} · {}",
                column.name, column.data_type
            )))
            .when(column.primary_key, |cell| {
                cell.child(
                    div()
                        .flex_none()
                        .text_size(crate::theme::text_px(8.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme().warning_text)
                        .child("PK"),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::theme::text_px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme().text)
                    .child(column.name.clone()),
            );
        // The content clips its labels; the handle sits outside that clip.
        div()
            .h_full()
            .flex_1()
            .min_w_0()
            .relative()
            .child(content)
            // A generous grab zone on the right edge. The table's own handle is
            // a 2px sliver that is nearly impossible to find.
            .child(
                div()
                    .id(("dbx-column-resize", col_ix))
                    .debug_selector(move || format!("dbx-column-resize-{col_ix}"))
                    .group("dbx-column-resize")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-trailing))
                    .w(crate::theme::ui_px(RESIZE_HANDLE_WIDTH))
                    .flex()
                    .justify_end()
                    .cursor_col_resize()
                    .occlude()
                    .child(
                        div()
                            .h_full()
                            .w(crate::theme::ui_px(2.))
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
            .into_any_element()
    }

    fn row_hover_enabled(&self, row_ix: usize, _cx: &App) -> bool {
        !(self.changes.marked.len() > 1 && self.changes.marked.contains(&row_ix))
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let base = if row_ix.is_multiple_of(2) {
            theme().canvas
        } else {
            theme().grid_alternate
        };
        let background = if self.changes.marked.contains(&row_ix) && self.changes.marked.len() > 1 {
            theme().grid_selected
        } else if self.changes.deletes.contains(&row_ix) {
            theme().danger.alpha(0.12)
        } else if self.insert_row(row_ix).is_some() {
            theme().success.alpha(0.12)
        } else {
            base
        };
        let row_key = self.accessible_row_key(row_ix);
        let author_id = format!("dbx-grid-{}-row-{row_key}", cx.entity_id());
        let row_index_text = (self.row_offset + row_ix + 1).to_string();
        let row = div()
            .id(SharedString::from(format!("dbx-result-row-{row_key}")))
            .a11y_synthetic_children(move |builder| {
                builder.parent_node().set_author_id(author_id.clone());
                builder
                    .parent_node()
                    .set_row_index_text(row_index_text.clone());
            })
            .when(!self.accessible_logical_tree, |row| {
                row.role(gpui::Role::Row).aria_row_index(row_ix + 1)
            })
            // A named row wrapper replaces its cells in Orca's full-row speech.
            // The row-header cell supplies the absolute row name.
            .aria_selected(
                self.accessible_selected_row == Some(row_ix)
                    || self.changes.marked.contains(&row_ix),
            )
            .when(
                !self.accessible_logical_tree
                    && self.accessible_focused
                    && self.accessible_selected_cell.is_none()
                    && self.accessible_selected_row == Some(row_ix),
                |row| row.aria_active_descendant(),
            )
            .debug_selector(move || format!("dbx-result-row-{row_ix}"))
            .border_color(theme().border)
            .bg(background)
            // Cell handlers stop propagation and supply their column. A click
            // on the row gutter must clear the previous menu's column instead.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|table, _, _, _| {
                    table.delegate_mut().context_column = None;
                }),
            );
        let Some((app, session_id, tab_id, true)) = self.row_actions.clone() else {
            return row;
        };
        // Cell clicks select through the table; this only reads the modifiers
        // so Shift extends and Cmd/Ctrl toggles a multi-row mark.
        let drag_app = app.clone();
        row.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            let (extend, toggle) = (event.modifiers.shift, event.modifiers.secondary());
            let app = app.clone();
            cx.defer(move |cx| {
                let _ = app.update(cx, |this, cx| {
                    this.row_drag = None;
                    if this.drag_row_selection
                        && this.data_tab(session_id, tab_id).is_some_and(|data| {
                            data.row_draft.is_none() && data.cell_editor.is_none()
                        })
                    {
                        this.row_drag = Some((session_id, tab_id, row_ix));
                    }
                    this.mark_row_for(session_id, tab_id, row_ix, extend, toggle, cx);
                });
            });
        })
        .on_mouse_move(move |event, _, cx| {
            let _ = drag_app.update(cx, |this, cx| {
                if !event.dragging() {
                    this.row_drag = None;
                    return;
                }
                if !this.drag_row_selection {
                    return;
                }
                if let Some((session, tab, last_row)) = this.row_drag
                    && session == session_id
                    && tab == tab_id
                    && last_row != row_ix
                {
                    this.mark_row_for(session_id, tab_id, row_ix, true, false, cx);
                    this.row_drag = Some((session_id, tab_id, row_ix));
                }
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
                for (column_ix, column) in self.columns.iter_mut().enumerate() {
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
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let mut menu = menu;
        let rows = self.menu_rows(row_ix);
        let multiple = rows.len() > 1;
        for (label, text) in [
            ("Copy as JSON", self.rows_as_json(&rows)),
            ("Copy as TSV", self.rows_as_tsv(&rows)),
            (
                "Copy as Markdown",
                self.result.as_ref().and_then(|result| {
                    let selected = QueryResult {
                        columns: result.columns.clone(),
                        rows: rows
                            .iter()
                            .map(|row| result.rows.get(*row).cloned())
                            .collect::<Option<Vec<_>>>()?,
                        rows_affected: None,
                        truncated: false,
                        elapsed_ms: result.elapsed_ms,
                    };
                    Some(crate::result_export::markdown(&selected))
                }),
            ),
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
            if let Some(column) = self.context_column
                && matches!(self.cell_value(row_ix, column), Some(CellValue::Bytes(_)))
            {
                use super::value_view::BinaryFormat;
                for (label, format) in [
                    ("View as binary", BinaryFormat::Binary),
                    ("View as hex", BinaryFormat::Hex),
                    ("View as UUID", BinaryFormat::Uuid),
                    ("View as UUID (MySQL swapped)", BinaryFormat::UuidSwapped),
                ] {
                    let table = cx.entity().downgrade();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(self.binary_format(column) == format)
                            .on_click(move |_, _, cx| {
                                let _ = table.update(cx, |table, cx| {
                                    let delegate = table.delegate_mut();
                                    delegate.binary_formats.insert(column, format);
                                    delegate.cell_text.fill(None);
                                    cx.notify();
                                });
                            }),
                    );
                }
            }
            return menu;
        }
        let supports_full_export = app.upgrade().is_some_and(|app| {
            app.read(cx).session(session_id).is_some_and(|session| {
                session
                    .engine
                    .as_ref()
                    .is_some_and(|engine| engine.capabilities().streaming_query_export)
            })
        });
        let supports_sql = app.upgrade().is_some_and(|app| {
            app.read(cx)
                .session(session_id)
                .is_some_and(|session| session.kind.is_sql())
        });
        let full_export = app.upgrade().is_some_and(|app| {
            app.read(cx).session(session_id).is_some_and(|session| {
                !session.busy
                    && session
                        .engine
                        .as_ref()
                        .is_some_and(|engine| engine.capabilities().streaming_query_export)
            })
        });
        let has_result = self.result.is_some();
        let copy_app = app.clone();
        let export_app = app.clone();
        menu = menu
            .separator()
            .label("Loaded rows · staged edits excluded")
            .item(
                PopupMenuItem::new("Copy all loaded rows as Markdown")
                    .disabled(!has_result)
                    .on_click(move |_, _, cx| {
                        let app = copy_app.clone();
                        cx.defer(move |cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.copy_data_markdown_for(session_id, tab_id, cx);
                            });
                        });
                    }),
            )
            .item(PopupMenuItem::submenu(
                "Export XLSX…",
                PopupMenu::build(window, cx, move |menu, _, _| {
                    let loaded = export_app.clone();
                    let full = export_app.clone();
                    menu.label("Loaded rows · staged edits excluded")
                        .item(
                            PopupMenuItem::new("Loaded rows only…")
                                .disabled(!has_result)
                                .on_click(move |_, _, cx| {
                                    let app = loaded.clone();
                                    cx.defer(move |cx| {
                                        let _ = app.update(cx, |this, cx| {
                                            this.export_data_xlsx_for(session_id, tab_id, cx);
                                        });
                                    });
                                }),
                        )
                        .when(supports_full_export, |menu| {
                            menu.separator()
                                .label("Full table · filters and staged edits excluded")
                                .item(
                                    PopupMenuItem::new("All table rows…")
                                        .disabled(!full_export)
                                        .on_click(move |_, _, cx| {
                                            let app = full.clone();
                                            cx.defer(move |cx| {
                                                let _ = app.update(cx, |this, cx| {
                                                    this.export_full_table_xlsx_for(
                                                        session_id, tab_id, cx,
                                                    );
                                                });
                                            });
                                        }),
                                )
                        })
                }),
            ))
            .separator();
        let sql = app.upgrade().and_then(|app| {
            let owner = app.read(cx);
            let kind = owner.session(session_id)?.kind;
            if !kind.is_sql() {
                return None;
            }
            let table = &owner.data_tab(session_id, tab_id)?.table;
            let result = self.result.as_ref()?;
            let columns = result
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            rows.iter()
                .map(|row| {
                    let row = result.rows.get(*row)?;
                    dbx_core::render_sql_insert(kind, table, &columns, &row.values)
                        .ok()
                        .map(|sql| format!("{sql};"))
                })
                .collect::<Option<Vec<_>>>()
                .map(|statements| statements.join("\n"))
        });
        menu = menu.when(supports_sql, |menu| {
            menu.item(
                PopupMenuItem::new("Copy as SQL")
                    .disabled(sql.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(sql) = &sql {
                            cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()));
                        }
                    }),
            )
            .separator()
        });
        if let Some(column) = self.context_column
            && row_ix < self.loaded_rows()
        {
            menu = self.column_menu(menu, &app, session_id, tab_id, (row_ix, column), cx);
        }
        let result_snapshot = self.result.clone();
        let insert_snapshot = self.changes.inserts.clone();
        let can_edit = app.upgrade().is_some_and(|app| {
            let owner = app.read(cx);
            owner.editable_table_for(session_id, tab_id).is_some()
                && owner.data_tab(session_id, tab_id).is_some_and(|data| {
                    !data.busy
                        && data.row_draft.is_none()
                        && data.cell_editor.is_none()
                        && data.recovered_changeset.is_none()
                })
        });
        let supports_edits = app
            .upgrade()
            .is_some_and(|app| app.read(cx).driver_supports_row_edits(session_id));
        for (label, redo) in [("Undo staged edit", false), ("Redo staged edit", true)]
            .into_iter()
            .filter(|_| supports_edits)
        {
            let enabled = can_edit
                && app.upgrade().is_some_and(|app| {
                    app.read(cx)
                        .data_tab(session_id, tab_id)
                        .is_some_and(|data| {
                            if redo {
                                data.edit_history.can_redo()
                            } else {
                                data.edit_history.can_undo()
                            }
                        })
                });
            let app = app.clone();
            menu = menu.item(PopupMenuItem::new(label).disabled(!enabled).on_click(
                move |_, _, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.undo_grid_edits_for(session_id, tab_id, redo, cx)
                    });
                },
            ));
        }
        let is_new = rows.iter().all(|row| self.insert_row(*row).is_some());
        let has_changes = rows.iter().any(|row| {
            self.insert_row(*row).is_some()
                || self.changes.deletes.contains(row)
                || self.changes.edits.keys().any(|(edited, _)| edited == row)
        });
        let restoring = rows.iter().all(|row| self.changes.deletes.contains(row));
        let count_label = |verb: &str| -> SharedString {
            if multiple {
                format!("{verb} {} rows", rows.len()).into()
            } else {
                format!("{verb} row").into()
            }
        };
        let delete_label = count_label(if restoring { "Restore" } else { "Delete" });
        for (label, action, enabled) in [
            ("Inspect row".into(), RowAction::Inspect, !multiple),
            ("Edit row".into(), RowAction::Edit, can_edit && !multiple),
            (count_label("Duplicate"), RowAction::Duplicate, can_edit),
            (delete_label, RowAction::Delete, can_edit),
            (
                if multiple {
                    format!("Revert changes to {} rows", rows.len()).into()
                } else if is_new {
                    "Remove new row".into()
                } else {
                    "Revert changes".into()
                },
                RowAction::Revert,
                can_edit && has_changes,
            ),
        ] {
            if !supports_edits && !matches!(action, RowAction::Inspect) {
                continue;
            }
            let app = app.clone();
            let rows = rows.clone();
            let result_snapshot = result_snapshot.clone();
            let insert_snapshot = insert_snapshot.clone();
            menu = menu.item(PopupMenuItem::new(label).disabled(!enabled).on_click(
                move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        // A refresh or insert removal invalidates row positions while
                        // the menu is open. Never apply a saved menu to different rows.
                        let valid = this.data_tab(session_id, tab_id).is_some_and(|data| {
                            let same_result = match (&data.result, &result_snapshot) {
                                (Some(current), Some(snapshot)) => Arc::ptr_eq(current, snapshot),
                                _ => false,
                            };
                            same_result
                                && data.data_grid.read(cx).delegate().changes.inserts
                                    == insert_snapshot
                                && !data.busy
                                && data.row_draft.is_none()
                                && data.cell_editor.is_none()
                                && data.recovered_changeset.is_none()
                        });
                        if !valid {
                            return;
                        }
                        match action {
                            RowAction::Delete => {
                                this.delete_explicit_rows_for(session_id, tab_id, rows.clone(), cx);
                                return;
                            }
                            RowAction::Duplicate => {
                                this.duplicate_rows_for(session_id, tab_id, &rows, cx);
                                return;
                            }
                            RowAction::Revert => {
                                this.revert_rows_for(session_id, tab_id, &rows, cx);
                                return;
                            }
                            _ => {}
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
                            RowAction::Delete
                            | RowAction::Duplicate
                            | RowAction::Revert
                            | RowAction::Inspect => {}
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
        let json = self.is_json_cell(row_ix, col_ix);
        let semantic_cell = self
            .accessible_cell(row_ix, col_ix, cx)
            .font_family(crate::theme::row_font_family());
        if let Some((_, _, editor)) = self.editing.as_ref().filter(|(row, column, _)| {
            *row == row_ix && Some(*column) == self.result_column(col_ix)
        }) {
            let focus = editor.read(cx).focus_handle();
            // Size the input to its text line and center it just like the
            // display cell. Its border plus padding keeps the same 8px inset.
            return semantic_cell
                .size_full()
                .flex()
                .items_center()
                .child(
                    crate::editor::input_with_key_context(
                        editor.clone(),
                        focus,
                        false,
                        super::cell_edits::CELL_EDITOR_CONTEXT,
                        cx,
                    )
                    .h_auto()
                    .py(crate::theme::ui_px(0.))
                    .px(crate::theme::ui_px(7.))
                    .text_size(crate::theme::row_text_px(11.)),
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
            let text = data_column.and_then(|column| {
                insert
                    .get(column)
                    .cloned()
                    .flatten()
                    .map(|value| match &value {
                        MutationValue::Parameter(value) => self.formatted_value(column, value),
                        _ => staged_text(&value),
                    })
            });
            let muted = text.is_none();
            return semantic_cell
                .size_full()
                .flex()
                .items_center()
                .px(crate::theme::ui_px(8.))
                .text_size(crate::theme::row_text_px(11.))
                .text_color(if muted {
                    theme().text_muted
                } else {
                    theme().text
                })
                .when(muted, |cell| cell.italic())
                .child(div().min_w_0().truncate().child(rendered_cell_text(
                    text.unwrap_or_else(|| "DEFAULT".into()).into(),
                    json && !muted,
                )))
                .on_mouse_down(MouseButton::Right, remember_column)
                .into_any_element();
        }
        let staged = data_column
            .and_then(|column| self.changes.edits.get(&(row_ix, column)))
            .cloned();
        if let Some(value) = staged.filter(|_| !deleted) {
            let muted =
                !matches!(&value, MutationValue::Parameter(value) if *value != CellValue::Null);
            return semantic_cell
                .size_full()
                .flex()
                .items_center()
                .px(crate::theme::ui_px(8.))
                .bg(theme().warning.alpha(0.16))
                .text_size(crate::theme::row_text_px(11.))
                .text_color(if muted {
                    theme().text_muted
                } else {
                    theme().text
                })
                .when(muted, |cell| cell.italic())
                .child(
                    div().min_w_0().truncate().child(rendered_cell_text(
                        match &value {
                            MutationValue::Parameter(value) => {
                                self.formatted_value(data_column.unwrap(), value)
                            }
                            _ => staged_text(&value),
                        }
                        .into(),
                        json && !muted,
                    )),
                )
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
                    Some(theme().text_muted)
                }
                Some(CellValue::Boolean(true)) => Some(theme().success_text),
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
                                .map(|value| self.formatted_value(data_column, value))
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
            (SharedString::default(), theme().text_muted)
        };
        let foreign_key = self.foreign_key_for_cell(row_ix, col_ix);
        let numeric = data_column
            .and_then(|index| self.numeric.get(index).copied())
            .unwrap_or(false);

        let mut cell = semantic_cell
            .size_full()
            .on_mouse_down(MouseButton::Right, remember_column)
            .flex()
            .items_center()
            .px(crate::theme::ui_px(8.))
            .whitespace_nowrap()
            .truncate()
            .text_size(crate::theme::row_text_px(11.))
            .text_color(if deleted {
                theme().text_muted
            } else {
                text_color
            })
            .when(null, |cell| cell.italic())
            .when(deleted, |cell| cell.line_through())
            .when(numeric && foreign_key.is_none(), |cell| cell.justify_end());
        if foreign_key.is_some() {
            cell = cell
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .when(numeric, |value| value.justify_end())
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(rendered_cell_text(text, json && !null && !deleted)),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "foreign-key-link-{row_ix}-{col_ix}"
                    )))
                    .with_size(gpui_component::Size::XSmall)
                    .compact()
                    .ghost()
                    .accessible_name("Open referenced row")
                    .tooltip("Open referenced row")
                    .text_color(theme().accent_text)
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
                    .debug_selector(move || format!("result-cell-text-{row_ix}-{col_ix}"))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(numeric, |value| value.text_right())
                    .child(rendered_cell_text(text, json && !null && !deleted)),
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

    #[test]
    fn binary_clipboard_is_full_and_follows_column_representation() {
        let mut delegate = ResultTableDelegate::default();
        let bytes = CellValue::Bytes(vec![0xff; CELL_TEXT_LIMIT * 2]);
        let text = "long text ".repeat(CELL_TEXT_LIMIT);
        delegate.set_result(
            Some(Arc::new(QueryResult {
                columns: vec![
                    ColumnInfo::result("bytes", 0, "BLOB"),
                    ColumnInfo::result("text", 1, "TEXT"),
                ],
                rows: vec![RowData::new(vec![
                    bytes.clone(),
                    CellValue::Text(text.clone()),
                ])],
                ..QueryResult::empty(None, 0)
            })),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        assert_eq!(delegate.cell_as_plain_text(0, 0), Some(bytes.to_string()));
        assert_eq!(delegate.cell_as_plain_text(0, 1), Some(text));
        assert!(delegate.formatted_value(0, &bytes).contains('…'));
        delegate
            .binary_formats
            .insert(0, super::super::value_view::BinaryFormat::Binary);
        assert!(
            delegate
                .row_as_tsv(0)
                .unwrap()
                .starts_with("11111111 11111111")
        );
        assert!(!delegate.cell_as_plain_text(0, 0).unwrap().contains('…'));
        let invalid = CellValue::Bytes(vec![0; 1024]);
        assert!(
            super::super::value_view::BinaryFormat::Uuid
                .limited_text(&invalid, 4)
                .contains("1024 bytes")
        );
    }

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
            accessible_grid(&self.grid, "Test results", false)
        }
    }

    #[test]
    fn menu_exports_respect_selected_rows_and_never_drop_invalid_rows() {
        let mut delegate = delegate_with_export_result();
        delegate.changes.marked = BTreeSet::from([0, 1]);
        assert_eq!(delegate.menu_rows(1), vec![0, 1]);
        assert_eq!(delegate.menu_rows(2), vec![2]);
        let rows = delegate.menu_rows(0);
        let json: serde_json::Value =
            serde_json::from_str(&delegate.rows_as_json(&rows).unwrap()).unwrap();
        assert_eq!(json["rows"].as_array().unwrap().len(), 2);
        assert_eq!(json["rows"][0][0], 7);
        assert_eq!(json["rows"][1][0], 8);
        assert_eq!(
            delegate.rows_as_tsv(&rows).unwrap(),
            format!(
                "{}\n{}",
                delegate.row_as_tsv(0).unwrap(),
                delegate.row_as_tsv(1).unwrap()
            )
        );
        assert!(delegate.rows_as_json(&[0, 100]).is_none());
        assert!(delegate.rows_as_tsv(&[0, 100]).is_none());
        // Unique column names use an array of ordinary row objects.
        let result = Arc::make_mut(delegate.result.as_mut().unwrap());
        result.columns[2].name = "other_note".into();
        let json: serde_json::Value =
            serde_json::from_str(&delegate.rows_as_json(&rows).unwrap()).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 2);
        assert_eq!(json[0]["id"], 7);
        assert!(json[0]["note"].is_null());
        assert_eq!(json[1]["other_note"], "NULL");
    }

    #[gpui::test]
    fn right_click_inside_selection_copies_all_marked_rows(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (_, cx) = cx.add_window_view(|window, cx| RowMenuHarness {
            grid: cx.new(|cx| {
                let mut delegate = delegate_with_export_result();
                delegate.changes.marked = BTreeSet::from([0, 1]);
                TableState::new(delegate, window, cx)
            }),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let row = cx.debug_bounds("dbx-result-row-1").unwrap();
        cx.simulate_mouse_down(row.center(), MouseButton::Right, Default::default());
        cx.simulate_mouse_up(row.center(), MouseButton::Right, Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("down enter");
        let json: serde_json::Value = serde_json::from_str(
            &cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        )
        .unwrap();
        assert_eq!(json["rows"].as_array().unwrap().len(), 2);
        assert_eq!(json["rows"][0][0], 7);
        assert_eq!(json["rows"][1][0], 8);
    }
}
