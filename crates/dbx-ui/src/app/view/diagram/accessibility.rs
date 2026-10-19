use super::*;
use crate::theme::ui_px;

fn relationship_label(document: &DiagramDocument, edge: &crate::diagram::DiagramEdge) -> String {
    let table_name = |id: &str| {
        document
            .nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| table_sidebar_label(&node.table, None))
            .unwrap_or_else(|| id.to_owned())
    };
    format!(
        "{} ({}) references {} ({}).{}",
        table_name(&edge.source),
        edge.source_columns.join(", "),
        table_name(&edge.target),
        edge.target_columns.join(", "),
        if edge.optional {
            " All reference columns allow NULL."
        } else {
            ""
        }
    )
}

impl DbxApp {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_diagram_list(
        &self,
        session_id: SessionId,
        document: Arc<DiagramDocument>,
        source: Option<Arc<RelationalSchema>>,
        selected: Option<String>,
        scroll: ScrollHandle,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = focus.is_focused(window);
        div().id("diagram-table-list").role(gpui::Role::List)
            .aria_label("Tables and relationships")
            .aria_description(format!("{} tables and {} relationships. Up and Down select tables. Tab reaches Open and layout controls.", document.nodes.len(), document.edges.len()))
            .flex_1().min_h_0().min_w_0().overflow_y_scroll().track_scroll(&scroll)
            .p(ui_px(12.)).flex().flex_col().gap(ui_px(12.))
            .children(document.nodes.iter().enumerate().map(|(index, node)| {
                let label = table_sidebar_label(&node.table, None);
                let id = node.id.clone();
                let active = selected.as_deref() == Some(id.as_str());
                let table = node.table.clone();
                let columns = source.as_ref().and_then(|schema| schema.tables.iter().find(|table| table.table == node.table))
                    .map(|table| table.structure.columns.iter().map(|column| format!("{} · {}{}{}", column.name, column.data_type,
                        if column.primary_key { " · primary key" } else { "" }, if column.nullable { " · NULL allowed" } else { " · required" })).collect::<Vec<_>>())
                    .unwrap_or_else(|| node.columns.iter().map(|column| format!("{} · {}", column.name, column.data_type)).collect());
                let relationships = document.edges.iter().filter(|edge| edge.source == id || edge.target == id)
                    .map(|edge| (edge.id.clone(), relationship_label(&document, edge))).collect::<Vec<_>>();
                let selection_id = id.clone();
                let row = div().id(SharedString::from(format!("diagram-list-row-{id}")))
                    .role(gpui::Role::ListItem).aria_label(label.clone()).aria_selected(active)
                    .aria_description(format!("{} columns, {} relationships. Card position {}, {}.", columns.len(), relationships.len(), node.x, node.y))
                    .when(active && focused, |row| row.aria_active_descendant())
                    .flex_none().min_w_0().p(ui_px(14.)).rounded(ui_px(RADIUS_PANEL)).border_1()
                    .border_color(if active { theme().focus_ring } else { theme().input_border }).bg(theme().panel)
                    .flex().flex_col().gap(ui_px(12.))
                    .child(div().flex().flex_wrap().items_center().gap(ui_px(8.))
                        .child(button(SharedString::from(format!("diagram-select-{id}")), label.clone(), ButtonKind::Quiet)
                            .accessible_name(format!("Select {label}"))
                            .debug_selector(move || format!("diagram-select-{selection_id}"))
                            .on_click(cx.listener({ let id = id.clone(); move |this, _, window, cx| { this.select_diagram_node_for(session_id, Some(id.clone()), cx); if let Some(diagram) = this.active_diagram_tab_mut(session_id) { diagram.focus.focus(window, cx); } } })))
                        .child(button(SharedString::from(format!("diagram-open-{id}")), "Open table", ButtonKind::Quiet)
                            .accessible_name(format!("Open table {label}"))
                            .debug_selector({ let id = id.clone(); move || format!("diagram-open-{id}") })
                            .on_click(cx.listener(move |this, _, window, cx| this.open_diagram_table_for(session_id, table.clone(), window, cx))))
                        .children([("←", "left", -20., 0.), ("→", "right", 20., 0.), ("↑", "up", 0., -20.), ("↓", "down", 0., 20.)]
                            .into_iter().map(|(glyph, direction, dx, dy)| {
                                let id = id.clone();
                                button(SharedString::from(format!("diagram-move-{id}-{direction}")), glyph, ButtonKind::Quiet)
                                    .accessible_name(format!("Move {label} {direction}"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.move_diagram_node_by_for(session_id, &id, dx, dy, cx)))
                            })))
                    .child(div().id("columns").role(gpui::Role::List).aria_label(format!("{label} columns"))
                        .flex().flex_col().gap(ui_px(4.))
                        .children(columns.into_iter().enumerate().map(|(index, text)| div().id(("column", index))
                            .debug_selector({ let id = id.clone(); move || format!("diagram-column-{id}-{index}") })
                            .role(gpui::Role::ListItem).aria_label(text.clone()).text_size(ui_px(12.)).child(text))))
                    .child(div().id("relationships").role(gpui::Role::List).aria_label(format!("{label} relationships"))
                        .flex().flex_col().gap(ui_px(6.))
                        .when(relationships.is_empty(), |view| view.child(div().id("no-relationships").role(gpui::Role::Label).aria_label("No relationships").text_size(ui_px(12.)).child("No relationships")))
                        .children(relationships.into_iter().map(|(id, text)| div().id(SharedString::from(id))
                            .role(gpui::Role::ListItem).aria_label(text.clone()).text_size(ui_px(12.)).child(text))));
                crate::accessibility::focus_reveal(("diagram-list-focus", index), &scroll, row)
            })).into_any_element()
    }
}
