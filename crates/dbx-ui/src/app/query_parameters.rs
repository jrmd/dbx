use std::collections::{HashMap, HashSet};

use super::*;

pub(super) const PARAMETER_PROMPT_CONTEXT: &str = "DbxTextEditor DbxQueryParameters";

pub(super) struct ParameterPrompt {
    pub(super) run_all: bool,
    pub(super) fields: Vec<(String, Entity<TextEditor>)>,
}

/// Byte ranges and keys of `:name`/`$1` placeholders outside strings, comments,
/// and `::` casts.
fn named_parameter_tokens(sql: &str) -> Vec<(std::ops::Range<usize>, String)> {
    editor::lex_sql(sql)
        .into_iter()
        .filter(|token| token.kind == editor::SqlTokenKind::Parameter)
        .filter_map(|token| {
            let text = sql.get(token.range.clone())?;
            if let Some(position) = text.strip_prefix('$') {
                return (!position.is_empty()
                    && position.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| (token.range, text.to_owned()));
            }
            let name = text.strip_prefix(':')?;
            let first = name.chars().next()?;
            (first.is_alphabetic() || first == '_').then(|| (token.range, name.to_owned()))
        })
        .collect()
}

fn parameter_label(name: &str) -> String {
    if name.starts_with('$') {
        name.to_owned()
    } else {
        format!(":{name}")
    }
}

/// Distinct parameter names in first-use order.
pub(super) fn named_parameters(sql: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    named_parameter_tokens(sql)
        .into_iter()
        .filter_map(|(_, name)| seen.insert(name.clone()).then_some(name))
        .collect()
}

/// JSON scalar input selects a type; other input is literal text.
/// SQL-looking text is always data, never an expression.
fn parameter_value(input: &str) -> CellValue {
    match serde_json::from_str::<serde_json::Value>(input) {
        Ok(serde_json::Value::Null) => CellValue::Null,
        Ok(serde_json::Value::Bool(value)) => CellValue::Boolean(value),
        Ok(serde_json::Value::Number(value)) => value
            .as_i64()
            .map(CellValue::Integer)
            .unwrap_or_else(|| CellValue::Real(value.as_f64().unwrap_or_default())),
        Ok(serde_json::Value::String(value)) => CellValue::Text(value),
        Ok(value) => CellValue::Json(value),
        Err(_) if input.eq_ignore_ascii_case("NULL") => CellValue::Null,
        Err(_) => CellValue::Text(input.to_owned()),
    }
}

fn prepare_parameters(
    kind: DatabaseKind,
    sql: &str,
    values: &HashMap<String, String>,
) -> Result<Vec<dbx_core::SqlStatement>, String> {
    dbx_core::checked_split_sql_for(Some(kind), sql)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|sql| {
            let tokens = named_parameter_tokens(&sql);
            let mut params = Vec::new();
            let mut output = String::new();
            let mut cursor = 0;
            for (range, name) in tokens {
                let value = values
                    .get(&name)
                    .ok_or_else(|| format!("Enter a value for {}", parameter_label(&name)))?;
                output.push_str(&sql[cursor..range.start]);
                params.push(parameter_value(value));
                output.push_str(&if kind.dialect() == DatabaseKind::PostgreSQL {
                    format!("${}", params.len())
                } else if kind == DatabaseKind::SqlServer {
                    format!("@P{}", params.len())
                } else {
                    "?".to_owned()
                });
                cursor = range.end;
            }
            output.push_str(&sql[cursor..]);
            Ok(dbx_core::SqlStatement::new(output, params))
        })
        .collect()
}

