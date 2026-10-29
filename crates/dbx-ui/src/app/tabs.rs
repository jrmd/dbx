//! Workbench tabs.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum TabCloseScope {
    This,
    Others,
    Right,
}

/// Moves `from` into `to`'s slot: dropping on a tab to the right lands after
/// it, to the left before it, matching browser tab strips.
pub(super) fn move_to_slot<T>(items: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

pub(super) struct TabGroupEdit {
    pub(super) session_id: SessionId,
    pub(super) group_id: TabGroupId,
    pub(super) name: Entity<TextEditor>,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
    _blur: Subscription,
}

impl ConnectionSession {
    /// Drops memberships of closed tabs and groups left without members.
    pub(super) fn prune_tab_groups(&mut self) {
        let open: HashSet<_> = self.secondary_tabs.iter().map(|tab| tab.id).collect();
        self.tab_group_members.retain(|tab, _| open.contains(tab));
        let used: HashSet<_> = self.tab_group_members.values().copied().collect();
        self.tab_groups.retain(|group| used.contains(&group.id));
    }

    /// Moves `tab_id` to sit directly after the last member of `group`.
    fn place_after_group(&mut self, tab_id: SecondaryTabId, group: TabGroupId) {
        let Some(from) = self.secondary_tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let tab = self.secondary_tabs.remove(from);
        let end = self
            .secondary_tabs
            .iter()
            .rposition(|t| self.tab_group_members.get(&t.id) == Some(&group))
            .map_or(self.secondary_tabs.len(), |index| index + 1);
        self.secondary_tabs.insert(end, tab);
    }

    /// Groups as saved: members are positions in `secondary_tabs`.
    pub(super) fn saved_tab_groups(&self) -> Vec<crate::workspace::SavedTabGroup> {
        self.tab_groups
            .iter()
            .map(|group| crate::workspace::SavedTabGroup {
                name: group.name.clone(),
                color: group.color,
                collapsed: group.collapsed,
                members: self
                    .secondary_tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, tab)| self.tab_group_members.get(&tab.id) == Some(&group.id))
                    .map(|(index, _)| index)
                    .collect(),
            })
            .collect()
    }

    pub(super) fn tab_group_label(&self, group: &TabGroup) -> String {
        if group.name.trim().is_empty() {
            "Group".into()
        } else {
            group.name.clone()
        }
    }
}

impl DbxApp {
    /// Saves group changes with the connection's tab layout.
    pub(super) fn persist_tab_layout(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        self.persist_startup_workspace(cx);
        self.persist_query_workspace_for(session_id, cx);
    }

    pub(super) fn add_tab_to_new_group(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let id = session.next_tab_group;
        session.next_tab_group += 1;
        let color = (id as usize - 1) % TAB_GROUP_COLORS.len();
        session.tab_groups.push(TabGroup {
            id,
            name: String::new(),
            color,
            collapsed: false,
        });
        session.tab_group_members.insert(tab_id, id);
        session.place_after_group(tab_id, id);
        session.prune_tab_groups();
        self.persist_tab_layout(session_id, cx);
        self.open_tab_group_edit(session_id, id, window, cx);
        cx.notify();
    }

    pub(super) fn add_tab_to_group(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        group: TabGroupId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        if !session.tab_groups.iter().any(|g| g.id == group) {
            return;
        }
        session.tab_group_members.insert(tab_id, group);
        session.place_after_group(tab_id, group);
        session.prune_tab_groups();
        self.persist_tab_layout(session_id, cx);
        cx.notify();
    }

    pub(super) fn remove_tab_from_group(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(group) = session.tab_group_members.remove(&tab_id) else {
            return;
        };
        // Leaving from the middle would split the group, so step past its end.
        session.place_after_group(tab_id, group);
        session.prune_tab_groups();
        self.persist_tab_layout(session_id, cx);
        cx.notify();
    }

