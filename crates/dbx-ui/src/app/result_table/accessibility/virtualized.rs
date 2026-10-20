//! Loaded grid semantics are independent of the visually painted viewport.
//! Only real painted cells have bounds. Native focus/scroll actions reveal cells
//! through TableState, retaining the same semantic IDs before and after reveal.

use super::*;
use gpui::accesskit::{Node, NodeId, Rect, Role};
use gpui::{
    A11ySubtreeBuilder, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Pixels,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(in crate::app::result_table) enum GridNodeKey {
    HeaderRow,
    Header(SharedString),
    Row(String),
    Cell(String, SharedString),
}

#[derive(Default)]
pub(in crate::app::result_table) struct GridGeometry {
    bounds: HashMap<GridNodeKey, Rect>,
}

impl GridGeometry {
    pub(in crate::app::result_table) fn clear(&mut self) {
        self.bounds.clear();
    }

    pub(in crate::app::result_table) fn record(
        &mut self,
        key: GridNodeKey,
        bounds: Bounds<Pixels>,
        window: &Window,
    ) {
        // A virtual-list overscan item may be prepared but wholly clipped.
        if bounds.intersect(&window.content_mask().bounds).is_empty() {
            return;
        }
        let scale = window.scale_factor() as f64;
        self.bounds.insert(
            key,
            Rect::new(
                f32::from(bounds.left()) as f64 * scale,
                f32::from(bounds.top()) as f64 * scale,
                f32::from(bounds.right()) as f64 * scale,
                f32::from(bounds.bottom()) as f64 * scale,
            ),
        );
    }
}

#[derive(Clone)]
enum GridTarget {
    Cell(AccessibleCellTarget),
    Row(AccessibleCellTarget),
    Header { key: SharedString, revision: u64 },
}

#[derive(Clone)]
pub(in crate::app::result_table) struct GridAction {
    id: NodeId,
    target: Rc<GridTarget>,
    action: gpui::AccessibleAction,
}

struct GridNode {
    key: GridNodeKey,
    parent: Option<GridNodeKey>,
    node: Node,
    target: Option<Rc<GridTarget>>,
}

struct MaterializedGrid {
    nodes: Vec<(NodeId, Node)>,
    roots: Vec<NodeId>,
    actions: Vec<GridAction>,
    active: Option<NodeId>,
}

pub(in crate::app::result_table) struct GridSnapshot {
    nodes: Vec<GridNode>,
    active: Option<GridNodeKey>,
    rows: usize,
    columns: usize,
    description: String,
}

impl GridSnapshot {
    pub(in crate::app::result_table) fn new(
        delegate: &ResultTableDelegate,
        table_id: gpui::EntityId,
    ) -> Self {
        let rows = delegate.loaded_rows() + delegate.changes.inserts.len();
        let columns = delegate.columns.len();
        let mut nodes =
            Vec::with_capacity(rows.saturating_mul(columns + 1).saturating_add(columns + 1));
        let mut header_row = Node::new(Role::Row);
        header_row.set_row_index(0);
        nodes.push(GridNode {
            key: GridNodeKey::HeaderRow,
            parent: None,
            node: header_row,
            target: None,
        });
        for column in 0..columns {
            let key = delegate.columns[column].key.clone();
            let node = delegate.accessible_header_node(column, table_id);
            nodes.push(GridNode {
                key: GridNodeKey::Header(key.clone()),
                parent: Some(GridNodeKey::HeaderRow),
                node,
                target: Some(Rc::new(GridTarget::Header {
                    key,
                    revision: delegate.accessible_revision,
                })),
            });
        }
        for row in 0..rows {
            let key = delegate.accessible_row_key(row);
            let mut node = Node::new(Role::Row);
            node.set_row_index(row + 1);
            node.set_row_index_text((delegate.row_offset + row + 1).to_string());
            node.set_author_id(format!("dbx-grid-{table_id}-row-{key}"));
            node.set_selected(
                delegate.accessible_selected_row == Some(row)
                    || delegate.changes.marked.contains(&row),
            );
            for action in [
                gpui::AccessibleAction::Focus,
                gpui::AccessibleAction::ScrollIntoView,
            ] {
                node.add_action(action);
            }
            let row_key = GridNodeKey::Row(key.clone());
            nodes.push(GridNode {
                key: row_key.clone(),
                parent: None,
                node,
                target: Some(Rc::new(GridTarget::Row(
                    delegate.accessible_cell_target(row, 0),
                ))),
            });
            for column in 0..columns {
                let mut node = delegate.accessible_cell_node(row, column, table_id);
                for action in [
                    gpui::AccessibleAction::Focus,
                    gpui::AccessibleAction::Click,
                    gpui::AccessibleAction::ScrollIntoView,
                ] {
                    node.add_action(action);
                }
                nodes.push(GridNode {
                    key: GridNodeKey::Cell(key.clone(), delegate.columns[column].key.clone()),
                    parent: Some(row_key.clone()),
                    node,
                    target: Some(Rc::new(GridTarget::Cell(
                        delegate.accessible_cell_target(row, column),
                    ))),
                });
            }
        }
        let active = if delegate.accessible_focused && delegate.editing.is_none() {
            delegate
                .accessible_header_focus
                .as_ref()
                .filter(|key| delegate.columns.iter().any(|column| column.key == **key))
                .map(|key| GridNodeKey::Header(key.clone()))
                .or_else(|| {
                    delegate.accessible_selected_cell.and_then(|(row, column)| {
                        (row < rows && column < columns).then(|| {
                            GridNodeKey::Cell(
                                delegate.accessible_row_key(row),
                                delegate.columns[column].key.clone(),
                            )
                        })
                    })
                })
                .or_else(|| {
                    delegate
                        .accessible_selected_row
                        .filter(|row| *row < rows)
                        .map(|row| GridNodeKey::Row(delegate.accessible_row_key(row)))
                })
        } else {
            None
        };
        let truncated = delegate
            .result
            .as_ref()
            .is_some_and(|result| result.truncated);
        let description = format!(
            "{} loaded rows and {} data columns. Starting at row {}{}. Arrow keys navigate cells. Control or Command plus Shift and Left or Right resizes the selected column; Home fits it.",
            rows,
            columns,
            delegate.row_offset + 1,
            if truncated {
                "; more rows are available"
            } else {
                ""
            }
        );
        Self {
            nodes,
            active,
            rows,
            columns,
            description,
        }
    }

    fn materialize(
        &self,
        id: impl Fn(&GridNodeKey) -> NodeId,
        geometry: &GridGeometry,
    ) -> MaterializedGrid {
        let mut children = HashMap::<GridNodeKey, Vec<NodeId>>::new();
        let mut roots = Vec::new();
        for item in &self.nodes {
            if let Some(parent) = &item.parent {
                children
                    .entry(parent.clone())
                    .or_default()
                    .push(id(&item.key));
            } else {
                roots.push(id(&item.key));
            }
        }
        let mut bounds = geometry.bounds.clone();
        // Row bounds are the union of actual painted cells, not estimated layout.
        for item in &self.nodes {
            if let Some(parent) = &item.parent
                && let Some(rect) = geometry.bounds.get(&item.key)
            {
                bounds
                    .entry(parent.clone())
                    .and_modify(|current| *current = current.union(*rect))
                    .or_insert(*rect);
            }
        }
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut actions = Vec::new();
        for item in &self.nodes {
            let node_id = id(&item.key);
            let mut node = item.node.clone();
            if let Some(children) = children.get(&item.key) {
                node.set_children(children.clone());
            }
            if let Some(rect) = bounds.get(&item.key) {
                node.set_bounds(*rect);
            }

            if let Some(target) = &item.target {
                for action in [
                    gpui::AccessibleAction::Focus,
                    gpui::AccessibleAction::Click,
                    gpui::AccessibleAction::ScrollIntoView,
                ] {
                    if node.supports_action(action) {
                        actions.push(GridAction {
                            id: node_id,
                            target: target.clone(),
                            action,
                        });
                    }
                }
            }
            nodes.push((node_id, node));
        }
        MaterializedGrid {
            nodes,
            roots,
            actions,
            active: self.active.as_ref().map(id),
        }
    }

    pub(in crate::app::result_table) fn publish(
        &self,
        builder: &mut A11ySubtreeBuilder,
        geometry: &GridGeometry,
        actions: &mut Vec<GridAction>,
    ) {
        let MaterializedGrid {
            nodes,
            roots,
            actions: listeners,
            active,
        } = self.materialize(|key| builder.synthetic_node_id(key), geometry);
        // Preserve real inline editors and any contextual controls. Painted row,
        // header and cell wrappers are transparent while this model is active.
        let mut children = builder.parent_node().children().to_vec();
        for (id, node) in nodes {
            builder.push_child(id, node);
        }
        children.extend(roots);
        let root = builder.parent_node();
        root.set_children(children);
        root.set_row_count(self.rows + 1);
        root.set_column_count(self.columns);
        root.set_description(self.description.clone());
        if let Some(active) = active {
            root.set_active_descendant(active);
        }
        *actions = listeners;
    }
}

impl ResultTableDelegate {
    fn accessible_header_node(&self, column: usize, table_id: gpui::EntityId) -> Node {
        let index = self.result_column(column);
        let metadata = index.and_then(|index| {
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
        let mut node = Node::new(Role::ColumnHeader);
        node.set_label(metadata.map_or_else(String::new, |column| column.name.clone()));
        node.set_row_index(0);
        node.set_column_index(column);
        node.set_author_id(format!(
            "dbx-grid-{table_id}-header-{}",
            self.columns[column].key
        ));
        let direction = index.and_then(|index| self.column_sort(index));
        let sort = match direction {
            Some(ColumnSort::Ascending) => ". Sorted ascending",
            Some(ColumnSort::Descending) => ". Sorted descending",
            _ => "",
        };
        node.set_description(metadata.map_or_else(
            || format!("Column {} of {}. ", column + 1, self.columns.len()),
            |metadata| {
                format!(
                    "Column {} of {}. {}{}{sort}",
                    column + 1,
                    self.columns.len(),
                    metadata.data_type,
                    if metadata.primary_key {
                        ", primary key"
                    } else {
                        ""
                    }
                )
            },
        ));
        if let Some(sort) = direction {
            node.add_action(gpui::AccessibleAction::Click);
            match sort {
                ColumnSort::Ascending => {
                    node.set_sort_direction(gpui::accesskit::SortDirection::Ascending)
                }
                ColumnSort::Descending => {
                    node.set_sort_direction(gpui::accesskit::SortDirection::Descending)
                }
                ColumnSort::Default => (),
            }
        }
        node.add_action(gpui::AccessibleAction::ScrollIntoView);
        node.add_action(gpui::AccessibleAction::Focus);
        node
    }
}

fn apply_action(
    target: &GridTarget,
    action: gpui::AccessibleAction,
    table: &mut TableState<ResultTableDelegate>,
    window: &mut Window,
    cx: &mut Context<TableState<ResultTableDelegate>>,
) {
    let row_request = matches!(target, GridTarget::Row(_));
    match target {
        GridTarget::Cell(target) | GridTarget::Row(target) => {
            let Some(column) = table.delegate().resolve_accessible_cell_target(target) else {
                return;
            };
            table.delegate_mut().accessible_header_focus = None;
            table.delegate_mut().context_column = table.delegate().result_column(column);
            if row_request {
                table.set_selected_row(target.row, cx);
            } else {
                table.set_selected_cell(target.row, column, cx);
            }
            table.focus_handle(cx).focus(window, cx);
            cx.notify();
        }
        GridTarget::Header { key, revision } => {
            if table.delegate().accessible_revision != *revision {
                return;
            }
            let Some(column) = table
                .delegate()
                .columns
                .iter()
                .position(|column| column.key == *key)
            else {
                return;
            };
            if action == gpui::AccessibleAction::Click {
                let next = match table.delegate().columns[column].sort {
                    Some(ColumnSort::Ascending) => ColumnSort::Descending,
                    Some(ColumnSort::Descending) => ColumnSort::Default,
                    _ => ColumnSort::Ascending,
                };
                table.delegate_mut().perform_sort(column, next, window, cx);
                table.refresh(cx);
            } else {
                let rows = table.delegate().rows_count(cx);
                if rows > 0 {
                    let row = table
                        .selected_cell()
                        .map_or(0, |(row, _)| row)
                        .min(rows - 1);
                    table.set_selected_cell(row, column, cx);
                }
                table.focus_handle(cx).focus(window, cx);
                let selection = table.selected_cell();
                table.delegate_mut().accessible_header_focus = Some(key.clone());
                table.delegate_mut().accessible_header_selection = selection;
                cx.notify();
            }
        }
    }
}

pub(super) fn header_key(
    event: &gpui::KeyDownEvent,
    state: &Entity<TableState<ResultTableDelegate>>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let modifiers = event.keystroke.modifiers;
    if modifiers.secondary() || modifiers.alt || modifiers.shift {
        return false;
    }
    header_command(event.keystroke.key.as_str(), state, window, cx)
}

fn header_command(
    key_name: &str,
    state: &Entity<TableState<ResultTableDelegate>>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    state.update(cx, |table, cx| {
        if !table.focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(key) = table.delegate().accessible_header_focus.clone() else {
            return false;
        };
        let Some(column) = table
            .delegate()
            .columns
            .iter()
            .position(|column| column.key == key)
        else {
            return false;
        };
        match key_name {
            "enter" | "space" => {
                let target = GridTarget::Header {
                    key,
                    revision: table.delegate().accessible_revision,
                };
                apply_action(&target, gpui::AccessibleAction::Click, table, window, cx);
            }
            "left" | "right" | "home" | "end" => {
                let last = table.delegate().columns.len().saturating_sub(1);
                let next = match key_name {
                    "left" => column.saturating_sub(1),
                    "right" => (column + 1).min(last),
                    "home" => 0,
                    _ => last,
                };
                let target = GridTarget::Header {
                    key: table.delegate().columns[next].key.clone(),
                    revision: table.delegate().accessible_revision,
                };
                apply_action(&target, gpui::AccessibleAction::Focus, table, window, cx);
            }
            "down" | "escape" => {
                table.delegate_mut().accessible_header_focus = None;
                if key_name == "down" && table.delegate().rows_count(cx) > 0 {
                    table.set_selected_cell(0, column, cx);
                }
                cx.notify();
            }
            "up" => (),
            _ => {
                table.delegate_mut().accessible_header_focus = None;
                cx.notify();
                return false;
            }
        }
        true
    })
}

pub(super) fn header_actions(
    mut root: Stateful<Div>,
    state: &Entity<TableState<ResultTableDelegate>>,
) -> Stateful<Div> {
    // Bound table keys dispatch actions before raw key events. Capture only
    // header navigation; normal cell/row actions continue through TableState.
    macro_rules! capture {
        ($action:ident, $key:literal) => {
            let table = state.clone();
            root = root.capture_action(move |_: &gpui_base::actions::$action, window, cx| {
                if header_command($key, &table, window, cx) {
                    cx.stop_propagation();
                }
            });
        };
    }
    capture!(SelectNextColumn, "right");
    capture!(SelectPrevColumn, "left");
    capture!(SelectDown, "down");
    capture!(SelectUp, "up");
    capture!(SelectFirst, "home");
    capture!(SelectLast, "end");
    capture!(Cancel, "escape");
    root
}

/// Registers synthetic action listeners during paint, after IDs were produced
/// by the subtree builder. Shares the original table's element and focus node.
pub(in crate::app::result_table) struct VirtualizedGridElement<E: Element> {
    inner: E,
    state: Entity<TableState<ResultTableDelegate>>,
    editable: bool,
    snapshot: Rc<RefCell<Option<GridSnapshot>>>,
    actions: Rc<RefCell<Vec<GridAction>>>,
}

impl<E: Element> VirtualizedGridElement<E> {
    pub(in crate::app::result_table) fn new(
        inner: E,
        state: Entity<TableState<ResultTableDelegate>>,
        editable: bool,
        snapshot: Rc<RefCell<Option<GridSnapshot>>>,
        actions: Rc<RefCell<Vec<GridAction>>>,
    ) -> Self {
        Self {
            inner,
            state,
            editable,
            snapshot,
            actions,
        }
    }
}

impl<E: Element> IntoElement for VirtualizedGridElement<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<E: Element> Element for VirtualizedGridElement<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;
    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }
    fn a11y_role(&self) -> Option<Role> {
        self.inner.a11y_role()
    }
    fn write_a11y_info(&self, node: &mut Node) {
        self.inner.write_a11y_info(node);
    }
    fn a11y_synthetic_children(
        &mut self,
        state: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        self.inner.a11y_synthetic_children(state, builder);
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if window.is_a11y_active() {
            let snapshot = self.state.update(cx, |table, cx| {
                let selected_cell = table.selected_cell();
                let selected_row = table.selected_row();
                let focused = table.focus_handle(cx).is_focused(window);
                let delegate = table.delegate_mut();
                if delegate.accessible_header_selection != selected_cell {
                    delegate.accessible_header_focus = None;
                }
                delegate.accessible_selected_cell = selected_cell;
                delegate.accessible_selected_row = selected_row;
                delegate.accessible_focused = focused;
                delegate.accessible_editable = self.editable;
                delegate.accessible_logical_tree = true;
                delegate.ensure_accessible_rows();
                delegate.accessible_geometry.borrow_mut().clear();
                GridSnapshot::new(delegate, cx.entity_id())
            });
            *self.snapshot.borrow_mut() = Some(snapshot);
        }
        self.inner
            .prepaint(id, inspector, bounds, state, window, cx)
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector, bounds, layout, state, window, cx);
        if !window.is_a11y_active() {
            return;
        }
        for spec in self.actions.borrow().iter() {
            let table = self.state.downgrade();
            let target = spec.target.clone();
            let action = spec.action;
            window.on_a11y_action(spec.id, action, move |_, window, cx| {
                let _ = table.update(cx, |table, cx| {
                    apply_action(&target, action, table, window, cx)
                });
            });
        }
    }
}
