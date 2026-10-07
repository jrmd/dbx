//! Column and filter shortcuts for data tabs: filter by a cell's value, hide,
//! pin and restore columns, and named filter sets. Layout changes persist per
//! table in the connection's encrypted workspace.

use super::*;
use crate::filters::filter_operator_options;
use crate::workspace::SavedFilter;

impl DbxApp {
    /// Add a filter on `column` from the value in `row`, then reload. Existing
    /// valid filters are kept.
    pub(super) fn filter_by_cell_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        (row, column): (usize, usize),
        operator: FilterOperator,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((name, value)) = self.data_tab(session_id, tab_id).and_then(|data| {
            let result = data.result.as_ref()?;
            Some((
                result.columns.get(column)?.name.clone(),
                result.rows.get(row)?.values.get(column)?.clone(),
            ))
        }) else {
            return;
        };
        let value = match operator {
            FilterOperator::IsNull | FilterOperator::IsNotNull => None,
            _ => Some(value),
        };
        let mut filters = self
            .active_filters_for(session_id, tab_id, cx)
            .unwrap_or_default();
        let filter = Filter::new(name, operator, value);
        if !filters.contains(&filter) {
            filters.push(filter);
        }
        self.load_data_tab_for(session_id, tab_id, filters, window, cx);
    }

    pub(super) fn hide_column_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.result_column_name(session_id, tab_id, column) else {
            return;
        };
        self.update_table_layout_for(session_id, tab_id, cx, |layout| {
            layout.pinned.retain(|pinned| *pinned != name);
            layout.hidden.insert(name);
        });
    }

    /// Show or hide a column from the Columns menu.
    pub(super) fn toggle_column_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        self.update_table_layout_for(session_id, tab_id, cx, |layout| {
            if !layout.hidden.remove(&name) {
                layout.pinned.retain(|pinned| *pinned != name);
                layout.hidden.insert(name);
            }
        });
    }

    pub(super) fn toggle_pin_column_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.result_column_name(session_id, tab_id, column) else {
            return;
        };
        self.update_table_layout_for(session_id, tab_id, cx, |layout| {
            if let Some(index) = layout.pinned.iter().position(|pinned| *pinned == name) {
                layout.pinned.remove(index);
            } else {
                layout.hidden.remove(&name);
                layout.pinned.push(name);
            }
        });
    }

    /// Show every column in table order at its automatic width.
    pub(super) fn reset_table_layout_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.result_column_widths.clear();
        }
        self.update_table_layout_for(session_id, tab_id, cx, |layout| {
            let saved_filters = std::mem::take(&mut layout.saved_filters);
            *layout = Default::default();
            layout.saved_filters = saved_filters;
        });
    }

    fn result_column_name(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        column: usize,
    ) -> Option<String> {
        Some(
            self.data_tab(session_id, tab_id)?
                .result
                .as_ref()?
                .columns
                .get(column)?
                .name
                .clone(),
        )
    }

    /// Save the valid filters under a name built from their conditions.
    pub(super) fn save_current_filters_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let filters = match self.active_filters_for(session_id, tab_id, cx) {
            Ok(filters) if !filters.is_empty() => filters,
            Ok(_) => return,
            Err(error) => {
                self.show_toast(ToastKind::Error, error, cx);
                return;
            }
        };
        let name = filter_set_name(&filters);
        if self.data_tab(session_id, tab_id).is_some_and(|data| {
            data.layout
                .saved_filters
                .iter()
                .any(|saved| saved.filters == filters)
        }) {
            self.show_toast(ToastKind::Info, "These filters are already saved", cx);
            return;
        }
        self.update_table_layout_for(session_id, tab_id, cx, |layout| {
            layout.saved_filters.push(SavedFilter {
                name: name.clone(),
                filters,
            });
        });
        self.show_toast(ToastKind::Success, format!("Saved “{name}”"), cx);
    }

    pub(super) fn apply_saved_filter_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(filters) = self
            .data_tab(session_id, tab_id)
            .and_then(|data| data.layout.saved_filters.get(index))
            .map(|saved| saved.filters.clone())
        else {
            return;
        };
        self.load_data_tab_for(session_id, tab_id, filters, window, cx);
    }

    pub(super) fn delete_saved_filter_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.update_table_layout_for(session_id, tab_id, cx, |layout| {
            if index < layout.saved_filters.len() {
                layout.saved_filters.remove(index);
            }
        });
    }
}

/// A readable name for a filter set, such as `status = active, age > 30`.
fn filter_set_name(filters: &[Filter]) -> String {
    let name = filters
        .iter()
        .map(|filter| {
            let operator = filter_operator_options()
                .iter()
                .find(|option| option.operator == filter.operator)
                .map_or("?", |option| option.label);
            match &filter.value {
                Some(value) => format!("{} {operator} {value}", filter.column),
                None => format!("{} {operator}", filter.column),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match name.char_indices().nth(60) {
        Some((end, _)) => format!("{}…", &name[..end]),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_sets_are_named_from_their_conditions() {
        assert_eq!(
            filter_set_name(&[
                Filter::new(
                    "status",
                    FilterOperator::Equals,
                    Some(CellValue::Text("active".into()))
                ),
                Filter::new("deleted_at", FilterOperator::IsNull, None),
            ]),
            format!(
                "status {} active, deleted_at {}",
                filter_operator_options()
                    .iter()
                    .find(|option| option.operator == FilterOperator::Equals)
                    .unwrap()
                    .label,
                filter_operator_options()
                    .iter()
                    .find(|option| option.operator == FilterOperator::IsNull)
                    .unwrap()
                    .label
            )
        );
    }
}
