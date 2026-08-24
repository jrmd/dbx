use super::super::*;
use crate::diagram::{DiagramPalette, HEADER_HEIGHT, ROW_HEIGHT};
use gpui::{CursorStyle, MouseButton, PathBuilder, canvas};
use gpui_component::checkbox::Checkbox;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tooltip::Tooltip;
const DIAGRAM_VIEWPORT_OVERSCAN: f32 = 256.0;

#[derive(Clone)]
struct DiagramColors {
    canvas: String,
    surface: String,
    surface_muted: String,
    border: String,
    text: String,
    muted_text: String,
    accent: String,
    relation: String,
}

impl DiagramColors {
    fn current() -> Self {
        Self {
            canvas: color_hex(theme().canvas),
            surface: color_hex(theme().panel),
            surface_muted: color_hex(theme().panel_raised),
            border: color_hex(theme().border_strong),
            text: color_hex(theme().text),
            muted_text: color_hex(theme().text_muted),
            accent: color_hex(theme().accent),
            relation: color_hex(theme().text_muted),
        }
    }

    fn palette(&self) -> DiagramPalette<'_> {
        DiagramPalette {
            canvas: &self.canvas,
            surface: &self.surface,
            surface_muted: &self.surface_muted,
            border: &self.border,
            text: &self.text,
            muted_text: &self.muted_text,
            accent: &self.accent,
            relation: &self.relation,
        }
    }
}

