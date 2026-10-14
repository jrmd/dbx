use super::*;
use gpui::{Focusable, RenderOnce, div, prelude::*};
use gpui_component::ElementExt;
use gpui_component::table::DataTable;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

mod virtualized;
pub(super) use virtualized::{GridGeometry, GridNodeKey};
use virtualized::{GridSnapshot, VirtualizedGridElement};

#[derive(gpui::IntoElement)]
pub(in crate::app) struct AccessibleGrid {
    state: Entity<TableState<ResultTableDelegate>>,
    label: SharedString,
    editable: bool,
}

pub(in crate::app) fn accessible_grid(
    state: &Entity<TableState<ResultTableDelegate>>,
    label: impl Into<SharedString>,
    editable: bool,
) -> AccessibleGrid {
    AccessibleGrid {
        state: state.clone(),
        label: label.into(),
        editable,
    }
}

impl RenderOnce for AccessibleGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = self.state.focus_handle(cx);
        let active = window.is_a11y_active();
        let focused = focus.is_focused(window);
        let (rows, columns, offset, truncated) = self.state.update(cx, |table, cx| {
            let selected_cell = table.selected_cell();
            let selected_row = table.selected_row();
            let delegate = table.delegate_mut();
            delegate.accessible_selected_cell = selected_cell;
            delegate.accessible_selected_row = selected_row;
            delegate.accessible_focused = focused;
            delegate.accessible_editable = self.editable;
            delegate.accessible_logical_tree = active;
            if active {
                delegate.ensure_accessible_rows();
            }
            (
                delegate.rows_count(cx),
                delegate.columns.len(),
                delegate.row_offset,
                delegate
                    .result
                    .as_ref()
                    .is_some_and(|result| result.truncated),
            )
        });
        let state = self.state.clone();
        let keyboard = self.state.clone();
        let pointer = self.state.clone();
        let mut metadata = div().id("table")
            .role(gpui::Role::Grid)
            .aria_label(self.label)
            .aria_row_count(rows + 1)
            .aria_column_count(columns)
            .aria_description(format!("{} loaded rows and {} data columns, with row numbers. Starting at row {}{}. Arrow keys navigate cells. Control or Command plus Shift and Left or Right resizes the selected column; Home fits it.", rows, columns.saturating_sub(1), offset + 1, if truncated { "; more rows are available" } else { "" }));
        for (action, delta) in [
            (gpui::AccessibleAction::ScrollDown, 20isize),
            (gpui::AccessibleAction::ScrollUp, -20),
        ] {
            let state = state.clone();
            metadata = metadata.on_a11y_action(action, move |_, window, cx| {
                state.update(cx, |table, cx| {
                    let rows = table.delegate().rows_count(cx);
                    if rows == 0 {
                        return;
                    }
                    let (row, col) = table
                        .selected_cell()
                        .unwrap_or((table.selected_row().unwrap_or(0), 0));
                    let next = row.saturating_add_signed(delta).min(rows - 1);
                    table.set_selected_cell(next, col, cx);
                    table.focus_handle(cx).focus(window, cx);
                });
            });
        }
        let snapshot = Rc::new(std::cell::RefCell::new(None::<GridSnapshot>));
        let actions = Rc::new(std::cell::RefCell::new(Vec::new()));
        let geometry = self.state.read(cx).delegate().accessible_geometry.clone();
        let snapshot_for_tree = snapshot.clone();
        let actions_for_tree = actions.clone();
        metadata = metadata.a11y_synthetic_children(move |builder| {
            if let Some(snapshot) = snapshot_for_tree.borrow().as_ref() {
                snapshot.publish(
                    builder,
                    &geometry.borrow(),
                    &mut actions_for_tree.borrow_mut(),
                );
            }
        });
        let table = DataTable::new(&self.state)
            .with_size(crate::theme::grid_row_height())
            .stripe(false)
            .bordered(false)
            .scrollbar_visible(true, true)
            .render(window, cx);
        div()
            .id("accessible-grid-container")
            .size_full()
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                pointer.update(cx, |table, cx| {
                    if table
                        .delegate_mut()
                        .accessible_header_focus
                        .take()
                        .is_some()
                    {
                        cx.notify();
                    }
                })
            })
            .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                if virtualized::header_key(event, &keyboard, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if !modifiers.secondary()
                    || !modifiers.shift
                    || modifiers.alt
                    || !keyboard.focus_handle(cx).is_focused(window)
                {
                    return;
                }
                let key = event.keystroke.key.as_str();
                if !matches!(key, "left" | "right" | "home") {
                    return;
                }
                cx.stop_propagation();
                keyboard.update(cx, |table, cx| {
                    let col = table.selected_cell().map_or(1, |(_, col)| col);
                    let delegate = table.delegate_mut();
                    let Some(column) = delegate.columns.get(col) else {
                        return;
                    };
                    let width = if key == "home" {
                        delegate.fitted_width(col).unwrap_or(column.width)
                    } else {
                        column.width + px(if key == "right" { 20. } else { -20. })
                    };
                    if delegate.set_column_width(col, width) {
                        table.refresh(cx);
                        emit_widths(table, cx);
                    }
                });
            })
            .map(|root| virtualized::header_actions(root, &self.state))
            .child(VirtualizedGridElement::new(
                crate::accessibility::with_semantics(table, metadata).into_element(),
                self.state,
                self.editable,
                snapshot,
                actions,
            ))
    }
}