    pub(super) fn ungroup_tabs(
        &mut self,
        session_id: SessionId,
        group: TabGroupId,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            session.tab_group_members.retain(|_, g| *g != group);
            session.prune_tab_groups();
            self.persist_tab_layout(session_id, cx);
            cx.notify();
        }
    }

    pub(super) fn toggle_tab_group(
        &mut self,
        session_id: SessionId,
        group: TabGroupId,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self
            .session_mut(session_id)
            .and_then(|s| s.tab_groups.iter_mut().find(|g| g.id == group))
        {
            entry.collapsed = !entry.collapsed;
            self.persist_tab_layout(session_id, cx);
            cx.notify();
        }
    }

    pub(super) fn set_tab_group_color(
        &mut self,
        session_id: SessionId,
        group: TabGroupId,
        color: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self
            .session_mut(session_id)
            .and_then(|s| s.tab_groups.iter_mut().find(|g| g.id == group))
        {
            entry.color = color % TAB_GROUP_COLORS.len();
            self.persist_tab_layout(session_id, cx);
            cx.notify();
        }
    }

    pub(super) fn tab_group_members_of(
        &self,
        session_id: SessionId,
        group: TabGroupId,
    ) -> Vec<SecondaryTabId> {
        self.session(session_id)
            .map(|session| {
                session
                    .secondary_tabs
                    .iter()
                    .filter(|tab| session.tab_group_members.get(&tab.id) == Some(&group))
                    .map(|tab| tab.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn open_tab_group_edit(
        &mut self,
        session_id: SessionId,
        group_id: TabGroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .session(session_id)
            .and_then(|s| s.tab_groups.iter().find(|g| g.id == group_id))
            .map(|g| g.name.clone())
            .unwrap_or_default();
        let name =
            cx.new(|cx| TextEditor::empty(false, window, cx).accessibility_label("Group name"));
        name.update(cx, |editor, cx| editor.set_text(current, cx));
        let subscription = cx.observe(&name, |_, _, cx| cx.notify());
        let return_focus = window.focused(cx);
        let focus = name.read(cx).focus_handle();
        focus.focus(window, cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| {
            this.submit_tab_group_edit(window, cx)
        });
        self.tab_group_edit = Some(TabGroupEdit {
            session_id,
            group_id,
            name,
            return_focus,
            _subscription: subscription,
            _blur: blur,
        });
        cx.notify();
    }

    pub(super) fn close_tab_group_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prompt) = self.tab_group_edit.take() {
            if let Some(focus) = prompt.return_focus {
                focus.focus(window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn submit_tab_group_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = &self.tab_group_edit else {
            return;
        };
        let name = prompt.name.read(cx).text(cx).trim().to_owned();
        let (session_id, group_id) = (prompt.session_id, prompt.group_id);
        if let Some(entry) = self
            .session_mut(session_id)
            .and_then(|s| s.tab_groups.iter_mut().find(|g| g.id == group_id))
        {
            entry.name = name;
        }
        self.close_tab_group_edit(window, cx);
        self.persist_tab_layout(session_id, cx);
    }

    /// Reorders a connection tab. Order is part of the startup workspace.
    pub(super) fn move_session(
        &mut self,
        session_id: SessionId,
        target: SessionId,
        cx: &mut Context<Self>,
    ) {
        let from = self
            .sessions
            .iter()
            .position(|session| session.id == session_id);
        let to = self
            .sessions
            .iter()
            .position(|session| session.id == target);
        if let (Some(from), Some(to)) = (from, to)
            && move_to_slot(&mut self.sessions, from, to)
        {
            self.persist_startup_workspace(cx);
            cx.notify();
        }
    }

    /// Reorders a document tab within its own connection.
    pub(super) fn move_secondary_tab(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        target: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let from = session
            .secondary_tabs
            .iter()
            .position(|tab| tab.id == tab_id);
        let to = session
            .secondary_tabs
            .iter()
            .position(|tab| tab.id == target);
        if let (Some(from), Some(to)) = (from, to)
            && move_to_slot(&mut session.secondary_tabs, from, to)
        {
            // Dropping on a grouped tab joins its group; dropping on a loose
            // tab leaves any group. Members stay adjacent either way.
            match session.tab_group_members.get(&target).copied() {
                Some(group) => session.tab_group_members.insert(tab_id, group),
                None => session.tab_group_members.remove(&tab_id),
            };
            session.prune_tab_groups();
            self.persist_tab_layout(session_id, cx);
            cx.notify();
        }
    }

    /// Moves a whole group so it lands beside `target`, after it when moving
    /// right and before it when moving left, never splitting another group.
    pub(super) fn move_tab_group(
        &mut self,
        session_id: SessionId,
        group: TabGroupId,
        target: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let in_group = |session: &ConnectionSession, id: SecondaryTabId, g: TabGroupId| {
            session.tab_group_members.get(&id) == Some(&g)
        };
        if in_group(session, target, group) {
            return;
        }
        let Some(first) = session
            .secondary_tabs
            .iter()
            .position(|tab| in_group(session, tab.id, group))
        else {
            return;
        };
        let Some(target_index) = session.secondary_tabs.iter().position(|t| t.id == target) else {
            return;
        };
        let moving_right = first < target_index;
        let target_group = session.tab_group_members.get(&target).copied();
        let span = |session: &ConnectionSession| match target_group {
            Some(g) => {
                let positions = session
                    .secondary_tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, tab)| in_group(session, tab.id, g))
                    .map(|(index, _)| index);
                (
                    positions.clone().min().unwrap_or(target_index),
                    positions.max().unwrap_or(target_index),
                )
            }
            None => (target_index, target_index),
        };
        let (start, end) = span(session);
        // Pull the members out, then insert at the boundary recomputed
        // against the shortened list.
        let all = std::mem::take(&mut session.secondary_tabs);
        let (moved, mut rest): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|tab| session.tab_group_members.get(&tab.id) == Some(&group));
        let removed_before = |index: usize, original: &[usize]| {
            original.iter().filter(|&&i| i < index).count()
        };
        let member_indexes: Vec<usize> = {
            // Original indexes of members, reconstructed from `first`.
            let mut indexes = Vec::new();
            let mut cursor = first;
            while indexes.len() < moved.len() {
                indexes.push(cursor);
                cursor += 1;
            }
            indexes
        };
        let at = if moving_right {
            end + 1 - removed_before(end + 1, &member_indexes)
        } else {
            start - removed_before(start, &member_indexes)
        };
        let at = at.min(rest.len());
        for (offset, tab) in moved.into_iter().enumerate() {
            rest.insert(at + offset, tab);
        }
        session.secondary_tabs = rest;
        self.persist_tab_layout(session_id, cx);
        cx.notify();
    }

    pub(super) fn tabs_to_close(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        scope: TabCloseScope,
    ) -> Vec<SecondaryTabId> {
        let Some(session) = self.session(session_id) else {
            return Vec::new();
        };
        let Some(index) = session
            .secondary_tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return Vec::new();
        };
        session
            .secondary_tabs
            .iter()
            .enumerate()
            .filter_map(|(i, tab)| {
                let close = match scope {
                    TabCloseScope::This => i == index,
                    TabCloseScope::Others => i != index,
                    TabCloseScope::Right => i > index,
                };
                close.then_some(tab.id)
            })
            .collect()
    }

    pub(super) fn request_close_tabs_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_ids = self.tabs_to_close(session_id, tab_id, scope);
        self.request_close_tab_ids_for(session_id, tab_ids, window, cx);
    }

    pub(super) fn request_close_tab_ids_for(
        &mut self,
        session_id: SessionId,
        tab_ids: Vec<SecondaryTabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tab_ids.is_empty() {
            return;
        }
        if tab_ids.len() == 1 {
            self.request_close_secondary_tab_for(session_id, tab_ids[0], window, cx);
            return;
        }
        let session = self.session(session_id).unwrap();
        let mut edited_data = false;
        let mut edited_queries = false;
        for tab in &session.secondary_tabs {
            if !tab_ids.contains(&tab.id) {
                continue;
            }
            match &tab.kind {
                SecondaryTabKind::Data(data) => edited_data |= data.has_unsaved_cell_work(),
                SecondaryTabKind::Query(query) => {
                    let text = query.query_editor.read(cx).text(cx);
                    edited_queries |=
                        !text.trim().is_empty() && text != Self::default_query(session.kind);
                }
                _ => {}
            }
        }
        if !edited_data && !edited_queries {
            self.close_secondary_tabs_for(session_id, tab_ids, cx);
            return;
        }
        // Confirm the entire batch before removing anything. Repeated single-tab
        // requests would overwrite one another's discard confirmations.
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: format!("Close {} tabs?", tab_ids.len()),
            detail: match (edited_data, edited_queries) {
                (true, true) => "Uncommitted row changes and open cell edits in these tabs will be discarded. Recently closed queries can be reopened from Query options until you disconnect.",
                (true, false) => "Uncommitted row changes and open cell edits in these tabs will be discarded.",
                _ => "Recently closed queries can be reopened from Query options until you disconnect.",
            }.into(),
            confirm_label: if edited_data { "Discard and close" } else { "Close tabs" },
            tone: if edited_data { ConfirmationTone::Danger } else { ConfirmationTone::Warning },
            action: ConfirmationAction::CloseTabs { session_id, tab_ids },
            focus: focus.clone(),
            return_focus: window.focused(cx),
            sql: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_secondary_tabs_for(
        &mut self,
        session_id: SessionId,
        tab_ids: Vec<SecondaryTabId>,
        cx: &mut Context<Self>,
    ) {
        for tab_id in tab_ids.into_iter().rev() {
            self.discard_pending_edits_for(session_id, tab_id, cx);
            self.close_secondary_tab_for(session_id, tab_id, cx);
        }
    }

    pub(super) fn close_other_sessions(&mut self, keep: SessionId, cx: &mut Context<Self>) {
        if self.session(keep).is_none() {
            return;
        }
        let others = self
            .sessions
            .iter()
            .filter(|session| session.id != keep)
            .collect::<Vec<_>>();
        if self.vault_state == Some(VaultState::Unlocked)
            && others.iter().any(|session| {
                session.secondary_tabs.iter().any(|tab| {
                matches!(&tab.kind, SecondaryTabKind::Data(data) if data.has_unsaved_cell_work())
            })
            })
        {
            self.show_toast(
                ToastKind::Info,
                "Save or discard cell edits before closing other connections",
                cx,
            );
            return;
        }
        let ids = others.iter().map(|session| session.id).collect::<Vec<_>>();
        for id in ids {
            self.close_session(id, cx);
        }
        self.activate_session(keep, cx);
    }

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
        query_tab
            .query_editor
            .update(cx, |editor, cx| editor.set_vim_mode(self.vim_mode, cx));
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
                    scroll: ScrollHandle::new(),
                    expanded_sections: HashSet::new(),
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

    pub(super) fn refresh_open_structures_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let expected_database = session.current_database.clone();
        let structures = session
            .secondary_tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                SecondaryTabKind::Structure(structure) => Some((tab.id, structure.table.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (id, table) in structures {
            let engine = engine.clone();
            let expected_engine = engine.clone();
            let database = expected_database.clone();
            if let Some(session) = self.session_mut(session_id)
                && let Some(tab) = session.secondary_tabs.iter_mut().find(|tab| tab.id == id)
                && let SecondaryTabKind::Structure(structure) = &mut tab.kind
            {
                structure.busy = true;
            }
            let task = self
                .runtime
                .spawn(async move { engine.table_structure(&table).await });
            if let Some(session) = self.session_mut(session_id) {
                session.track_background_task(&task);
            }
            cx.spawn(async move |this, cx| {
                let result = task.await?;
                this.update(cx, |this, cx| {
                    let Some(session) = this.session_mut(session_id) else {
                        return;
                    };
                    if session.current_database != database
                        || !session
                            .engine
                            .as_ref()
                            .is_some_and(|engine| Arc::ptr_eq(engine, &expected_engine))
                    {
                        return;
                    }
                    let Some(tab) = session.secondary_tabs.iter_mut().find(|tab| tab.id == id)
                    else {
                        return;
                    };
                    let SecondaryTabKind::Structure(structure) = &mut tab.kind else {
                        return;
                    };
                    structure.busy = false;
                    match result {
                        Ok(metadata) => {
                            structure.columns = metadata.columns;
                            structure.foreign_keys = metadata.foreign_keys;
                            structure.indexes = metadata.indexes;
                            structure.checks = metadata.checks;
                            structure.definition = metadata.definition;
                            structure.error = None;
                        }
                        Err(error) => {
                            structure.error = Some(format!(
                                "Structure refresh failed; displayed metadata may be stale: {error}"
                            ));
                        }
                    }
                    cx.notify();
                })?;
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        }
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
        diagram
            .scroll_handle
            .set_offset(point(crate::theme::ui_px(0.), crate::theme::ui_px(0.)));
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
        diagram
            .scroll_handle
            .set_offset(point(crate::theme::ui_px(0.), crate::theme::ui_px(0.)));
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
        anchor.moved |= travel > crate::theme::ui_px(3.);
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

    pub(super) fn move_diagram_node_by_for(
        &mut self,
        session_id: SessionId,
        node_id: &str,
        dx: f32,
        dy: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let Some(document) = diagram.document.as_mut() else {
            return;
        };
        let Some((x, y)) = document.node_position(node_id) else {
            return;
        };
        if Arc::make_mut(document).move_node(node_id, x + dx, y + dy)
            && let Some(position) = document.node_position(node_id)
        {
            diagram
                .arranged_positions
                .insert(node_id.to_owned(), position);
            cx.notify();
        }
    }

    pub(super) fn select_diagram_relative_for(
        &mut self,
        session_id: SessionId,
        direction: isize,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let Some(document) = diagram.document.as_ref() else {
            return;
        };
        if document.nodes.is_empty() {
            return;
        }
        let current = diagram
            .selected_node
            .as_ref()
            .and_then(|id| document.nodes.iter().position(|node| &node.id == id));
        let next = current.map_or(
            if direction < 0 {
                document.nodes.len() - 1
            } else {
                0
            },
            |index| {
                index
                    .saturating_add_signed(direction)
                    .min(document.nodes.len() - 1)
            },
        );
        diagram.selected_node = Some(document.nodes[next].id.clone());
        diagram.list_scroll.scroll_to_top_of_item(next);
        cx.notify();
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
        if let Some((table, counts, editing)) = self.data_tab(session_id, tab_id).map(|data| {
            (
                table_ref_label(&data.table),
                data.change_counts(),
                data.cell_editor.is_some(),
            )
        }) && (counts.total() > 0 || editing)
        {
            let focus = cx.focus_handle();
            self.confirmation_dialog = Some(ConfirmationDialog {
                title: format!("Discard changes to {table}?"),
                detail: if counts.total() > 0 {
                    format!(
                        "{} will be lost.",
                        counted(counts.total(), "staged change", "staged changes")
                    )
                } else {
                    "The open cell edit will be lost.".into()
                },
                confirm_label: "Discard",
                tone: ConfirmationTone::Danger,
                action: ConfirmationAction::DiscardDataTab { session_id, tab_id },
                focus: focus.clone(),
                return_focus: window.focused(cx),
                sql: None,
            });
            focus.focus(window, cx);
            cx.notify();
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
            sql: None,
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
        session.prune_tab_groups();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_a_tab_takes_the_target_slot_in_either_direction() {
        let mut tabs = vec!['a', 'b', 'c', 'd'];
        assert!(move_to_slot(&mut tabs, 0, 2));
        assert_eq!(
            tabs,
            ['b', 'c', 'a', 'd'],
            "rightward drops land after the target"
        );
        assert!(move_to_slot(&mut tabs, 3, 0));
        assert_eq!(
            tabs,
            ['d', 'b', 'c', 'a'],
            "leftward drops land before the target"
        );
        assert!(!move_to_slot(&mut tabs, 1, 1));
        assert!(!move_to_slot(&mut tabs, 0, 9));
        assert_eq!(tabs, ['d', 'b', 'c', 'a']);
    }

    #[gpui::test]
    fn bulk_tab_close_confirms_before_removing_any_tabs_and_uses_clicked_order(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let session_id = uuid::Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.sessions.push(ConnectionSession::new(
                session_id,
                None,
                "Tabs".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            ));
            app.active_session_id = Some(session_id);
            for _ in 0..4 {
                app.add_query_tab_for(session_id, window, cx);
            }
            app.active_query_tab_mut(session_id)
                .unwrap()
                .query_editor
                .update(cx, |editor, cx| {
                    editor.set_text("SELECT 42", cx);
                });
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let ids = app
                    .session(session_id)
                    .unwrap()
                    .secondary_tabs
                    .iter()
                    .map(|tab| tab.id)
                    .collect::<Vec<_>>();
                assert_eq!(
                    app.tabs_to_close(session_id, ids[1], TabCloseScope::Right),
                    ids[2..]
                );
                assert!(
                    app.tabs_to_close(session_id, ids[3], TabCloseScope::Right)
                        .is_empty()
                );
                app.request_close_tabs_for(session_id, ids[1], TabCloseScope::Right, window, cx);
                assert!(matches!(
                    app.confirmation_dialog
                        .as_ref()
                        .map(|dialog| &dialog.action),
                    Some(ConfirmationAction::CloseTabs { .. })
                ));
                assert_eq!(app.session(session_id).unwrap().secondary_tabs.len(), 4);
                app.cancel_confirmation(window, cx);
                assert_eq!(app.session(session_id).unwrap().secondary_tabs.len(), 4);
                app.request_close_tabs_for(session_id, ids[1], TabCloseScope::Right, window, cx);
                app.confirm_pending_action(window, cx);
                let session = app.session(session_id).unwrap();
                assert_eq!(
                    session
                        .secondary_tabs
                        .iter()
                        .map(|tab| tab.id)
                        .collect::<Vec<_>>(),
                    ids[..2]
                );
                assert_eq!(session.active_secondary_tab, Some(ids[1]));
                assert_eq!(session.closed_queries, vec!["SELECT 42"]);
                app.request_close_tabs_for(session_id, ids[1], TabCloseScope::Others, window, cx);
                assert_eq!(app.session(session_id).unwrap().secondary_tabs.len(), 1);
                assert_eq!(
                    app.session(session_id).unwrap().secondary_tabs[0].id,
                    ids[1]
                );
            })
        });
    }

    #[gpui::test]
    fn bulk_closing_preserves_dirty_data_until_confirmed(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let keep = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        let dirty = uuid::Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            for id in [keep, other] {
                app.sessions.push(ConnectionSession::new(
                    id,
                    None,
                    "Connection".into(),
                    DatabaseKind::SQLite,
                    None,
                    window,
                    cx,
                ));
            }
            app.active_session_id = Some(other);
            let mut data = DataTab::new(other, dirty, TableRef::new("items"), true, window, cx);
            data.pending_edits.insert(
                (0, 0),
                MutationValue::Parameter(CellValue::Text("unsaved".into())),
            );
            app.session_mut(other)
                .unwrap()
                .secondary_tabs
                .push(SecondaryTab {
                    id: dirty,
                    kind: SecondaryTabKind::Data(Box::new(data)),
                });
            app.add_query_tab_for(other, window, cx);
            app.add_query_tab_for(other, window, cx);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let anchor = app.session(other).unwrap().active_secondary_tab.unwrap();
                app.close_other_sessions(keep, cx);
                assert_eq!(app.sessions.len(), 2);
                assert!(app.data_tab(other, dirty).unwrap().has_pending_edits());
                app.request_close_tabs_for(other, anchor, TabCloseScope::Others, window, cx);
                assert_eq!(app.session(other).unwrap().secondary_tabs.len(), 3);
                assert_eq!(
                    app.confirmation_dialog.as_ref().unwrap().tone,
                    ConfirmationTone::Danger
                );
                app.cancel_confirmation(window, cx);
                assert!(app.data_tab(other, dirty).unwrap().has_pending_edits());
                app.request_close_tabs_for(other, anchor, TabCloseScope::Others, window, cx);
                app.confirm_pending_action(window, cx);
                assert_eq!(app.session(other).unwrap().secondary_tabs.len(), 1);
                assert!(app.data_tab(other, dirty).is_none());
                app.close_other_sessions(keep, cx);
                assert_eq!(app.sessions.len(), 1);
                assert_eq!(app.active_session_id, Some(keep));
            })
        });
    }
}
