//! The data tab changeset. Inline cell edits, inspector edits, new rows and
//! deletes are staged locally and committed together: native SQL engines
//! apply the whole set in one transaction, with primary-key guarded,
//! original-value checked updates and deletes.

use std::collections::{BTreeMap, BTreeSet};

use dbx_core::{MutationValue, RowChange};

use super::result_table::GridChanges;
use super::*;
use crate::editor::EditorLanguage;
use crate::row_drafts::{field_editor_text, parse_field_value};

/// The combined context lets the cell's Enter/Tab/Escape bindings sit at the
/// editor's own dispatch depth.
pub(super) const CELL_EDITOR_CONTEXT: &str = "DbxTextEditor DbxCellEditor";

/// Staged values keyed by (row, data column) in the loaded page.
pub(super) type PendingEdits = BTreeMap<(usize, usize), MutationValue>;

/// A staged new row. Values are keyed by data column; a missing column keeps
/// its database default.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PendingInsert {
    pub(super) values: BTreeMap<usize, MutationValue>,
}

pub(super) struct CellEditor {
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) editor: Entity<TextEditor>,
    pub(super) structured: bool,
    _blur: Subscription,
}

/// Counts for the changes bar.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ChangeCounts {
    pub(super) edited: usize,
    pub(super) inserted: usize,
    pub(super) deleted: usize,
}

impl ChangeCounts {
    pub(super) fn total(self) -> usize {
        self.edited + self.inserted + self.deleted
    }
}

impl DataTab {
    pub(super) fn has_pending_edits(&self) -> bool {
        !self.pending_edits.is_empty()
            || !self.pending_inserts.is_empty()
            || !self.pending_deletes.is_empty()
    }

    pub(super) fn has_unsaved_cell_work(&self) -> bool {
        self.has_pending_edits() || self.cell_editor.is_some()
    }

    pub(super) fn change_counts(&self) -> ChangeCounts {
        let edited = self
            .pending_edits
            .keys()
            .map(|(row, _)| *row)
            .filter(|row| !self.pending_deletes.contains(row))
            .collect::<BTreeSet<_>>()
            .len();
        ChangeCounts {
            edited,
            inserted: self.pending_inserts.len(),
            deleted: self.pending_deletes.len(),
        }
    }

    fn loaded_rows(&self) -> usize {
        self.result.as_ref().map_or(0, |result| result.rows.len())
    }

    /// Grid rows: the loaded page followed by staged new rows.
    pub(super) fn loaded_rows_with_inserts(&self) -> usize {
        self.loaded_rows() + self.pending_inserts.len()
    }

    /// The staged insert a grid row shows, if any.
    pub(super) fn insert_index(&self, row: usize) -> Option<usize> {
        row.checked_sub(self.loaded_rows())
            .filter(|index| *index < self.pending_inserts.len())
    }

    /// Mirror the changeset, marks and open editor into the grid delegate.
    pub(super) fn sync_cell_edits(&self, cx: &mut Context<DbxApp>) {
        let columns = self
            .result
            .as_ref()
            .map_or(0, |result| result.columns.len());
        let changes = GridChanges {
            edits: self
                .pending_edits
                .iter()
                .map(|(key, value)| (*key, value.clone()))
                .collect(),
            inserts: self
                .pending_inserts
                .iter()
                .map(|insert| {
                    (0..columns)
                        .map(|column| insert.values.get(&column).cloned())
                        .collect()
                })
                .collect(),
            deletes: self.pending_deletes.clone(),
            marked: self.marked_rows.clone(),
        };
        let editing = self
            .cell_editor
            .as_ref()
            .filter(|editor| !editor.structured)
            .map(|editor| (editor.row, editor.column, editor.editor.clone()));
        self.data_grid.update(cx, |table, cx| {
            table.delegate_mut().set_cell_edits(changes, editing);
            table.refresh(cx);
            cx.notify();
        });
    }

    /// The value a cell currently shows: staged, or as loaded.
    fn staged_or_loaded(&self, row: usize, column: usize) -> Option<StagedCell> {
        if let Some(index) = self.insert_index(row) {
            return Some(match self.pending_insert_value(index, column) {
                Some(value) => StagedCell::Value(value),
                None => StagedCell::Default,
            });
        }
        if let Some(value) = self.pending_edits.get(&(row, column)) {
            return Some(StagedCell::Value(value.clone()));
        }
        let value = self.result.as_ref()?.rows.get(row)?.values.get(column)?;
        Some(StagedCell::Value(MutationValue::Parameter(value.clone())))
    }

    /// The value the grid shows for a cell, for the inspector. SQL
    /// expressions show as typed; a new row's default shows as `None`.
    pub(super) fn shown_value(&self, row: usize, column: usize) -> Option<CellValue> {
        match self.staged_or_loaded(row, column)? {
            StagedCell::Value(MutationValue::Parameter(value)) => Some(value),
            StagedCell::Value(MutationValue::Expression(expression)) => {
                Some(CellValue::Text(expression))
            }
            StagedCell::Default => None,
        }
    }

