//! Keyboard focus traversal. Tab and Shift-Tab walk every tab stop in the
//! window; while a modal is open they stay inside it, so the workspace behind
//! the scrim can't be reached until the modal closes.

use super::*;

/// Tab and Shift-Tab walk every control; F6 does the same from widgets that
/// keep Tab for themselves, like the SQL editor's indent.
///
/// These carry no context on purpose. After a tab restores, focus can sit on a
/// handle that isn't painted, which leaves no key context to match. GPUI ranks
/// a context-free binding as the deepest and breaks ties by registration order,
/// so register these before the editor and grid bindings: indent and cell
/// commit-and-move then still win in their own widgets.
pub fn focus_key_bindings() -> Vec<gpui::KeyBinding> {
    vec![
        gpui::KeyBinding::new("tab", FocusNext, None),
        gpui::KeyBinding::new("shift-tab", FocusPrevious, None),
        gpui::KeyBinding::new("f6", FocusNext, None),
        gpui::KeyBinding::new("shift-f6", FocusPrevious, None),
    ]
}

/// Bound on wrap-around steps. Backgrounds hold a few dozen tab stops, so this
/// only stops a modal with no stops from spinning.
const MAX_FOCUS_STEPS: usize = 2048;

impl DbxApp {
    /// Whether the workspace is the screen being drawn, which is where the
    /// export dialog, mutation error and table menu live. Mirrors the choice
    /// `render` makes between settings, the connection list and the workspace.
    pub(super) fn workspace_screen_drawn(&self) -> bool {
        self.vault_state == Some(VaultState::Unlocked)
            && !self.settings_open
            && !self.connection_picker_open
            && self.active_session().is_some()
    }

    /// Focus handles of the open modals, topmost first. Each is tracked on its
    /// modal's container element. Only modals that are painted are listed: a
    /// handle nothing draws can't hold focus, and chasing one would fight
    /// `reclaim_stray_focus` every frame.
    pub(super) fn modal_traps(&self) -> Vec<FocusHandle> {
        if self.vault_state != Some(VaultState::Unlocked) {
            return Vec::new();
        }
        let mut traps = Vec::new();
        traps.extend(self.confirmation_dialog.as_ref().map(|d| d.focus.clone()));
        if self.workspace_screen_drawn() {
            traps.extend(self.mutation_error_dialog.as_ref().map(|d| d.focus.clone()));
            traps.extend(self.value_popover_focus());
        }
        traps.extend(self.data_import_dialog.as_ref().map(|d| d.focus.clone()));
        traps.extend(
            self.profile_transfer_dialog
                .as_ref()
                .map(|d| d.focus.clone()),
        );
        traps.extend(self.quick_open.as_ref().map(|d| d.focus.clone()));
        traps.extend(self.nonrelational_editor.as_ref().map(|d| d.focus.clone()));
        traps.extend(self.table_name_prompt.as_ref().map(|d| d.focus.clone()));
        if self.workspace_screen_drawn() {
            traps.extend(
                self.database_export_dialog
                    .as_ref()
                    .map(|d| d.focus.clone()),
            );
            traps.extend(self.table_context_menu.as_ref().map(|m| m.focus.clone()));
        }
        traps
    }

    pub(super) fn modal_is_background(&self, focus: &FocusHandle) -> bool {
        self.modal_traps().first().is_some_and(|top| top != focus)
    }

    /// Pull focus back to the window root when it sits on a handle that isn't
    /// painted, such as the editor of a tab that was restored but isn't showing.
    /// With no focused element in the tree, shortcuts and Tab have nowhere to
    /// dispatch.
    ///
    /// Containment is judged against the last painted frame, so an element
    /// that was only just given focus looks stray for one render. The first
    /// sighting asks for another render; focus moves only if it's still
    /// outside once that frame has painted.
    pub(super) fn reclaim_stray_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_handle.contains_focused(window, cx) {
            self.stray_focus_seen = false;
        } else if std::mem::replace(&mut self.stray_focus_seen, true) {
            self.stray_focus_seen = false;
            self.focus_handle.focus(window, cx);
        } else {
            cx.notify();
        }
    }

    /// Move focus into the topmost modal when it opens, and back into it after
    /// a click on its scrim lands focus on the window root.
    pub(super) fn ensure_modal_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let top = self.modal_traps().into_iter().next();
        let opened = top.is_some() && top != self.last_modal_trap;
        self.last_modal_trap = top.clone();
        let Some(trap) = top else {
            self.modal_focus_pending = false;
            return;
        };
        let on_root = match window.focused(cx) {
            None => true,
            Some(handle) => handle == self.focus_handle,
        };
        if on_root {
            self.modal_focus_pending = false;
            trap.focus(window, cx);
        } else if opened {
            // Not every opener focuses its modal, and focus may be on an editor
            // or menu behind the scrim. Check once the modal has painted, so one
            // that already focused a field of its own isn't overridden.
            self.modal_focus_pending = true;
            cx.notify();
        } else if std::mem::take(&mut self.modal_focus_pending)
            && !trap.contains_focused(window, cx)
        {
            trap.focus(window, cx);
        }
    }

    /// Tab (`forward`) or Shift-Tab. Wraps within the topmost modal, or across
    /// the whole window when none is open.
    pub(super) fn move_focus(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let step = |window: &mut Window, cx: &mut Context<Self>| {
            if forward {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
        };
        let Some(trap) = self.modal_traps().into_iter().next() else {
            step(window, cx);
            return;
        };
        if !trap.contains_focused(window, cx) {
            trap.focus(window, cx);
        }
        let start = window.focused(cx);
        step(window, cx);
        // Stepping past the modal's last (or before its first) stop lands in
        // the workspace; keep going until the walk wraps back inside.
        let mut steps = 0;
        while !trap.contains_focused(window, cx) && steps < MAX_FOCUS_STEPS {
            step(window, cx);
            steps += 1;
        }
        if !trap.contains_focused(window, cx) {
            // A modal without tab stops (a context menu) keeps its own focus.
            match start {
                Some(start) => start.focus(window, cx),
                None => trap.focus(window, cx),
            }
        }
        cx.notify();
    }
}
