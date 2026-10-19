//! Identity-aware MongoDB and structured Redis editors.
use super::*;
use serde_json::{Value, json};

#[derive(Clone)]
enum EditSource {
    Mongo { collection: String, original: Value },
    Redis(dbx_core::RedisEditSnapshot),
}
pub(super) struct NonrelationalEditor {
    session_id: SessionId,
    tab_id: SecondaryTabId,
    engine: Arc<DatabaseEngine>,
    database: Option<String>,
    source: EditSource,
    editor: Entity<TextEditor>,
    error: Option<String>,
    review: Option<String>,
    review_summary: String,
    busy: bool,
    pub(super) focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}
fn json_cell(value: &CellValue) -> Value {
    match value {
        CellValue::Null => Value::Null,
        CellValue::Boolean(v) => json!(v),
        CellValue::Integer(v) => json!(v),
        CellValue::Unsigned(v) => json!(v),
        CellValue::Real(v) => json!(v),
        CellValue::Text(v) => json!(v),
        CellValue::Json(v) => v.clone(),
        CellValue::Bytes(_) => Value::Null,
    }
}
impl DbxApp {
    pub(super) fn invalidate_nonrelational_for(&mut self, session_id: Option<SessionId>) {
        if session_id.is_some()
            && self
                .nonrelational_editor
                .as_ref()
                .is_some_and(|editor| Some(editor.session_id) != session_id)
        {
            return;
        }
        self.nonrelational_generation = self.nonrelational_generation.wrapping_add(1);
        self.nonrelational_editor = None;
    }
    pub(super) fn validate_nonrelational_editor(&mut self) {
        let stale = self.nonrelational_editor.as_ref().is_some_and(|editor| {
            self.vault_state != Some(VaultState::Unlocked)
                || !self.session(editor.session_id).is_some_and(|session| {
                    session.current_database == editor.database
                        && session.data_tab(editor.tab_id).is_some()
                        && session
                            .engine
                            .as_ref()
                            .is_some_and(|engine| Arc::ptr_eq(engine, &editor.engine))
                })
        });
        if stale {
            self.invalidate_nonrelational_for(None);
        }
    }
    pub(super) fn open_nonrelational_editor(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nonrelational_generation = self.nonrelational_generation.wrapping_add(1);
        let generation = self.nonrelational_generation;
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(engine) = session.engine.clone() else {
            return;
        };
        if engine.is_read_only() || session.busy {
            return;
        }
        let Some(data) = session.data_tab(tab_id) else {
            return;
        };
        let Some(row) = data
            .selected_row
            .and_then(|row| data.result.as_ref()?.rows.get(row))
        else {
            return;
        };
        let Some(result) = &data.result else {
            return;
        };
        let field = if session.kind == DatabaseKind::Redis {
            "key"
        } else {
            "_id"
        };
        let Some(index) = result
            .columns
            .iter()
            .position(|column| column.name == field)
        else {
            return;
        };
        let Some(identity) = row.values.get(index).cloned() else {
            return;
        };
        let collection = data.table.name.clone();
        let kind = session.kind;
        let database = session.current_database.clone();
        let expected = engine.clone();
        let return_focus = window.focused(cx);
        let task = self.runtime.spawn(async move {
            match kind {
                DatabaseKind::MongoDB => {
                    let original = engine.document_for_edit(&TableRef::new(collection.clone()), &json_cell(&identity)).await?;
                    Ok(EditSource::Mongo { collection, original })
                }
                DatabaseKind::Redis => {
                    let key = match identity { CellValue::Text(key) => key, _ => return Err(dbx_core::DbxError::Parse("The structured editor currently requires a UTF-8 key".into())) };
                    let DatabaseEngine::Redis(engine) = engine.as_ref() else { return Err(dbx_core::DbxError::Parse("This community driver does not expose the native Redis structured editor".into())); };
                    engine.read_structured(&key).await.map(EditSource::Redis)
                }
                _ => Err(dbx_core::DbxError::Parse("No structured editor for this connection".into())),
            }
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await?;
            this.update_in(cx, |this, window, cx| {
                if this.vault_state != Some(VaultState::Unlocked)
                    || this.nonrelational_generation != generation
                    || !this.session(session_id).is_some_and(|session| {
                        session.current_database == database
                            && session
                                .engine
                                .as_ref()
                                .is_some_and(|engine| Arc::ptr_eq(engine, &expected))
                    })
                {
                    return;
                }
                match result {
                    Ok(source) => {
                        let document = match &source {
                            EditSource::Mongo { original, .. } => original,
                            EditSource::Redis(snapshot) => &snapshot.document,
                        };
                        let text = serde_json::to_string_pretty(document).unwrap();
                        let editor = cx.new(|cx| {
                            let text = cx.new(|_| text);
                            TextEditor::new_json(text, window, cx)
                                .accessibility_label("Structured document")
                        });
                        editor.read(cx).focus_handle().focus(window, cx);
                        this.nonrelational_editor = Some(NonrelationalEditor {
                            session_id,
                            tab_id,
                            engine: expected,
                            database,
                            source,
                            editor,
                            error: None,
                            review: None,
                            review_summary: String::new(),
                            busy: false,
                            focus: cx.focus_handle(),
                            return_focus,
                        });
                        cx.notify();
                    }
                    Err(error) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
    pub(super) fn close_nonrelational_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .nonrelational_editor
            .as_ref()
            .is_some_and(|editor| editor.busy)
        {
            return;
        }
        self.nonrelational_generation = self.nonrelational_generation.wrapping_add(1);
        if let Some(editor) = self.nonrelational_editor.take() {
            if let Some(focus) = editor.return_focus {
                focus.focus(window, cx);
            }
            cx.notify();
        }
    }
    fn review_nonrelational_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.nonrelational_editor else {
            return;
        };
        let text = editor.editor.read(cx).text(cx);
        let validation = serde_json::from_str::<Value>(&text)
            .map_err(|error| error.to_string())
            .and_then(|document| match &editor.source {
                EditSource::Mongo {
                    collection,
                    original,
                } => dbx_core::mongo_replacement(collection, original, &document)
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                EditSource::Redis(snapshot) => {
                    dbx_core::validate_redis_edit(&snapshot.document, &document)
                        .map_err(|error| error.to_string())
                }
            });
        match validation {
            Ok(()) => {
                let original = match &editor.source {
                    EditSource::Mongo { original, .. } => original,
                    EditSource::Redis(snapshot) => &snapshot.document,
                };
                let document: Value = serde_json::from_str(&text).expect("validated JSON");
                editor.review_summary = describe_changes(original, &document);
                editor.review = Some(text);
                editor.error = None;
            }
            Err(error) => editor.error = Some(error),
        }
        cx.notify();
    }
    fn apply_nonrelational_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.nonrelational_editor else {
            return;
        };
        if editor.busy {
            return;
        }
        let Some(text) = editor.review.clone() else {
            return;
        };
        let session_id = editor.session_id;
        let tab_id = editor.tab_id;
        let engine = editor.engine.clone();
        let database = editor.database.clone();
        if self.vault_state != Some(VaultState::Unlocked)
            || !self.session(session_id).is_some_and(|session| {
                session.current_database == database
                    && session.data_tab(tab_id).is_some()
                    && session
                        .engine
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &engine))
            })
        {
            self.show_toast(
                ToastKind::Error,
                "The connection or database changed. Reload before editing.",
                cx,
            );
            return;
        }
        let document: Value = match serde_json::from_str(&text) {
            Ok(document) => document,
            Err(_) => return,
        };
        let generation = self.nonrelational_generation;
        let source = editor.source.clone();
        let task = self.runtime.spawn(async move {
            match source {
                EditSource::Mongo {
                    collection,
                    original,
                } => {
                    engine
                        .replace_mongo_document(&collection, &original, &document)
                        .await
                }
                EditSource::Redis(snapshot) => match engine.as_ref() {
                    DatabaseEngine::Redis(redis) => {
                        redis.save_structured(&snapshot, &document).await
                    }
                    _ => unreachable!(),
                },
            }
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        self.nonrelational_editor.as_mut().unwrap().busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await?;
            this.update_in(cx, |this, window, cx| {
                if this.vault_state != Some(VaultState::Unlocked)
                    || this.nonrelational_generation != generation
                {
                    return;
                }
                if let Some(editor) = &mut this.nonrelational_editor {
                    editor.busy = false;
                }
                match result {
                    Ok(()) => {
                        this.close_nonrelational_editor(window, cx);
                        this.refresh_table_for(session_id, tab_id, cx);
                        this.show_toast(ToastKind::Success, "Changes saved", cx);
                    }
                    Err(error) => {
                        if let Some(editor) = &mut this.nonrelational_editor {
                            editor.error = Some(error.to_string());
                        }
                        cx.notify();
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_nonrelational_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.nonrelational_editor else {
            return div().into_any_element();
        };
        let focus = editor.focus.clone();
        let input = editor.editor.clone();
        let busy = editor.busy;
        let review = editor.review.clone();
        let error = editor.error.clone();
        let (title, detail) = match &editor.source {
            EditSource::Mongo {
                collection,
                original,
            } => (
                format!("Edit {} in {}", short_json(&original["_id"]), collection),
                "Review the document replacement before applying. _id cannot change. Concurrent changes require a reload.",
            ),
            EditSource::Redis(snapshot) if snapshot.document["type"] == "stream" => (
                format!("Edit stream {}", snapshot.key),
                "Review stream operations before applying. The first 1,000 entries are read-only. Add [field,value] pairs to append, or entry IDs to delete. TTL is preserved; concurrent changes require a reload.",
            ),
            EditSource::Redis(snapshot) => (
                format!("Edit key {}", snapshot.key),
                "Review the value replacement before applying. TTL is preserved; empty collections remove the key. Concurrent changes require a reload.",
            ),
        };
        let environment = self
            .session(editor.session_id)
            .and_then(|session| session.tag.as_ref())
            .map(|tag| {
                if tag.is_production() {
                    format!("Production · {}", tag.name)
                } else {
                    tag.name.clone()
                }
            });
        let background = self.modal_is_background(&focus);
        deferred(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme().overlay)
                .child(
                    glass_raised(div(), RADIUS_GLASS)
                        .id("nonrelational-editor")
                        .w(crate::theme::ui_px(760.))
                        .max_w(relative(0.95))
                        .max_h(relative(0.92))
                        .p(crate::theme::ui_px(18.))
                        .flex()
                        .flex_col()
                        .gap(crate::theme::ui_px(12.))
                        .child(div().text_ellipsis().child(title.clone()))
                        .when_some(environment, |view, label| {
                            view.child(div().font_weight(gpui::FontWeight::MEDIUM).child(label))
                        })
                        .child(
                            div()
                                .text_color(theme().text_muted)
                                .text_size(crate::theme::text_px(12.))
                                .child(detail),
                        )
                        .map(|view| {
                            if let Some(text) = review.clone() {
                                view.child(
                                    div()
                                        .id("nonrelational-review")
                                        .h(crate::theme::ui_px(360.))
                                        .overflow_y_scroll()
                                        .flex()
                                        .flex_col()
                                        .gap(crate::theme::ui_px(12.))
                                        .font_family(
                                            gpui_component::Theme::global(cx)
                                                .mono_font_family
                                                .clone(),
                                        )
                                        .child(editor.review_summary.clone())
                                        .child(
                                            div()
                                                .text_color(theme().text_muted)
                                                .child("Complete reviewed document"),
                                        )
                                        .child(text),
                                )
                            } else {
                                view.child(editor::input_with_height(
                                    input.clone(),
                                    input.read(cx).focus_handle(),
                                    true,
                                    crate::theme::ui_px(360.),
                                    cx,
                                ))
                            }
                        })
                        .when_some(error, |view, error| {
                            view.child(div().text_color(theme().danger).child(error))
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(crate::theme::ui_px(8.))
                                .child(
                                    button("cancel-nonrelational", "Cancel", ButtonKind::Quiet)
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close_nonrelational_editor(window, cx)
                                        })),
                                )
                                .when(review.is_some(), |view| {
                                    view.child(
                                        button(
                                            "back-nonrelational",
                                            "Back to edit",
                                            ButtonKind::Quiet,
                                        )
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                if let Some(editor) = &mut this.nonrelational_editor
                                                {
                                                    editor.review = None;
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                })
                                .child(
                                    button(
                                        "save-nonrelational",
                                        if busy {
                                            "Saving…"
                                        } else if review.is_some() {
                                            "Apply changes"
                                        } else {
                                            "Review changes"
                                        },
                                        ButtonKind::Primary,
                                    )
                                    .disabled(busy)
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            if review.is_some() {
                                                this.apply_nonrelational_edit(window, cx);
                                            } else {
                                                this.review_nonrelational_edit(cx);
                                            }
                                        },
                                    )),
                                ),
                        )
                        .focus_trap("nonrelational-focus", &focus)
                        .accessible_dialog("nonrelational-editor", title, detail, background, true),
                ),
        )
        .with_priority(44)
        .into_any_element()
    }
}

/// A bounded summary accompanies the complete reviewed JSON; omission is explicit.
fn short_json(value: &impl serde::Serialize) -> String {
    struct Preview(Vec<u8>);
    impl std::io::Write for Preview {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let length = bytes.len().min(1024 - self.0.len());
            self.0.extend_from_slice(&bytes[..length]);
            if length < bytes.len() {
                return Err(std::io::Error::other("preview limit"));
            }
            Ok(length)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // Serialize into a bounded sink instead of allocating an entire large value.
    let mut output = Preview(Vec::new());
    let limited = serde_json::to_writer(&mut output, value).is_err();
    let valid = std::str::from_utf8(&output.0).map_or_else(|error| error.valid_up_to(), str::len);
    let text = std::str::from_utf8(&output.0[..valid]).expect("valid prefix");
    let mut chars = text.chars();
    let preview = chars.by_ref().take(240).collect::<String>();
    if limited || chars.next().is_some() {
        format!("{preview}… [truncated]")
    } else {
        preview
    }
}
fn describe_changes(original: &Value, edited: &Value) -> String {
    let (Some(before), Some(after)) = (original.as_object(), edited.as_object()) else {
        return if original == edited {
            "No changes.".into()
        } else {
            format!(
                "Changed value\nBefore: {}\nAfter: {}",
                short_json(original),
                short_json(edited)
            )
        };
    };
    let mut count = 0;
    let mut changes = Vec::new();
    for (key, old) in before {
        let new = after.get(key);
        if new == Some(old) {
            continue;
        }
        count += 1;
        if changes.len() < 200 {
            changes.push(match new {
                Some(new) => format!(
                    "Changed {}\nBefore: {}\nAfter: {}",
                    short_json(key),
                    short_json(old),
                    short_json(new)
                ),
                None => format!("Removed {}\nBefore: {}", short_json(key), short_json(old)),
            });
        }
    }
    for (key, new) in after {
        if before.contains_key(key) {
            continue;
        }
        count += 1;
        if changes.len() < 200 {
            changes.push(format!(
                "Added {}\nAfter: {}",
                short_json(key),
                short_json(new)
            ));
        }
    }
    if count == 0 {
        return "No changes.".into();
    }
    let mut summary = changes.join("\n\n");
    if count > 200 {
        summary.push_str(&format!(
            "\n\n{} more changed fields. Inspect the complete document below.",
            count - 200
        ));
    }
    summary
}

#[cfg(test)]
mod review_tests {
    use super::*;
    #[test]
    fn review_distinguishes_removed_null_added_and_nested_changes() {
        let before = json!({"_id": 7, "removed": null, "nested": {"x": 1}});
        let after = json!({"_id": 7, "added": null, "nested": {"x": 2}});
        let summary = describe_changes(&before, &after);
        assert!(summary.contains("Removed \"removed\"\nBefore: null"));
        assert!(summary.contains("Added \"added\"\nAfter: null"));
        assert!(summary.contains("Changed \"nested\""));
        assert!(!summary.contains("_id"));
        assert_eq!(describe_changes(&before, &before), "No changes.");
    }
    #[test]
    fn large_reviews_bound_names_values_and_number_of_formatted_fields() {
        let after = (0..205)
            .map(|index| {
                (
                    format!("{index:03}{}", "x".repeat(1000)),
                    json!("y".repeat(5000)),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let summary = describe_changes(&json!({}), &Value::Object(after));
        assert!(summary.contains("5 more changed fields"));
        assert!(summary.len() < 110_000);
        assert!(summary.contains("[truncated]"));
    }
    #[test]
    fn preview_truncates_unicode_without_breaking_codepoints() {
        assert!(short_json(&json!("é".repeat(300))).ends_with("… [truncated]"));
    }
}
