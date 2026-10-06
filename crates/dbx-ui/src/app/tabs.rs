//! Workbench tabs.
use super::*;

impl DbxApp {
    pub(super) fn add_query_tab_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.session(session_id).map(|session| session.kind) else {
            return;
        };

        let id = Uuid::new_v4();
        let query_tab = QueryTab::new(kind, session_id, id, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.secondary_tabs.push(SecondaryTab {
            id,
            kind: SecondaryTabKind::Query(Box::new(query_tab)),
        });
        session.active_secondary_tab = Some(id);
        session.pane = Pane::Query;
        let focus = session
            .secondary_tabs
            .last()
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(query.query_editor.read(cx).focus_handle()),
                SecondaryTabKind::Data(_)
                | SecondaryTabKind::Structure(_)
                | SecondaryTabKind::Diagram(_) => None,
            });
        if let Some(focus) = focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Load a persisted history item into the active query document, creating
    /// one when the current document is Data or Structure. History never
    /// executes implicitly; the user still chooses Run.
    pub(super) fn load_query_history_entry_for(
        &mut self,
        session_id: SessionId,
        entry: &QueryHistoryEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query_is_active = self.session(session_id).is_some_and(|session| {
            session.active_secondary_tab.is_some_and(|tab_id| {
                session
                    .secondary_tabs
                    .iter()
                    .any(|tab| tab.id == tab_id && matches!(&tab.kind, SecondaryTabKind::Query(_)))
            })
        });
        if !query_is_active {
            self.add_query_tab_for(session_id, window, cx);
        }
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };
        let focus = query_tab.query_editor.read(cx).focus_handle();
        query_tab.query_editor.update(cx, |editor, cx| {
            editor.set_text(entry.sql.clone(), cx);
        });
        query_tab.error = None;
        query_tab.error_highlight = None;
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn open_structure_tab_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, table_ref)) = self.session(session_id).and_then(|session| {
            session
                .engine
                .clone()
                .map(|engine| (engine, table_ref(&table)))
        }) else {
            return;
        };
        let id = Uuid::new_v4();
        if let Some(session) = self.session_mut(session_id) {
            session.secondary_tabs.push(SecondaryTab {
                id,
                kind: SecondaryTabKind::Structure(Box::new(StructureTab {
                    designer: None,
                    table: table_ref.clone(),
                    columns: Vec::new(),
                    foreign_keys: Vec::new(),
                    indexes: Vec::new(),
                    checks: Vec::new(),
                    definition: None,
                    busy: true,
                    error: None,
                })),
            });
            session.active_secondary_tab = Some(id);
            session.pane = Pane::Structure;
        }
        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move { engine.table_structure(&table_ref).await });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                let Some(tab) = session.secondary_tabs.iter_mut().find(|tab| tab.id == id) else {
                    return;
                };
                let SecondaryTabKind::Structure(structure) = &mut tab.kind else {
                    return;
                };
                structure.busy = false;
                match result {
                    Ok(table_structure) => {
                        session.completion_columns.insert(
                            completion_table_key(&structure.table),
                            table_structure.columns.clone(),
                        );
                        structure.columns = table_structure.columns;
                        structure.foreign_keys = table_structure.foreign_keys;
                        structure.indexes = table_structure.indexes;
                        structure.checks = table_structure.checks;
                        structure.definition = table_structure.definition;
                        structure.error = None;
                    }
                    Err(error) => structure.error = Some(error.to_string()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Open the one relationship diagram for this connection, or return to it
    /// when it is already open. Redis deliberately has no relational surface.
    pub(super) fn open_diagram_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, existing, tables, explorer_schema)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.secondary_tabs.iter().find_map(|tab| {
                        matches!(&tab.kind, SecondaryTabKind::Diagram(_)).then_some(tab.id)
                    }),
                    session.tables.clone(),
                    session.schema_filter.clone(),
                )
            })
        else {
            return;
        };
        if !kind.is_sql() {
            if let Some(session) = self.session_mut(session_id) {
                session.error =
                    Some("Database diagrams are available for relational connections".into());
            }
            cx.notify();
            return;
        }
        if let Some(tab_id) = existing {
            self.activate_secondary_tab_for(session_id, tab_id, window, cx);
            return;
        }

        let id = Uuid::new_v4();
        let diagram = DiagramTab::loading(kind, &tables, explorer_schema.as_deref(), cx);
        let focus = diagram.focus.clone();
        if let Some(session) = self.session_mut(session_id) {
            session.secondary_tabs.push(SecondaryTab {
                id,
                kind: SecondaryTabKind::Diagram(Box::new(diagram)),
            });
            session.active_secondary_tab = Some(id);
            session.pane = Pane::Diagram;
        }
        focus.focus(window, cx);
        self.load_diagram_for(session_id, id, false, cx);
    }

    /// Reload the active diagram while retaining its last successful scene as
    /// a visible, explicitly stale snapshot.
    pub(super) fn refresh_diagram_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(tab_id) = self.session(session_id).and_then(|session| {
            session
                .secondary_tabs
                .iter()
                .find(|tab| matches!(&tab.kind, SecondaryTabKind::Diagram(_)))
                .map(|tab| tab.id)
        }) else {
            return;
        };
        self.load_diagram_for(session_id, tab_id, true, cx);
    }

    pub(super) fn load_diagram_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        retain_document: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self
            .session(session_id)
            .and_then(|session| session.engine.clone())
        else {
            return;
        };
        let runtime = self.runtime.clone();
        let Some(tab) = self.session_mut(session_id).and_then(|session| {
            session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
        }) else {
            return;
        };
        let SecondaryTabKind::Diagram(diagram) = &mut tab.kind else {
            return;
        };
        diagram.invalidate_request();
        diagram.busy = true;
        diagram.stale = retain_document && diagram.document.is_some();
        diagram.error = None;
        diagram.request_generation = diagram.request_generation.saturating_add(1);
        let generation = diagram.request_generation;
        let task = runtime.spawn(async move { engine.relational_schema().await });
        diagram.abort_handle.replace(task.abort_handle());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.session_mut(session_id).and_then(|session| {
                    session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| tab.id == tab_id)
                }) else {
                    return;
                };
                let SecondaryTabKind::Diagram(diagram) = &mut tab.kind else {
                    return;
                };
                if generation != diagram.request_generation {
                    return;
                }
                diagram.busy = false;
                diagram.abort_handle.clear();
                match result {
                    Ok(Ok(schema)) => {
                        let source_schema = Arc::new(schema);
                        diagram.available_schemas = relational_schema_names(&source_schema);
                        normalize_diagram_schema_selection(
                            &mut diagram.selected_schemas,
                            &diagram.available_schemas,
                        );
                        let mut document = diagram_document_for_selection(
                            &source_schema,
                            diagram.selected_schemas.as_ref(),
                        );
                        document.place_nodes(&diagram.arranged_positions);
                        diagram.document = Some(Arc::new(document));
                        diagram.source_schema = Some(source_schema);
                        diagram.stale = false;
                        diagram.error = None;
                    }
                    Ok(Err(error)) => {
                        diagram.stale = diagram.document.is_some();
                        diagram.error = Some(error.to_string());
                    }
                    Err(error) => {
                        diagram.stale = diagram.document.is_some();
                        diagram.error =
                            Some(format!("Diagram request stopped unexpectedly: {error}"));
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn set_diagram_zoom_for(
        &mut self,
        session_id: SessionId,
        zoom: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let next_zoom = zoom.clamp(0.35, 2.0);
        if let Some(document) = diagram.document.as_ref() {
            let old_scene = point(
                px(document.width * diagram.zoom + DIAGRAM_SCENE_PADDING * 2.0),
                px(document.height * diagram.zoom + DIAGRAM_SCENE_PADDING * 2.0),
            );
            let next_scene = point(
                px(document.width * next_zoom + DIAGRAM_SCENE_PADDING * 2.0),
                px(document.height * next_zoom + DIAGRAM_SCENE_PADDING * 2.0),
            );
            let offset = diagram.scroll_handle.offset();
            let max_offset = diagram.scroll_handle.max_offset();
            diagram.scroll_handle.set_offset(point(
                remap_diagram_scroll_axis(offset.x, max_offset.x, old_scene.x, next_scene.x),
                remap_diagram_scroll_axis(offset.y, max_offset.y, old_scene.y, next_scene.y),
            ));
        }
        diagram.zoom = next_zoom;
        cx.notify();
    }

    pub(super) fn reset_diagram_view_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.zoom = 1.0;
        diagram.scroll_handle.set_offset(point(px(0.), px(0.)));
        diagram.drag_anchor = None;
        cx.notify();
    }

    pub(super) fn fit_diagram_for(
        &mut self,
        session_id: SessionId,
        zoom: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.zoom = zoom.clamp(0.35, 2.0);
        diagram.scroll_handle.set_offset(point(px(0.), px(0.)));
        diagram.drag_anchor = None;
        cx.notify();
    }

    pub(super) fn begin_diagram_pan_for(
        &mut self,
        session_id: SessionId,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.drag_anchor = Some(DiagramDragAnchor {
            pointer,
            scroll_offset: diagram.scroll_handle.offset(),
            moved: false,
        });
        cx.notify();
    }

    pub(super) fn pan_diagram_to_for(
        &mut self,
        session_id: SessionId,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let Some(anchor) = diagram.drag_anchor.as_mut() else {
            return;
        };
        let travel = (pointer.x - anchor.pointer.x)
            .abs()
            .max((pointer.y - anchor.pointer.y).abs());
        anchor.moved |= travel > px(3.);
        let anchor = *anchor;
        let offset = point(
            anchor.scroll_offset.x + (pointer.x - anchor.pointer.x),
            anchor.scroll_offset.y + (pointer.y - anchor.pointer.y),
        );
        diagram
            .scroll_handle
            .set_offset(clamp_diagram_scroll_offset(
                offset,
                diagram.scroll_handle.max_offset(),
            ));
        cx.notify();
    }

    pub(super) fn pan_diagram_by_for(
        &mut self,
        session_id: SessionId,
        horizontal: f32,
        vertical: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let offset = diagram.scroll_handle.offset();
        let requested = point(offset.x - px(horizontal), offset.y - px(vertical));
        diagram
            .scroll_handle
            .set_offset(clamp_diagram_scroll_offset(
                requested,
                diagram.scroll_handle.max_offset(),
            ));
        cx.notify();
    }

    pub(super) fn end_diagram_pan_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(diagram) = self.active_diagram_tab_mut(session_id)
            && (diagram.drag_anchor.is_some() || diagram.node_drag.is_some())
        {
            if diagram.drag_anchor.is_some_and(|anchor| !anchor.moved) {
                diagram.selected_node = None;
            }
            diagram.drag_anchor = None;
            diagram.node_drag = None;
            cx.notify();
        }
    }

    pub(super) fn begin_diagram_node_drag_for(
        &mut self,
        session_id: SessionId,
        node_id: String,
        pointer: Point<Pixels>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let Some(origin) = diagram
            .document
            .as_ref()
            .and_then(|document| document.node_position(&node_id))
        else {
            return;
        };
        diagram.drag_anchor = None;
        diagram.node_drag = Some(DiagramNodeDrag {
            node_id,
            pointer,
            origin,
        });
    }

    /// Move the dragged card with the pointer. Returns whether a drag is in
    /// progress so the caller can skip canvas panning.
    pub(super) fn drag_diagram_node_to_for(
        &mut self,
        session_id: SessionId,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return false;
        };
        let Some(drag) = diagram.node_drag.clone() else {
            return false;
        };
        let zoom = diagram.zoom.max(0.01);
        let x = drag.origin.0 + f32::from(pointer.x - drag.pointer.x) / zoom;
        let y = drag.origin.1 + f32::from(pointer.y - drag.pointer.y) / zoom;
        if let Some(document) = diagram.document.as_mut()
            && Arc::make_mut(document).move_node(&drag.node_id, x, y)
            && let Some(position) = document.node_position(&drag.node_id)
        {
            diagram
                .arranged_positions
                .insert(drag.node_id.clone(), position);
            cx.notify();
        }
        true
    }

    /// Discard hand-arranged card positions and return to the automatic
    /// layout.
    pub(super) fn reset_diagram_layout_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.arranged_positions.clear();
        rebuild_diagram_document(diagram);
        cx.notify();
    }

    /// Zoom while keeping the document point under `anchor` (window
    /// coordinates) fixed, as pinch and Ctrl+wheel zoom do in native canvases.
    pub(super) fn zoom_diagram_at_for(
        &mut self,
        session_id: SessionId,
        zoom: f32,
        anchor: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let Some(document) = diagram.document.as_ref() else {
            return;
        };
        let next_zoom = zoom.clamp(0.35, 2.0);
        if (next_zoom - diagram.zoom).abs() < f32::EPSILON {
            return;
        }
        let viewport = diagram.scroll_handle.bounds();
        let offset = diagram.scroll_handle.offset();
        let local = point(
            f32::from(anchor.x - viewport.origin.x),
            f32::from(anchor.y - viewport.origin.y),
        );
        let document_point = (
            (local.x - f32::from(offset.x) - DIAGRAM_SCENE_PADDING) / diagram.zoom,
            (local.y - f32::from(offset.y) - DIAGRAM_SCENE_PADDING) / diagram.zoom,
        );
        let scene = (
            document.width * next_zoom + DIAGRAM_SCENE_PADDING * 2.0,
            document.height * next_zoom + DIAGRAM_SCENE_PADDING * 2.0,
        );
        let max_offset = point(
            px((scene.0 - f32::from(viewport.size.width)).max(0.0)),
            px((scene.1 - f32::from(viewport.size.height)).max(0.0)),
        );
        let next_offset = point(
            px(local.x - (document_point.0 * next_zoom + DIAGRAM_SCENE_PADDING)),
            px(local.y - (document_point.1 * next_zoom + DIAGRAM_SCENE_PADDING)),
        );
        diagram
            .scroll_handle
            .set_offset(clamp_diagram_scroll_offset(next_offset, max_offset));
        diagram.zoom = next_zoom;
        cx.notify();
    }

    pub(super) fn active_diagram_zoom(&self, session_id: SessionId) -> Option<f32> {
        let session = self.session(session_id)?;
        let tab_id = session.active_secondary_tab?;
        session
            .secondary_tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Diagram(diagram) => Some(diagram.zoom),
                _ => None,
            })
    }

    /// Window-space bounds of the minimap's drawing area. The minimap is
    /// pinned to the viewport's bottom-right corner, so its geometry follows
    /// from the tracked scroll bounds and the document's aspect ratio.
    pub(super) fn diagram_minimap_bounds(
        &self,
        session_id: SessionId,
    ) -> Option<gpui::Bounds<Pixels>> {
        let session = self.session(session_id)?;
        let tab_id = session.active_secondary_tab?;
        let SecondaryTabKind::Diagram(diagram) = &session
            .secondary_tabs
            .iter()
            .find(|tab| tab.id == tab_id)?
            .kind
        else {
            return None;
        };
        let document = diagram.document.as_ref()?;
        let (width, height) = diagram_minimap_size(document);
        let viewport = diagram.scroll_handle.bounds();
        let inset = px(DIAGRAM_MINIMAP_MARGIN + DIAGRAM_MINIMAP_PADDING);
        Some(gpui::Bounds::new(
            point(
                viewport.origin.x + viewport.size.width - inset - px(width),
                viewport.origin.y + viewport.size.height - inset - px(height),
            ),
            gpui::size(px(width), px(height)),
        ))
    }

    /// Scroll so a document point sits in the centre of the viewport; used by
    /// the minimap.
    pub(super) fn center_diagram_on_for(
        &mut self,
        session_id: SessionId,
        document_point: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let viewport = diagram.scroll_handle.bounds().size;
        let target = point(
            px(f32::from(viewport.width) / 2.0
                - (document_point.0 * diagram.zoom + DIAGRAM_SCENE_PADDING)),
            px(f32::from(viewport.height) / 2.0
                - (document_point.1 * diagram.zoom + DIAGRAM_SCENE_PADDING)),
        );
        diagram
            .scroll_handle
            .set_offset(clamp_diagram_scroll_offset(
                target,
                diagram.scroll_handle.max_offset(),
            ));
        cx.notify();
    }

    pub(super) fn set_all_diagram_schemas_for(
        &mut self,
        session_id: SessionId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.selected_schemas = if enabled { None } else { Some(BTreeSet::new()) };
        rebuild_diagram_document(diagram);
        cx.notify();
    }

    pub(super) fn set_diagram_schema_enabled_for(
        &mut self,
        session_id: SessionId,
        schema: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        if diagram.available_schemas.binary_search(&schema).is_err() {
            return;
        }

        let mut selected = diagram
            .selected_schemas
            .clone()
            .unwrap_or_else(|| diagram.available_schemas.iter().cloned().collect());
        if enabled {
            selected.insert(schema);
        } else {
            selected.remove(&schema);
        }
        diagram.selected_schemas = Some(selected);
        normalize_diagram_schema_selection(
            &mut diagram.selected_schemas,
            &diagram.available_schemas,
        );
        rebuild_diagram_document(diagram);
        cx.notify();
    }

    pub(super) fn select_diagram_node_for(
        &mut self,
        session_id: SessionId,
        node_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.selected_node = node_id;
        cx.notify();
    }

    /// Drill into a table from the diagram using the same data-loading path as
    /// the explorer, keeping filters, paging, and mutation safety consistent.
    pub(super) fn open_diagram_table_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_table_for(session_id, table, window, cx);
    }

    /// Save pre-rendered diagram bytes through the native file picker. The
    /// renderer supplies bytes so app state remains presentation agnostic.
    pub(super) fn export_diagram_for(
        &mut self,
        session_id: SessionId,
        format: DiagramExportFormat,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let Some(database) = self.session(session_id).map(|session| {
            session
                .current_database
                .clone()
                .unwrap_or_else(|| session.name.clone())
        }) else {
            return;
        };
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let stem = database
            .chars()
            .map(|character| match character {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
                _ => '-',
            })
            .collect::<String>();
        let suggested = format!("{}-diagram.{}", stem.trim_matches('-'), format.extension());
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(path))) => {
                    let destination = path.display().to_string();
                    let result = runtime
                        .spawn_blocking(move || std::fs::write(path, bytes))
                        .await;
                    this.update(cx, |this, cx| {
                        let (kind, message) = match result {
                            Ok(Ok(())) => (
                                ToastKind::Success,
                                format!("Exported diagram to {destination}"),
                            ),
                            Ok(Err(error)) => (
                                ToastKind::Error,
                                format!("Could not export diagram: {error}"),
                            ),
                            Err(error) => (
                                ToastKind::Error,
                                format!("Diagram export task stopped: {error}"),
                            ),
                        };
                        this.show_toast(kind, message, cx);
                    })?;
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(
                            ToastKind::Error,
                            format!("Could not open the save dialog: {error}"),
                            cx,
                        );
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(
                            ToastKind::Error,
                            format!("Save dialog closed unexpectedly: {error}"),
                            cx,
                        );
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn active_diagram_tab_mut(
        &mut self,
        session_id: SessionId,
    ) -> Option<&mut DiagramTab> {
        let session = self.session_mut(session_id)?;
        let tab_id = session.active_secondary_tab?;
        let tab = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)?;
        let SecondaryTabKind::Diagram(diagram) = &mut tab.kind else {
            return None;
        };
        Some(diagram)
    }

    pub(super) fn activate_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session.secondary_tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        session.active_secondary_tab = Some(tab_id);
        let tab_focus = match &tab.kind {
            SecondaryTabKind::Data(_) => {
                session.pane = Pane::Data;
                session.recent_data_tab = Some(tab_id);
                None
            }
            SecondaryTabKind::Query(query) => {
                session.pane = Pane::Query;
                Some(query.query_editor.read(cx).focus_handle())
            }
            SecondaryTabKind::Structure(_) => {
                session.pane = Pane::Structure;
                None
            }
            SecondaryTabKind::Diagram(diagram) => {
                session.pane = Pane::Diagram;
                Some(diagram.focus.clone())
            }
        };
        if let Some(focus) = tab_focus {
            focus.focus(window, cx);
        }
        self.persist_query_workspace_for(session_id, cx);
        cx.notify();
    }

    /// Ask before discarding an edited query document. The confirmation keeps
    /// accidental tab closes from silently losing work.
    pub(super) fn request_close_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_edits_block(session_id, tab_id, cx) {
            return;
        }
        let Some((kind, text, return_focus)) = self.session(session_id).and_then(|session| {
            session
                .secondary_tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| {
                    let SecondaryTabKind::Query(query) = &tab.kind else {
                        return None;
                    };
                    let editor = query.query_editor.read(cx);
                    Some((session.kind, editor.text(cx), editor.focus_handle()))
                })
        }) else {
            self.close_secondary_tab_for(session_id, tab_id, cx);
            return;
        };
        if text.trim().is_empty() || text == Self::default_query(kind) {
            self.close_secondary_tab_for(session_id, tab_id, cx);
            return;
        }
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: "Close query?".into(),
            detail: "You can reopen it from Query options until you disconnect.".into(),
            confirm_label: "Close query",
            tone: ConfirmationTone::Warning,
            action: ConfirmationAction::CloseQuery { session_id, tab_id },
            focus: focus.clone(),
            return_focus: Some(return_focus),
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(index) = session
            .secondary_tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let tab = session.secondary_tabs.remove(index);
        match tab.kind {
            SecondaryTabKind::Query(mut query) => {
                query.invalidate_request();
                let text = query.query_editor.read(cx).text(cx);
                if !text.trim().is_empty() && text != Self::default_query(session.kind) {
                    session.closed_queries.push(text);
                    const MAX_CLOSED_QUERIES: usize = 10;
                    let overflow = session
                        .closed_queries
                        .len()
                        .saturating_sub(MAX_CLOSED_QUERIES);
                    if overflow > 0 {
                        session.closed_queries.drain(..overflow);
                    }
                }
            }
            SecondaryTabKind::Diagram(mut diagram) => diagram.invalidate_request(),
            SecondaryTabKind::Data(mut data) => data.invalidate_request(),
            SecondaryTabKind::Structure(_) => {}
        }
        if session.recent_data_tab == Some(tab_id) {
            session.recent_data_tab = None;
        }
        if session.active_secondary_tab == Some(tab_id) {
            let next = index.min(session.secondary_tabs.len().saturating_sub(1));
            session.active_secondary_tab = session.secondary_tabs.get(next).map(|tab| tab.id);
            session.pane = session
                .secondary_tabs
                .get(next)
                .map(|tab| tab.kind.pane())
                .unwrap_or(Pane::Data);
            if session.pane == Pane::Data {
                session.recent_data_tab = session.active_secondary_tab;
            }
        }
        self.persist_query_workspace_for(session_id, cx);
        cx.notify();
    }

    /// Reopen the most recently closed query document in this connection.
    pub(super) fn reopen_last_closed_query_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self
            .session_mut(session_id)
            .and_then(|session| session.closed_queries.pop());
        let Some(text) = text else { return };
        self.add_query_tab_for(session_id, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(SecondaryTab {
            kind: SecondaryTabKind::Query(query),
            ..
        }) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        query
            .query_editor
            .update(cx, |editor, cx| editor.set_text(text, cx));
        cx.notify();
    }
}
