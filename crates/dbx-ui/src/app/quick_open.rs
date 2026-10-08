//! Cmd/Ctrl+P quick open: jump to any table, saved query or connection by
//! typing part of its name.

use super::*;
use crate::profiles::SavedConnection;
use crate::workspace::SavedQuery;

pub(super) const QUICK_OPEN_CONTEXT: &str = "DbxTextEditor DbxQuickOpen";
const MAX_RESULTS: usize = 50;

pub(super) struct QuickOpen {
    pub(super) query: Entity<TextEditor>,
    pub(super) selected: usize,
    _subscription: Subscription,
}

#[derive(Clone)]
enum Target {
    Table(SessionId, TableInfo),
    SavedQuery(SessionId, SavedQuery),
    History(SessionId, QueryHistoryEntry),
    Session(SessionId),
    Connection(Box<SavedConnection>),
}

#[derive(Clone)]
struct Item {
    label: String,
    detail: String,
    icon: Icon,
    target: Target,
}

/// Score `candidate` against a typed `query` as a case-insensitive
/// subsequence. Consecutive matches and matches at word starts score higher;
/// `None` means some query character is missing.
fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let query = query
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    if query.is_empty() {
        return Some(0);
    }
    let mut score = 0;
    let mut wanted = query.iter().peekable();
    let mut previous: Option<char> = None;
    let mut streak = 0;
    for (index, character) in candidate.chars().enumerate() {
        let Some(&&next) = wanted.peek() else {
            break;
        };
        let lowered = character.to_lowercase().next().unwrap_or(character);
        if lowered == next {
            wanted.next();
            streak += 1;
            score += 1 + streak * 2;
            let word_start = previous.is_none_or(|previous| {
                !previous.is_alphanumeric() || (previous.is_lowercase() && character.is_uppercase())
            });
            if word_start {
                score += 6;
            }
            if index == 0 {
                score += 4;
            }
        } else {
            streak = 0;
        }
        previous = Some(character);
    }
    if wanted.peek().is_some() {
        return None;
    }
    // Prefer shorter names when the match quality is otherwise equal.
    Some(score * 100 - candidate.chars().count() as i64)
}

fn history_date(value: &str) -> Option<u64> {
    let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    u64::try_from(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis()).ok()
}

impl DbxApp {
    pub(super) fn open_quick_open_action(
        &mut self,
        _: &OpenQuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        if let Some(open) = &self.quick_open {
            open.query
                .update(cx, |editor, cx| editor.select_all_text(cx));
            open.query.read(cx).focus_handle().focus(window, cx);
            return;
        }
        let query = cx.new(|cx| TextEditor::empty(false, window, cx));
        let subscription = cx.observe(&query, |this, _, cx| {
            if let Some(open) = this.quick_open.as_mut() {
                open.selected = 0;
            }
            cx.notify();
        });
        query.read(cx).focus_handle().focus(window, cx);
        self.quick_open = Some(QuickOpen {
            query,
            selected: 0,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn close_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_open.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    pub(super) fn move_quick_open_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.quick_open_items(cx).len();
        if let Some(open) = self.quick_open.as_mut()
            && count > 0
        {
            open.selected = (open.selected as isize + delta).rem_euclid(count as isize) as usize;
            cx.notify();
        }
    }

    pub(super) fn confirm_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.quick_open.as_ref().map(|open| open.selected) else {
            return;
        };
        let item = self.quick_open_items(cx).into_iter().nth(selected);
        self.close_quick_open(window, cx);
        if let Some(item) = item {
            self.open_quick_open_target(item.target, window, cx);
        }
    }

    fn open_quick_open_target(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Table(session_id, table) => {
                self.activate_session(session_id, cx);
                self.select_table_for(session_id, table, window, cx);
            }
            Target::SavedQuery(session_id, saved) => {
                self.activate_session(session_id, cx);
                self.open_saved_query_for(session_id, saved, window, cx);
            }
            Target::History(session_id, entry) => {
                self.activate_session(session_id, cx);
                self.add_query_tab_for(session_id, window, cx);
                self.load_query_history_entry_for(session_id, &entry, window, cx);
            }
            Target::Session(session_id) => self.activate_session(session_id, cx),
            Target::Connection(profile) => self.open_saved_connection(*profile, window, cx),
        }
    }

