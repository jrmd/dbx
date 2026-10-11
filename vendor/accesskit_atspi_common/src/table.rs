// DBX local extension of AccessKit's AT-SPI adapter.
// Licensed under MIT OR Apache-2.0, like the surrounding adapter.

use super::{NodeWrapper, PlatformNode};
use crate::{Error, Result, filters::filter};
use accesskit::Role;
use accesskit_consumer::{FilterResult, Node, NodeId};

pub(super) fn is_table(role: Role) -> bool {
    matches!(role, Role::Table | Role::Grid | Role::TreeGrid)
}

fn is_cell(role: Role) -> bool {
    matches!(
        role,
        Role::Cell | Role::GridCell | Role::RowHeader | Role::ColumnHeader
    )
}

fn containing_table<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    containing_table_with_filter(node, &filter)
}

fn containing_table_with_filter<'a>(
    node: &Node<'a>,
    node_filter: &impl Fn(&Node<'_>) -> FilterResult,
) -> Option<Node<'a>> {
    let mut parent = node.parent();
    while let Some(node) = parent {
        if node_filter(&node) == FilterResult::ExcludeSubtree {
            return None;
        }
        if is_table(node.role()) {
            return Some(node);
        }
        parent = node.parent();
    }
    None
}

pub(super) fn supports_table_cell(
    node: &Node<'_>,
    node_filter: &impl Fn(&Node<'_>) -> FilterResult,
) -> bool {
    is_cell(node.role())
        && node_filter(node) != FilterResult::ExcludeSubtree
        && containing_table_with_filter(node, node_filter).is_some()
        && cell_position(node).is_some()
}

fn cell_position(node: &Node<'_>) -> Option<(usize, usize)> {
    let column = node.data().column_index()?;
    let mut row = node.data().row_index();
    let mut parent = node.parent();
    while row.is_none() {
        let node = parent?;
        if is_table(node.role()) {
            break;
        }
        if node.role() == Role::Row {
            row = node.data().row_index();
        }
        parent = node.parent();
    }
    Some((row?, column))
}

#[derive(Clone, Debug)]
struct Cell {
    id: NodeId,
    role: Role,
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
    selected: bool,
    name: String,
}

impl Cell {
    fn from_node(node: &Node<'_>) -> Option<Self> {
        let (row, column) = cell_position(node)?;
        let row_span = node.data().row_span().unwrap_or(1).max(1);
        let column_span = node.data().column_span().unwrap_or(1).max(1);
        // Do not wrap malformed producer coordinates or advertise negative extents.
        let row_end = row.checked_add(row_span)?;
        let column_end = column.checked_add(column_span)?;
        i32::try_from(row_end).ok()?;
        i32::try_from(column_end).ok()?;
        Some(Self {
            id: node.id(),
            role: node.role(),
            row,
            column,
            row_span,
            column_span,
            selected: node.is_selected().unwrap_or(false),
            name: NodeWrapper(node).name().unwrap_or_default(),
        })
    }

    fn covers(&self, row: usize, column: usize) -> bool {
        (self.row..self.row + self.row_span).contains(&row)
            && (self.column..self.column + self.column_span).contains(&column)
    }
}

/// A snapshot of actual exposed cells, rebuilt under the consumer tree read lock.
/// Missing virtualized cells stay missing; this adapter never invents accessibles.
pub(crate) struct Table {
    id: NodeId,
    rows: usize,
    columns: usize,
    cells: Vec<Cell>,
    children: Vec<NodeId>,
    selected_rows: Vec<(usize, usize)>,
}

#[cfg(test)]
thread_local! {
    static SNAPSHOTS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Table {
    fn cached(node: &Node<'_>, context: &crate::context::Context) -> Result<std::sync::Arc<Self>> {
        let mut cache = context.table_cache.lock().unwrap();
        if let Some(table) = cache.get(&node.id()) {
            return Ok(table.clone());
        }
        let table = std::sync::Arc::new(Self::new(node)?);
        cache.insert(node.id(), table.clone());
        Ok(table)
    }

    fn new(node: &Node<'_>) -> Result<Self> {
        if !is_table(node.role()) {
            return Err(Error::UnsupportedInterface);
        }
        #[cfg(test)]
        SNAPSHOTS_BUILT.with(|count| count.set(count.get() + 1));
        let mut cells = Vec::new();
        let mut selected_rows = Vec::new();
        let children = node
            .filtered_children(&filter)
            .map(|child| child.id())
            .collect();
        let mut pending: Vec<_> = node.filtered_children(&filter).rev().collect();
        while let Some(child) = pending.pop() {
            // Nested tables belong to their own interface, not the enclosing table.
            if is_table(child.role()) {
                continue;
            }
            if child.role() == Role::Row && child.is_selected() == Some(true) {
                if let Some(row) = child.data().row_index() {
                    let span = child.data().row_span().unwrap_or(1).max(1);
                    if let Some(end) = row
                        .checked_add(span)
                        .filter(|end| i32::try_from(*end).is_ok())
                    {
                        selected_rows.push((row, end));
                    }
                }
            }
            if is_cell(child.role()) {
                if let Some(cell) = Cell::from_node(&child) {
                    cells.push(cell);
                }
                // Embedded controls/editors must not become additional cells.
                continue;
            }
            pending.extend(child.filtered_children(&filter).rev());
        }
        let rows = node.data().row_count().unwrap_or_else(|| {
            cells
                .iter()
                .map(|cell| cell.row + cell.row_span)
                .max()
                .unwrap_or(0)
        });
        let columns = node.data().column_count().unwrap_or_else(|| {
            cells
                .iter()
                .map(|cell| cell.column + cell.column_span)
                .max()
                .unwrap_or(0)
        });
        i32::try_from(rows).map_err(|_| Error::TooManyChildren)?;
        i32::try_from(columns).map_err(|_| Error::TooManyChildren)?;
        cells.retain(|cell| {
            cell.row + cell.row_span <= rows && cell.column + cell.column_span <= columns
        });
        selected_rows.retain(|(_, end)| *end <= rows);
        for cell in &mut cells {
            cell.selected |= selected_rows
                .iter()
                .any(|(start, end)| *start <= cell.row && cell.row + cell.row_span <= *end);
        }
        Ok(Self {
            id: node.id(),
            rows,
            columns,
            cells,
            children,
            selected_rows,
        })
    }

    fn coordinate(&self, row: i32, column: i32) -> Result<(usize, usize)> {
        let row = usize::try_from(row).map_err(|_| Error::IndexOutOfRange)?;
        let column = usize::try_from(column).map_err(|_| Error::IndexOutOfRange)?;
        if row >= self.rows || column >= self.columns {
            return Err(Error::IndexOutOfRange);
        }
        Ok((row, column))
    }

    fn axis(&self, index: i32, rows: bool) -> Result<usize> {
        let index = usize::try_from(index).map_err(|_| Error::IndexOutOfRange)?;
        if index >= if rows { self.rows } else { self.columns } {
            return Err(Error::IndexOutOfRange);
        }
        Ok(index)
    }

    fn at(&self, row: usize, column: usize) -> Option<&Cell> {
        self.cells.iter().find(|cell| cell.covers(row, column))
    }

    fn at_child_index(&self, index: i32) -> Option<&Cell> {
        let id = self.children.get(usize::try_from(index).ok()?)?;
        self.cells.iter().find(|cell| &cell.id == id)
    }

    fn headers(&self, index: usize, rows: bool) -> impl Iterator<Item = &Cell> {
        self.cells.iter().filter(move |cell| {
            if rows {
                cell.role == Role::RowHeader
                    && (cell.row..cell.row + cell.row_span).contains(&index)
            } else {
                cell.role == Role::ColumnHeader
                    && (cell.column..cell.column + cell.column_span).contains(&index)
            }
        })
    }

    fn selected_axis(&self, index: usize, rows: bool) -> bool {
        // Partial cell selection is not full row/column selection. Require actual
        // selected cells to cover the entire advertised axis, including spans.
        if rows
            && self
                .selected_rows
                .iter()
                .any(|(start, end)| (*start..*end).contains(&index))
        {
            return true;
        }
        let count = if rows { self.columns } else { self.rows };
        if count == 0 {
            return false;
        }
        let mut spans: Vec<_> = self
            .cells
            .iter()
            .filter_map(|cell| {
                let (position, span, start, length) = if rows {
                    (cell.row, cell.row_span, cell.column, cell.column_span)
                } else {
                    (cell.column, cell.column_span, cell.row, cell.row_span)
                };
                (cell.selected && (position..position + span).contains(&index))
                    .then_some((start, start + length))
            })
            .collect();
        if !rows {
            spans.extend(self.selected_rows.iter().copied());
        }
        spans.sort_unstable();
        let mut end = 0;
        for (start, next_end) in spans {
            if start > end {
                return false;
            }
            end = end.max(next_end);
        }
        end >= count
    }

    fn selected_axes(&self, rows: bool) -> Result<Vec<i32>> {
        // Candidate axes come from real cells, avoiding iteration over huge
        // declared dimensions on a sparse/virtualized table.
        // The DBus array must fit bounded memory even for malformed span metadata.
        const MAX_SELECTED_AXES: usize = 100_000;
        if !rows {
            let mut intervals = self.selected_rows.clone();
            intervals.sort_unstable();
            let mut end = 0;
            for (start, next_end) in intervals {
                if start > end {
                    break;
                }
                end = end.max(next_end);
            }
            if self.rows > 0 && end >= self.rows {
                if self.columns > MAX_SELECTED_AXES {
                    return Err(Error::TooManyChildren);
                }
                return Ok((0..self.columns as i32).collect());
            }
        }
        let candidates = self
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .try_fold(0usize, |count, cell| {
                count.checked_add(if rows {
                    cell.row_span
                } else {
                    cell.column_span
                })
            })
            .ok_or(Error::TooManyChildren)?;
        let candidates = if rows {
            self.selected_rows
                .iter()
                .try_fold(candidates, |count, (start, end)| {
                    count.checked_add(end - start)
                })
                .ok_or(Error::TooManyChildren)?
        } else {
            candidates
        };
        if candidates > MAX_SELECTED_AXES {
            return Err(Error::TooManyChildren);
        }
        let mut indices: Vec<_> = self
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .flat_map(|cell| {
                if rows {
                    cell.row..cell.row + cell.row_span
                } else {
                    cell.column..cell.column + cell.column_span
                }
            })
            .collect();
        if rows {
            indices.extend(
                self.selected_rows
                    .iter()
                    .flat_map(|(start, end)| *start..*end),
            );
        }
        indices.sort_unstable();
        indices.dedup();
        Ok(indices
            .into_iter()
            .filter(|index| self.selected_axis(*index, rows))
            .map(|index| index as i32)
            .collect())
    }
}

/// Native cell coordinates and extents in zero-based AT-SPI table space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCellInfo {
    pub row: i32,
    pub column: i32,
    pub row_span: i32,
    pub column_span: i32,
}

impl From<&Cell> for TableCellInfo {
    fn from(cell: &Cell) -> Self {
        Self {
            row: cell.row as i32,
            column: cell.column as i32,
            row_span: cell.row_span as i32,
            column_span: cell.column_span as i32,
        }
    }
}

impl PlatformNode {
    fn resolve_table<T>(&self, f: impl FnOnce(&Table) -> Result<T>) -> Result<T> {
        self.resolve_with_context(|node, _, context| f(Table::cached(&node, context)?.as_ref()))
    }

    fn resolve_table_cell<T>(&self, f: impl FnOnce(&Table, &Cell) -> Result<T>) -> Result<T> {
        self.resolve_with_context(|node, _, context| {
            if !supports_table_cell(&node, &filter) {
                return Err(Error::UnsupportedInterface);
            }
            let table = Table::cached(
                &containing_table(&node).ok_or(Error::UnsupportedInterface)?,
                context,
            )?;
            let cell = table
                .cells
                .iter()
                .find(|cell| cell.id == node.id())
                .ok_or(Error::IndexOutOfRange)?;
            f(&table, cell)
        })
    }

    pub fn table_dimensions(&self) -> Result<(i32, i32)> {
        self.resolve_table(|table| Ok((table.rows as i32, table.columns as i32)))
    }

    pub fn table_cell_at(&self, row: i32, column: i32) -> Result<Option<NodeId>> {
        self.resolve_table(|table| {
            let (row, column) = table.coordinate(row, column)?;
            Ok(table.at(row, column).map(|cell| cell.id))
        })
    }

    pub fn table_index_at(&self, row: i32, column: i32) -> Result<i32> {
        self.resolve_table(|table| {
            let (row, column) = table.coordinate(row, column)?;
            // The legacy API promises an index usable by Accessible.GetChildAtIndex.
            // Nested row cells have no such index; return -1 rather than a fabricated flat offset.
            Ok(table
                .at(row, column)
                .and_then(|cell| table.children.iter().position(|id| *id == cell.id))
                .map(|index| index as i32)
                .unwrap_or(-1))
        })
    }

    pub fn table_cell_at_index(&self, index: i32) -> Result<Option<(TableCellInfo, bool)>> {
        self.resolve_table(|table| {
            Ok(table
                .at_child_index(index)
                .map(|cell| (cell.into(), cell.selected)))
        })
    }

    pub fn table_cell_extent_at(&self, row: i32, column: i32, rows: bool) -> Result<i32> {
        self.resolve_table(|table| {
            let (row, column) = table.coordinate(row, column)?;
            Ok(table
                .at(row, column)
                .map(|cell| {
                    if rows {
                        cell.row_span
                    } else {
                        cell.column_span
                    }
                })
                .unwrap_or(0) as i32)
        })
    }

    pub fn table_header(&self, index: i32, rows: bool) -> Result<Option<NodeId>> {
        self.resolve_table(|table| {
            let index = table.axis(index, rows)?;
            Ok(table.headers(index, rows).next().map(|cell| cell.id))
        })
    }

    pub fn table_description(&self, index: i32, rows: bool) -> Result<String> {
        self.resolve_table(|table| {
            let index = table.axis(index, rows)?;
            Ok(table
                .headers(index, rows)
                .map(|cell| cell.name.as_str())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
                .join(" "))
        })
    }

    pub fn table_is_selected(&self, row: i32, column: i32) -> Result<bool> {
        self.resolve_table(|table| {
            let (row, column) = table.coordinate(row, column)?;
            Ok(table
                .selected_rows
                .iter()
                .any(|(start, end)| (*start..*end).contains(&row))
                || table.at(row, column).is_some_and(|cell| cell.selected))
        })
    }

    pub fn table_is_axis_selected(&self, index: i32, rows: bool) -> Result<bool> {
        self.resolve_table(|table| Ok(table.selected_axis(table.axis(index, rows)?, rows)))
    }

    pub fn table_selected_axes(&self, rows: bool) -> Result<Vec<i32>> {
        self.resolve_table(|table| table.selected_axes(rows))
    }

    pub fn table_change_axis_selection(&self, index: i32, rows: bool) -> Result<bool> {
        self.resolve_table(|table| {
            table.axis(index, rows)?;
            // AccessKit has no atomic full-row/column selection action. Never
            // pretend that selecting one cell performs a whole-axis operation.
            Ok(false)
        })
    }

    pub fn table_cell_info(&self) -> Result<TableCellInfo> {
        self.resolve_table_cell(|_, cell| Ok(cell.into()))
    }

    pub fn table_cell_table(&self) -> Result<NodeId> {
        self.resolve_table_cell(|table, _| Ok(table.id))
    }

    pub fn table_cell_headers(&self, rows: bool) -> Result<Vec<NodeId>> {
        self.resolve_table_cell(|table, cell| {
            let (start, span) = if rows {
                (cell.row, cell.row_span)
            } else {
                (cell.column, cell.column_span)
            };
            Ok(table
                .cells
                .iter()
                .filter(|header| {
                    if header.id == cell.id {
                        return false;
                    }
                    let (role, position, length) = if rows {
                        (Role::RowHeader, header.row, header.row_span)
                    } else {
                        (Role::ColumnHeader, header.column, header.column_span)
                    };
                    header.role == role && position < start + span && start < position + length
                })
                .map(|header| header.id)
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionHandlerNoMut, WindowBounds,
        context::{AppContext, Context},
    };
    use accesskit::{
        ActionRequest, Node as Data, NodeId as LocalId, Tree as TreeData, TreeId, TreeUpdate,
    };
    use accesskit_consumer::Tree;
    use atspi_common::Interface;
    use std::sync::Arc;

    struct NoActions;
    impl ActionHandlerNoMut for NoActions {
        fn do_action(&self, _: ActionRequest) {}
    }

    fn context(nodes: Vec<(u64, Data)>) -> Arc<Context> {
        let tree = Tree::new(
            TreeUpdate {
                nodes: nodes
                    .into_iter()
                    .map(|(id, node)| (LocalId(id), node))
                    .collect(),
                tree: Some(TreeData::new(LocalId(0))),
                tree_id: TreeId::ROOT,
                focus: LocalId(0),
            },
            true,
        );
        Context::new(
            &AppContext::new(None),
            tree,
            Arc::new(NoActions),
            WindowBounds::default(),
        )
    }

    fn root(context: &Arc<Context>) -> PlatformNode {
        PlatformNode::new(context, 42, context.read_tree().state().root_id())
    }

    fn cell(role: Role, row: usize, column: usize, label: &str) -> Data {
        let mut node = Data::new(role);
        node.set_row_index(row);
        node.set_column_index(column);
        node.set_label(label);
        node
    }

    #[test]
    fn repeated_table_queries_share_one_snapshot_until_the_tree_changes() {
        let mut grid = Data::new(Role::Grid);
        grid.set_row_count(1001);
        grid.set_column_count(41);
        grid.set_children(vec![LocalId(1)]);
        let context = context(vec![(0, grid), (1, cell(Role::GridCell, 1000, 40, "last"))]);
        let table = root(&context);
        let last = child(&context, 0);
        SNAPSHOTS_BUILT.with(|count| count.set(0));
        for _ in 0..8 {
            assert_eq!(table.table_dimensions().unwrap(), (1001, 41));
            assert_eq!(table.table_cell_at(1000, 40).unwrap(), Some(last.id));
            assert_eq!(last.table_cell_info().unwrap().row, 1000);
            assert_eq!(last.table_cell_table().unwrap(), table.id);
            assert!(last.table_cell_headers(false).unwrap().is_empty());
        }
        assert_eq!(SNAPSHOTS_BUILT.with(|count| count.get()), 1);
    }

    #[test]
    fn cached_queries_follow_reposition_selection_headers_hidden_rows_and_removal() {
        fn nodes(moved: bool, removed: bool) -> Vec<(u64, Data)> {
            let mut grid = Data::new(Role::Grid);
            grid.set_row_count(if moved { 4 } else { 3 });
            grid.set_column_count(2);
            grid.set_children(vec![LocalId(1), LocalId(2), LocalId(5)]);
            let index = if moved { 3 } else { 1 };
            let mut row = Data::new(Role::Row);
            row.set_row_index(index);
            row.set_selected(moved);
            row.set_children(if removed {
                vec![LocalId(3)]
            } else {
                vec![LocalId(3), LocalId(4)]
            });
            let mut hidden_row = Data::new(Role::Row);
            hidden_row.set_row_index(2);
            hidden_row.set_children(vec![LocalId(6), LocalId(7)]);
            if moved {
                hidden_row.set_hidden();
            }
            let mut result = vec![
                (0, grid),
                (
                    1,
                    cell(
                        Role::ColumnHeader,
                        0,
                        1,
                        if moved { "Updated header" } else { "Header" },
                    ),
                ),
                (2, row),
                (
                    3,
                    cell(
                        Role::RowHeader,
                        index,
                        0,
                        if moved { "Row 4" } else { "Row 2" },
                    ),
                ),
                (5, hidden_row),
                (6, cell(Role::RowHeader, 2, 0, "Row 3")),
                (7, cell(Role::GridCell, 2, 1, "hidden after update")),
            ];
            if !removed {
                result.push((4, cell(Role::GridCell, index, 1, "value")));
            }
            result
        }
        let context = context(nodes(false, false));
        let table = root(&context);
        let value = {
            let tree = context.read_tree();
            let id = tree
                .state()
                .root()
                .children()
                .nth(1)
                .unwrap()
                .children()
                .nth(1)
                .unwrap()
                .id();
            PlatformNode::new(&context, 42, id)
        };
        // Warm all cached relationships before changing the tree.
        assert_eq!(table.table_dimensions().unwrap(), (3, 2));
        assert_eq!(value.table_cell_info().unwrap().row, 1);
        assert_eq!(table.table_description(1, false).unwrap(), "Header");
        assert!(!table.table_is_axis_selected(1, true).unwrap());
        for removed in [false, true] {
            *context.write_tree() = Tree::new(
                TreeUpdate {
                    nodes: nodes(true, removed)
                        .into_iter()
                        .map(|(id, node)| (LocalId(id), node))
                        .collect(),
                    tree: Some(TreeData::new(LocalId(0))),
                    tree_id: TreeId::ROOT,
                    focus: LocalId(0),
                },
                true,
            );
            assert_eq!(table.table_dimensions().unwrap(), (4, 2));
            assert_eq!(table.table_description(1, false).unwrap(), "Updated header");
            assert!(table.table_is_axis_selected(3, true).unwrap());
            assert_eq!(table.table_cell_at(1, 1).unwrap(), None);
            assert_eq!(table.table_cell_at(2, 1).unwrap(), None);
            if removed {
                assert_eq!(table.table_cell_at(3, 1).unwrap(), None);
                assert!(matches!(value.table_cell_info(), Err(Error::Defunct)));
            } else {
                assert_eq!(table.table_cell_at(3, 1).unwrap(), Some(value.id));
                assert_eq!(value.table_cell_info().unwrap().row, 3);
                assert!(table.table_is_selected(3, 1).unwrap());
                assert_eq!(value.table_cell_headers(true).unwrap().len(), 1);
                assert_eq!(table.table_description(3, true).unwrap(), "Row 4");
            }
        }
    }

    #[test]
    fn bulk_cache_matches_accessible_metadata_and_excludes_hidden_and_password_values() {
        struct NoCallbacks;
        impl crate::AdapterCallback for NoCallbacks {
            fn register_interfaces(
                &self,
                _: &crate::Adapter,
                _: NodeId,
                _: atspi_common::InterfaceSet,
            ) {
            }
            fn unregister_interfaces(
                &self,
                _: &crate::Adapter,
                _: NodeId,
                _: atspi_common::InterfaceSet,
            ) {
            }
            fn emit_event(&self, _: &crate::Adapter, _: crate::Event) {}
        }
        let app = AppContext::new(Some("QA app".into()));
        let mut window = Data::new(Role::Window);
        window.set_children(vec![LocalId(1), LocalId(2), LocalId(7), LocalId(8)]);
        let mut transparent = Data::new(Role::GenericContainer);
        transparent.set_children(vec![LocalId(3)]);
        let mut hidden = Data::new(Role::Group);
        hidden.set_hidden();
        hidden.set_children(vec![LocalId(5)]);
        let mut button = Data::new(Role::Button);
        button.set_label("Visible button");
        let mut secret = Data::new(Role::PasswordInput);
        secret.set_label("Vault passphrase");
        secret.set_value("private-value-sentinel");
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(9)]);
        let mut row = Data::new(Role::Row);
        row.set_row_index(1);
        row.set_children(vec![LocalId(10), LocalId(11)]);
        let mut value = cell(Role::GridCell, 1, 1, "alpha");
        value.add_action(accesskit::Action::Focus);
        value.set_bounds(accesskit::Rect::new(10., 10., 20., 20.));
        let mut adapter = crate::Adapter::with_wrapped_action_handler(
            42,
            &app,
            NoCallbacks,
            TreeUpdate {
                nodes: vec![
                    (LocalId(0), window),
                    (LocalId(1), transparent),
                    (LocalId(2), hidden),
                    (LocalId(3), button),
                    (LocalId(5), cell(Role::Cell, 0, 0, "hidden-sentinel")),
                    (LocalId(7), secret),
                    (LocalId(8), grid),
                    (LocalId(9), row),
                    (LocalId(10), cell(Role::RowHeader, 1, 0, "Row 1")),
                    (LocalId(11), value),
                ],
                tree: Some(TreeData::new(LocalId(0))),
                tree_id: TreeId::ROOT,
                focus: LocalId(11),
            },
            true,
            WindowBounds::default(),
            Arc::new(NoActions),
        );
        let snapshot = adapter.platform_root().cache_snapshot().unwrap();
        assert_eq!(snapshot.name, "QA app");
        assert_eq!(snapshot.windows, 1);
        assert_eq!(snapshot.nodes.len(), 7);
        for info in &snapshot.nodes {
            let node = adapter.platform_node(info.id);
            assert_eq!(info.parent, node.parent().unwrap());
            assert_eq!(info.index, node.index_in_parent().unwrap());
            assert_eq!(info.children, node.child_count().unwrap());
            assert_eq!(info.name, node.name().unwrap());
            assert_eq!(info.role, node.role().unwrap());
            assert_eq!(info.states, node.state());
            assert_eq!(info.interfaces, node.interfaces().unwrap());
            assert_eq!(info.description, node.description().unwrap());
            assert!(!info.name.contains("sentinel"));
        }
        let protected = snapshot
            .nodes
            .iter()
            .find(|info| info.name == "Vault passphrase")
            .unwrap();
        assert_eq!(protected.role, atspi_common::Role::PasswordText);
        assert_eq!(protected.children, 0);
        assert!(!protected.interfaces.contains(Interface::Text));
        let focused = snapshot
            .nodes
            .iter()
            .find(|info| info.name == "alpha")
            .unwrap();
        assert!(focused.states.contains(atspi_common::State::Focused));
        assert!(focused.interfaces.contains(Interface::TableCell));
        assert!(
            snapshot
                .nodes
                .iter()
                .all(|info| info.role != atspi_common::Role::Section)
        );
        let node = adapter.platform_node(focused.id);
        let single = node.cache_info().unwrap();
        assert_eq!(single.parent, focused.parent);
        assert_eq!(single.index, focused.index);
        let mut updated = cell(Role::GridCell, 1, 1, "beta");
        updated.set_selected(true);
        adapter.update(TreeUpdate {
            nodes: vec![(LocalId(11), updated)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: LocalId(11),
        });
        let current = adapter.platform_root().cache_snapshot().unwrap();
        assert!(!current.nodes.iter().any(|info| info.name == "alpha"));
        assert!(
            current
                .nodes
                .iter()
                .find(|info| info.name == "beta")
                .unwrap()
                .states
                .contains(atspi_common::State::Selected)
        );
        let mut hidden_grid = Data::new(Role::Grid);
        hidden_grid.set_children(vec![LocalId(9)]);
        hidden_grid.set_hidden();
        adapter.update(TreeUpdate {
            nodes: vec![(LocalId(8), hidden_grid)],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: LocalId(0),
        });
        assert!(
            !adapter
                .platform_root()
                .cache_snapshot()
                .unwrap()
                .nodes
                .iter()
                .any(|info| info.id == focused.id)
        );
        assert!(matches!(node.cache_info(), Err(Error::Defunct)));
    }

    #[test]
    fn exposes_native_table_and_table_cell_interfaces() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let context = context(vec![(0, grid), (1, cell(Role::GridCell, 0, 0, "value"))]);
        let tree = context.read_tree();
        let grid = tree.state().root();
        let child = grid.children().next().unwrap();
        assert!(NodeWrapper(&grid).interfaces().contains(Interface::Table));
        assert!(
            NodeWrapper(&child)
                .interfaces()
                .contains(Interface::TableCell)
        );
    }

    fn child(context: &Arc<Context>, index: usize) -> PlatformNode {
        let tree = context.read_tree();
        let id = tree.state().root().children().nth(index).unwrap().id();
        PlatformNode::new(context, 42, id)
    }

    #[test]
    fn reports_declared_dimensions_without_fabricating_virtualized_cells() {
        let mut grid = Data::new(Role::Grid);
        grid.set_row_count(1001);
        grid.set_column_count(3);
        grid.set_children(vec![LocalId(1)]);
        let context = context(vec![(0, grid), (1, cell(Role::GridCell, 5, 2, "visible"))]);
        let table = root(&context);
        assert_eq!(table.table_dimensions().unwrap(), (1001, 3));
        assert_eq!(
            table.table_cell_at(5, 2).unwrap(),
            Some(child(&context, 0).id)
        );
        assert_eq!(table.table_cell_at(999, 2).unwrap(), None);
        assert_eq!(table.table_cell_extent_at(999, 2, true).unwrap(), 0);
        for (row, column) in [(-1, 0), (0, -1), (1001, 0), (0, 3)] {
            assert!(matches!(
                table.table_cell_at(row, column),
                Err(Error::IndexOutOfRange)
            ));
        }
    }

    #[test]
    fn honors_spans_and_returns_real_child_indices_for_flat_tables() {
        let mut table = Data::new(Role::Table);
        table.set_children(vec![LocalId(1)]);
        let mut value = cell(Role::Cell, 2, 3, "spanning");
        value.set_row_span(2);
        value.set_column_span(3);
        let context = context(vec![(0, table), (1, value)]);
        let table = root(&context);
        let value = child(&context, 0);
        assert_eq!(table.table_dimensions().unwrap(), (4, 6));
        assert_eq!(table.table_cell_at(3, 5).unwrap(), Some(value.id));
        assert_eq!(table.table_index_at(3, 5).unwrap(), 0);
        assert_eq!(
            table.table_cell_at_index(0).unwrap().unwrap().0,
            TableCellInfo {
                row: 2,
                column: 3,
                row_span: 2,
                column_span: 3
            }
        );
        assert_eq!(value.table_cell_table().unwrap(), table.id);
        assert_eq!(value.table_cell_info().unwrap().row_span, 2);
        assert_eq!(table.table_cell_at_index(-1).unwrap(), None);
        assert_eq!(table.table_cell_at_index(1).unwrap(), None);
    }

    #[test]
    fn inherits_row_metadata_through_wrappers_without_fabricating_child_indices() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let mut row = Data::new(Role::Row);
        row.set_row_index(7);
        row.set_children(vec![LocalId(2)]);
        let mut wrapper = Data::new(Role::GenericContainer);
        wrapper.set_children(vec![LocalId(3)]);
        let mut value = Data::new(Role::GridCell);
        value.set_column_index(2);
        let context = context(vec![(0, grid), (1, row), (2, wrapper), (3, value)]);
        let table = root(&context);
        assert_eq!(table.table_dimensions().unwrap(), (8, 3));
        assert!(table.table_cell_at(7, 2).unwrap().is_some());
        assert_eq!(table.table_index_at(7, 2).unwrap(), -1);
        assert_eq!(table.table_cell_at_index(0).unwrap(), None);
    }

    #[test]
    fn excludes_hidden_cells_and_nested_table_contents() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1), LocalId(2), LocalId(3)]);
        let mut hidden = cell(Role::GridCell, 88, 88, "hidden");
        hidden.set_hidden();
        let mut inner = Data::new(Role::Table);
        inner.set_children(vec![LocalId(4)]);
        let context = context(vec![
            (0, grid),
            (1, cell(Role::GridCell, 0, 0, "outer")),
            (2, hidden),
            (3, inner),
            (4, cell(Role::Cell, 10, 10, "inner")),
        ]);
        assert_eq!(root(&context).table_dimensions().unwrap(), (1, 1));
        assert_eq!(child(&context, 2).table_dimensions().unwrap(), (11, 11));
        assert!(
            !child(&context, 1)
                .interfaces()
                .unwrap()
                .contains(Interface::TableCell)
        );
    }

    #[test]
    fn returns_row_and_column_header_objects_including_spanning_headers() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1), LocalId(2), LocalId(3), LocalId(4)]);
        let mut group = cell(Role::ColumnHeader, 0, 1, "group");
        group.set_column_span(2);
        let context = context(vec![
            (0, grid),
            (1, group),
            (2, cell(Role::ColumnHeader, 1, 2, "name")),
            (3, cell(Role::RowHeader, 2, 0, "row two")),
            (4, cell(Role::GridCell, 2, 2, "value")),
        ]);
        let table = root(&context);
        let value = child(&context, 3);
        assert_eq!(
            table.table_header(2, false).unwrap(),
            Some(child(&context, 0).id)
        );
        assert_eq!(table.table_description(2, false).unwrap(), "group name");
        assert_eq!(
            table.table_header(2, true).unwrap(),
            Some(child(&context, 2).id)
        );
        assert_eq!(
            value.table_cell_headers(false).unwrap(),
            vec![child(&context, 0).id, child(&context, 1).id]
        );
        assert_eq!(
            value.table_cell_headers(true).unwrap(),
            vec![child(&context, 2).id]
        );
        assert!(
            child(&context, 2)
                .table_cell_headers(true)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn does_not_advertise_cells_without_coordinates_or_containing_table() {
        let mut window = Data::new(Role::Window);
        window.set_children(vec![LocalId(1)]);
        let context = context(vec![(0, window), (1, cell(Role::GridCell, 0, 0, "orphan"))]);
        assert!(
            !child(&context, 0)
                .interfaces()
                .unwrap()
                .contains(Interface::TableCell)
        );
        assert!(matches!(
            child(&context, 0).table_cell_info(),
            Err(Error::UnsupportedInterface)
        ));
        assert!(matches!(
            root(&context).table_dimensions(),
            Err(Error::UnsupportedInterface)
        ));
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let missing = Data::new(Role::GridCell);
        let context = super::tests::context(vec![(0, grid), (1, missing)]);
        assert!(
            !child(&context, 0)
                .interfaces()
                .unwrap()
                .contains(Interface::TableCell)
        );
    }

    #[test]
    fn distinguishes_partial_cell_selection_from_full_axis_selection() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1), LocalId(2), LocalId(3)]);
        grid.set_row_count(2);
        grid.set_column_count(2);
        let mut a = cell(Role::GridCell, 0, 0, "a");
        a.set_selected(true);
        let mut b = cell(Role::GridCell, 0, 1, "b");
        b.set_selected(true);
        let mut c = cell(Role::GridCell, 1, 0, "c");
        c.set_selected(true);
        let context = context(vec![(0, grid), (1, a), (2, b), (3, c)]);
        let table = root(&context);
        assert!(table.table_is_selected(1, 0).unwrap());
        assert!(!table.table_is_selected(1, 1).unwrap());
        assert_eq!(table.table_selected_axes(true).unwrap(), vec![0]);
        assert_eq!(table.table_selected_axes(false).unwrap(), vec![0]);
        assert!(!table.table_is_axis_selected(1, true).unwrap());
        assert!(!table.table_change_axis_selection(0, true).unwrap());
        assert!(matches!(
            table.table_change_axis_selection(-1, true),
            Err(Error::IndexOutOfRange)
        ));
    }

    #[test]
    fn reports_explicit_row_selection_even_when_not_all_cells_are_painted() {
        let mut grid = Data::new(Role::Grid);
        grid.set_row_count(10);
        grid.set_column_count(3);
        grid.set_children(vec![LocalId(1)]);
        let mut row = Data::new(Role::Row);
        row.set_row_index(4);
        row.set_selected(true);
        row.set_children(vec![LocalId(2)]);
        let context = context(vec![
            (0, grid),
            (1, row),
            (2, cell(Role::GridCell, 4, 1, "selected row value")),
        ]);
        let table = root(&context);
        assert_eq!(table.table_selected_axes(true).unwrap(), vec![4]);
        assert!(table.table_is_axis_selected(4, true).unwrap());
        assert!(table.table_is_selected(4, 1).unwrap());
        assert!(table.table_is_selected(4, 2).unwrap());
        assert!(table.table_selected_axes(false).unwrap().is_empty());
    }

    #[test]
    fn rejects_overflow_and_out_of_bounds_metadata_and_bounds_selection_memory() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1), LocalId(2)]);
        grid.set_row_count(2);
        grid.set_column_count(2);
        let context = context(vec![
            (0, grid),
            (1, cell(Role::GridCell, usize::MAX, 0, "overflow")),
            (2, cell(Role::GridCell, 2, 2, "outside")),
        ]);
        assert_eq!(root(&context).table_cell_at(0, 0).unwrap(), None);
        assert!(matches!(
            child(&context, 1).table_cell_info(),
            Err(Error::IndexOutOfRange)
        ));
        let mut grid = Data::new(Role::Grid);
        grid.set_row_count(usize::MAX);
        let context = super::tests::context(vec![(0, grid)]);
        assert!(matches!(
            root(&context).table_dimensions(),
            Err(Error::TooManyChildren)
        ));
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let mut value = cell(Role::GridCell, 0, 0, "huge selected span");
        value.set_row_span(i32::MAX as usize);
        value.set_selected(true);
        let context = super::tests::context(vec![(0, grid), (1, value)]);
        assert!(matches!(
            root(&context).table_selected_axes(true),
            Err(Error::TooManyChildren)
        ));
    }

    #[test]
    fn removed_nodes_and_changed_roles_stop_answering_table_queries() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let context = context(vec![(0, grid), (1, cell(Role::GridCell, 0, 0, "old"))]);
        let table = root(&context);
        let old = child(&context, 0);
        *context.write_tree() = Tree::new(
            TreeUpdate {
                nodes: vec![(LocalId(0), Data::new(Role::Window))],
                tree: Some(TreeData::new(LocalId(0))),
                tree_id: TreeId::ROOT,
                focus: LocalId(0),
            },
            true,
        );
        assert!(matches!(old.table_cell_info(), Err(Error::Defunct)));
        assert!(matches!(
            table.table_dimensions(),
            Err(Error::UnsupportedInterface)
        ));
        drop(context);
        assert!(matches!(table.table_dimensions(), Err(Error::Defunct)));
    }
    #[test]
    fn logical_focusable_cells_support_component_without_invented_bounds() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let mut value = cell(Role::GridCell, 4, 1, "unpainted");
        value.add_action(accesskit::Action::Focus);
        value.add_action(accesskit::Action::ScrollIntoView);
        let context = context(vec![(0, grid), (1, value)]);
        let value = child(&context, 0);
        assert!(value.interfaces().unwrap().contains(Interface::Component));
        assert_eq!(
            value.extents(atspi_common::CoordType::Window).unwrap(),
            crate::Rect::INVALID
        );
        assert!(!value.state().contains(atspi_common::State::Showing));
        assert!(value.state().contains(atspi_common::State::Visible));
        assert!(value.grab_focus().unwrap());
    }
    #[test]
    fn painted_logical_cells_report_showing_and_actual_bounds() {
        let mut grid = Data::new(Role::Grid);
        grid.set_children(vec![LocalId(1)]);
        let mut value = cell(Role::GridCell, 4, 1, "item 5");
        value.set_bounds(accesskit::Rect::new(10.0, 20.0, 110.0, 50.0));
        value.add_action(accesskit::Action::Focus);
        let context = context(vec![(0, grid), (1, value)]);
        let value = child(&context, 0);
        assert!(value.state().contains(atspi_common::State::Showing));
        assert!(value.state().contains(atspi_common::State::Visible));
        let bounds = value.extents(atspi_common::CoordType::Window).unwrap();
        assert_ne!(bounds, crate::Rect::INVALID);
    }
}
