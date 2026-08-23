use super::super::*;
use crate::diagram::DiagramPalette;
use gpui::{CursorStyle, MouseButton};
use gpui_component::checkbox::Checkbox;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tooltip::Tooltip;

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

    fn cache_key(&self) -> DiagramPaletteKey {
        DiagramPaletteKey::new([
            self.canvas.clone(),
            self.surface.clone(),
            self.surface_muted.clone(),
            self.border.clone(),
            self.text.clone(),
            self.muted_text.clone(),
            self.accent.clone(),
            self.relation.clone(),
        ])
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
                let colors = DiagramColors::current();
                let base_document = document.clone();
                let base_colors = colors.clone();
                let image = self
                    .diagram_base_scene_image_for(
                        session_id,
                        document.clone(),
                        colors.cache_key(),
                        move || {
                            Arc::new(Image::from_bytes(
                                ImageFormat::Svg,
                                base_document.svg(base_colors.palette(), None).into_bytes(),
                            ))
                        },
                    )
                    .expect("active diagram tab remains available while rendering");
                let selection_overlay = document
                    .selection_overlay_svg(selected_node.as_deref(), &colors.accent)
                    .map(|svg| Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())));
                let scene_padding = DIAGRAM_SCENE_PADDING;
                let scene_width = document.width * zoom + scene_padding * 2.0;
                let scene_height = document.height * zoom + scene_padding * 2.0;
                let node_hitboxes = document.nodes.iter().map(|node| {
                    let node_id = node.id.clone();
                    let table = node.table.clone();
                    let label = format!("{} — double-click to open data", node.id);
                    let node_focus = focus.clone();
                    div()
                        .id(SharedString::from(format!("diagram-node-{}", node.id)))
                        .absolute()
                        .left(px(scene_padding + node.x * zoom))
                        .top(px(scene_padding + node.y * zoom))
                        .w(px(node.width * zoom))
                        .h(px(node.height * zoom))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                        .hover(|style| style.bg(theme().selection))
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
                                this.open_diagram_table_for(session_id, table.clone(), window, cx);
                            } else {
                                this.select_diagram_node_for(session_id, Some(node_id.clone()), cx);
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
                            .child(
                                img(image)
                                    .absolute()
                                    .left(px(scene_padding))
                                    .top(px(scene_padding))
                                    .w(px(document.width * zoom))
                                    .h(px(document.height * zoom))
                                    .object_fit(gpui::ObjectFit::Fill),
                            )
                            // Selection stays below hitboxes, so its async decode cannot blank the base
                            // scene or intercept table presses.
                            .when_some(selection_overlay, |scene, overlay| {
                                scene.child(
                                    img(overlay)
                                        .absolute()
                                        .left(px(scene_padding))
                                        .top(px(scene_padding))
                                        .w(px(document.width * zoom))
                                        .h(px(document.height * zoom))
                                        .object_fit(gpui::ObjectFit::Fill),
                                )
                            })
                            .children(node_hitboxes),
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
}