    fn pending_insert_value(&self, index: usize, column: usize) -> Option<MutationValue> {
        self.pending_inserts
            .get(index)?
            .values
            .get(&column)
            .cloned()
    }

    /// Stage `value` for a cell. Restoring a loaded row's original value
    /// unstages it.
    pub(super) fn stage_cell(&mut self, row: usize, column: usize, value: Option<MutationValue>) {
        if let Some(index) = self.insert_index(row) {
            let insert = &mut self.pending_inserts[index];
            match value {
                Some(value) => insert.values.insert(column, value),
                None => insert.values.remove(&column),
            };
            return;
        }
        let original = self
            .result
            .as_ref()
            .and_then(|result| result.rows.get(row)?.values.get(column))
            .cloned();
        match value {
            Some(MutationValue::Parameter(value)) if Some(&value) == original.as_ref() => {
                self.pending_edits.remove(&(row, column));
            }
            Some(value) => {
                self.pending_edits.insert((row, column), value);
            }
            None => {
                self.pending_edits.remove(&(row, column));
            }
        }
    }
}

enum StagedCell {
    Value(MutationValue),
    /// A new row's column left to its database default.
    Default,
}

impl DbxApp {
    /// Describe why a data tab cannot reload, for a blocking toast.
    pub(super) fn pending_edits_block(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| data.cell_editor.is_some())
        {
            self.show_toast(
                ToastKind::Info,
                "Finish or cancel the open cell edit first",
                cx,
            );
            return true;
        }
        let Some(count) = self
            .data_tab(session_id, tab_id)
            .map(|data| data.change_counts().total())
            .filter(|count| *count > 0)
        else {
            return false;
        };
        self.show_toast(
            ToastKind::Info,
            format!(
                "Commit or discard {} first",
                counted(count, "staged change", "staged changes")
            ),
            cx,
        );
        true
    }

    pub(super) fn begin_cell_edit_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        if data.row_draft.is_some() {
            self.show_toast(ToastKind::Info, "Finish the open row edit first", cx);
            return;
        }
        if data.pending_deletes.contains(&row) {
            self.show_toast(ToastKind::Info, "Restore the row to edit it", cx);
            return;
        }
        if data.staged_or_loaded(row, column).is_none() {
            return;
        }
        if data.cell_editor.is_some() {
            self.commit_cell_edit_for(session_id, tab_id, None, window, cx);
        }
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        // A failed commit keeps its editor open for correction.
        if data.cell_editor.is_some() {
            return;
        }
        let value = match data.staged_or_loaded(row, column) {
            Some(StagedCell::Value(MutationValue::Parameter(value))) => value,
            Some(StagedCell::Value(MutationValue::Expression(expression))) => {
                CellValue::Text(expression)
            }
            Some(StagedCell::Default) | None => CellValue::Null,
        };
        let structured = value_view::json_preview(&value).is_some()
            || data
                .table_columns
                .get(column)
                .is_some_and(|column| column.data_type.to_ascii_lowercase().contains("json"));
        let text = match &value {
            CellValue::Null => String::new(),
            value => value_view::json_preview(value).unwrap_or_else(|| field_editor_text(value)),
        };
        let value = cx.new(|_| text);
        let editor = cx.new(|cx| {
            TextEditor::new_with_language(
                value,
                structured,
                if structured {
                    EditorLanguage::Json
                } else {
                    EditorLanguage::PlainText
                },
                window,
                cx,
            )
        });
        let focus = editor.read(cx).focus_handle();
        let blur = cx.on_blur(&focus, window, move |this, window, cx| {
            if !structured {
                this.commit_cell_edit_for(session_id, tab_id, None, window, cx);
            }
        });
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.cell_editor = Some(CellEditor {
            row,
            column,
            editor: editor.clone(),
            structured,
            _blur: blur,
        });
        data.sync_cell_edits(cx);
        editor.update(cx, |editor, cx| editor.select_all_text(cx));
        focus.focus(window, cx);
        cx.notify();
    }

    /// Stage the open editor's value. `advance` moves the editor to the next
    /// (`Some(true)`) or previous (`Some(false)`) column afterwards.
    pub(super) fn commit_cell_edit_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        advance: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        let Some(cell) = data.cell_editor.as_ref() else {
            return;
        };
        let (row, column) = (cell.row, cell.column);
        let structured = cell.structured;
        let text = cell.editor.read(cx).text(cx);
        let Some((metadata, was_null, column_count)) = data.result.as_ref().and_then(|result| {
            let result_column = result.columns.get(column)?;
            let metadata = data
                .table_columns
                .iter()
                .find(|candidate| candidate.name == result_column.name)
                .unwrap_or(result_column)
                .clone();
            let was_null = match data.staged_or_loaded(row, column)? {
                StagedCell::Value(MutationValue::Parameter(value)) => value == CellValue::Null,
                StagedCell::Value(MutationValue::Expression(_)) => false,
                StagedCell::Default => true,
            };
            Some((metadata, was_null, result.columns.len()))
        }) else {
            return;
        };
        // An emptied cell that showed NULL (or a new row's default) keeps it
        // instead of becoming an empty string the user never typed.
        let unchanged_empty = text.is_empty() && was_null;
        if structured
            && !unchanged_empty
            && let Err(error) = serde_json::from_str::<serde_json::Value>(&text)
        {
            self.show_toast(ToastKind::Error, format!("Invalid JSON: {error}"), cx);
            return;
        }
        let staged = if unchanged_empty {
            None
        } else {
            match parse_field_value(&metadata, &text) {
                Ok(value) => Some(Some(MutationValue::Parameter(value))),
                Err(error) => {
                    self.show_toast(ToastKind::Error, error.to_string(), cx);
                    return;
                }
            }
        };
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if let Some(value) = staged {
            data.stage_cell(row, column, value);
        }
        data.cell_editor = None;
        data.sync_cell_edits(cx);
        let next = advance.and_then(|forward| {
            if forward {
                (column + 1 < column_count).then_some(column + 1)
            } else {
                column.checked_sub(1)
            }
        });
        match next {
            Some(next) => self.begin_cell_edit_for(session_id, tab_id, row, next, window, cx),
            None => {
                let grid = data.data_grid.read(cx).focus_handle(cx);
                grid.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_cell_edit_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if data.cell_editor.take().is_none() {
            return;
        }
        data.sync_cell_edits(cx);
        let grid = data.data_grid.read(cx).focus_handle(cx);
        grid.focus(window, cx);
        cx.notify();
    }

    /// Stage NULL for the open editor's cell when its column allows it.
    pub(super) fn set_cell_null_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        let Some(cell) = data.cell_editor.as_ref() else {
            return;
        };
        let (row, column) = (cell.row, cell.column);
        let Some(nullable) = data.result.as_ref().and_then(|result| {
            let name = &result.columns.get(column)?.name;
            Some(
                data.table_columns
                    .iter()
                    .find(|candidate| candidate.name == *name)
                    .is_none_or(|candidate| candidate.nullable),
            )
        }) else {
            return;
        };
        if !nullable {
            self.show_toast(ToastKind::Error, "This column does not allow NULL", cx);
            return;
        }
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.stage_cell(row, column, Some(MutationValue::Parameter(CellValue::Null)));
        data.cell_editor = None;
        data.sync_cell_edits(cx);
        let grid = data.data_grid.read(cx).focus_handle(cx);
        grid.focus(window, cx);
        cx.notify();
    }

    pub(super) fn discard_pending_edits_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let had_inserts = !data.pending_inserts.is_empty();
        data.pending_edits.clear();
        data.pending_inserts.clear();
        data.pending_deletes.clear();
        data.cell_editor = None;
        if had_inserts
            && data
                .selected_row
                .is_some_and(|row| row >= data.loaded_rows())
        {
            data.selected_row = None;
            data.clear_grid_selection(cx);
        }
        data.sync_cell_edits(cx);
        cx.notify();
    }

    /// Track Shift/Cmd-click marks for bulk row actions.
    pub(super) fn mark_row_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: usize,
        extend: bool,
        toggle: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        match (extend, data.mark_anchor) {
            (true, Some(anchor)) => {
                data.marked_rows = (anchor.min(row)..=anchor.max(row)).collect();
            }
            _ if toggle => {
                if data.marked_rows.is_empty()
                    && let Some(selected) = data.selected_row
                {
                    data.marked_rows.insert(selected);
                }
                if !data.marked_rows.remove(&row) {
                    data.marked_rows.insert(row);
                }
                data.mark_anchor = Some(row);
            }
            _ => {
                data.marked_rows = BTreeSet::from([row]);
                data.mark_anchor = Some(row);
            }
        }
        data.sync_cell_edits(cx);
        cx.notify();
    }

    /// The rows a bulk action applies to: the marked rows when `row` is one of
    /// them (or no row is given), otherwise `row` alone.
    fn action_rows(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: Option<usize>,
    ) -> Vec<usize> {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return Vec::new();
        };
        match row {
            Some(row) if !data.marked_rows.contains(&row) => vec![row],
            _ if !data.marked_rows.is_empty() => data.marked_rows.iter().copied().collect(),
            Some(row) => vec![row],
            None => data.selected_row.into_iter().collect(),
        }
    }

    /// Stage deletes for the marked (or given) rows. A row already staged for
    /// deletion is restored instead, and a new row is simply removed.
    pub(super) fn delete_rows_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let rows = self.action_rows(session_id, tab_id, row);
        if rows.is_empty() {
            return;
        }
        // Loaded rows need a primary key to be deleted safely.
        if let Some(data) = self.data_tab(session_id, tab_id)
            && rows.iter().any(|row| *row < data.loaded_rows())
            && let Some(row) = data
                .result
                .as_ref()
                .and_then(|result| result.rows.first())
                .cloned()
            && let Err(error) = self.identity_filters_for(session_id, tab_id, &row)
        {
            self.show_toast(ToastKind::Error, error, cx);
            return;
        }
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if data.row_draft.is_some() {
            return;
        }
        let restore = rows.iter().all(|row| data.pending_deletes.contains(row));
        let loaded = data.loaded_rows();
        let mut removed_inserts = Vec::new();
        for row in rows {
            if row >= loaded {
                removed_inserts.push(row - loaded);
            } else if restore {
                data.pending_deletes.remove(&row);
            } else {
                data.pending_deletes.insert(row);
            }
        }
        removed_inserts.sort_unstable();
        for index in removed_inserts.into_iter().rev() {
            if index < data.pending_inserts.len() {
                data.pending_inserts.remove(index);
            }
        }
        data.marked_rows.clear();
        data.mark_anchor = None;
        if data
            .selected_row
            .is_some_and(|row| row >= loaded + data.pending_inserts.len())
        {
            data.selected_row = None;
            data.clear_grid_selection(cx);
        }
        data.sync_cell_edits(cx);
        cx.notify();
    }

    /// Stage a copy of a row as a new row, leaving primary keys to their
    /// defaults so the copy does not collide with its source.
    pub(super) fn duplicate_row_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let Some(result) = data.result.clone() else {
            return;
        };
        let mut insert = PendingInsert::default();
        for (column, info) in result.columns.iter().enumerate() {
            let primary_key = data
                .table_columns
                .iter()
                .any(|candidate| candidate.name == info.name && candidate.primary_key);
            if primary_key {
                continue;
            }
            if let Some(StagedCell::Value(value)) = data.staged_or_loaded(row, column) {
                insert.values.insert(column, value);
            }
        }
        data.pending_inserts.push(insert);
        let new_row = data.loaded_rows() + data.pending_inserts.len() - 1;
        data.sync_cell_edits(cx);
        data.data_grid.update(cx, |table, cx| {
            table.set_selected_row(new_row, cx);
            table.scroll_to_row(new_row, cx);
        });
        cx.notify();
    }

    /// Drop every staged change for one row.
    pub(super) fn revert_row_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if let Some(index) = data.insert_index(row) {
            data.pending_inserts.remove(index);
            if data.selected_row == Some(row) {
                data.selected_row = None;
                data.clear_grid_selection(cx);
            }
        } else {
            data.pending_edits.retain(|(edited, _), _| *edited != row);
            data.pending_deletes.remove(&row);
        }
        data.sync_cell_edits(cx);
        cx.notify();
    }

    /// Stage a new row from the inspector, or replace the staged row it edits.
    pub(super) fn stage_insert_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        values: Vec<(String, MutationValue)>,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let Some(result) = data.result.clone() else {
            return;
        };
        let mut insert = PendingInsert::default();
        for (name, value) in values {
            if let Some(column) = result.columns.iter().position(|column| column.name == name) {
                insert.values.insert(column, value);
            }
        }
        let index = match data.draft_insert.take() {
            Some(index) if index < data.pending_inserts.len() => {
                data.pending_inserts[index] = insert;
                index
            }
            _ => {
                data.pending_inserts.push(insert);
                data.pending_inserts.len() - 1
            }
        };
        let row = data.loaded_rows() + index;
        data.row_draft = None;
        data.row_draft_subscriptions.clear();
        data.draft_mode = DraftMode::Update;
        data.selected_row = Some(row);
        data.sync_cell_edits(cx);
        data.suppress_next_grid_selection_event = true;
        data.data_grid.update(cx, |table, cx| {
            table.set_selected_row(row, cx);
            table.scroll_to_row(row, cx);
        });
        cx.notify();
    }

    /// Replace a loaded row's staged values with the inspector's changes.
    pub(super) fn stage_update_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: usize,
        assignments: Vec<(String, MutationValue)>,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let Some(result) = data.result.clone() else {
            return;
        };
        data.pending_edits.retain(|(edited, _), _| *edited != row);
        for (name, value) in assignments {
            if let Some(column) = result.columns.iter().position(|column| column.name == name) {
                data.stage_cell(row, column, Some(value));
            }
        }
        data.row_draft = None;
        data.row_draft_subscriptions.clear();
        data.sync_cell_edits(cx);
        cx.notify();
    }

    /// Build the checked changes in commit order: deletes free unique values
    /// before updates and inserts reuse them.
    pub(super) fn row_changes_for(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
    ) -> Result<Vec<RowChange>, String> {
        let table = self
            .editable_table_for(session_id, tab_id)
            .cloned()
            .ok_or("This table cannot be edited")?;
        let data = self
            .data_tab(session_id, tab_id)
            .ok_or("No table is open")?;
        let result = data.result.clone().ok_or("No rows are loaded")?;
        let originals = |row: &RowData, columns: &mut dyn Iterator<Item = usize>| {
            columns
                .map(|column| {
                    (
                        result.columns[column].name.clone(),
                        row.values[column].clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut changes = Vec::new();
        for row_index in &data.pending_deletes {
            let row = result
                .rows
                .get(*row_index)
                .ok_or("A deleted row is no longer loaded")?;
            changes.push(RowChange::Delete {
                table: table.clone(),
                filters: self.identity_filters_for(session_id, tab_id, row)?,
                originals: originals(row, &mut (0..result.columns.len())),
            });
        }
        let mut rows: BTreeMap<usize, Vec<(usize, MutationValue)>> = BTreeMap::new();
        for ((row, column), value) in &data.pending_edits {
            if !data.pending_deletes.contains(row) {
                rows.entry(*row).or_default().push((*column, value.clone()));
            }
        }
        for (row_index, cells) in rows {
            let row = result
                .rows
                .get(row_index)
                .ok_or("An edited row is no longer loaded")?;
            let filters = self.identity_filters_for(session_id, tab_id, row)?;
            changes.push(RowChange::Update {
                request: UpdateRequest::new_with_mutation_values(
                    table.clone(),
                    cells
                        .iter()
                        .map(|(column, value)| {
                            (result.columns[*column].name.clone(), value.clone())
                        })
                        .collect(),
                    filters,
                ),
                originals: originals(row, &mut cells.iter().map(|(column, _)| *column)),
            });
        }
        for insert in &data.pending_inserts {
            changes.push(RowChange::Insert(InsertRequest::from_mutation_row(
                table.clone(),
                insert
                    .values
                    .iter()
                    .map(|(column, value)| (result.columns[*column].name.clone(), value.clone()))
                    .collect(),
            )));
        }
        Ok(changes)
    }

    /// Show the changeset as SQL before committing it.
    pub(super) fn review_changes_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_cell_editor_blocks(session_id, tab_id, window, cx) {
            return;
        }
        let Some(kind) = self.session(session_id).map(|session| session.kind) else {
            return;
        };
        let changes = match self.row_changes_for(session_id, tab_id) {
            Ok(changes) if !changes.is_empty() => changes,
            Ok(_) => return,
            Err(error) => {
                self.show_toast(ToastKind::Error, error, cx);
                return;
            }
        };
        let sql = changes
            .iter()
            .map(|change| dbx_core::render_row_change(kind, change))
            .collect::<dbx_core::Result<Vec<_>>>();
        let sql = match sql {
            Ok(sql) => sql.join("\n"),
            Err(error) => {
                self.show_toast(ToastKind::Error, error.to_string(), cx);
                return;
            }
        };
        let counts = self
            .data_tab(session_id, tab_id)
            .map(DataTab::change_counts)
            .unwrap_or_default();
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: format!("Commit {}?", counted(counts.total(), "change", "changes")),
            detail: String::new(),
            confirm_label: "Commit",
            tone: if counts.deleted > 0 {
                ConfirmationTone::Danger
            } else {
                ConfirmationTone::Warning
            },
            action: ConfirmationAction::CommitChanges { session_id, tab_id },
            focus: focus.clone(),
            return_focus: window.focused(cx),
            sql: Some(sql),
        });
        focus.focus(window, cx);
        cx.notify();
    }

    /// Commit from the changes bar or Cmd+S. Deletes always go through review,
    /// as single-row deletes used to ask for confirmation.
    pub(super) fn request_commit_changes_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| !data.pending_deletes.is_empty())
        {
            self.review_changes_for(session_id, tab_id, window, cx);
        } else {
            self.save_pending_edits_for(session_id, tab_id, window, cx);
        }
    }

    /// Stage the open inline editor before a commit or review, so a typed
    /// value is never silently left out.
    fn pending_cell_editor_blocks(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| data.cell_editor.is_some())
        {
            self.commit_cell_edit_for(session_id, tab_id, None, window, cx);
        }
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return true;
        };
        if data.cell_editor.is_some() {
            return true;
        }
        if data.row_draft.is_some() {
            self.show_toast(ToastKind::Info, "Stage or cancel the open row first", cx);
            return true;
        }
        false
    }

    /// Apply the whole changeset. Every edited or deleted row must still hold
    /// its displayed values; a conflict keeps the changeset for review.
    pub(super) fn save_pending_edits_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_cell_editor_blocks(session_id, tab_id, window, cx) {
            return;
        }
        let Some(engine) = self
            .session(session_id)
            .and_then(|session| session.engine.clone())
        else {
            return;
        };
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        if data.busy || !data.has_pending_edits() {
            return;
        }
        let known = (data.table.clone(), data.table_columns.clone());
        let changes = match self.row_changes_for(session_id, tab_id) {
            Ok(changes) => changes,
            Err(error) => {
                self.show_mutation_error_for(session_id, tab_id, error, None, window, cx);
                return;
            }
        };
        let count = changes.len();
        let runtime = self.runtime.clone();
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.busy = true;
        data.error = None;
        data.status = format!("Committing {}…", counted(count, "change", "changes"));
        data.request_generation += 1;
        let generation = data.request_generation;
        let task = runtime.spawn(async move {
            engine
                .apply_row_changes(&changes, Some((&known.0, &known.1)))
                .await
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let outcome = task
                .await
                .map_err(|error| format!("Row change task failed: {error}"))
                .and_then(|outcome| outcome.map_err(|error| error.to_string()));
            this.update_in(cx, |this, window, cx| {
                let Some(data) = this.data_tab_mut(session_id, tab_id) else {
                    return;
                };
                if generation != data.request_generation {
                    return;
                }
                data.busy = false;
                match outcome {
                    Ok(saved) => {
                        data.pending_edits.clear();
                        data.pending_inserts.clear();
                        data.pending_deletes.clear();
                        data.cell_editor = None;
                        data.sync_cell_edits(cx);
                        this.show_toast(
                            ToastKind::Success,
                            format!("Committed {}", counted(saved, "change", "changes")),
                            cx,
                        );
                        this.refresh_table_for(session_id, tab_id, cx);
                    }
                    Err(error) => {
                        this.show_mutation_error_for(session_id, tab_id, error, None, window, cx);
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::OrderDirection;

    // Tokio database jobs use real threads; drain GPUI completions while
    // waiting for their notifications, with a bounded wall-clock deadline.
    fn wait_for_database_ui(
        cx: &mut gpui::VisualTestContext,
        app: &Entity<DbxApp>,
        predicate: impl Fn(&DbxApp) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            // Backend tasks complete on the test thread, so their JoinHandle
            // wakes obey GPUI's deterministic scheduler contract.
            let runtime = cx.update(|_, cx| app.read(cx).runtime.clone());
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| predicate(app.read(cx))) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "database UI completion timed out"
            );
        }
    }

    #[gpui::test]
    fn json_editor_validates_and_persists_the_complete_document(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app,cx) = cx.add_window_view(|window,cx| {
            let mut app=DbxApp::new(window,cx);
            app.runtime=Arc::new(tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap());
            app.vault_state=Some(VaultState::Unlocked);
            let engine=Arc::new(app.runtime.block_on(DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::SQLite,"sqlite::memory:"))).unwrap());
            app.runtime.block_on(engine.execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, document JSON); INSERT INTO items VALUES(1, '{\"valid\":true}'),(2,NULL)")).unwrap();
            let result=app.runtime.block_on(engine.query("SELECT * FROM items",QueryOptions::default())).unwrap();
            let mut session=ConnectionSession::new(session_id,None,"JSON editor".into(),DatabaseKind::SQLite,None,window,cx);
            session.tables=vec![TableInfo::table("items",None)];
            let mut data=DataTab::new(session_id,tab_id,TableRef::new("items"),true,window,cx);
            data.table_columns=app.runtime.block_on(engine.describe_table(&data.table)).unwrap(); data.result_table=Some(data.table.clone());
            data.set_result(Some(result),&session.tables,cx);
            session.engine=Some(engine);session.secondary_tabs.push(SecondaryTab{id:tab_id,kind:SecondaryTabKind::Data(Box::new(data))});session.active_secondary_tab=Some(tab_id);session.pane=Pane::Data;
            app.sessions.push(session);app.active_session_id=Some(session_id);app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_cell_edit_for(session_id, tab_id, 1, 1, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .as_ref()
                        .unwrap()
                        .structured
                );
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    !app.data_tab(session_id, tab_id)
                        .unwrap()
                        .has_pending_edits(),
                    "Opening a SQL NULL JSON cell must not turn it into JSON null"
                );
            })
        });
        let document = serde_json::json!({"complete":"x".repeat(1200)}).to_string();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_cell_edit_for(session_id, tab_id, 0, 1, window, cx);
                let cell = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap();
                assert!(cell.structured);
                let editor = cell.editor.clone();
                editor.update(cx, |editor, cx| editor.set_text("{invalid", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .is_some()
                );
                assert!(
                    !app.data_tab(session_id, tab_id)
                        .unwrap()
                        .has_pending_edits()
                );
                editor.update(cx, |editor, cx| editor.set_text(&document, cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .is_none()
                );
                app.save_pending_edits_for(session_id, tab_id, window, cx);
            })
        });
        wait_for_database_ui(cx, &app, |app| {
            app.data_tab(session_id, tab_id)
                .is_some_and(|data| !data.busy)
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let data = app.data_tab(session_id, tab_id).unwrap();
            assert!(data.error.is_none(), "{:?}", data.error);
            assert!(!data.has_pending_edits());
            let result = app
                .runtime
                .block_on(
                    app.session(session_id)
                        .unwrap()
                        .engine
                        .as_ref()
                        .unwrap()
                        .query("SELECT document FROM items", QueryOptions::default()),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&result.rows[0].values[0].to_string())
                    .unwrap(),
                serde_json::from_str::<serde_json::Value>(&document).unwrap()
            );
        });
    }

    #[gpui::test]
    fn edits_new_rows_and_deletes_commit_as_one_changeset(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap());
            app.vault_state = Some(VaultState::Unlocked);
            let engine = Arc::new(app.runtime.block_on(DatabaseEngine::connect(ConnectionConfig::new(
                DatabaseKind::SQLite,
                format!("sqlite://{}?mode=rwc", directory.path().join("rows.sqlite").display()),
            ))).unwrap());
            app.runtime.block_on(engine.execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO items VALUES (1, 'one'), (2, 'two'), (3, 'three')")).unwrap();
            let result = app.runtime.block_on(engine.query("SELECT * FROM items ORDER BY id", QueryOptions::default())).unwrap();
            let columns = app.runtime.block_on(engine.describe_table(&TableRef::new("items"))).unwrap();
            let mut session = ConnectionSession::new(session_id, None, "Changeset test".into(), DatabaseKind::SQLite, None, window, cx);
            session.engine = Some(engine);
            session.tables = vec![TableInfo::table("items", None)];
            let mut data = DataTab::new(session_id, tab_id, TableRef::new("items"), true, window, cx);
            data.table_columns = columns;
            data.result_table = Some(data.table.clone());
            data.set_result(Some(result), &session.tables, cx);
            session.secondary_tabs.push(SecondaryTab { id: tab_id, kind: SecondaryTabKind::Data(Box::new(data)) });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Data;
            app.sessions.push(session);
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                // Edit row 1 inline, mark rows 2 and 3 with Shift-click and
                // delete them, then add a new row through the inspector path.
                app.begin_cell_edit_for(session_id, tab_id, 0, 1, window, cx);
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("uno", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                app.mark_row_for(session_id, tab_id, 1, false, false, cx);
                app.mark_row_for(session_id, tab_id, 2, true, false, cx);
                app.delete_rows_for(session_id, tab_id, None, cx);
                app.stage_insert_for(
                    session_id,
                    tab_id,
                    vec![(
                        "name".into(),
                        MutationValue::Parameter(CellValue::Text("four".into())),
                    )],
                    cx,
                );
                app.duplicate_row_for(session_id, tab_id, 0, cx);
                let data = app.data_tab(session_id, tab_id).unwrap();
                assert_eq!(
                    data.change_counts(),
                    ChangeCounts {
                        edited: 1,
                        inserted: 2,
                        deleted: 2
                    }
                );
                // The duplicate copies the staged value, not the primary key.
                assert_eq!(
                    data.pending_inserts[1].values.get(&1),
                    Some(&MutationValue::Parameter(CellValue::Text("uno".into())))
                );
                assert!(!data.pending_inserts[1].values.contains_key(&0));
                // Nothing is written before the commit.
                let kind = app.session(session_id).unwrap().kind;
                let sql = app
                    .row_changes_for(session_id, tab_id)
                    .unwrap()
                    .iter()
                    .map(|change| dbx_core::render_row_change(kind, change).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(sql[0], "DELETE FROM \"items\" WHERE \"id\" = 2;");
                assert_eq!(
                    sql[2],
                    "UPDATE \"items\" SET \"name\" = 'uno' WHERE \"id\" = 1;"
                );
                assert_eq!(sql[3], "INSERT INTO \"items\" (\"name\") VALUES ('four');");
                app.save_pending_edits_for(session_id, tab_id, window, cx);
            })
        });
        wait_for_database_ui(cx, &app, |app| {
            app.data_tab(session_id, tab_id)
                .is_some_and(|data| !data.busy && !data.has_pending_edits())
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let rows = app
                .runtime
                .block_on(
                    app.session(session_id)
                        .unwrap()
                        .engine
                        .as_ref()
                        .unwrap()
                        .query(
                            "SELECT name FROM items ORDER BY name",
                            QueryOptions::default(),
                        ),
                )
                .unwrap();
            let names = rows
                .rows
                .into_iter()
                .map(|row| row.values[0].clone())
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                ["four", "uno", "uno"].map(|name| CellValue::Text(name.into()))
            );
        });
    }

    #[gpui::test]
    fn saving_inline_edits_persists_and_server_sort_resets_paging(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap());
            app.vault_state = Some(VaultState::Unlocked);
            let engine = Arc::new(app.runtime.block_on(DatabaseEngine::connect(ConnectionConfig::new(
                DatabaseKind::SQLite,
                format!("sqlite://{}?mode=rwc", directory.path().join("rows.sqlite").display()),
            ))).unwrap());
            app.runtime.block_on(engine.execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO items VALUES (1, 'original'), (2, 'second')")).unwrap();
            let result = app.runtime.block_on(engine.query("SELECT * FROM items ORDER BY id", QueryOptions::default())).unwrap();
            let columns = app.runtime.block_on(engine.describe_table(&TableRef::new("items"))).unwrap();
            let mut session = ConnectionSession::new(session_id, None, "Persistence test".into(), DatabaseKind::SQLite, None, window, cx);
            session.engine = Some(engine);
            session.tables = vec![TableInfo::table("items", None)];
            let mut data = DataTab::new(session_id, tab_id, TableRef::new("items"), true, window, cx);
            data.table_columns = columns;
            data.result_table = Some(data.table.clone());
            data.set_result(Some(result), &session.tables, cx);
            session.secondary_tabs.push(SecondaryTab { id: tab_id, kind: SecondaryTabKind::Data(Box::new(data)) });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Data;
            app.sessions.push(session);
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.on_data_grid_event(
                    session_id,
                    tab_id,
                    &TableEvent::DoubleClickedCell(0, 2),
                    window,
                    cx,
                );
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("saved", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                app.save_pending_edits_for(session_id, tab_id, window, cx);
            })
        });
        wait_for_database_ui(cx, &app, |app| {
            app.data_tab(session_id, tab_id)
                .is_some_and(|data| !data.busy && !data.has_pending_edits())
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let rows = app
                .runtime
                .block_on(
                    app.session(session_id)
                        .unwrap()
                        .engine
                        .as_ref()
                        .unwrap()
                        .query("SELECT name FROM items WHERE id=1", QueryOptions::default()),
                )
                .unwrap();
            assert_eq!(rows.rows[0].values[0], CellValue::Text("saved".into()));
        });
        for direction in [
            Some(OrderDirection::Ascending),
            Some(OrderDirection::Descending),
            None,
        ] {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.data_tab_mut(session_id, tab_id).unwrap().table_page = 3;
                    app.set_table_sort_for(
                        session_id,
                        tab_id,
                        direction.map(|direction| Order {
                            column: "id".into(),
                            direction,
                        }),
                        cx,
                    );
                })
            });
            wait_for_database_ui(cx, &app, |app| {
                app.data_tab(session_id, tab_id)
                    .is_some_and(|data| !data.busy)
            });
            cx.update(|_, cx| {
                let data = app.read(cx).data_tab(session_id, tab_id).unwrap();
                assert_eq!(data.table_page, 0);
                assert_eq!(
                    data.result.as_ref().unwrap().rows[0].values[0],
                    CellValue::Integer(if direction == Some(OrderDirection::Descending) {
                        2
                    } else {
                        1
                    })
                );
                assert!(data.error.is_none(), "{:?}", data.error);
            });
        }
    }

    #[gpui::test]
    fn inline_edits_stage_validate_block_navigation_and_discard(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            );
            app.vault_state = Some(VaultState::Unlocked);
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Cell test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.tables = vec![TableInfo::table("items", None)];
            let mut id = ColumnInfo::result("id", 0, "INTEGER");
            id.primary_key = true;
            id.nullable = false;
            let columns = vec![id, ColumnInfo::result("name", 1, "TEXT")];
            let mut data =
                DataTab::new(session_id, tab_id, TableRef::new("items"), true, window, cx);
            data.table_columns = columns.clone();
            data.result_table = Some(data.table.clone());
            data.set_result(
                Some(QueryResult {
                    columns,
                    rows: vec![RowData::new(vec![
                        CellValue::Integer(1),
                        CellValue::Text("original".into()),
                    ])],
                    ..QueryResult::empty(None, 0)
                }),
                &session.tables,
                cx,
            );
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Data(Box::new(data)),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Data;
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.on_data_grid_event(
                    session_id,
                    tab_id,
                    &TableEvent::DoubleClickedCell(0, 2),
                    window,
                    cx,
                );
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("changed", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert_eq!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .pending_edits
                        .get(&(0, 1)),
                    Some(&MutationValue::Parameter(CellValue::Text("changed".into())))
                );
                assert!(app.has_pending_lock_work());
                app.close_session(session_id, cx);
                assert!(app.session(session_id).is_some());
                app.set_table_sort_for(
                    session_id,
                    tab_id,
                    Some(Order {
                        column: "id".into(),
                        direction: OrderDirection::Descending,
                    }),
                    cx,
                );
                assert!(app.data_tab(session_id, tab_id).unwrap().sort.is_none());
                app.request_close_secondary_tab_for(session_id, tab_id, window, cx);
                assert!(app.data_tab(session_id, tab_id).is_some());
                app.begin_cell_edit_for(session_id, tab_id, 0, 1, window, cx);
                assert_eq!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .as_ref()
                        .unwrap()
                        .editor
                        .read(cx)
                        .text(cx),
                    "changed"
                );
                app.cancel_cell_edit_for(session_id, tab_id, window, cx);
                app.begin_cell_edit_for(session_id, tab_id, 0, 0, window, cx);
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("invalid integer", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .is_some()
                );
                app.cancel_cell_edit_for(session_id, tab_id, window, cx);
                app.discard_pending_edits_for(session_id, tab_id, cx);
                assert!(
                    !app.data_tab(session_id, tab_id)
                        .unwrap()
                        .has_unsaved_cell_work()
                );
                assert_eq!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap()
                        .rows[0]
                        .values[1],
                    CellValue::Text("original".into())
                );
            })
        });
    }
}