    /// Every destination, best match first. With an empty query the active
    /// connection's tables lead.
    fn quick_open_items(&self, cx: &App) -> Vec<Item> {
        let Some(open) = &self.quick_open else {
            return Vec::new();
        };
        let query = open.query.read(cx).text(cx);
        if let Some(search) = query.strip_prefix("history:") {
            return self.history_search_items(search);
        }
        let active = self.active_session_id();
        let mut sessions = self.sessions.iter().collect::<Vec<_>>();
        sessions.sort_by_key(|session| Some(session.id) != active);
        let multiple = sessions.len() > 1;
        let mut items = Vec::new();
        for session in &sessions {
            if session.engine.is_none() {
                continue;
            }
            for table in &session.tables {
                let label = match &table.schema {
                    Some(schema)
                        if multiple
                            || session
                                .tables
                                .iter()
                                .any(|other| other.schema != table.schema) =>
                    {
                        format!("{schema}.{}", table.name)
                    }
                    _ => table.name.clone(),
                };
                items.push(Item {
                    label,
                    detail: session.name.clone(),
                    // Views match the explorer's magnifier icon.
                    icon: match table.kind {
                        dbx_core::EntityKind::View => Icon::Search,
                        _ => Icon::Table,
                    },
                    target: Target::Table(session.id, table.clone()),
                });
            }
            for saved in self.saved_queries_for(session.id) {
                items.push(Item {
                    label: saved.name.clone(),
                    detail: format!("Saved query · {}", session.name),
                    icon: Icon::Query,
                    target: Target::SavedQuery(session.id, saved),
                });
            }
        }
        for session in &sessions {
            items.push(Item {
                label: session.name.clone(),
                detail: "Open connection".into(),
                icon: Icon::Database,
                target: Target::Session(session.id),
            });
        }
        for profile in &self.saved_connections {
            if self
                .sessions
                .iter()
                .any(|session| session.profile_id == Some(profile.id))
            {
                continue;
            }
            items.push(Item {
                label: profile.name.clone(),
                detail: "Connect".into(),
                icon: Icon::Database,
                target: Target::Connection(Box::new(profile.clone())),
            });
        }
        let mut scored = items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| {
                fuzzy_score(&query, &item.label).map(|score| (score, index, item))
            })
            .collect::<Vec<_>>();
        if !query.trim().is_empty() {
            scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        }
        scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, _, item)| item)
            .collect()
    }

    pub(super) fn search_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_quick_open_action(&OpenQuickOpen, window, cx);
        if let Some(open) = &self.quick_open {
            open.query
                .update(cx, |editor, cx| editor.set_text("history:", cx));
        }
    }

    fn history_search_items(&self, search: &str) -> Vec<Item> {
        let mut text = Vec::new();
        let mut success = None;
        let mut after = None;
        let mut before = None;
        let mut connection = None;
        for token in search.split_whitespace() {
            match token {
                "success:true" => success = Some(true),
                "success:false" => success = Some(false),
                _ if token.starts_with("after:") => {
                    let Some(date) = history_date(&token[6..]) else {
                        return Vec::new();
                    };
                    after = Some(date);
                }
                _ if token.starts_with("before:") => {
                    let Some(date) = history_date(&token[7..]) else {
                        return Vec::new();
                    };
                    before = Some(date);
                }
                _ if token.starts_with("connection:") => {
                    connection = Some(token[11..].to_lowercase())
                }
                _ => text.push(token.to_lowercase()),
            }
        }
        self.recent_query_history
            .iter()
            .filter_map(|entry| {
                let session = self.sessions.iter().find(|session| {
                    query_history_connection(session).as_ref() == Some(&entry.connection)
                })?;
                let succeeded = matches!(entry.outcome, QueryHistoryOutcome::Success(_));
                if success.is_some_and(|value| value != succeeded)
                    || after.is_some_and(|date| entry.executed_at_ms < date)
                    || before.is_some_and(|date| entry.executed_at_ms >= date)
                    || connection
                        .as_ref()
                        .is_some_and(|name| !session.name.to_lowercase().contains(name))
                    || !text
                        .iter()
                        .all(|word| entry.sql.to_lowercase().contains(word))
                {
                    return None;
                }
                let timestamp =
                    chrono::DateTime::from_timestamp_millis(entry.executed_at_ms as i64)
                        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_default();
                Some(Item {
                    label: entry.sql.split_whitespace().collect::<Vec<_>>().join(" "),
                    detail: format!(
                        "{} · {} · {timestamp}",
                        session.name,
                        if succeeded { "Success" } else { "Failed" }
                    ),
                    icon: Icon::Query,
                    target: Target::History(session.id, entry.clone()),
                })
            })
            .take(MAX_RESULTS)
            .collect()
    }

    pub(super) fn set_history_policy(
        &mut self,
        session_id: SessionId,
        disabled: Option<bool>,
        retention: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(identity) = self.session(session_id).and_then(query_history_connection) else {
            return;
        };
        let key = crate::workspace::connection_key(&identity);
        let document = self.workspace_documents.entry(key).or_default();
        if let Some(value) = disabled {
            document.history_disabled = value;
        }
        if let Some(value) = retention {
            document.history_retention = value.clamp(1, 100);
        }
        self.persist_query_workspace_for(session_id, cx);
        if let Some(limit) = retention {
            let Some(store) = self.query_history_store.clone() else {
                return;
            };
            let runtime = self.runtime.clone();
            cx.spawn(async move |this, cx| {
                let entries = runtime
                    .spawn_blocking(move || {
                        store.retain(&identity, limit)?;
                        store.load()
                    })
                    .await?;
                if let Ok(entries) = entries {
                    this.update(cx, |this, cx| {
                        this.recent_query_history = entries.into_iter().rev().collect();
                        cx.notify();
                    })?;
                }
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn render_quick_open(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = &self.quick_open else {
            return div().into_any_element();
        };
        let editor = open.query.clone();
        let focus = editor.read(cx).focus_handle();
        let items = self.quick_open_items(cx);
        let selected = open.selected.min(items.len().saturating_sub(1));
        div()
            .id("quick-open-overlay")
            .occlude()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(96.))
            .bg(theme().overlay)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_quick_open(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &QuickOpenNext, _, cx| this.move_quick_open_selection(1, cx)),
            )
            .on_action(cx.listener(|this, _: &QuickOpenPrevious, _, cx| {
                this.move_quick_open_selection(-1, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickOpenConfirm, window, cx| {
                this.confirm_quick_open(window, cx)
            }))
            .child(
                glass_raised(div(), RADIUS_GLASS)
                    .id("quick-open")
                    .debug_selector(|| "quick-open".into())
                    .w(px(560.))
                    .max_h(px(440.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .p(px(8.))
                            .border_b_1()
                            .border_color(theme().border)
                            .child(editor::input_with_key_context(
                                editor,
                                focus,
                                false,
                                QUICK_OPEN_CONTEXT,
                            )),
                    )
                    .child(
                        div()
                            .id("quick-open-results")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p(px(6.))
                            .when(items.is_empty(), |list| {
                                list.child(
                                    div()
                                        .p(px(10.))
                                        .text_size(px(12.))
                                        .text_color(theme().text_muted)
                                        .child("No matches"),
                                )
                            })
                            .children(items.into_iter().enumerate().map(|(index, item)| {
                                let active = index == selected;
                                div()
                                    .id(("quick-open-item", index))
                                    .h(px(34.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .rounded(px(RADIUS_PANEL - 2.))
                                    .cursor_pointer()
                                    .when(active, |row| row.bg(theme().accent_soft))
                                    .when(!active, |row| {
                                        row.hover(|style| style.bg(theme().glass_hover))
                                    })
                                    .child(icon(
                                        item.icon,
                                        if active {
                                            theme().accent
                                        } else {
                                            theme().text_muted
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(13.))
                                            .child(item.label),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .max_w(px(200.))
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(theme().text_muted)
                                            .child(item.detail),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(open) = this.quick_open.as_mut() {
                                            open.selected = index;
                                        }
                                        this.confirm_quick_open(window, cx);
                                    }))
                            })),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::fuzzy_score;

    #[test]
    fn quick_open_prefers_word_starts_and_contiguous_matches() {
        assert!(fuzzy_score("ord", "orders").is_some());
        assert!(fuzzy_score("xyz", "orders").is_none());
        assert!(fuzzy_score("ORD", "orders").is_some());
        assert!(fuzzy_score("oi", "order_items") > fuzzy_score("oi", "notification"));
        assert!(fuzzy_score("user", "users") > fuzzy_score("user", "audit_user_log"));
        assert!(fuzzy_score("users", "users") > fuzzy_score("users", "users_archive"));
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }
}