#[derive(Clone)]
struct AccessibleCellTarget {
    row: usize,
    row_key: String,
    column_key: SharedString,
    revision: u64,
}

impl ResultTableDelegate {
    fn accessible_cell_target(&self, row: usize, col: usize) -> AccessibleCellTarget {
        AccessibleCellTarget {
            row,
            row_key: self.accessible_row_key(row),
            column_key: self.columns[col].key.clone(),
            revision: self.accessible_revision,
        }
    }

    fn resolve_accessible_cell_target(&self, target: &AccessibleCellTarget) -> Option<usize> {
        if target.revision != self.accessible_revision
            || target.row >= self.loaded_rows() + self.changes.inserts.len()
            || self.accessible_row_key(target.row) != target.row_key
        {
            return None;
        }
        self.columns
            .iter()
            .position(|column| column.key == target.column_key)
    }

    pub(super) fn ensure_accessible_rows(&mut self) {
        let Some(result) = self.result.as_ref() else {
            return;
        };
        if self.accessible_rows.len() == result.rows.len() {
            return;
        }
        let primary = if self.is_data_result {
            let keys = self
                .table_columns
                .iter()
                .filter(|column| column.primary_key)
                .collect::<Vec<_>>();
            keys.iter()
                .map(|key| {
                    result
                        .columns
                        .iter()
                        .position(|column| column.name == key.name)
                })
                .collect::<Option<Vec<_>>>()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // Trust complete catalog keys for table records, never query joins.
        // Compute full values only for ambiguous rows, keeping large keyed
        // payloads out of the UI-thread fingerprint work.
        let primary_keys = result
            .rows
            .iter()
            .map(|row| {
                let values = primary
                    .iter()
                    .map(|index| row.values.get(*index))
                    .collect::<Vec<_>>();
                (!values.is_empty()
                    && values.iter().all(
                        |value| matches!(value, Some(value) if !matches!(value, CellValue::Null)),
                    ))
                .then(|| row_fingerprint(&values))
            })
            .collect::<Vec<_>>();
        let mut counts = HashMap::<u64, usize>::new();
        for primary in primary_keys.iter().flatten() {
            *counts.entry(*primary).or_default() += 1;
        }
        let mut occurrences = HashMap::<(bool, u64), u64>::new();
        self.accessible_rows = result
            .rows
            .iter()
            .zip(primary_keys)
            .map(|(row, primary)| {
                let unique_primary = primary.filter(|key| counts[key] == 1);
                let identity = unique_primary
                    .map_or_else(|| (false, row_fingerprint(&row.values)), |key| (true, key));
                let scope = if unique_primary.is_some() {
                    (0, 0)
                } else {
                    (self.row_offset, self.accessible_dataset_revision)
                };
                let occurrence = occurrences.entry(identity).or_default();
                let mut key = std::hash::DefaultHasher::new();
                (identity, scope, *occurrence).hash(&mut key);
                *occurrence += 1;
                key.finish()
            })
            .collect();
    }

    pub(super) fn accessible_row_key(&self, row: usize) -> String {
        if self.insert_row(row).is_some() {
            return format!(
                "new-{}",
                self.accessible_insert_keys
                    .get(row - self.loaded_rows())
                    .copied()
                    .unwrap_or(0)
            );
        }
        format!(
            "loaded-{}",
            self.accessible_rows
                .get(row)
                .copied()
                .unwrap_or((self.row_offset + row) as u64)
        )
    }

    fn accessible_cell_node(
        &self,
        row: usize,
        col: usize,
        table_id: gpui::EntityId,
    ) -> gpui::accesskit::Node {
        let data_column = self.result_column(col);
        let value = if col == 0 {
            format!("Row {}", self.row_offset + row + 1)
        } else if let Some(insert) = self.insert_row(row) {
            data_column
                .and_then(|index| insert.get(index))
                .and_then(Option::as_ref)
                .map(staged_text)
                .unwrap_or_else(|| "DEFAULT".into())
        } else if let Some(value) =
            data_column.and_then(|column| self.changes.edits.get(&(row, column)))
        {
            staged_text(value)
        } else {
            data_column
                .and_then(|column| self.cell_value(row, column))
                .map(|value| {
                    if matches!(value, CellValue::Text(text) if text.is_empty()) {
                        "Empty string".into()
                    } else {
                        plain_cell_text(value)
                    }
                })
                .unwrap_or_else(|| "Unavailable".into())
        };
        let deleted = self.changes.deletes.contains(&row);
        let editing = self
            .editing
            .as_ref()
            .is_some_and(|(editing_row, column, _)| {
                *editing_row == row && Some(*column) == data_column
            });
        let changed =
            data_column.is_some_and(|column| self.changes.edits.contains_key(&(row, column)));
        let read_only = !self.accessible_editable || deleted || col == 0;
        let selected = self.accessible_selected_cell == Some((row, col));
        let description = format!(
            "Row {}, column {} of {}. {}{}{}{}",
            self.row_offset + row + 1,
            col + 1,
            self.columns.len(),
            self.columns[col].name,
            if deleted {
                ", pending deletion"
            } else if self.insert_row(row).is_some() {
                ", new row"
            } else {
                ""
            },
            if changed { ", unsaved change" } else { "" },
            if editing { ", editing" } else { "" }
        );
        let mut node = gpui::accesskit::Node::new(if col == 0 {
            gpui::Role::RowHeader
        } else {
            gpui::Role::GridCell
        });
        node.set_label(value);
        node.set_description(description);
        node.set_row_index(row + 1);
        node.set_column_index(col);
        node.set_selected(selected);
        node.set_author_id(format!(
            "dbx-grid-{table_id}-{}-{}",
            self.accessible_row_key(row),
            self.columns[col].key
        ));
        node.set_row_index_text((self.row_offset + row + 1).to_string());
        node.set_column_index_text((col + 1).to_string());
        if read_only {
            node.set_read_only();
        }
        node
    }

    pub(super) fn accessible_cell(
        &self,
        row: usize,
        col: usize,
        cx: &Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let key = self.columns[col].key.clone();
        let target = self.accessible_cell_target(row, col);
        let row_key = target.row_key.clone();
        if self.accessible_logical_tree {
            let geometry = self.accessible_geometry.clone();
            let geometry_key = GridNodeKey::Cell(row_key, key.clone());
            return div()
                .id(SharedString::from(format!("dbx-result-cell-{key}")))
                .on_prepaint(move |bounds, window, _| {
                    geometry
                        .borrow_mut()
                        .record(geometry_key.clone(), bounds, window);
                });
        }
        let node = self.accessible_cell_node(row, col, cx.entity_id());
        let value = node.label().unwrap_or_default().to_owned();
        let description = node.description().unwrap_or_default().to_owned();
        let read_only = node.is_read_only();
        let selected = node.is_selected().unwrap_or(false);
        let editing = self
            .editing
            .as_ref()
            .is_some_and(|(editing_row, column, _)| {
                *editing_row == row && Some(*column) == self.result_column(col)
            });
        let table = cx.entity();
        let author_id = format!("dbx-grid-{}-{row_key}-{key}", table.entity_id());
        let row_index_text = (self.row_offset + row + 1).to_string();
        let column_index_text = (col + 1).to_string();
        let mut cell = div()
            .id(SharedString::from(format!("dbx-result-cell-{key}")))
            .role(if col == 0 {
                gpui::Role::RowHeader
            } else {
                gpui::Role::GridCell
            })
            .aria_label(value)
            .aria_description(description)
            .aria_column_index(col)
            .aria_row_index(row + 1)
            .aria_selected(selected)
            .a11y_synthetic_children(move |builder| {
                builder.parent_node().set_author_id(author_id.clone());
                builder
                    .parent_node()
                    .set_row_index_text(row_index_text.clone());
                builder
                    .parent_node()
                    .set_column_index_text(column_index_text.clone());
                if read_only {
                    builder.parent_node().set_read_only();
                }
            })
            .when(self.accessible_focused && selected && !editing, |cell| {
                cell.aria_active_descendant()
            });
        for action in [
            gpui::AccessibleAction::Focus,
            gpui::AccessibleAction::Click,
            gpui::AccessibleAction::ScrollIntoView,
        ] {
            let table = table.clone();
            let target = target.clone();
            cell = cell.on_a11y_action(action, move |_, window, cx| {
                table.update(cx, |table, cx| {
                    let Some(col) = table.delegate().resolve_accessible_cell_target(&target) else {
                        return;
                    };
                    table.delegate_mut().context_column = table.delegate().result_column(col);
                    table.set_selected_cell(row, col, cx);
                    table.focus_handle(cx).focus(window, cx);
                });
            });
        }
        cell
    }
}

fn row_fingerprint(values: &impl serde::Serialize) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    // Tagged serialization preserves NULL, strings and numeric types.
    serde_json::to_vec(values)
        .expect("cell values serialize")
        .hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::{MutationValue, RowData};