impl DbxApp {
    /// Open the parameter prompt when `query` has placeholders that have not
    /// been confirmed for this run. Returns true when the run must wait.
    pub(super) fn prompt_query_parameters_for(
        &mut self,
        session_id: SessionId,
        query: &str,
        run_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let names = named_parameters(query);
        if names.is_empty() {
            return false;
        }
        let Some((ready, remembered)) = self
            .active_query_tab(session_id)
            .map(|query| (query.parameters_ready, query.parameter_values.clone()))
        else {
            return false;
        };
        if ready {
            return false;
        }
        let fields = names
            .into_iter()
            .map(|name| {
                let value = cx.new(|_| remembered.get(&name).cloned().unwrap_or_default());
                let editor = cx.new(|cx| TextEditor::new(value, false, window, cx));
                (name, editor)
            })
            .collect::<Vec<_>>();
        let first = fields[0].1.read(cx).focus_handle();
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query.parameter_prompt = Some(ParameterPrompt { run_all, fields });
        }
        first.focus(window, cx);
        cx.notify();
        true
    }

    pub(super) fn submit_query_parameters_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self
            .active_query_tab_mut(session_id)
            .and_then(|query| query.parameter_prompt.take())
        else {
            return;
        };
        let values = prompt
            .fields
            .iter()
            .map(|(name, editor)| (name.clone(), editor.read(cx).text(cx)))
            .collect::<Vec<_>>();
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query.parameter_values.extend(values);
            query.parameters_ready = true;
        }
        self.request_run_query_for(session_id, prompt.run_all, window, cx);
        // A confirmation dialog may still be pending; it reuses the values.
        self.focus_active_query_editor_for(session_id, window, cx);
    }

    pub(super) fn cancel_query_parameters_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query.parameter_prompt = None;
            query.parameters_ready = false;
            query.prepared_parameters = None;
        }
        self.focus_active_query_editor_for(session_id, window, cx);
        cx.notify();
    }

    /// Prepare driver-bound values, consuming the prompt confirmation.
    /// Returns an error message when a placeholder has no value.
    pub(super) fn bind_query_parameters_for(
        &mut self,
        session_id: SessionId,
        query: &str,
    ) -> Result<String, String> {
        let names = named_parameters(query);
        let kind = self
            .session(session_id)
            .map(|session| session.kind)
            .unwrap_or(DatabaseKind::SQLite);
        let Some(tab) = self.active_query_tab_mut(session_id) else {
            return Ok(query.to_owned());
        };
        tab.parameters_ready = false;
        tab.prepared_parameters = None;
        if names.is_empty() {
            return Ok(query.to_owned());
        }
        if let Some(missing) = names
            .iter()
            .find(|name| !tab.parameter_values.contains_key(*name))
        {
            return Err(format!(
                "Run the query to enter a value for {}",
                parameter_label(missing)
            ));
        }
        tab.prepared_parameters = Some(prepare_parameters(kind, query, &tab.parameter_values)?);
        Ok(query.to_owned())
    }

    pub(super) fn active_query_tab(&self, session_id: SessionId) -> Option<&QueryTab> {
        let session = self.session(session_id)?;
        let tab_id = session.active_secondary_tab?;
        match &session
            .secondary_tabs
            .iter()
            .find(|tab| tab.id == tab_id)?
            .kind
        {
            SecondaryTabKind::Query(query) => Some(query),
            _ => None,
        }
    }

    pub(super) fn active_query_tab_mut(&mut self, session_id: SessionId) -> Option<&mut QueryTab> {
        let session = self.session_mut(session_id)?;
        let tab_id = session.active_secondary_tab?;
        match &mut session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)?
            .kind
        {
            SecondaryTabKind::Query(query) => Some(query),
            _ => None,
        }
    }

    pub(super) fn render_parameter_prompt(
        &self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fields = self
            .active_query_tab(session_id)?
            .parameter_prompt
            .as_ref()?
            .fields
            .clone();
        Some(
            div()
                .flex_none()
                .px(px(10.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .border_b_1()
                .border_color(theme().border)
                .bg(theme().panel)
                .on_action(
                    cx.listener(move |this, _: &SubmitQueryParameters, window, cx| {
                        this.submit_query_parameters_for(session_id, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_action(
                    cx.listener(move |this, _: &CancelQueryParameters, window, cx| {
                        this.cancel_query_parameters_for(session_id, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme().text)
                                .child("Parameters"),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(theme().text_muted)
                                .child("Bound values: text, 42, true, null, or JSON"),
                        ),
                )
                .children(fields.into_iter().map(|(name, editor)| {
                    let focus = editor.read(cx).focus_handle();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(140.))
                                .flex_none()
                                .truncate()
                                .font_family("monospace")
                                .text_size(px(11.))
                                .text_color(theme().text)
                                .child(parameter_label(&name)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(editor::input_with_key_context(
                                    editor,
                                    focus,
                                    false,
                                    PARAMETER_PROMPT_CONTEXT,
                                )),
                        )
                }))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            button("cancel-query-parameters", "Cancel", ButtonKind::Quiet)
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.cancel_query_parameters_for(session_id, window, cx)
                                })),
                        )
                        .child(
                            button("run-query-parameters", "Run", ButtonKind::Primary)
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.submit_query_parameters_for(session_id, window, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bound_values_cannot_change_sql_structure() {
        let values = HashMap::from([
            ("value".into(), "1; DROP TABLE items".into()),
            ("$1".into(), "42".into()),
        ]);
        let prepared = prepare_parameters(
            DatabaseKind::SQLite,
            "SELECT :value, $1, ':value'; SELECT :value",
            &values,
        )
        .unwrap();
        assert_eq!(prepared.len(), 2);
        assert_eq!(prepared[0].sql, "SELECT ?, ?, ':value'");
        assert_eq!(
            prepared[0].params,
            vec![
                CellValue::Text("1; DROP TABLE items".into()),
                CellValue::Integer(42)
            ]
        );
        assert_eq!(prepared[1].params.len(), 1);
    }
    #[test]
    fn parameters_skip_casts_strings_comments_and_dollar_quotes() {
        assert_eq!(
            named_parameters(
                "SELECT :id::int, ':skip', $1, $$ :skip $$ -- :comment\nWHERE a=:name OR b=:id"
            ),
            ["id", "$1", "name"]
        );
        let values = HashMap::from([("id".into(), "7".into())]);
        let prepared = prepare_parameters(
            DatabaseKind::PostgreSQL,
            "SELECT :id; SELECT :id, :id",
            &values,
        )
        .unwrap();
        assert_eq!(prepared[1].sql, "SELECT $1, $2");
    }
    #[test]
    fn typed_values_and_empty_text() {
        assert_eq!(parameter_value(""), CellValue::Text(String::new()));
        assert_eq!(parameter_value("null"), CellValue::Null);
        assert_eq!(parameter_value("true"), CellValue::Boolean(true));
        assert_eq!(parameter_value("\"42\""), CellValue::Text("42".into()));
        assert!(matches!(parameter_value("{\"a\":1}"), CellValue::Json(_)));
    }
}
