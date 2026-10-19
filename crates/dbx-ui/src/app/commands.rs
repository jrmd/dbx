//! Commands shared by the palette and keyboard shortcuts settings.
use super::*;
use crate::keybindings::{self, KeymapState};
use gpui_component::ActiveTheme;

pub(super) struct ShortcutEditor {
    editor: Entity<TextEditor>,
    error: Option<String>,
}

pub(super) fn palette_commands() -> &'static [(&'static str, &'static str)] {
    &[
        ("dbx_ui::NewConnection", "New connection"),
        ("dbx_ui::NewQuery", "New query"),
        ("dbx_ui::CloseTab", "Close tab"),
        ("dbx_ui::OpenQuickOpen", "Go to table, query or connection"),
        ("dbx_ui::OpenSettings", "Open settings"),
        (
            "dbx_ui::OpenKeyboardShortcuts",
            "Keyboard shortcuts and keymap",
        ),
        ("dbx_ui::SearchQueryHistory", "Search query history"),
        ("dbx_ui::RefreshData", "Refresh data"),
        ("dbx_ui::ToggleSidebar", "Toggle sidebar"),
        ("dbx_ui::FocusExplorer", "Focus explorer"),
        ("dbx_ui::NextConnection", "Next connection"),
        ("dbx_ui::PreviousConnection", "Previous connection"),
        ("dbx_ui::NextTab", "Next tab"),
        ("dbx_ui::PreviousTab", "Previous tab"),
        ("dbx_ui::RunQuery", "Run query"),
        ("dbx_ui::RunQueryAll", "Run all statements"),
        ("dbx_ui::FormatQuery", "Format query"),
        ("dbx_ui::CheckForUpdates", "Check for updates"),
    ]
}

pub(super) fn shortcut_for(name: &str, cx: &App) -> String {
    let Ok(action) = cx.build_action(name, None) else {
        return String::new();
    };
    cx.key_bindings()
        .borrow()
        .bindings_for_action(action.as_ref())
        .rfind(|binding| {
            if cfg!(target_os = "macos") {
                !binding
                    .keystrokes()
                    .iter()
                    .any(|key| key.to_string().starts_with("ctrl-"))
            } else {
                !binding
                    .keystrokes()
                    .iter()
                    .any(|key| key.to_string().contains("cmd-"))
            }
        })
        .map(|binding| {
            binding
                .keystrokes()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
impl DbxApp {
    pub(super) fn open_command_palette_action(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_open_action(&OpenQuickOpen, window, cx);
        if let Some(open) = &self.quick_open {
            open.query.update(cx, |editor, cx| editor.set_text(">", cx));
        }
    }
    pub(super) fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
    }
    pub(super) fn open_shortcuts_action(
        &mut self,
        _: &OpenKeyboardShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_section(SettingsSection::Shortcuts, window, cx);
    }
    pub(super) fn search_history_action(
        &mut self,
        _: &SearchQueryHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_history(window, cx);
    }

    pub(super) fn render_settings_shortcuts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.shortcut_editor.is_none() {
            let (document, error) = if cx.has_global::<KeymapState>() {
                let state = cx.global::<KeymapState>();
                (state.document.clone(), state.load_error.clone())
            } else {
                (keybindings::initial_document(), None)
            };
            let editor = cx.new(|cx| {
                let text = cx.new(|_| document);
                TextEditor::new_json(text, window, cx).accessibility_label("Editable JSON keymap")
            });
            self.shortcut_editor = Some(ShortcutEditor { editor, error });
        }
        let state = self.shortcut_editor.as_ref().unwrap();
        let input = state.editor.clone();
        let error = state.error.clone();
        let mut rows = Vec::new();
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        for binding in keymap.bindings() {
            let name = binding.action().name();
            if name.contains("NoAction") {
                continue;
            }
            if !keymap
                .bindings_for_action(binding.action())
                .any(|effective| {
                    effective.keystrokes() == binding.keystrokes()
                        && effective.predicate().map(|p| p.to_string())
                            == binding.predicate().map(|p| p.to_string())
                })
            {
                continue;
            }
            let keys = binding
                .keystrokes()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ");
            let context = binding
                .predicate()
                .map(|context| context.to_string())
                .unwrap_or_else(|| "Global".into());
            rows.push((name.to_owned(), keys, context));
        }
        rows.sort();
        rows.dedup();
        div().flex().flex_col().gap(crate::theme::ui_px(16.))
            .child(div().text_size(crate::theme::text_px(16.)).child("Custom keymap"))
            .child(format!("Edit Zed-style JSON. Use null to disable a shortcut; key chords use spaces. Saved changes apply immediately. File: {}", keybindings::path().display()))
            .child(div().h(crate::theme::ui_px(240.)).overflow_hidden().child(editor::input_with_height(input.clone(), input.read(cx).focus_handle(), true, crate::theme::ui_px(240.), cx)))
            .when_some(error, |view, error| view.child(div().text_color(theme().danger).child(error)))
            .child(div().flex().gap(crate::theme::ui_px(8.))
                .child(button("save-keymap", "Save keymap", ButtonKind::Primary).on_click(cx.listener(|this, _, _, cx| {
                    let Some(editor) = &this.shortcut_editor else { return; };
                    let document = editor.editor.read(cx).text(cx);
                    match keybindings::save(document, cx) {
                        Ok(()) => { if let Some(editor) = &mut this.shortcut_editor { editor.error = None; } this.show_toast(ToastKind::Success, "Keymap saved", cx); }
                        Err(error) => { if let Some(editor) = &mut this.shortcut_editor { editor.error = Some(error); } }
                    }
                    cx.notify();
                })))
                .child(button("reset-keymap", "Reset to defaults", ButtonKind::Quiet).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(editor) = &mut this.shortcut_editor { editor.editor.update(cx, |editor, cx| editor.set_text(keybindings::initial_document(), cx)); editor.error = None; }
                    cx.notify();
                }))))
            .child(div().text_size(crate::theme::text_px(16.)).child("Keyboard shortcuts"))
            .children(rows.into_iter().map(|(name, keys, context)| {
                div().py(crate::theme::ui_px(8.)).border_b_1().border_color(theme().hairline).flex().gap(crate::theme::ui_px(16.))
                    .child(div().flex_1().child(keybindings::label(&name)).child(div().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted).child(format!("{name} · {context}"))))
                    .child(div().font_family(cx.theme().mono_font_family.clone()).child(keys))
            }))
            .into_any_element()
    }
}