impl DbxApp {
    pub(super) fn render_diagram(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((
            document,
            busy,
            stale,
            error,
            zoom,
            selected_node,
            scroll_handle,
            focus,
            dragging,
            kind,
            available_schemas,
            selected_schemas,
        )) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Diagram(diagram) = &tab.kind else {
                return None;
            };
            Some((
                diagram.document.clone(),
                diagram.busy,
                diagram.stale,
                diagram.error.clone(),
                diagram.zoom,
                diagram.selected_node.clone(),
                diagram.scroll_handle.clone(),
                diagram.focus.clone(),
                diagram.drag_anchor.is_some(),
                session.kind,
                diagram.available_schemas.clone(),
                diagram.selected_schemas.clone(),
            ))
        })
        else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().text_muted)
                .child("Open a database diagram from Explorer actions")
                .into_any_element();
        };

        let refresh = cx.entity().downgrade();
        let scroll_offset = scroll_handle.offset();
        let viewport_at_origin = scroll_offset.x == px(0.) && scroll_offset.y == px(0.);
        let window_size = window.bounds().size;
        let sidebar_width = if self.compact_layout { 180.0 } else { 224.0 };
        let pane_width = (f32::from(window_size.width) - sidebar_width - 46.0).max(0.0);
        let available_width = (pane_width - 48.0).max(320.0);
        let available_height = (f32::from(window_size.height) - 198.0).max(240.0);
        let fit_zoom = document
            .as_ref()
            .map(|document| {
                (available_width / document.width)
                    .min(available_height / document.height)
                    .clamp(0.35, 1.0)
            })
            .unwrap_or(1.0);
        let heading = document
            .as_ref()
            .map(|document| {
                format!(
                    "{} tables · {} relationships",
                    document.nodes.len(),
                    document.edges.len()
                )
            })
            .unwrap_or_else(|| "Discovering tables and relationships".into());
        let schema_filter_active = selected_schemas.is_some();
        let schema_filter_control = (kind == DatabaseKind::PostgreSQL
            && !available_schemas.is_empty())
        .then(|| {
            let summary =
                diagram_schema_filter_label(selected_schemas.as_ref(), available_schemas.len());
            let selected_count = selected_schemas
                .as_ref()
                .map_or(available_schemas.len(), BTreeSet::len);
            let all_schemas = cx.entity().downgrade();
            let schema_rows = available_schemas.iter().cloned().map(|schema| {
                let checked = selected_schemas
                    .as_ref()
                    .is_none_or(|selected| selected.contains(&schema));
                let schema_toggle = cx.entity().downgrade();
                let schema_for_click = schema.clone();
                Checkbox::new(SharedString::from(format!("diagram-schema-{schema}")))
                    .w_full()
                    .with_size(Size::Small)
                    .checked(checked)
                    .label(schema)
                    .on_click(move |enabled, _, cx| {
                        let _ = schema_toggle.update(cx, |this, cx| {
                            this.set_diagram_schema_enabled_for(
                                session_id,
                                schema_for_click.clone(),
                                *enabled,
                                cx,
                            );
                        });
                    })
            });

            Popover::new("diagram-schema-filter")
                .p_0()
                .w(px(220.))
                .trigger(
                    Button::new("diagram-schema-filter-trigger")
                        .with_size(Size::XSmall)
                        .compact()
                        .outline()
                        .selected(schema_filter_active)
                        .tooltip("Choose the PostgreSQL schemas shown in this diagram")
                        .label(format!("Schemas · {summary}")),
                )
                .child(
                    div()
                        .px(px(10.))
                        .py(px(8.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(theme().border)
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme().text)
                                .child("Schemas in diagram"),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(theme().text_muted)
                                .child(format!("{selected_count}/{}", available_schemas.len())),
                        ),
                )
                .child(
                    div()
                        .px(px(9.))
                        .py(px(7.))
                        .border_b_1()
                        .border_color(theme().border)
                        .child(
                            Checkbox::new("diagram-schema-all")
                                .w_full()
                                .with_size(Size::Small)
                                .checked(selected_schemas.is_none())
                                .label("All schemas")
                                .on_click(move |enabled, _, cx| {
                                    let _ = all_schemas.update(cx, |this, cx| {
                                        this.set_all_diagram_schemas_for(session_id, *enabled, cx);
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .id("diagram-schema-options")
                        .max_h(px(260.))
                        .overflow_y_scrollbar()
                        .px(px(9.))
                        .py(px(6.))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .children(schema_rows),
                )
                .child(
                    div()
                        .px(px(10.))
                        .py(px(7.))
                        .border_t_1()
                        .border_color(theme().border)
                        .text_size(px(9.))
                        .text_color(theme().text_muted)
                        .child("Applies to this diagram and its exports."),
                )
        });
        let toolbar = div()
            .h(px(48.))
            .flex_none()
            .px(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .border_b_1()
            .border_color(theme().border)
            .bg(theme().panel)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .child(icon(Icon::Diagram, theme().accent))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text)
                                    .child("Database diagram"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(9.))
                                    .text_color(theme().text_muted)
                                    .child(heading),
                            ),
                    )
                    .when(busy || stale, |view| {
                        view.child(
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(if stale {
                                    theme().warning.alpha(0.12)
                                } else {
                                    theme().accent_soft
                                })
                                .text_size(px(9.))
                                .text_color(if stale {
                                    theme().warning
                                } else {
                                    theme().accent
                                })
                                .child(if busy {
                                    if stale { "Refreshing" } else { "Loading" }
                                } else {
                                    "Out of date"
                                }),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .when_some(schema_filter_control, |actions, control| {
                        actions.child(control)
                    })
                    .when_some(document.clone(), |actions, document| {
                        let zoom_out = cx.entity().downgrade();
                        let zoom_in = cx.entity().downgrade();
                        let fit_diagram = cx.entity().downgrade();
                        let export_svg = cx.entity().downgrade();
                        let export_png = cx.entity().downgrade();
                        let colors = DiagramColors::current();
                        let svg_document = document.clone();
                        let svg_colors = colors.clone();
                        let svg_selection = selected_node.clone();
                        let png_document = document.clone();
                        let png_colors = colors.clone();
                        let png_selection = selected_node.clone();
                        actions
                            .child(
                                div()
                                    .h(px(26.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(theme().border)
                                    .bg(theme().panel_raised)
                                    .child(
                                        Button::new("diagram-zoom-out")
                                            .with_size(Size::XSmall)
                                            .compact()
                                            .ghost()
                                            .tooltip("Zoom out (−)")
                                            .label("−")
                                            .disabled(zoom <= 0.35)
                                            .on_click(move |_, _, cx| {
                                                let _ = zoom_out.update(cx, |this, cx| {
                                                    this.set_diagram_zoom_for(
                                                        session_id,
                                                        zoom - 0.15,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                    .child(
                                        div()
                                            .w(px(42.))
                                            .text_center()
                                            .text_size(px(9.))
                                            .text_color(theme().text_muted)
                                            .child(format!("{:.0}%", zoom * 100.0)),
                                    )
                                    .child(
                                        Button::new("diagram-zoom-in")
                                            .with_size(Size::XSmall)
                                            .compact()
                                            .ghost()
                                            .tooltip("Zoom in (=)")
                                            .label("+")
                                            .disabled(zoom >= 2.0)
                                            .on_click(move |_, _, cx| {
                                                let _ = zoom_in.update(cx, |this, cx| {
                                                    this.set_diagram_zoom_for(
                                                        session_id,
                                                        zoom + 0.15,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    ),
                            )
                            .child(
                                Button::new("diagram-fit")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Fit the whole diagram (F)")
                                    .label("Fit")
                                    .disabled((zoom - fit_zoom).abs() < 0.01 && viewport_at_origin)
                                    .on_click(move |_, _, cx| {
                                        let _ = fit_diagram.update(cx, |this, cx| {
                                            this.fit_diagram_for(session_id, fit_zoom, cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new("diagram-export")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .outline()
                                    .label("Export")
                                    .tooltip("Export the whole diagram")
                                    .dropdown_menu(move |menu, _, _| {
                                        let svg_document = svg_document.clone();
                                        let svg_colors = svg_colors.clone();
                                        let svg_selection = svg_selection.clone();
                                        let png_document = png_document.clone();
                                        let png_colors = png_colors.clone();
                                        let png_selection = png_selection.clone();
                                        let export_svg = export_svg.clone();
                                        let export_png = export_png.clone();
                                        menu.item(PopupMenuItem::new("Export SVG…").on_click(
                                            move |_, _, cx| {
                                                let bytes = svg_document
                                                    .svg(
                                                        svg_colors.palette(),
                                                        svg_selection.as_deref(),
                                                    )
                                                    .into_bytes();
                                                let _ = export_svg.update(cx, |this, cx| {
                                                    this.export_diagram_for(
                                                        session_id,
                                                        DiagramExportFormat::Svg,
                                                        bytes,
                                                        cx,
                                                    );
                                                });
                                            },
                                        ))
                                        .item(
                                            PopupMenuItem::new("Export PNG (2×)…").on_click(
                                                move |_, _, cx| match png_document.png(
                                                    &cx.svg_renderer(),
                                                    png_colors.palette(),
                                                    png_selection.as_deref(),
                                                    2.0,
                                                ) {
                                                    Ok(bytes) => {
                                                        let _ =
                                                            export_png.update(cx, |this, cx| {
                                                                this.export_diagram_for(
                                                                    session_id,
                                                                    DiagramExportFormat::Png,
                                                                    bytes,
                                                                    cx,
                                                                );
                                                            });
                                                    }
                                                    Err(error) => {
                                                        let _ =
                                                            export_png.update(cx, |this, cx| {
                                                                this.set_error(format!(
                                                            "Could not render diagram PNG: {error}"
                                                        ));
                                                                cx.notify();
                                                            });
                                                    }
                                                },
                                            ),
                                        )
                                    }),
                            )
                    })
                    .child(
                        Button::new("diagram-refresh")
                            .with_size(Size::XSmall)
                            .compact()
                            .ghost()
                            .tooltip("Refresh diagram (R)")
                            .child(icon(Icon::Refresh, theme().text_muted))
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                let _ = refresh.update(cx, |this, cx| {
                                    this.refresh_diagram_for(session_id, cx)
                                });
                            }),
                    ),
            );

        let show_shortcut_hint = document
            .as_ref()
            .is_some_and(|document| !document.nodes.is_empty());
        let body_content = match document {
            Some(document) if document.nodes.is_empty() => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(Icon::Diagram, theme().text_muted))
                        .child(div().text_size(px(12.)).text_color(theme().text).child(
                            if schema_filter_active {
                                "No tables in the selected schemas"
                            } else {
                                "No relational tables found"
                            },
                        ))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(theme().text_muted)
                                .child(if schema_filter_active {
                                    "Choose another schema or select All schemas."
                                } else {
                                    "Create or import tables, then refresh the diagram."
                                }),
                        ),
                )
                .into_any_element(),
            Some(document) => {
                let scene_padding = DIAGRAM_SCENE_PADDING;
                let scene_width = document.width * zoom + scene_padding * 2.0;
                let scene_height = document.height * zoom + scene_padding * 2.0;
                let visible_scene = diagram_visible_scene_bounds(
                    &document,
                    zoom,
                    scroll_offset,
                    available_width,
                    available_height,
                );
                let related_node_ids = selected_node
                    .as_deref()
                    .map(|selected| {
                        document
                            .edges
                            .iter()
                            .filter(|edge| edge.source == selected || edge.target == selected)
                            .flat_map(|edge| [edge.source.clone(), edge.target.clone()])
                            .collect::<HashSet<_>>()
                    })
                    .unwrap_or_default();
                let edge_routes = document
                    .edges
                    .iter()
                    .filter(|edge| diagram_edge_intersects_scene(&edge.points, visible_scene))
                    .map(|edge| DiagramEdgeRoute {
                        points: edge.points.clone(),
                        highlighted: selected_node.as_deref().is_some_and(|selected| {
                            edge.source == selected || edge.target == selected
                        }),
                    })
                    .collect::<Vec<_>>();
                let relationships = diagram_relationship_canvas(
                    edge_routes,
                    zoom,
                    theme().text_muted,
                    theme().accent,
                )
                .absolute()
                .left(px(scene_padding))
                .top(px(scene_padding))
                .w(px(document.width * zoom))
                .h(px(document.height * zoom));
                let node_cards = document
                    .nodes
                    .iter()
                    .filter(|node| {
                        diagram_rect_intersects_scene(
                            node.x,
                            node.y,
                            node.width,
                            node.height,
                            visible_scene,
                        )
                    })
                    .map(|node| {
                        let node_id = node.id.clone();
                        let table = node.table.clone();
                        let label = format!("{} — double-click to open data", node.id);
                        let selected = selected_node.as_deref() == Some(node.id.as_str());
                        let related = related_node_ids.contains(node.id.as_str());
                        let node_focus = focus.clone();
                        let title_size = (14.0 * zoom).max(7.0);
                        let schema_size = (10.0 * zoom).max(6.0);
                        let row_size = (11.0 * zoom).max(6.0);
                        let key_size = (10.0 * zoom).max(6.0);
                        let horizontal_padding = (12.0 * zoom).max(4.0);
                        let key_width = (26.0 * zoom).max(12.0);
                        let schema = node
                            .table
                            .schema
                            .clone()
                            .unwrap_or_else(|| "default".into());
                        let rows = node.columns.iter().map(|column| {
                            let key = if column.primary_key {
                                "PK"
                            } else if column.foreign_key {
                                "FK"
                            } else {
                                ""
                            };
                            let data_type = format!(
                                "{}{}",
                                column.data_type,
                                if column.nullable { "" } else { " · not null" }
                            );
                            div()
                                .h(px(ROW_HEIGHT * zoom))
                                .px(px(horizontal_padding))
                                .flex_none()
                                .flex()
                                .items_center()
                                .text_size(px(row_size))
                                .child(
                                    div()
                                        .w(px(key_width))
                                        .flex_none()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(key_size))
                                        .text_color(theme().accent)
                                        .child(key),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme().text)
                                        .child(column.name.clone()),
                                )
                                .child(
                                    div()
                                        .ml(px((8.0 * zoom).max(2.0)))
                                        .max_w(px(node.width * zoom * 0.46))
                                        .truncate()
                                        .text_color(theme().text_muted)
                                        .child(data_type),
                                )
                        });
                        let omitted_row = (node.omitted_columns > 0).then(|| {
                            div()
                                .h(px(ROW_HEIGHT * zoom))
                                .px(px(horizontal_padding))
                                .flex_none()
                                .flex()
                                .items_center()
                                .text_size(px(row_size))
                                .text_color(theme().text_muted)
                                .child(format!("+{} more columns", node.omitted_columns))
                        });

                        div()
                            .id(SharedString::from(format!("diagram-node-{}", node.id)))
                            .absolute()
                            .left(px(scene_padding + node.x * zoom))
                            .top(px(scene_padding + node.y * zoom))
                            .w(px(node.width * zoom))
                            .h(px(node.height * zoom))
                            .overflow_hidden()
                            .rounded(px((8.0 * zoom).max(3.0)))
                            .border_1()
                            .border_color(theme().border_strong)
                            .bg(theme().panel)
                            .when(related && !selected, |view| {
                                view.border_color(theme().accent.alpha(0.72))
                            })
                            .when(selected, |view| {
                                view.border_2().border_color(theme().accent)
                            })
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                Tooltip::new(label.clone()).build(window, cx)
                            })
                            .hover(|style| style.bg(theme().selection))
                            .child(
                                div()
                                    .h(px(HEADER_HEIGHT * zoom))
                                    .px(px(horizontal_padding))
                                    .flex_none()
                                    .flex()
                                    .flex_col()
                                    .justify_center()
                                    .bg(theme().panel_raised)
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(title_size))
                                            .text_color(theme().text)
                                            .child(node.table.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(schema_size))
                                            .text_color(theme().text_muted)
                                            .child(schema),
                                    ),
                            )
                            .children(rows)
                            .when_some(omitted_row, |view, row| view.child(row))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_, _, window, cx| {
                                    node_focus.focus(window, cx);
                                    // A card press belongs to selection/drill-in, not canvas panning.
                                    cx.stop_propagation();
                                }),
                            )
                            .on_click(cx.listener(move |this, event, window, cx| {
                                let double_click = matches!(
                                    event,
                                    gpui::ClickEvent::Mouse(mouse) if mouse.up.click_count > 1
                                );
                                if double_click {
                                    this.open_diagram_table_for(
                                        session_id,
                                        table.clone(),
                                        window,
                                        cx,
                                    );
                                } else {
                                    this.select_diagram_node_for(
                                        session_id,
                                        Some(node_id.clone()),
                                        cx,
                                    );
                                }
                            }))
                    });
                div()
                    .id("diagram-scroll")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .border_1()
                    .border_color(theme().canvas)
                    .cursor(if dragging {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    })
                    .on_mouse_down(MouseButton::Left, {
                        let focus = focus.clone();
                        cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                            focus.focus(window, cx);
                            this.begin_diagram_pan_for(session_id, event.position, cx);
                        })
                    })
                    .on_mouse_move(
                        cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                            if event.dragging() {
                                this.pan_diagram_to_for(session_id, event.position, cx);
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.end_diagram_pan_for(session_id, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.end_diagram_pan_for(session_id, cx);
                        }),
                    )
                    .on_mouse_exit(cx.listener(move |this, _, _, cx| {
                        this.end_diagram_pan_for(session_id, cx);
                    }))
                    .overflow_scroll()
                    .track_scroll(&scroll_handle)
                    .bg(theme().canvas)
                    .child(
                        div()
                            .relative()
                            .w(px(scene_width))
                            .h(px(scene_height))
                            .child(relationships)
                            .children(node_cards),
                    )
                    .into_any_element()
            }
            None => {
                let retry = cx.entity().downgrade();
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .max_w(px(460.))
                            .px(px(24.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.))
                            .child(icon(
                                if error.is_some() {
                                    Icon::Diagram
                                } else {
                                    Icon::Refresh
                                },
                                if error.is_some() {
                                    theme().danger
                                } else {
                                    theme().accent
                                },
                            ))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme().text)
                                    .child(if error.is_some() {
                                        "Could not build the diagram"
                                    } else {
                                        "Discovering your schema…"
                                    }),
                            )
                            .when_some(error.clone(), |view, error| {
                                view.child(
                                    div()
                                        .text_center()
                                        .text_size(px(10.))
                                        .text_color(theme().text_muted)
                                        .child(error),
                                )
                                .child(
                                    Button::new("diagram-retry")
                                        .with_size(Size::Small)
                                        .compact()
                                        .outline()
                                        .label("Try again")
                                        .on_click(move |_, _, cx| {
                                            let _ = retry.update(cx, |this, cx| {
                                                this.refresh_diagram_for(session_id, cx)
                                            });
                                        }),
                                )
                            }),
                    )
                    .into_any_element()
            }
        };

        let body = div()
            .id("diagram-shortcuts")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .relative()
            .border_1()
            .border_color(theme().canvas)
            .key_context("DbxDiagram")
            .track_focus(&focus)
            .focus_visible(|style| style.border_color(theme().focus_ring))
            .on_mouse_down(MouseButton::Left, {
                let focus = focus.clone();
                move |_, window, cx| focus.focus(window, cx)
            })
            .on_action(cx.listener(move |this, _: &DiagramPanLeft, _, cx| {
                this.pan_diagram_by_for(session_id, -48.0, 0.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanRight, _, cx| {
                this.pan_diagram_by_for(session_id, 48.0, 0.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanUp, _, cx| {
                this.pan_diagram_by_for(session_id, 0.0, -48.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanDown, _, cx| {
                this.pan_diagram_by_for(session_id, 0.0, 48.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanLeftLarge, _, cx| {
                this.pan_diagram_by_for(session_id, -160.0, 0.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanRightLarge, _, cx| {
                this.pan_diagram_by_for(session_id, 160.0, 0.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanUpLarge, _, cx| {
                this.pan_diagram_by_for(session_id, 0.0, -160.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramPanDownLarge, _, cx| {
                this.pan_diagram_by_for(session_id, 0.0, 160.0, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramZoomIn, _, cx| {
                this.set_diagram_zoom_for(session_id, zoom + 0.15, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramZoomOut, _, cx| {
                this.set_diagram_zoom_for(session_id, zoom - 0.15, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramResetView, _, cx| {
                this.reset_diagram_view_for(session_id, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramFit, _, cx| {
                this.fit_diagram_for(session_id, fit_zoom, cx);
            }))
            .on_action(cx.listener(move |this, _: &DiagramRefresh, _, cx| {
                this.refresh_diagram_for(session_id, cx);
            }))
            .child(body_content)
            .when(show_shortcut_hint, |view| {
                view.child(
                    div()
                        .absolute()
                        .right(px(12.))
                        .bottom(px(10.))
                        .px(px(7.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(theme().panel.alpha(0.92))
                        .border_1()
                        .border_color(theme().border)
                        .text_size(px(9.))
                        .text_color(theme().text_muted)
                        .child(
                            "Drag to pan · arrows move · Shift arrows jump · +/− zoom · F fit · 0 reset",
                        ),
                )
            });

        div()
            .flex_1()
            .w(px(pane_width))
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DiagramSceneBounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn diagram_visible_scene_bounds(
    document: &DiagramDocument,
    zoom: f32,
    scroll_offset: Point<Pixels>,
    available_width: f32,
    available_height: f32,
) -> DiagramSceneBounds {
    let zoom = zoom.max(0.01);
    let document_width = document.width.max(1.0);
    let document_height = document.height.max(1.0);
    let scroll_x = (-f32::from(scroll_offset.x)).max(0.0);
    let scroll_y = (-f32::from(scroll_offset.y)).max(0.0);
    let viewport_left = ((scroll_x - DIAGRAM_SCENE_PADDING) / zoom).clamp(0.0, document_width);
    let viewport_top = ((scroll_y - DIAGRAM_SCENE_PADDING) / zoom).clamp(0.0, document_height);
    let viewport_right = ((scroll_x - DIAGRAM_SCENE_PADDING + available_width.max(1.0)) / zoom)
        .clamp(0.0, document_width);
    let viewport_bottom = ((scroll_y - DIAGRAM_SCENE_PADDING + available_height.max(1.0)) / zoom)
        .clamp(0.0, document_height);

    DiagramSceneBounds {
        left: (viewport_left - DIAGRAM_VIEWPORT_OVERSCAN).max(0.0),
        top: (viewport_top - DIAGRAM_VIEWPORT_OVERSCAN).max(0.0),
        right: (viewport_right + DIAGRAM_VIEWPORT_OVERSCAN).min(document_width),
        bottom: (viewport_bottom + DIAGRAM_VIEWPORT_OVERSCAN).min(document_height),
    }
}

fn diagram_rect_intersects_scene(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    scene: DiagramSceneBounds,
) -> bool {
    left < scene.right
        && left + width > scene.left
        && top < scene.bottom
        && top + height > scene.top
}

fn diagram_edge_intersects_scene(points: &[(f32, f32)], scene: DiagramSceneBounds) -> bool {
    const EDGE_CULL_MARGIN: f32 = 40.0;
    let expanded = DiagramSceneBounds {
        left: scene.left - EDGE_CULL_MARGIN,
        top: scene.top - EDGE_CULL_MARGIN,
        right: scene.right + EDGE_CULL_MARGIN,
        bottom: scene.bottom + EDGE_CULL_MARGIN,
    };
    points.windows(2).any(|segment| {
        let left = segment[0].0.min(segment[1].0);
        let top = segment[0].1.min(segment[1].1);
        let right = segment[0].0.max(segment[1].0);
        let bottom = segment[0].1.max(segment[1].1);
        left <= expanded.right
            && right >= expanded.left
            && top <= expanded.bottom
            && bottom >= expanded.top
    })
}

struct DiagramPaintedEdge {
    line: gpui::Path<Pixels>,
    arrow: Option<gpui::Path<Pixels>>,
    highlighted: bool,
}

struct DiagramEdgeRoute {
    points: Vec<(f32, f32)>,
    highlighted: bool,
}

fn diagram_relationship_canvas(
    routes: Vec<DiagramEdgeRoute>,
    zoom: f32,
    color: Rgba,
    highlight_color: Rgba,
) -> gpui::Canvas<Vec<DiagramPaintedEdge>> {
    canvas(
        move |bounds, _, _| {
            routes
                .into_iter()
                .filter_map(|route| {
                    diagram_painted_edge(bounds, &route.points, zoom, route.highlighted)
                })
                .collect::<Vec<_>>()
        },
        move |_, edges, window, _| {
            for edge in edges {
                let edge_color = if edge.highlighted {
                    highlight_color
                } else {
                    color
                };
                window.paint_path(edge.line, edge_color);
                if let Some(arrow) = edge.arrow {
                    window.paint_path(arrow, edge_color);
                }
            }
        },
    )
}

fn diagram_painted_edge(
    bounds: gpui::Bounds<Pixels>,
    points: &[(f32, f32)],
    zoom: f32,
    highlighted: bool,
) -> Option<DiagramPaintedEdge> {
    let first = *points.first()?;
    let to_canvas = |(x, y): (f32, f32)| {
        point(
            bounds.origin.x + px(x * zoom),
            bounds.origin.y + px(y * zoom),
        )
    };
    let stroke_width = if highlighted { 2.0 } else { 1.5 };
    let mut line = PathBuilder::stroke(px((stroke_width * zoom).max(1.0)));
    line.move_to(to_canvas(first));
    for point in points.iter().skip(1).copied() {
        line.line_to(to_canvas(point));
    }
    let line = line.build().ok()?;

    let arrow = points.windows(2).rev().find_map(|segment| {
        let previous = segment[0];
        let tip = segment[1];
        let dx = tip.0 - previous.0;
        let dy = tip.1 - previous.1;
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            return None;
        }
        let direction = (dx / length, dy / length);
        let arrow_length = (8.0 * zoom).max(4.0);
        let arrow_half_width = (4.0 * zoom).max(2.0);
        let tip = to_canvas(tip);
        let base = point(
            tip.x - px(direction.0 * arrow_length),
            tip.y - px(direction.1 * arrow_length),
        );
        let perpendicular = (-direction.1, direction.0);
        let left = point(
            base.x + px(perpendicular.0 * arrow_half_width),
            base.y + px(perpendicular.1 * arrow_half_width),
        );
        let right = point(
            base.x - px(perpendicular.0 * arrow_half_width),
            base.y - px(perpendicular.1 * arrow_half_width),
        );
        let mut arrow = PathBuilder::fill();
        arrow.add_polygon(&[tip, left, right], true);
        arrow.build().ok()
    });

    Some(DiagramPaintedEdge {
        line,
        arrow,
        highlighted,
    })
}

fn color_hex(color: Rgba) -> String {
    format!("#{:06x}", u32::from(color) >> 8)
}

fn diagram_schema_filter_label(
    selected_schemas: Option<&BTreeSet<String>>,
    available_count: usize,
) -> String {
    let Some(selected_schemas) = selected_schemas else {
        return "All".into();
    };
    match selected_schemas.len() {
        0 => "None".into(),
        1 => compact_schema_name(selected_schemas.first().expect("one schema exists")),
        count => format!("{count}/{available_count}"),
    }
}

fn compact_schema_name(schema: &str) -> String {
    const MAX_CHARACTERS: usize = 18;
    let mut characters = schema.chars();
    let prefix = characters.by_ref().take(MAX_CHARACTERS).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_colors_drop_alpha_without_reordering_channels() {
        assert_eq!(color_hex(gpui::rgba(0x12345678)), "#123456");
    }

    #[test]
    fn schema_filter_label_stays_compact_and_describes_selection() {
        let selected = BTreeSet::from(["analytics".to_owned(), "public".to_owned()]);
        assert_eq!(diagram_schema_filter_label(None, 4), "All");
        assert_eq!(
            diagram_schema_filter_label(Some(&BTreeSet::new()), 4),
            "None"
        );
        assert_eq!(diagram_schema_filter_label(Some(&selected), 4), "2/4");
        assert_eq!(
            compact_schema_name("a_very_long_schema_name"),
            "a_very_long_schema…"
        );
    }

    #[test]
    fn native_diagram_viewport_keeps_render_work_bounded() {
        let document = DiagramDocument {
            database: "large".into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            width: 100_000.0,
            height: 80_000.0,
        };
        let visible = diagram_visible_scene_bounds(
            &document,
            1.0,
            point(px(-50_000.0), px(-20_000.0)),
            1_000.0,
            600.0,
        );

        assert!(visible.right - visible.left <= 1_000.0 + DIAGRAM_VIEWPORT_OVERSCAN * 2.0);
        assert!(visible.bottom - visible.top <= 600.0 + DIAGRAM_VIEWPORT_OVERSCAN * 2.0);
        assert!(visible.left > 0.0);
        assert!(visible.right < document.width);
    }

    #[test]
    fn native_diagram_culls_offscreen_cards_and_relationships() {
        let visible = DiagramSceneBounds {
            left: 1_000.0,
            top: 1_000.0,
            right: 2_000.0,
            bottom: 2_000.0,
        };

        assert!(diagram_rect_intersects_scene(
            1_900.0, 1_900.0, 200.0, 200.0, visible
        ));
        assert!(!diagram_rect_intersects_scene(
            100.0, 100.0, 200.0, 200.0, visible
        ));
        assert!(diagram_edge_intersects_scene(
            &[(500.0, 1_500.0), (2_500.0, 1_500.0)],
            visible,
        ));
        assert!(!diagram_edge_intersects_scene(
            &[(100.0, 100.0), (500.0, 100.0)],
            visible,
        ));
        assert!(!diagram_edge_intersects_scene(
            &[(500.0, 500.0), (2_500.0, 500.0), (2_500.0, 2_500.0)],
            visible,
        ));
    }
}
