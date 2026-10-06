//! Find and replace in a query tab's editor.

use std::ops::Range;

use super::*;

pub(super) const FIND_CONTEXT: &str = "DbxTextEditor DbxFind";

pub(super) struct FindBar {
    pub(super) needle: Entity<TextEditor>,
    pub(super) replacement: Entity<TextEditor>,
    pub(super) show_replace: bool,
    pub(super) case_sensitive: bool,
    pub(super) results: bool,
    _needle_subscription: Subscription,
}

/// Non-overlapping UTF-8 byte ranges of `needle` in `haystack`.
pub(super) fn find_matches(
    haystack: &str,
    needle: &str,
    case_sensitive: bool,
) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return haystack
            .match_indices(needle)
            .map(|(start, found)| start..start + found.len())
            .collect();
    }
    // Lowercasing can change byte lengths, so compare char by char on the
    // original text instead of searching a lowercased copy.
    let needle = needle
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let mut matches = Vec::new();
    let mut start = 0;
    while start < haystack.len() {
        let mut chars = haystack[start..].char_indices();
        let mut expected = needle.iter();
        let mut end = None;
        'scan: for (offset, character) in chars.by_ref() {
            for lowered in character.to_lowercase() {
                match expected.next() {
                    Some(wanted) if *wanted == lowered => {}
                    _ => break 'scan,
                }
            }
            if expected.len() == 0 {
                end = Some(start + offset + character.len_utf8());
                break;
            }
        }
        match end {
            Some(end) => {
                matches.push(start..end);
                start = end;
            }
            None => {
                start += haystack[start..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    matches
}

impl DbxApp {
    fn find_target_for(&self, session_id: SessionId) -> Option<(Entity<TextEditor>, &FindBar)> {
        let query = self.active_query_tab(session_id)?;
        Some((query.query_editor.clone(), query.find.as_ref()?))
    }

    pub(super) fn open_find_for(
        &mut self,
        session_id: SessionId,
        show_replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(query) = self.active_query_tab(session_id) else {
            return;
        };
        let selected = query
            .query_editor
            .read(cx)
            .selected_text(cx)
            .filter(|text| !text.contains('\n'));
        let needle = match &query.find {
            Some(find) => {
                let needle = find.needle.clone();
                if let Some(selected) = selected {
                    needle.update(cx, |editor, cx| editor.set_text(selected, cx));
                }
                needle
            }
            None => {
                let value = cx.new(|_| selected.unwrap_or_default());
                let needle = cx.new(|cx| TextEditor::new(value, false, window, cx));
                let replacement = cx.new(|cx| TextEditor::empty(false, window, cx));
                let subscription = cx.observe(&needle, move |this, _, cx| {
                    this.find_step_for(session_id, None, cx);
                });
                if let Some(query) = self.active_query_tab_mut(session_id) {
                    query.find = Some(FindBar {
                        needle: needle.clone(),
                        replacement,
                        show_replace,
                        case_sensitive: false,
                        results: false,
                        _needle_subscription: subscription,
                    });
                }
                needle
            }
        };
        if let Some(find) = self
            .active_query_tab_mut(session_id)
            .and_then(|query| query.find.as_mut())
        {
            find.show_replace |= show_replace;
        }
        needle.update(cx, |editor, cx| editor.select_all_text(cx));
        needle.read(cx).focus_handle().focus(window, cx);
        self.find_step_for(session_id, None, cx);
        cx.notify();
    }

    pub(super) fn close_find_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query.find = None;
        }
        self.focus_active_query_editor_for(session_id, window, cx);
        cx.notify();
    }

    /// Select the next (`Some(true)`), previous (`Some(false)`), or first
    /// match at or after the selection start (`None`).
    pub(super) fn find_step_for(
        &mut self,
        session_id: SessionId,
        forward: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some((editor, find)) = self.find_target_for(session_id) else {
            return;
        };
        let needle = find.needle.read(cx).text(cx);
        let case_sensitive = find.case_sensitive;
        if find.results {
            let Some(query) = self.active_query_tab(session_id) else {
                return;
            };
            let grid = query.result_grid.clone();
            let matches = grid
                .read(cx)
                .delegate()
                .matching_cells(&needle, case_sensitive);
            let selected = grid.read(cx).selected_cell();
            let position =
                selected.and_then(|cell| matches.iter().position(|found| *found == cell));
            let next = match (forward, position) {
                (Some(true), Some(index)) => matches.get((index + 1) % matches.len()),
                (Some(false), Some(index)) => {
                    matches.get((index + matches.len() - 1) % matches.len())
                }
                _ => matches.first(),
            }
            .copied();
            if let Some((row, column)) = next {
                grid.update(cx, |grid, cx| grid.set_selected_cell(row, column, cx));
                if let Some(query) = self.active_query_tab_mut(session_id) {
                    query.result_selection = QueryResultSelection::Cell;
                }
            }
            cx.notify();
            return;
        }
        let text = editor.read(cx).text(cx);
        let matches = find_matches(&text, &needle, case_sensitive);
        let selection = editor.read(cx).selected_range();
        let next = match forward {
            None => matches
                .iter()
                .find(|range| range.start >= selection.start)
                .or_else(|| matches.first()),
            Some(true) => matches
                .iter()
                .find(|range| range.start >= selection.end && **range != selection)
                .or_else(|| matches.first()),
            Some(false) => matches
                .iter()
                .rev()
                .find(|range| range.end <= selection.start)
                .or_else(|| matches.last()),
        }
        .cloned();
        if let Some(range) = next {
            editor.update(cx, |editor, cx| editor.select_range(range, cx));
        }
        cx.notify();
    }

    pub(super) fn toggle_find_case_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(find) = self
            .active_query_tab_mut(session_id)
            .and_then(|query| query.find.as_mut())
        {
            find.case_sensitive = !find.case_sensitive;
        }
        self.find_step_for(session_id, None, cx);
    }

    /// Replace the selected match, then select the next one.
    pub(super) fn replace_match_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some((editor, find)) = self.find_target_for(session_id) else {
            return;
        };
        if find.results {
            return;
        }
        let needle = find.needle.read(cx).text(cx);
        let replacement = find.replacement.read(cx).text(cx);
        let case_sensitive = find.case_sensitive;
        let text = editor.read(cx).text(cx);
        let selection = editor.read(cx).selected_range();
        if find_matches(&text, &needle, case_sensitive).contains(&selection) {
            editor.update(cx, |editor, cx| {
                editor.replace_range(selection, &replacement, cx)
            });
        }
        self.find_step_for(session_id, Some(true), cx);
    }

    pub(super) fn replace_all_matches_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some((editor, find)) = self.find_target_for(session_id) else {
            return;
        };
        if find.results {
            return;
        }
        let needle = find.needle.read(cx).text(cx);
        let replacement = find.replacement.read(cx).text(cx);
        let case_sensitive = find.case_sensitive;
        let text = editor.read(cx).text(cx);
        let matches = find_matches(&text, &needle, case_sensitive);
        if matches.is_empty() {
            return;
        }
        let mut replaced = text.clone();
        for range in matches.iter().rev() {
            replaced.replace_range(range.clone(), &replacement);
        }
        // One edit keeps Replace All a single undo step.
        editor.update(cx, |editor, cx| {
            editor.replace_range(0..text.len(), &replaced, cx)
        });
        self.show_toast(
            ToastKind::Success,
            format!("Replaced {}", counted(matches.len(), "match", "matches")),
            cx,
        );
    }

    pub(super) fn render_find_bar(
        &self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (editor, find) = self.find_target_for(session_id)?;
        let needle = find.needle.clone();
        let replacement = find.replacement.clone();
        let show_replace = find.show_replace && !find.results;
        let results = find.results;
        let case_sensitive = find.case_sensitive;
        let text = editor.read(cx).text(cx);
        let needle_text = needle.read(cx).text(cx);
        let status = if results {
            let count = self
                .active_query_tab(session_id)
                .map(|query| {
                    query
                        .result_grid
                        .read(cx)
                        .delegate()
                        .matching_cells(&needle_text, case_sensitive)
                        .len()
                })
                .unwrap_or_default();
            format!("{count} cells")
        } else {
            let matches = find_matches(&text, &needle_text, case_sensitive);
            let selection = editor.read(cx).selected_range();
            match matches.iter().position(|range| *range == selection) {
                _ if needle_text.is_empty() => String::new(),
                _ if matches.is_empty() => "No matches".into(),
                Some(index) => format!("{} of {}", index + 1, matches.len()),
                None => counted(matches.len(), "match", "matches"),
            }
        };
        let needle_focus = needle.read(cx).focus_handle();
        let replacement_focus = replacement.read(cx).focus_handle();
        let row = || div().flex().items_center().gap(px(6.));
        Some(
            div()
                .flex_none()
                .px(px(9.))
                .py(px(5.))
                .flex()
                .flex_col()
                .gap(px(5.))
                .border_b_1()
                .border_color(theme().border)
                .bg(theme().panel)
                .on_action(cx.listener(move |this, _: &FindNext, _, cx| {
                    this.find_step_for(session_id, Some(true), cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(move |this, _: &FindPrevious, _, cx| {
                    this.find_step_for(session_id, Some(false), cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(move |this, _: &CloseFind, window, cx| {
                    this.close_find_for(session_id, window, cx);
                    cx.stop_propagation();
                }))
                .child(
                    row()
                        .child(
                            button(
                                "find-target",
                                if results { "Results" } else { "Editor" },
                                ButtonKind::Quiet,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(find) = this
                                        .active_query_tab_mut(session_id)
                                        .and_then(|query| query.find.as_mut())
                                    {
                                        find.results = !find.results;
                                    }
                                    this.find_step_for(session_id, None, cx);
                                },
                            )),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(editor::input_with_key_context(
                                    needle,
                                    needle_focus,
                                    false,
                                    FIND_CONTEXT,
                                )),
                        )
                        .child(
                            div()
                                .w(px(76.))
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(theme().text_muted)
                                .child(status),
                        )
                        .child(
                            button(
                                "find-case",
                                "Aa",
                                if case_sensitive {
                                    ButtonKind::Primary
                                } else {
                                    ButtonKind::Quiet
                                },
                            )
                            .tooltip("Match case")
                            .cursor_pointer()
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.toggle_find_case_for(session_id, cx),
                            )),
                        )
                        .child(
                            Button::new("find-previous")
                                .with_size(Size::XSmall)
                                .compact()
                                .ghost()
                                .tooltip("Previous match (Shift+Enter)")
                                .child(icon(Icon::ChevronUp, theme().text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.find_step_for(session_id, Some(false), cx)
                                })),
                        )
                        .child(
                            Button::new("find-next")
                                .with_size(Size::XSmall)
                                .compact()
                                .ghost()
                                .tooltip("Next match (Enter)")
                                .child(icon(Icon::ChevronDown, theme().text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.find_step_for(session_id, Some(true), cx)
                                })),
                        )
                        .child(
                            Button::new("find-close")
                                .with_size(Size::XSmall)
                                .compact()
                                .ghost()
                                .tooltip("Close (Esc)")
                                .child(icon(Icon::Close, theme().text_muted))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_find_for(session_id, window, cx)
                                })),
                        ),
                )
                .when(show_replace, |view| {
                    view.child(
                        row()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(editor::input_with_key_context(
                                        replacement,
                                        replacement_focus,
                                        false,
                                        FIND_CONTEXT,
                                    )),
                            )
                            .child(
                                button("replace-one", "Replace", ButtonKind::Quiet)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.replace_match_for(session_id, cx)
                                    })),
                            )
                            .child(
                                button("replace-all", "Replace all", ButtonKind::Quiet)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.replace_all_matches_for(session_id, cx)
                                    })),
                            ),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_case_folded_on_the_original_byte_offsets() {
        let text = "SELECT Ünïcode, select FROM t";
        assert_eq!(
            find_matches(text, "select", true),
            std::iter::once(18..24).collect::<Vec<_>>()
        );
        assert_eq!(find_matches(text, "select", false), [0..6, 18..24]);
        assert_eq!(
            find_matches(text, "ünï", false),
            std::iter::once(7..12).collect::<Vec<_>>()
        );
        assert_eq!(&text[7..12], "Ünï");
        assert!(find_matches(text, "", false).is_empty());
        assert_eq!(find_matches("aaaa", "aa", true), [0..2, 2..4]);
    }
}
