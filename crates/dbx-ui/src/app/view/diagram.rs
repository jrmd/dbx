use super::super::*;
use crate::diagram::{
    DiagramPalette, EDGE_CORNER_RADIUS, HEADER_HEIGHT, ROW_HEIGHT, RouteStep, edge_markers,
    rounded_route,
};
use gpui::{BoxShadow, CursorStyle, MouseButton, PathBuilder, canvas};
use gpui_component::checkbox::Checkbox;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tooltip::Tooltip;
const DIAGRAM_VIEWPORT_OVERSCAN: f32 = 256.0;
/// Every control in the diagram toolbar shares the standard capsule height.
const DIAGRAM_TOOLBAR_CONTROL_HEIGHT: f32 = 28.0;

/// A circular, icon-sized toolbar control.
fn toolbar_icon_button(id: &'static str) -> Button {
    Button::new(id)
        .with_size(Size::XSmall)
        .compact()
        .ghost()
        .rounded_full()
}

#[derive(Clone)]
struct DiagramColors {
    canvas: String,
    surface: String,
    surface_muted: String,
    border: String,
    text: String,
    muted_text: String,
    accent: String,
    key: String,
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
            key: color_hex(theme().warning),
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
            key: &self.key,
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
            dragging_node,
            arranged,
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
                diagram.node_drag.as_ref().map(|drag| drag.node_id.clone()),
                !diagram.arranged_positions.is_empty(),
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
                .child("No diagram open")
                .into_any_element();
        };

        let refresh = cx.entity().downgrade();
        let scroll_offset = scroll_handle.offset();
        let viewport_at_origin = scroll_offset.x == px(0.) && scroll_offset.y == px(0.);
        let window_size = window.bounds().size;
        let sidebar_width = self.explorer_width();
        let pane_width = (f32::from(window_size.width) - sidebar_width - 46.0).max(0.0);
        // Prefer the measured viewport; the window-derived estimate only
        // covers the first frame, before the scroller has been laid out.
        let measured = scroll_handle.bounds().size;
        let (available_width, available_height) = if measured.width > px(0.) {
            (f32::from(measured.width), f32::from(measured.height))
        } else {
            (
                (pane_width - 48.0).max(320.0),
                (f32::from(window_size.height) - 198.0).max(240.0),
            )
        };
        let fit_zoom = document
            .as_ref()
            .map(|document| {
                let inner = |size: f32| (size - DIAGRAM_SCENE_PADDING * 2.0).max(1.0);
                (inner(available_width) / document.width)
                    .min(inner(available_height) / document.height)
                    .clamp(0.35, 1.0)
            })
            .unwrap_or(1.0);
        let schema_filter_active = selected_schemas.is_some();
        let schema_filter_control = (kind.dialect() == DatabaseKind::PostgreSQL
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

            let popover = Popover::new("diagram-schema-filter")
                .p_0()
                .w(px(220.))
                .trigger(
                    button(
                        "diagram-schema-filter-trigger",
                        format!("Schemas · {summary}"),
                        ButtonKind::Quiet,
                    )
                    .selected(schema_filter_active)
                    .tooltip("Schemas"),
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
                );
            crate::popups::ScrollBlockingPopover::new("diagram-schema-filter", popover)
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
                                    .h(px(DIAGRAM_TOOLBAR_CONTROL_HEIGHT))
                                    .px(px(1.))
                                    .flex()
                                    .items_center()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(theme().hairline)
                                    .bg(theme().glass_hover)
                                    .child(
                                        toolbar_icon_button("diagram-zoom-out")
                                            .size(px(DIAGRAM_TOOLBAR_CONTROL_HEIGHT - 4.))
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
                                            .w(px(40.))
                                            .text_center()
                                            .text_size(px(11.))
                                            .text_color(theme().text)
                                            .child(format!("{:.0}%", zoom * 100.0)),
                                    )
                                    .child(
                                        toolbar_icon_button("diagram-zoom-in")
                                            .size(px(DIAGRAM_TOOLBAR_CONTROL_HEIGHT - 4.))
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
                                button("diagram-fit", "Fit", ButtonKind::Quiet)
                                    .tooltip("Fit (F) · reset zoom (0)")
                                    .disabled((zoom - fit_zoom).abs() < 0.01 && viewport_at_origin)
                                    .on_click(move |_, _, cx| {
                                        let _ = fit_diagram.update(cx, |this, cx| {
                                            this.fit_diagram_for(session_id, fit_zoom, cx)
                                        });
                                    }),
                            )
                            .when(arranged, |actions| {
                                let reset_layout = cx.entity().downgrade();
                                actions.child(
                                    button(
                                        "diagram-reset-layout",
                                        "Auto layout",
                                        ButtonKind::Quiet,
                                    )
                                    .tooltip("Discard hand-arranged positions")
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = reset_layout.update(cx, |this, cx| {
                                                this.reset_diagram_layout_for(session_id, cx)
                                            });
                                        },
                                    ),
                                )
                            })
                            .child(
                                button("diagram-export", "Export", ButtonKind::Quiet)
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
                        toolbar_icon_button("diagram-refresh")
                            .size(px(DIAGRAM_TOOLBAR_CONTROL_HEIGHT))
                            .tooltip("Refresh (R)")
                            .child(icon(Icon::Refresh, theme().text_muted))
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                let _ = refresh.update(cx, |this, cx| {
                                    this.refresh_diagram_for(session_id, cx)
                                });
                            }),
                    ),
            );

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
                        )),
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
                    .map(|edge| {
                        let emphasis = match selected_node.as_deref() {
                            Some(selected)
                                if edge.source == selected || edge.target == selected =>
                            {
                                DiagramEdgeEmphasis::Highlighted
                            }
                            Some(_) => DiagramEdgeEmphasis::Dimmed,
                            None => DiagramEdgeEmphasis::Normal,
                        };
                        DiagramEdgeRoute {
                            points: edge.points.clone(),
                            optional: edge.optional,
                            emphasis,
                        }
                    })
                    .collect::<Vec<_>>();
                let relationships = diagram_relationship_canvas(edge_routes, zoom)
                    .absolute()
                    .left(px(scene_padding))
                    .top(px(scene_padding))
                    .w(px(document.width * zoom))
                    .h(px(document.height * zoom));
                let has_selection = selected_node.is_some();
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
                        let drag_node_id = node.id.clone();
                        let node_focus = focus.clone();
                        let header_label = format!("{} — double-click to open data", node.id);
                        let table = node.table.clone();
                        let selected = selected_node.as_deref() == Some(node.id.as_str());
                        let related = related_node_ids.contains(node.id.as_str());
                        let dimmed = has_selection && !selected && !related;
                        let title_size = (13.5 * zoom).max(7.0);
                        let schema_size = (10.0 * zoom).max(6.0);
                        let row_size = (11.5 * zoom).max(6.0);
                        let key_size = (8.5 * zoom).max(5.0);
                        let horizontal_padding = (12.0 * zoom).max(4.0);
                        let key_width = (26.0 * zoom).max(12.0);
                        let radius = (10.0 * zoom).max(3.0);
                        let schema = node.table.schema.clone();
                        let column_total = node.columns.len() + node.omitted_columns;
                        let rows = node.columns.iter().enumerate().map(|(index, column)| {
                            let key = if column.primary_key {
                                Some(("PK", theme().warning))
                            } else if column.foreign_key {
                                Some(("FK", theme().focus_ring))
                            } else {
                                None
                            };
                            div()
                                .id(SharedString::from(format!(
                                    "diagram-row-{}-{index}",
                                    node.id
                                )))
                                .h(px(ROW_HEIGHT * zoom))
                                .px(px(horizontal_padding))
                                .flex_none()
                                .flex()
                                .items_center()
                                .text_size(px(row_size))
                                .hover(|style| style.bg(theme().glass_hover))
                                .child(div().w(px(key_width)).flex_none().when_some(
                                    key,
                                    |view, (label, color)| {
                                        view.child(
                                            div()
                                                .flex_none()
                                                .w(px((20.0 * zoom).max(10.0)))
                                                .py(px((1.0 * zoom).max(0.5)))
                                                .rounded(px((4.0 * zoom).max(2.0)))
                                                .bg(color.alpha(0.14))
                                                .text_center()
                                                .font_weight(FontWeight::BOLD)
                                                .text_size(px(key_size))
                                                .text_color(color)
                                                .child(label),
                                        )
                                    },
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme().text)
                                        .when(column.primary_key, |view| {
                                            view.font_weight(FontWeight::SEMIBOLD)
                                        })
                                        .child(column.name.clone()),
                                )
                                .child(
                                    div()
                                        .ml(px((8.0 * zoom).max(2.0)))
                                        .max_w(px(node.width * zoom * 0.46))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .child(
                                            div()
                                                .truncate()
                                                .text_color(theme().text_muted)
                                                .child(column.data_type.clone()),
                                        )
                                        .child(
                                            div()
                                                .w(px((8.0 * zoom).max(4.0)))
                                                .text_color(theme().warning.alpha(0.8))
                                                .when(column.nullable, |view| view.child("?")),
                                        ),
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
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .rounded(px(radius))
                            .border_1()
                            .border_color(theme().border_strong)
                            .bg(theme().panel)
                            .shadow(glass_shadow(8.0 * zoom))
                            .when(dimmed, |view| view.opacity(0.38))
                            .when(related && !selected, |view| {
                                view.border_color(theme().accent.alpha(0.6))
                            })
                            .when(selected, |view| {
                                view.border_color(theme().accent).shadow(vec![BoxShadow {
                                    color: theme().accent.alpha(0.35).into(),
                                    offset: point(px(0.), px(0.)),
                                    blur_radius: px(18. * zoom),
                                    spread_radius: px(1.),
                                    inset: false,
                                }])
                            })
                            .cursor(if dragging_node.as_deref() == Some(node.id.as_str()) {
                                CursorStyle::ClosedHand
                            } else {
                                CursorStyle::PointingHand
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("diagram-header-{}", node.id)))
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(header_label.clone()).build(window, cx)
                                    })
                                    .h(px(HEADER_HEIGHT * zoom))
                                    .flex_none()
                                    .px(px(horizontal_padding))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0 * zoom))
                                    .border_b_1()
                                    .border_color(theme().border)
                                    .bg(theme().panel_raised)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_size(px(title_size))
                                                    .text_color(theme().text)
                                                    .child(node.table.name.clone()),
                                            )
                                            .when_some(schema, |view, schema| {
                                                view.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(schema_size))
                                                        .text_color(theme().text_muted)
                                                        .child(schema),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(px(schema_size))
                                            .text_color(theme().text_muted)
                                            .child(format!("{column_total}")),
                                    ),
                            )
                            .children(rows)
                            .when_some(omitted_row, |view, row| view.child(row))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                                        node_focus.focus(window, cx);
                                        // A card press selects, drags, or drills in; it never pans.
                                        cx.stop_propagation();
                                        this.begin_diagram_node_drag_for(
                                            session_id,
                                            drag_node_id.clone(),
                                            event.position,
                                        );
                                    },
                                ),
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
                    })
                    .collect::<Vec<_>>();
                let minimap = diagram_minimap(
                    &document,
                    zoom,
                    scroll_offset,
                    scroll_handle.bounds().size,
                    selected_node.as_deref(),
                )
                .map(|(width, height, canvas)| {
                    let minimap_zoom = width / document.width.max(1.0);
                    div()
                        .id("diagram-minimap")
                        .absolute()
                        .right(px(DIAGRAM_MINIMAP_MARGIN))
                        .bottom(px(DIAGRAM_MINIMAP_MARGIN))
                        .w(px(width + DIAGRAM_MINIMAP_PADDING * 2.))
                        .h(px(height + DIAGRAM_MINIMAP_PADDING * 2.))
                        .p(px(DIAGRAM_MINIMAP_PADDING))
                        .rounded(px(RADIUS_PANEL))
                        .bg(theme().glass_raised)
                        .border_1()
                        .border_color(theme().hairline)
                        .shadow(glass_shadow(12.))
                        .cursor_pointer()
                        .child(canvas.size_full())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if let Some(point) = minimap_document_point(
                                    this,
                                    session_id,
                                    event.position,
                                    minimap_zoom,
                                ) {
                                    this.center_diagram_on_for(session_id, point, cx);
                                }
                            }),
                        )
                        .on_mouse_move(cx.listener(
                            move |this, event: &gpui::MouseMoveEvent, _, cx| {
                                if event.dragging()
                                    && let Some(point) = minimap_document_point(
                                        this,
                                        session_id,
                                        event.position,
                                        minimap_zoom,
                                    )
                                {
                                    cx.stop_propagation();
                                    this.center_diagram_on_for(session_id, point, cx);
                                }
                            },
                        ))
                });
                let scroller = div()
                    .id("diagram-scroll")
                    .size_full()
                    .relative()
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
                            if event.dragging()
                                && !this.drag_diagram_node_to_for(session_id, event.position, cx)
                            {
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
                    .overflow_scroll()
                    .track_scroll(&scroll_handle)
                    .bg(theme().canvas)
                    .child(
                        div()
                            .relative()
                            .w(px(scene_width))
                            .h(px(scene_height))
                            .min_w_full()
                            .min_h_full()
                            // Registered on the scene (inside the scroller) so a
                            // modified wheel zooms before the scroller consumes it.
                            .on_scroll_wheel(cx.listener(
                                move |this, event: &gpui::ScrollWheelEvent, _, cx| {
                                    if !event.modifiers.secondary() && !event.modifiers.control {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    let delta = event.delta.pixel_delta(px(16.));
                                    let factor = match event.delta {
                                        gpui::ScrollDelta::Lines(_) => {
                                            if f32::from(delta.y) > 0.0 {
                                                1.12
                                            } else {
                                                1.0 / 1.12
                                            }
                                        }
                                        gpui::ScrollDelta::Pixels(_) => {
                                            (f32::from(delta.y) * 0.006).exp()
                                        }
                                    };
                                    let current =
                                        this.active_diagram_zoom(session_id).unwrap_or(1.0);
                                    this.zoom_diagram_at_for(
                                        session_id,
                                        current * factor,
                                        event.position,
                                        cx,
                                    );
                                },
                            ))
                            .child(relationships)
                            .children(node_cards),
                    );
                div()
                    .id("diagram-viewport")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .child(scroller)
                    .when_some(minimap, |view, minimap| view.child(minimap))
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
                                        "Couldn’t build the diagram"
                                    } else {
                                        "Loading diagram…"
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
                                        .with_size(Size::XSmall)
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
            .child(body_content);

        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
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

#[derive(Clone, Copy, PartialEq)]
enum DiagramEdgeEmphasis {
    Normal,
    Highlighted,
    Dimmed,
}

struct DiagramEdgeRoute {
    points: Vec<(f32, f32)>,
    optional: bool,
    emphasis: DiagramEdgeEmphasis,
}

struct DiagramPaintedEdge {
    line: gpui::Path<Pixels>,
    marks: Option<gpui::Path<Pixels>>,
    ring: Option<(gpui::Path<Pixels>, gpui::Path<Pixels>)>,
    emphasis: DiagramEdgeEmphasis,
}

/// Spacing of the canvas dot grid in document units.
const DIAGRAM_GRID_SPACING: f32 = 24.0;

fn diagram_relationship_canvas(
    routes: Vec<DiagramEdgeRoute>,
    zoom: f32,
) -> gpui::Canvas<Vec<DiagramPaintedEdge>> {
    let palette = *theme();
    canvas(
        move |bounds, _, _| {
            let mut routes = routes;
            // Highlighted relationships paint last so they sit on top.
            routes.sort_by_key(|route| route.emphasis == DiagramEdgeEmphasis::Highlighted);
            routes
                .into_iter()
                .filter_map(|route| diagram_painted_edge(bounds, &route, zoom))
                .collect::<Vec<_>>()
        },
        move |bounds, edges, window, _| {
            paint_diagram_grid(bounds, zoom, palette.border_strong.alpha(0.55), window);
            for edge in edges {
                let color = match edge.emphasis {
                    DiagramEdgeEmphasis::Normal => palette.text_muted.alpha(0.75),
                    DiagramEdgeEmphasis::Highlighted => palette.accent,
                    DiagramEdgeEmphasis::Dimmed => palette.text_muted.alpha(0.18),
                };
                window.paint_path(edge.line, color);
                if let Some(marks) = edge.marks {
                    window.paint_path(marks, color);
                }
                if let Some((fill, ring)) = edge.ring {
                    window.paint_path(fill, palette.canvas);
                    window.paint_path(ring, color);
                }
            }
        },
    )
}

/// A quiet dot grid, painted only inside the visible clip so large diagrams
/// cost the same as small ones.
fn paint_diagram_grid(bounds: gpui::Bounds<Pixels>, zoom: f32, color: Rgba, window: &mut Window) {
    let mut spacing = DIAGRAM_GRID_SPACING * zoom;
    while spacing < 14.0 {
        spacing *= 2.0;
    }
    let visible = bounds.intersect(&window.content_mask().bounds);
    if visible.size.width <= px(0.) || visible.size.height <= px(0.) {
        return;
    }
    let dot = (1.6 * zoom).clamp(1.0, 2.0);
    let first_column = (f32::from(visible.origin.x - bounds.origin.x) / spacing).floor();
    let first_row = (f32::from(visible.origin.y - bounds.origin.y) / spacing).floor();
    let columns = (f32::from(visible.size.width) / spacing).ceil() as usize + 1;
    let rows = (f32::from(visible.size.height) / spacing).ceil() as usize + 1;
    for row in 0..rows {
        let y = bounds.origin.y + px((first_row + row as f32) * spacing);
        for column in 0..columns {
            let x = bounds.origin.x + px((first_column + column as f32) * spacing);
            window.paint_quad(gpui::fill(
                gpui::Bounds::new(point(x, y), gpui::size(px(dot), px(dot))),
                color,
            ));
        }
    }
}

fn diagram_painted_edge(
    bounds: gpui::Bounds<Pixels>,
    route: &DiagramEdgeRoute,
    zoom: f32,
) -> Option<DiagramPaintedEdge> {
    let to_canvas = |(x, y): (f32, f32)| {
        point(
            bounds.origin.x + px(x * zoom),
            bounds.origin.y + px(y * zoom),
        )
    };
    let stroke_width = match route.emphasis {
        DiagramEdgeEmphasis::Highlighted => 2.0,
        _ => 1.4,
    };
    let stroke = px((stroke_width * zoom).max(1.0));
    let mut line = PathBuilder::stroke(stroke);
    for step in rounded_route(&route.points, EDGE_CORNER_RADIUS) {
        match step {
            RouteStep::Move(point) => line.move_to(to_canvas(point)),
            RouteStep::Line(point) => line.line_to(to_canvas(point)),
            RouteStep::Curve(point, control) => line.curve_to(to_canvas(point), to_canvas(control)),
        }
    }
    let line = line.build().ok()?;

    let markers = edge_markers(&route.points, route.optional);
    let marks = (!markers.lines.is_empty())
        .then(|| {
            let mut marks = PathBuilder::stroke(stroke);
            for [from, to] in &markers.lines {
                marks.move_to(to_canvas(*from));
                marks.line_to(to_canvas(*to));
            }
            marks.build().ok()
        })
        .flatten();
    let ring = markers.ring.and_then(|(center, radius)| {
        let polygon = (0..16)
            .map(|index| {
                let angle = index as f32 / 16.0 * std::f32::consts::TAU;
                to_canvas((
                    center.0 + radius * angle.cos(),
                    center.1 + radius * angle.sin(),
                ))
            })
            .collect::<Vec<_>>();
        let mut fill = PathBuilder::fill();
        fill.add_polygon(&polygon, true);
        let mut outline = PathBuilder::stroke(stroke);
        outline.add_polygon(&polygon, true);
        Some((fill.build().ok()?, outline.build().ok()?))
    });

    Some(DiagramPaintedEdge {
        line,
        marks,
        ring,
        emphasis: route.emphasis,
    })
}

/// A scaled overview of every card with the current viewport outlined.
/// Returns `None` when the whole diagram already fits on screen.
fn diagram_minimap(
    document: &DiagramDocument,
    zoom: f32,
    scroll_offset: Point<Pixels>,
    viewport: gpui::Size<Pixels>,
    selected: Option<&str>,
) -> Option<(f32, f32, gpui::Canvas<()>)> {
    let viewport = (f32::from(viewport.width), f32::from(viewport.height));
    let scene = (
        document.width * zoom + DIAGRAM_SCENE_PADDING * 2.0,
        document.height * zoom + DIAGRAM_SCENE_PADDING * 2.0,
    );
    if viewport.0 <= 0.0 || (scene.0 <= viewport.0 + 1.0 && scene.1 <= viewport.1 + 1.0) {
        return None;
    }
    let (width, height) = diagram_minimap_size(document);
    let scale = width / document.width.max(1.0);
    let cards = document
        .nodes
        .iter()
        .map(|node| {
            (
                node.x * scale,
                node.y * scale,
                (node.width * scale).max(2.0),
                (node.height * scale).max(2.0),
                selected == Some(node.id.as_str()),
            )
        })
        .collect::<Vec<_>>();
    let view = (
        ((-f32::from(scroll_offset.x) - DIAGRAM_SCENE_PADDING) / zoom * scale).max(0.0),
        ((-f32::from(scroll_offset.y) - DIAGRAM_SCENE_PADDING) / zoom * scale).max(0.0),
        (viewport.0 / zoom * scale).min(width),
        (viewport.1 / zoom * scale).min(height),
    );
    let palette = *theme();
    let canvas = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let at = |x: f32, y: f32, w: f32, h: f32| {
                gpui::Bounds::new(
                    point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                    gpui::size(px(w), px(h)),
                )
            };
            for (x, y, w, h, selected) in &cards {
                let color = if *selected {
                    palette.accent
                } else {
                    palette.text_muted.alpha(0.45)
                };
                window.paint_quad(gpui::fill(at(*x, *y, *w, *h), color).corner_radii(px(1.5)));
            }
            let (x, y, w, h) = view;
            window.paint_quad(gpui::quad(
                at(x, y, w.min(width - x), h.min(height - y)),
                px(3.),
                palette.accent.alpha(0.1),
                px(1.),
                palette.accent,
                gpui::BorderStyle::Solid,
            ));
        },
    );
    Some((width, height, canvas))
}

/// Convert a window position over the minimap to a document point.
fn minimap_document_point(
    app: &DbxApp,
    session_id: SessionId,
    position: Point<Pixels>,
    minimap_zoom: f32,
) -> Option<(f32, f32)> {
    let bounds = app.diagram_minimap_bounds(session_id)?;
    Some((
        f32::from(position.x - bounds.origin.x) / minimap_zoom,
        f32::from(position.y - bounds.origin.y) / minimap_zoom,
    ))
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
