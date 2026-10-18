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

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::{ForeignKeyInfo, RelationalTable, TableStructure};

    fn schema() -> Arc<RelationalSchema> {
        let users = RelationalTable {
            table: TableInfo::table("users", Some("public".into())),
            structure: TableStructure {
                columns: (0..22)
                    .map(|index| ColumnInfo::result(format!("column_{index}"), index, "TEXT"))
                    .collect(),
                ..Default::default()
            },
        };
        let posts = RelationalTable {
            table: TableInfo::table("posts", Some("public".into())),
            structure: TableStructure {
                columns: vec![
                    ColumnInfo::result("user_a", 0, "TEXT"),
                    ColumnInfo::result("user_b", 1, "TEXT"),
                ],
                foreign_keys: vec![ForeignKeyInfo {
                    constraint_name: None,
                    columns: vec!["user_a".into(), "user_b".into()],
                    referenced_schema: Some("public".into()),
                    referenced_table: "users".into(),
                    referenced_columns: vec!["column_0".into(), "column_1".into()],
                    on_update: None,
                    on_delete: None,
                }],
                ..Default::default()
            },
        };
        Arc::new(RelationalSchema {
            database: "test".into(),
            tables: vec![users, posts],
            ..Default::default()
        })
    }

    #[gpui::test]
    fn keyboard_list_selects_moves_and_resets_the_real_diagram_and_scales_full_columns(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        cx.update(|cx| {
            cx.bind_keys([
                gpui::KeyBinding::new("down", DiagramPanDown, Some("DbxDiagram")),
                gpui::KeyBinding::new("up", DiagramPanUp, Some("DbxDiagram")),
                gpui::KeyBinding::new("shift-down", DiagramPanDownLarge, Some("DbxDiagram")),
            ])
        });
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let source = schema();
        let document = Arc::new(DiagramDocument::from_schema(&source));
        let first = document.nodes[0].id.clone();
        let second = document.nodes[1].id.clone();
        let before = document.node_position(&first).unwrap();
        let routes = document.edges[0].points.clone();
        let directory = tempfile::tempdir().unwrap();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.settings_store = Some(SettingsStore::at(directory.path().join("settings.json")));
            app.vault_state = Some(VaultState::Unlocked);
            app.connection_picker_open = false;
            app.saved_connections.clear();
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Diagram test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            let mut diagram = DiagramTab::loading(DatabaseKind::SQLite, &[], None, cx);
            diagram.document = Some(document);
            diagram.source_schema = Some(source);
            diagram.busy = false;
            diagram.list_view = true;
            diagram.focus.focus(window, cx);
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Diagram(Box::new(diagram)),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Diagram;
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for key in ["down", "up", "up"] {
            cx.simulate_keystrokes(key);
        }
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                assert_eq!(
                    app.active_diagram_tab_mut(session_id)
                        .unwrap()
                        .selected_node
                        .as_deref(),
                    Some(first.as_str())
                )
            })
        });
        cx.simulate_keystrokes("secondary-shift-right");
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                let diagram = app.active_diagram_tab_mut(session_id).unwrap();
                assert_eq!(
                    diagram.document.as_ref().unwrap().node_position(&first),
                    Some((before.0 + 20., before.1))
                );
                assert_ne!(diagram.document.as_ref().unwrap().edges[0].points, routes);
                assert!(diagram.arranged_positions.contains_key(&first));
            })
        });
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("down");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert_eq!(
                    app.active_diagram_tab_mut(session_id)
                        .unwrap()
                        .selected_node
                        .as_deref(),
                    Some(second.as_str())
                );
                app.reset_diagram_layout_for(session_id, cx);
                assert_eq!(
                    app.active_diagram_tab_mut(session_id)
                        .unwrap()
                        .document
                        .as_ref()
                        .unwrap()
                        .node_position(&first),
                    Some(before)
                );
                app.set_text_scale(200, false, window, cx);
            })
        });
        cx.simulate_resize(gpui::size(px(800.), px(600.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let header = cx.debug_bounds("explorer-header").unwrap();
        let actions = cx.debug_bounds("explorer-more").unwrap();
        let title = cx.debug_bounds("explorer-title").unwrap();
        assert!(
            title.size.width >= crate::theme::ui_px(40.),
            "enlarged Explorer title must remain visible: {title:?}"
        );
        assert!(title.right() <= header.right());

        assert!(
            actions.left() >= header.left() && actions.right() <= header.right(),
            "explorer actions must remain inside their visible header at 200%: {actions:?}, {header:?}"
        );
        let last_column = cx
            .debug_bounds("diagram-column-public.users-21")
            .expect("list includes columns omitted by the canvas");
        assert!(last_column.size.height >= px(24.));
        let open = cx.debug_bounds("diagram-open-public.users").unwrap();
        assert!(
            open.left() >= px(0.) && open.right() <= px(800.),
            "list controls fit at 200%: {open:?}"
        );
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.set_text_scale(100, false, window, cx))
        });
        crate::theme::set_text_scales(100, 100);
    }
}
