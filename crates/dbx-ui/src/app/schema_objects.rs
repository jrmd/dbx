use super::*;
use crate::workspace::SavedQuery;
use dbx_core::SchemaObjectKind;

impl DbxApp {
    pub(super) fn load_schema_objects_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if !session.kind.is_sql() {
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let database = session.current_database.clone();
        let expected = engine.clone();
        let task = self
            .runtime
            .spawn(async move { engine.schema_objects().await });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if session.current_database != database
                    || !session
                        .engine
                        .as_ref()
                        .is_some_and(|engine| Arc::ptr_eq(engine, &expected))
                {
                    return;
                }
                match result {
                    Ok(Ok(objects)) => {
                        session.schema_objects = objects;
                        session.schema_objects_error = None;
                    }
                    Ok(Err(error)) => {
                        session.schema_objects.clear();
                        session.schema_objects_error = Some(error.to_string());
                    }
                    Err(_) => return,
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_schema_objects_for(
        &self,
        session_id: SessionId,
        table: Option<&TableRef>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.session(session_id) else {
            return div().into_any_element();
        };
        let objects = session
            .schema_objects
            .iter()
            .filter(|object| {
                table.is_none_or(|table| {
                    object.table.as_deref() == Some(&table.name)
                        && (table.schema.is_none() || object.schema == table.schema)
                })
            })
            .filter(|object| {
                table.is_some()
                    || object
                        .name
                        .to_lowercase()
                        .contains(&session.editors.sidebar_search.read(cx).to_lowercase())
            })
            .cloned()
            .collect::<Vec<_>>();
        div()
            .id(if table.is_some() {
                "structure-objects"
            } else {
                "explorer-objects"
            })
            .max_h(px(260.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .when(!objects.is_empty(), |view| {
                view.child(
                    div()
                        .px(px(14.))
                        .pt(px(8.))
                        .text_size(px(11.))
                        .text_color(theme().text_muted)
                        .child("Schema objects"),
                )
            })
            .when_some(session.schema_objects_error.clone(), |view, error| {
                view.child(
                    div()
                        .p(px(8.))
                        .text_color(theme().danger)
                        .text_size(px(10.))
                        .child(error),
                )
            })
            .children(objects.into_iter().enumerate().map(|(index, object)| {
                let kind = match object.kind {
                    SchemaObjectKind::Trigger => "Trigger",
                    SchemaObjectKind::Sequence => "Sequence",
                    SchemaObjectKind::Function => "Function",
                    SchemaObjectKind::Procedure => "Procedure",
                };
                div().w_full().px(px(6.)).child(
                    div()
                        .id(SharedString::from(format!(
                            "schema-object-{session_id}-{index}"
                        )))
                        .w_full()
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(RADIUS_CONTROL))
                        .text_size(px(12.))
                        .text_color(theme().text)
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .cursor_pointer()
                        .hover(|style| style.bg(theme().glass_hover))
                        .tooltip(tip(object.name.clone()))
                        .child(icon(Icon::Query, theme().text_muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(object.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .text_color(theme().text_muted)
                                .child(kind),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(definition) = &object.definition {
                                this.open_saved_query_for(
                                    session_id,
                                    SavedQuery {
                                        name: object.name.clone(),
                                        sql: definition.clone(),
                                    },
                                    window,
                                    cx,
                                );
                            } else {
                                this.show_toast(
                                    ToastKind::Info,
                                    "This account cannot read the object definition",
                                    cx,
                                );
                            }
                        })),
                )
            }))
            .into_any_element()
    }
}