    fn result() -> QueryResult {
        let mut key = ColumnInfo::result("id", 0, "INTEGER");
        key.primary_key = true;
        QueryResult {
            columns: vec![key, ColumnInfo::result("name", 1, "TEXT")],
            rows: vec![
                RowData::new(vec![CellValue::Integer(1), CellValue::Text("b".into())]),
                RowData::new(vec![CellValue::Integer(1), CellValue::Text("a".into())]),
                RowData::new(vec![CellValue::Integer(2), CellValue::Text("c".into())]),
                RowData::new(vec![CellValue::Integer(2), CellValue::Text("c".into())]),
            ],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn identities_follow_rows_across_sorting_and_distinguish_repeated_keys() {
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(Some(Arc::new(result())), &HashMap::new(), None, &[], &[]);
        delegate.ensure_accessible_rows();
        let before = delegate.accessible_rows.clone();
        assert_eq!(
            before
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
        delegate.sort_locally(1, Some(OrderDirection::Ascending));
        delegate.ensure_accessible_rows();
        assert_eq!(
            delegate.accessible_rows,
            vec![before[1], before[0], before[2], before[3]]
        );
        delegate.sort_locally(1, None);
        delegate.ensure_accessible_rows();
        assert_eq!(delegate.accessible_rows, before);
        delegate.set_row_offset(1000);
        delegate.ensure_accessible_rows();
        assert!(
            delegate
                .accessible_rows
                .iter()
                .all(|key| !before.contains(key))
        );
    }

    #[test]
    fn indistinguishable_rows_on_other_pages_do_not_alias_previous_accessible_nodes() {
        let mut delegate = ResultTableDelegate::default();
        let page = QueryResult {
            columns: vec![ColumnInfo::result("name", 0, "TEXT")],
            rows: vec![RowData::new(vec![CellValue::Text("same".into())]); 2],
            ..QueryResult::empty(None, 0)
        };
        delegate.set_result(
            Some(Arc::new(page.clone())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate.ensure_accessible_rows();
        let first_page = delegate.accessible_rows.clone();
        delegate.set_row_offset(1000);
        delegate.set_result(Some(Arc::new(page)), &HashMap::new(), None, &[], &[]);
        delegate.ensure_accessible_rows();
        assert!(
            delegate
                .accessible_rows
                .iter()
                .all(|key| !first_page.contains(key)),
            "different pages must not reuse an indistinguishable row's node identity"
        );
    }

    #[test]
    fn query_key_metadata_does_not_alias_distinct_join_rows_on_other_pages() {
        let mut delegate = ResultTableDelegate::default();
        let mut key = ColumnInfo::result("id", 0, "INTEGER");
        key.primary_key = true;
        let page = |name: &str| QueryResult {
            columns: vec![key.clone(), ColumnInfo::result("name", 1, "TEXT")],
            rows: vec![RowData::new(vec![
                CellValue::Integer(1),
                CellValue::Text(name.into()),
            ])],
            ..QueryResult::empty(None, 0)
        };
        delegate.set_result(
            Some(Arc::new(page("first join match"))),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate.ensure_accessible_rows();
        let first = delegate.accessible_row_key(0);
        delegate.set_row_offset(1000);
        delegate.set_result(
            Some(Arc::new(page("second join match"))),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate.ensure_accessible_rows();
        assert_ne!(
            first,
            delegate.accessible_row_key(0),
            "a query can repeat catalog primary keys across pages"
        );
    }

    #[test]
    fn table_primary_keys_survive_reloads_but_ambiguous_snapshots_and_stale_actions_do_not() {
        let mut delegate = ResultTableDelegate {
            is_data_result: true,
            ..Default::default()
        };
        let mut page = result();
        delegate.set_table_columns(&page.columns);
        for column in &mut page.columns {
            column.primary_key = false;
        }
        page.rows = vec![RowData::new(vec![
            CellValue::Integer(1),
            CellValue::Text("before".into()),
        ])];
        delegate.set_result(
            Some(Arc::new(page.clone())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate.ensure_accessible_rows();
        let key = delegate.accessible_row_key(0);
        let old_target = delegate.accessible_cell_target(0, 2);
        assert_eq!(
            delegate.resolve_accessible_cell_target(&old_target),
            Some(2)
        );
        page.rows[0].values[1] = CellValue::Text("after".into());
        delegate.set_result(
            Some(Arc::new(page.clone())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate.ensure_accessible_rows();
        assert_eq!(
            delegate.accessible_row_key(0),
            key,
            "valid table keys survive value changes"
        );
        assert_eq!(
            delegate.resolve_accessible_cell_target(&old_target),
            None,
            "queued old callbacks cannot act on a replaced snapshot"
        );
        let current = delegate.accessible_cell_target(0, 2);
        assert_eq!(delegate.resolve_accessible_cell_target(&current), Some(2));
        delegate.is_data_result = false;
        delegate.set_result(
            Some(Arc::new(page.clone())),
            &HashMap::new(),
            None,
            &[],
            &[],
        );
        delegate.ensure_accessible_rows();
        let ambiguous = delegate.accessible_row_key(0);
        let retained = delegate.result.clone();
        delegate.set_result(retained, &HashMap::new(), None, &[], &[]);
        delegate.ensure_accessible_rows();
        assert_eq!(
            delegate.accessible_row_key(0),
            ambiguous,
            "layout refresh retains the same source identity"
        );
        delegate.set_result(Some(Arc::new(page)), &HashMap::new(), None, &[], &[]);
        delegate.ensure_accessible_rows();
        assert_ne!(
            delegate.accessible_row_key(0),
            ambiguous,
            "a new ambiguous snapshot must not reuse a previous node"
        );
    }

    #[test]
    fn null_or_incomplete_primary_keys_are_not_trusted_across_pages() {
        for values in [
            vec![CellValue::Null, CellValue::Text("same".into())],
            vec![],
        ] {
            let mut delegate = ResultTableDelegate {
                is_data_result: true,
                ..Default::default()
            };
            let mut page = result();
            delegate.set_table_columns(&page.columns);
            page.rows = vec![RowData::new(values)];
            delegate.set_result(Some(Arc::new(page)), &HashMap::new(), None, &[], &[]);
            delegate.ensure_accessible_rows();
            let before = delegate.accessible_row_key(0);
            delegate.set_row_offset(1000);
            delegate.ensure_accessible_rows();
            assert_ne!(delegate.accessible_row_key(0), before);
        }
    }

    #[test]
    fn new_row_identity_survives_editing_and_removing_a_previous_insert() {
        let mut delegate = ResultTableDelegate::default();
        let insert = |n| vec![Some(MutationValue::parameter(CellValue::Integer(n)))];
        let mut changes = GridChanges {
            inserts: vec![insert(1), insert(2)],
            ..Default::default()
        };
        delegate.set_cell_edits(changes.clone(), None);
        let first = delegate.accessible_row_key(0);
        let second = delegate.accessible_row_key(1);
        changes.inserts[1] = insert(3);
        delegate.set_cell_edits(changes.clone(), None);
        assert_eq!(delegate.accessible_row_key(1), second);
        changes.inserts.remove(0);
        delegate.set_cell_edits(changes, None);
        assert_eq!(delegate.accessible_row_key(0), second);
        assert_ne!(delegate.accessible_row_key(0), first);
    }
}
