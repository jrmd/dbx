//! A small, native text editor for DBX.
//!
//! GPUI's `EntityInputHandler` is deliberately low level: it is the bridge
//! between the operating system's text input (including IME composition) and
//! the view that paints the text.  This module keeps that bridge in one place
//! so the SQL editor, filter bar, and editable table cells can all share the
//! same behavior.
//!
//! The editor stores its value in an `Entity<String>`.  That makes the value
//! usable by the surrounding DBX view without introducing a second model or
//! a callback protocol.  Selection and composition ranges are UTF-8 byte
//! ranges internally, while GPUI's input protocol is UTF-16; all conversion
//! happens at the boundary below.

mod accessibility;
mod commands;
mod folding;
mod signatures;
mod syntax;
mod vim;
pub use signatures::function_signature;
pub use syntax::*;
pub use vim::VimMode;

use std::ops::Range;

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InteractiveElement as _,
    IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ScrollHandle, ShapedLine, SharedString, Size,
    StatefulInteractiveElement as _, Style, Subscription, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, anchored, deferred, div, fill, point, prelude::*, px, size,
};
use unicode_segmentation::UnicodeSegmentation;

use dbx_ui_components::theme::theme;
use gpui_component::ActiveTheme as _;
use gpui_component::input::{
    MoveEnd, MoveHome, MoveToNextWord, MoveToPreviousWord, SelectToEndOfLine, SelectToNextWordEnd,
    SelectToPreviousWordStart, SelectToStartOfLine,
};

/// Key context placed on the editor's root element.
///
/// DBX can bind the actions returned from [`default_key_bindings`] globally;
/// this context keeps those bindings scoped to text editors.
pub const TEXT_EDITOR_CONTEXT: &str = "DbxTextEditor";

/// Additional context placed around the SQL editor so completion navigation
/// can shadow the generic Up/Down/Enter bindings without changing filter or
/// row-value editors.
pub const SQL_EDITOR_CONTEXT: &str = "DbxSqlEditor";

// Keep both contexts on the query editor itself. GPUI gives a child context
// precedence over its parents, so putting only SQL_EDITOR_CONTEXT on the
// surrounding panel still lets the deeper text-editor bindings win for
// Up/Down/Enter. A combined context lets the completion bindings win at the
// same depth while retaining all of the normal text-editing bindings.
const SQL_TEXT_EDITOR_CONTEXT: &str = "DbxTextEditor DbxSqlEditor";

/// Backwards-compatible name for callers that use the conventional GPUI
/// input terminology.
#[allow(dead_code)]
pub const TEXT_INPUT_CONTEXT: &str = TEXT_EDITOR_CONTEXT;

actions!(
    dbx_text_editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Up,
        Down,
        Enter,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        ShowCharacterPalette,
        Indent,
        Outdent,
        ToggleLineComment,
        DuplicateLines,
        MoveLinesUp,
        MoveLinesDown,
        DeleteLines,
        SelectUp,
        SelectDown,
        MoveToStart,
        MoveToEnd,
        SelectToStart,
        SelectToEnd,
        DeleteWordLeft,
        DeleteToLineStart,
        SelectLine,
        SelectNextOccurrence,
        SelectAllOccurrences,
        Fold,
        Unfold,
    ]
);

/// Construct the standard keymap for [`TextEditor`].
///
/// The caller normally installs this once during application startup with
/// `cx.bind_keys(default_key_bindings())`.  Keeping the bindings here makes
/// the component usable by a shell that wants to merge them with its own
/// keymap instead of requiring global initialization from this module.
pub fn default_key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("backspace", Backspace, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("left", Left, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("right", Right, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-c", Copy, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-c", Copy, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-x", Cut, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-x", Cut, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-z", Undo, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-z", Undo, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-cmd-z", Redo, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-z", Redo, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("home", Home, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("end", End, Some(TEXT_EDITOR_CONTEXT)),
        // Keep DBX's custom renderer on the same navigation contract as the
        // gpui-component input controls. These are intentionally bound in
        // addition to the component's own `Input` context because DBX's
        // syntax-highlighting editor uses its own context.
        KeyBinding::new("cmd-left", MoveHome, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-right", MoveEnd, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new(
            "shift-cmd-left",
            SelectToStartOfLine,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new(
            "shift-cmd-right",
            SelectToEndOfLine,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new("ctrl-left", MoveToPreviousWord, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-right", MoveToNextWord, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new(
            "ctrl-shift-left",
            SelectToPreviousWordStart,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new(
            "ctrl-shift-right",
            SelectToNextWordEnd,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new("up", Up, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("down", Down, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("enter", Enter, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new(
            "ctrl-cmd-space",
            ShowCharacterPalette,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new("shift-up", SelectUp, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-home", SelectToStartOfLine, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-end", SelectToEndOfLine, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-up", MoveToStart, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-down", MoveToEnd, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-home", MoveToStart, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-end", MoveToEnd, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-cmd-up", SelectToStart, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("shift-cmd-down", SelectToEnd, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-home", SelectToStart, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-end", SelectToEnd, Some(TEXT_EDITOR_CONTEXT)),
        // macOS moves by word with Option; other platforms with Ctrl.
        KeyBinding::new("alt-left", MoveToPreviousWord, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("alt-right", MoveToNextWord, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new(
            "alt-shift-left",
            SelectToPreviousWordStart,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new(
            "alt-shift-right",
            SelectToNextWordEnd,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        KeyBinding::new("alt-backspace", DeleteWordLeft, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new(
            "cmd-backspace",
            DeleteToLineStart,
            Some(TEXT_EDITOR_CONTEXT),
        ),
        // Code-editing commands apply to the multiline query editor only, so
        // Tab still moves focus between form fields.
        KeyBinding::new("tab", Indent, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("shift-tab", Outdent, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-]", Indent, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-[", Outdent, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-/", ToggleLineComment, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-/", ToggleLineComment, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("shift-cmd-d", DuplicateLines, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-d", DuplicateLines, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("alt-up", MoveLinesUp, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("alt-down", MoveLinesDown, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("shift-cmd-k", DeleteLines, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-k", DeleteLines, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-l", SelectLine, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-l", SelectLine, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("cmd-d", SelectNextOccurrence, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-d", SelectNextOccurrence, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("f2", SelectAllOccurrences, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("alt-cmd-[", Fold, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("alt-cmd-]", Unfold, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-[", Fold, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new("ctrl-shift-]", Unfold, Some(SQL_EDITOR_CONTEXT)),
        KeyBinding::new(
            "shift-cmd-l",
            SelectAllOccurrences,
            Some(SQL_EDITOR_CONTEXT),
        ),
        KeyBinding::new(
            "ctrl-shift-l",
            SelectAllOccurrences,
            Some(SQL_EDITOR_CONTEXT),
        ),
    ]
    .into_iter()
    .chain(vim::bindings())
    .collect()
}

/// A native single-line or multiline text editor.
const HISTORY_LIMIT: usize = 100;

/// How long the pointer rests on code before its hover card appears.
const HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(350);

pub use dbx_core::language::DiagnosticSeverity;

/// A problem painted with a wavy underline; its message shows on hover.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorDiagnostic {
    pub range: Range<usize>,
    pub severity: DiagnosticSeverity,
    pub message: SharedString,
}

/// What the editor shows when the pointer rests on a range of code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoverInfo {
    pub range: Range<usize>,
    pub title: SharedString,
    pub detail: Option<SharedString>,
}

/// Schema-aware services the host application provides to the SQL editor.
pub trait SqlLanguageHost: 'static {
    /// Information about the code at `offset`, if any.
    fn hover(&self, text: &str, offset: usize, cx: &App) -> Option<HoverInfo>;
    /// Open the object named at `offset`; returns whether there was one.
    fn go_to_definition(
        &self,
        text: &str,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
}

struct EditorHover {
    info: HoverInfo,
    position: Point<Pixels>,
    visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EditorSnapshot {
    text: String,
    selection: Range<usize>,
    selection_reversed: bool,
}

impl EditorSnapshot {
    fn from_editor(editor: &TextEditor, cx: &App) -> Self {
        Self {
            text: editor.text(cx),
            selection: editor.selected_range.clone(),
            selection_reversed: editor.selection_reversed,
        }
    }
}

pub struct TextEditor {
    /// The surrounding view owns this entity and can observe it for changes.
    value: Entity<String>,
    focus_handle: FocusHandle,
    multiline: bool,
    language: EditorLanguage,
    /// Paint text as bullets while retaining the backing value for input and
    /// connection-string construction.
    password: bool,
    /// Allow selection, copy and scrolling but no text changes from input,
    /// clipboard, undo or assistive technology. The owner may still replace
    /// the value programmatically with [`Self::set_text`].
    read_only: bool,
    row_value: bool,
    accessibility_label: SharedString,
    accessible_text: std::rc::Rc<std::cell::RefCell<accessibility::AccessibleText>>,
    /// UTF-8 byte ranges at grapheme boundaries.
    selected_range: Range<usize>,
    /// Further selections edited together with `selected_range`, sorted and
    /// never overlapping it. Their carets sit at their ends. Anything other
    /// than typing and deleting collapses them.
    extra_selections: Vec<Range<usize>>,
    /// Opening offsets of folded blocks, valid for `fold_basis`.
    folds: Vec<usize>,
    /// The text `folds` were last checked against, kept only while folded.
    fold_basis: String,
    /// The folded blocks as of the last render.
    folded: Vec<folding::FoldRegion>,
    /// UTF-8 byte ranges painted with a wavy error underline. The SQL shell
    /// derives these from failed-query messages; they are advisory paint only
    /// and never participate in selection or input handling.
    diagnostics: Vec<EditorDiagnostic>,
    language_host: Option<std::rc::Rc<dyn SqlLanguageHost>>,
    hover: Option<EditorHover>,
    _hover_task: Option<gpui::Task<()>>,
    selection_reversed: bool,
    /// The currently composing IME text, as a UTF-8 byte range.
    marked_range: Option<Range<usize>>,
    scroll_handle: ScrollHandle,
    last_layout: LineLayouts,
    last_bounds: Option<Bounds<Pixels>>,
    /// Lexer tokens, line starts and the measured width, reused across frames
    /// until the text changes.
    paint_cache: std::cell::RefCell<PaintCache>,
    /// Visible, ancestor-clipped viewport used for pointer selection.
    last_viewport: Option<Bounds<Pixels>>,
    selection_pointer: Option<Point<Pixels>>,
    last_revealed_cursor: std::cell::Cell<Option<usize>>,
    is_selecting: bool,
    /// A trackpad wheel gesture that began over this editor owns its scroll.
    wheel_gesture: bool,
    undo_history: Vec<EditorSnapshot>,
    redo_history: Vec<EditorSnapshot>,
    /// Where the current run of typed word characters ends. Continuing the
    /// run joins the previous undo step, so undo removes words, not letters.
    typing_run: Option<usize>,
    /// Scroll the caret into view on the next paint even while unfocused,
    /// so a find bar can reveal the match it selected.
    reveal_requested: std::cell::Cell<bool>,
    _subscriptions: Vec<Subscription>,
    vim: Option<vim::VimState>,
}

/// The old short name is useful when migrating an existing GPUI screen to
/// this component and costs no additional runtime type.
#[allow(dead_code)]
pub type Editor = TextEditor;

impl TextEditor {
    /// Create an editor backed by an existing string entity.
    pub fn new(
        value: Entity<String>,
        multiline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_language(value, multiline, EditorLanguage::PlainText, window, cx)
    }

    /// Create an editor with an explicit syntax language.
    pub fn new_with_language(
        value: Entity<String>,
        multiline: bool,
        language: EditorLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Every field is reachable with Tab; `tab_stop` writes through to the
        // window's handle registry, which the tracked shell element honours.
        let focus_handle = cx.focus_handle().tab_stop(true);
        let observed = cx.observe(&value, |this, value, cx| {
            let text = value.read(cx);
            this.selected_range = clamp_range(text, this.selected_range.clone());
            this.extra_selections.retain(|range| {
                range.end <= text.len()
                    && text.is_char_boundary(range.start)
                    && text.is_char_boundary(range.end)
            });
            this.marked_range = this
                .marked_range
                .take()
                .map(|range| clamp_range(text, range));
            cx.notify();
        });
        let focus = cx.on_focus(&focus_handle, window, |this, _, cx| {
            this.reveal_requested.set(true);
            cx.notify();
        });
        let blur = cx.on_blur(&focus_handle, window, |this, _, cx| {
            this.is_selecting = false;
            this.selection_pointer = None;
            cx.notify();
        });

        Self {
            value,
            focus_handle,
            multiline,
            language,
            password: false,
            read_only: false,
            row_value: false,
            accessibility_label: match language {
                EditorLanguage::Sql(_) => "SQL query",
                EditorLanguage::Redis => "Redis command",
                EditorLanguage::Json => "JSON value",
                EditorLanguage::PlainText => "Text",
            }
            .into(),
            accessible_text: Default::default(),
            selected_range: 0..0,
            extra_selections: Vec::new(),
            folds: Vec::new(),
            fold_basis: String::new(),
            folded: Vec::new(),
            diagnostics: Vec::new(),
            language_host: None,
            hover: None,
            _hover_task: None,
            selection_reversed: false,
            marked_range: None,
            scroll_handle: ScrollHandle::new(),
            last_layout: LineLayouts::default(),
            last_bounds: None,
            paint_cache: Default::default(),
            last_viewport: None,
            selection_pointer: None,
            last_revealed_cursor: std::cell::Cell::new(None),
            is_selecting: false,
            wheel_gesture: false,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            typing_run: None,
            reveal_requested: std::cell::Cell::new(false),
            _subscriptions: vec![observed, focus, blur],
            vim: None,
        }
    }

    /// Create a multiline SQL editor backed by an existing value entity.
    pub fn new_sql(
        value: Entity<String>,
        dialect: SqlDialect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_language(value, true, EditorLanguage::Sql(dialect), window, cx)
    }

    /// Create a multiline Redis command editor backed by an existing value.
    pub fn new_redis(value: Entity<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_language(value, true, EditorLanguage::Redis, window, cx)
    }

    /// Create a multiline JSON editor backed by an existing value entity.
    pub fn new_json(value: Entity<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_language(value, true, EditorLanguage::Json, window, cx)
    }

    /// Paint this editor as a password field without changing its value.
    ///
    /// Password fields are plain-text editors because syntax colors would
    /// otherwise disclose details of the underlying value.
    pub fn password(mut self) -> Self {
        self.password = true;
        self.language = EditorLanguage::PlainText;
        self
    }

    /// Make the editor a selectable, copyable view of its value.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Use the row-value font, including for JSON, and follow preference changes.
    pub fn row_value(mut self) -> Self {
        self.row_value = true;
        self
    }

    /// A persistent field name, independent of its value or placeholder.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = label.into();
        self
    }

    /// Designer fields can change purpose when their operation changes.
    pub fn set_accessibility_label(
        &mut self,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let label = label.into();
        if self.accessibility_label != label {
            self.accessibility_label = label;
            cx.notify();
        }
    }

    fn accessible_action(
        &mut self,
        action: gpui::AccessibleAction,
        data: Option<&gpui::accesskit::ActionData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui::accesskit::{Action, ActionData};
        if self.read_only && matches!(action, Action::SetValue | Action::ReplaceSelectedText) {
            return;
        }
        match (action, data) {
            (Action::SetValue, Some(ActionData::Value(value))) => self.set_text(value.as_ref(), cx),
            (Action::ReplaceSelectedText, Some(ActionData::Value(value))) => {
                self.replace(self.selected_range.clone(), value, cx);
            }
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection)))
                if !self.password =>
            {
                let positions = {
                    let snapshot = self.accessible_text.borrow();
                    if snapshot.text != *self.value.read(cx) {
                        return; // An action from a tree older than the current edit.
                    }
                    snapshot
                        .offset(selection.anchor)
                        .zip(snapshot.offset(selection.focus))
                };
                let Some((anchor, focus)) = positions else {
                    return;
                };
                self.extra_selections.clear();
                self.marked_range = None;
                self.selected_range = anchor.min(focus)..anchor.max(focus);
                self.selection_reversed = focus < anchor;
            }
            (Action::ScrollIntoView, _) => {}
            _ => return,
        }
        self.reveal_requested.set(true);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Create an editor with a new empty value entity.
    #[allow(dead_code)]
    pub fn empty(multiline: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = cx.new(|_| String::new());
        Self::new(value, multiline, window, cx)
    }

    /// Create an empty editor with an explicit syntax language.
    #[allow(dead_code)]
    pub fn empty_with_language(
        multiline: bool,
        language: EditorLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = cx.new(|_| String::new());
        Self::new_with_language(value, multiline, language, window, cx)
    }

    /// The entity containing the current text.
    #[allow(dead_code)]
    pub fn value_entity(&self) -> Entity<String> {
        self.value.clone()
    }

    /// The focus handle used by GPUI to route keyboard and IME input.
    /// Logical scroll geometry for a native host's scroll-edge effects.
    pub fn scroll_geometry(&self) -> (f32, f32) {
        (
            (-self.scroll_handle.offset().y).as_f32(),
            self.scroll_handle.max_offset().y.as_f32(),
        )
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    /// Whether this editor accepts line breaks and vertical navigation.
    #[allow(dead_code)]
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// Read the current value from the backing entity.
    pub fn text(&self, cx: &App) -> String {
        self.value.read(cx).clone()
    }

    /// Replace the current value and put the caret at its end.
    #[allow(dead_code)]
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.record_history(cx);
        self.extra_selections.clear();
        let text = normalize_value(&text.into(), self.multiline);
        let cursor = text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.value.update(cx, |value, cx| {
            *value = text;
            cx.notify();
        });
        cx.notify();
    }

    /// Forget a secret field without leaving its value in undo/redo history.
    pub fn clear_secret(&mut self, cx: &mut Context<Self>) {
        use zeroize::Zeroize;
        self.value.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
        for snapshot in self
            .undo_history
            .iter_mut()
            .chain(self.redo_history.iter_mut())
        {
            snapshot.text.zeroize();
        }
        self.undo_history.clear();
        self.redo_history.clear();
        self.fold_basis.zeroize();
        self.paint_cache.borrow_mut().text.zeroize();
        *self.paint_cache.borrow_mut() = PaintCache::default();
        self.selected_range = 0..0;
        self.extra_selections.clear();
        self.marked_range = None;
        self.typing_run = None;
        cx.notify();
    }

    /// The current selection as UTF-8 byte offsets.
    #[allow(dead_code)]
    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    /// The selected text, if there is a non-empty UTF-8 selection.
    #[allow(dead_code)]
    pub fn selected_text(&self, cx: &App) -> Option<String> {
        let text = self.text(cx);
        (!self.selected_range.is_empty()).then(|| text[self.selected_range.clone()].to_owned())
    }

    /// The syntax this editor highlights and executes.
    pub fn language(&self) -> EditorLanguage {
        self.language
    }

    /// The SQL dialect this editor lexes and splits statements with.
    pub fn sql_dialect(&self) -> SqlDialect {
        match self.language {
            EditorLanguage::Sql(dialect) => dialect,
            _ => None,
        }
    }

    /// Resolve the text DBX should execute for the supplied scope.
    #[allow(dead_code)]
    pub fn execution_text(&self, scope: QueryExecutionScope, cx: &App) -> String {
        let text = self.text(cx);
        let range = execution_range(
            &text,
            self.selected_range.clone(),
            self.cursor_offset(),
            scope,
            self.sql_dialect(),
        );
        text[range].to_owned()
    }

    /// Resolve the UTF-8 byte range DBX should execute for the supplied scope.
    #[allow(dead_code)]
    pub fn execution_range(&self, scope: QueryExecutionScope, cx: &App) -> Range<usize> {
        let text = self.text(cx);
        execution_range(
            &text,
            self.selected_range.clone(),
            self.cursor_offset(),
            scope,
            self.sql_dialect(),
        )
    }

    /// Return the painted location immediately below the caret for a
    /// completion popover. Before the editor has been laid out this safely
    /// falls back to the origin, allowing callers to render without a fixed
    /// query-editor-specific offset.
    #[allow(dead_code)]
    pub fn completion_anchor(&self, cx: &App) -> Point<Pixels> {
        let text = self.value.read(cx);
        completion_anchor(
            text,
            self.cursor_offset_internal(),
            self.last_bounds,
            &self.last_layout,
        )
    }

    /// The UTF-8 byte offset where the next edit will be inserted.
    pub fn cursor_offset(&self) -> usize {
        self.cursor_offset_internal()
    }

    /// Replace a UTF-8 range while keeping the caret immediately after the
    /// inserted text. Completion uses this instead of replacing the entire
    /// backing entity so the rest of a query remains untouched.
    pub fn replace_range(
        &mut self,
        range: Range<usize>,
        inserted: impl AsRef<str>,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, inserted.as_ref(), cx);
    }

    /// Replace the error ranges painted with a wavy underline. Ranges are
    /// UTF-8 byte offsets into the current value; passing an empty vector
    /// clears highlighting. No-ops when the set is unchanged so callers can
    /// invoke this every render frame without invalidating the window.
    pub fn set_diagnostics(&mut self, diagnostics: Vec<EditorDiagnostic>, cx: &mut Context<Self>) {
        if self.diagnostics == diagnostics {
            return;
        }
        let text = self.text(cx);
        self.diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| EditorDiagnostic {
                range: clamp_range(&text, diagnostic.range),
                ..diagnostic
            })
            .collect();
        cx.notify();
    }

    /// Every selection in document order, and which one is the primary.
    fn all_selections(&self) -> (Vec<Range<usize>>, usize) {
        let mut selections = self.extra_selections.clone();
        let primary = selections.partition_point(|range| range.start < self.selected_range.start);
        selections.insert(primary, self.selected_range.clone());
        (selections, primary)
    }

    /// Apply one edit to every selection as a single undo step.
    fn edit_all_selections(
        &mut self,
        edit: impl FnOnce(&str, &[Range<usize>]) -> commands::MultiEdit,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let (selections, primary) = self.all_selections();
        let edit = edit(self.value.read(cx), &selections);
        self.typing_run = None;
        self.marked_range = None;
        self.record_history(cx);
        self.set_text_without_selection(edit.text, cx);
        let mut selections = edit.selections;
        self.selected_range = selections.remove(primary);
        selections.dedup();
        selections.retain(|range| *range != self.selected_range);
        self.extra_selections = selections;
        self.selection_reversed = false;
        self.reveal_requested.set(true);
        cx.notify();
    }

    /// Drop every selection but the primary one; returns whether there were
    /// others. The query shell calls this from Escape.
    pub fn collapse_selections(&mut self, cx: &mut Context<Self>) -> bool {
        if self.extra_selections.is_empty() {
            return false;
        }
        self.extra_selections.clear();
        cx.notify();
        true
    }

    /// Select the word at the caret, or add the next match of the selection.
    fn select_next_occurrence(
        &mut self,
        _: &SelectNextOccurrence,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_code() {
            cx.propagate();
            return;
        }
        let text = self.text(cx);
        if self.selected_range.is_empty() {
            let word = commands::word_at(&text, self.cursor_offset_internal());
            if text[word.clone()]
                .chars()
                .any(|c| c.is_alphanumeric() || c == '_')
            {
                self.selected_range = word;
                self.selection_reversed = false;
                cx.notify();
            }
            return;
        }
        let needle = &text[self.selected_range.clone()];
        let (taken, _) = self.all_selections();
        if let Some(next) =
            commands::next_occurrence(&text, needle, self.selected_range.end, &taken)
        {
            let previous = std::mem::replace(&mut self.selected_range, next);
            let at = self
                .extra_selections
                .partition_point(|range| range.start < previous.start);
            self.extra_selections.insert(at, previous);
            self.selection_reversed = false;
            self.reveal_requested.set(true);
            cx.notify();
        }
    }

    /// Select every match of the selection, or with none every use of the
    /// identifier at the caret in its statement, to rename them together.
    fn select_all_occurrences(
        &mut self,
        _: &SelectAllOccurrences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_code() {
            cx.propagate();
            return;
        }
        let text = self.text(cx);
        let cursor = self.cursor_offset_internal();
        let ranges = if self.selected_range.is_empty() {
            let mut cache = self.paint_cache.borrow_mut();
            cache.refresh(&text, self.language);
            let mut ranges = cache.occurrences(&text, cursor, self.sql_dialect());
            if ranges.is_empty() {
                ranges = cache
                    .sql_tokens(cursor.saturating_sub(1)..cursor)
                    .into_iter()
                    .filter(|token| {
                        token.kind == SqlTokenKind::Identifier
                            && token.range.start <= cursor
                            && cursor <= token.range.end
                    })
                    .map(|token| token.range)
                    .take(1)
                    .collect();
            }
            // Rename the name inside quotes and keep the quotes.
            ranges
                .into_iter()
                .map(|range| {
                    if text[range.clone()].starts_with(['"', '`', '[']) && range.len() >= 2 {
                        range.start + 1..range.end - 1
                    } else {
                        range
                    }
                })
                .collect::<Vec<_>>()
        } else {
            let needle = &text[self.selected_range.clone()];
            text.match_indices(needle)
                .map(|(index, _)| index..index + needle.len())
                .collect()
        };
        // The caret may sit just outside a quoted name's renamed contents.
        let Some(primary) = ranges
            .iter()
            .enumerate()
            .min_by_key(|(_, range)| range.start.abs_diff(cursor).min(range.end.abs_diff(cursor)))
            .map(|(index, _)| index)
        else {
            return;
        };
        let mut ranges = ranges;
        self.selected_range = ranges.remove(primary);
        self.extra_selections = ranges;
        self.selection_reversed = false;
        cx.notify();
    }

    /// Bring folds up to date with the text and selections: follow edits,
    /// drop blocks that no longer fold, and open any fold hiding a caret.
    fn reconcile_folds(&mut self, cx: &App) {
        if self.folds.is_empty() {
            self.folded.clear();
            return;
        }
        let text = self.value.read(cx);
        if self.fold_basis != *text {
            folding::shift_offsets(&self.fold_basis, text, &mut self.folds);
            self.fold_basis.clone_from(text);
        }
        let carets = std::iter::once(&self.selected_range)
            .chain(&self.extra_selections)
            .flat_map(|range| [range.start, range.end])
            .map(|offset| line_and_column(text, clamp_boundary(text, offset)).0)
            .collect::<Vec<_>>();
        let mut cache = self.paint_cache.borrow_mut();
        cache.refresh(text, self.language);
        let regions = cache.fold_regions(text);
        self.folded = regions
            .iter()
            .filter(|region| self.folds.contains(&region.open))
            .filter(|region| !carets.iter().any(|line| region.hidden().contains(line)))
            .cloned()
            .collect();
        self.folds = self.folded.iter().map(|region| region.open).collect();
        if self.folds.is_empty() {
            self.fold_basis.clear();
        }
    }

    /// The innermost foldable block around `line` that `filter` accepts.
    fn fold_region_at(
        &self,
        line: usize,
        cx: &App,
        filter: impl Fn(&folding::FoldRegion) -> bool,
    ) -> Option<folding::FoldRegion> {
        let text = self.value.read(cx);
        let mut cache = self.paint_cache.borrow_mut();
        cache.refresh(text, self.language);
        cache
            .fold_regions(text)
            .iter()
            .filter(|region| (region.start_line..=region.end_line).contains(&line))
            .filter(|region| filter(region))
            .max_by_key(|region| region.start_line)
            .cloned()
    }

    fn fold_region(&mut self, region: folding::FoldRegion, cx: &mut Context<Self>) {
        if self.folds.is_empty() {
            self.fold_basis = self.text(cx);
        }
        self.folds.push(region.open);
        // Park the caret on the visible opening line.
        if region
            .hidden()
            .contains(&line_and_column(&self.text(cx), self.cursor_offset()).0)
            || line_and_column(&self.text(cx), self.cursor_offset()).0 == region.end_line
        {
            self.move_to(region.open + 1, cx);
        }
        self.reconcile_folds(cx);
        cx.notify();
    }

    fn fold(&mut self, _: &Fold, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_code() {
            cx.propagate();
            return;
        }
        let line = line_and_column(&self.text(cx), self.cursor_offset()).0;
        if let Some(region) =
            self.fold_region_at(line, cx, |region| !self.folds.contains(&region.open))
        {
            self.fold_region(region, cx);
        }
    }

    fn unfold(&mut self, _: &Unfold, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_code() {
            cx.propagate();
            return;
        }
        let line = line_and_column(&self.text(cx), self.cursor_offset()).0;
        if let Some(region) =
            self.fold_region_at(line, cx, |region| self.folds.contains(&region.open))
        {
            self.folds.retain(|open| *open != region.open);
            self.reconcile_folds(cx);
            cx.notify();
        }
    }

    /// Fold or unfold the block that starts on `line`, from the gutter.
    fn toggle_fold_at(&mut self, line: usize, cx: &mut Context<Self>) {
        if let Some(region) = self.folded.iter().find(|region| region.start_line == line) {
            let open = region.open;
            self.folds.retain(|folded| *folded != open);
            self.reconcile_folds(cx);
            cx.notify();
        } else if let Some(region) =
            self.fold_region_at(line, cx, |region| region.start_line == line)
        {
            self.fold_region(region, cx);
        }
    }

    /// Provide hover information and go-to-definition for SQL.
    pub fn set_language_host(&mut self, host: std::rc::Rc<dyn SqlLanguageHost>) {
        self.language_host = Some(host);
    }

    /// The hover card for the code under the pointer: a diagnostic's message
    /// first, otherwise what the language host knows.
    fn hover_info_at(&self, offset: usize, cx: &App) -> Option<HoverInfo> {
        if let Some(diagnostic) = self
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.start <= offset && offset < diagnostic.range.end)
            .max_by_key(|diagnostic| diagnostic.severity)
        {
            return Some(HoverInfo {
                range: diagnostic.range.clone(),
                title: diagnostic.message.clone(),
                detail: None,
            });
        }
        let text = self.value.read(cx);
        self.language_host
            .as_ref()
            .and_then(|host| host.hover(text, offset, cx))
            .or_else(|| self.function_hover(text, offset))
    }

    /// The signature of the built-in function named at `offset`.
    fn function_hover(&self, text: &str, offset: usize) -> Option<HoverInfo> {
        if !self.is_code() {
            return None;
        }
        let mut cache = self.paint_cache.borrow_mut();
        cache.refresh(text, self.language);
        let tokens = cache.sql_tokens(offset..offset);
        let (range, signature) = signatures::function_at(text, offset, &tokens)?;
        Some(HoverInfo {
            range,
            title: signature.label().into(),
            detail: Some(signature.summary.into()),
        })
    }

    /// The alias or CTE definition for a use of it at `offset`.
    fn local_definition(&self, text: &str, offset: usize) -> Option<Range<usize>> {
        if !self.is_code() {
            return None;
        }
        let statement = sql_statement_range(text, offset, self.sql_dialect());
        let mut cache = self.paint_cache.borrow_mut();
        cache.refresh(text, self.language);
        let tokens = cache.sql_tokens(statement);
        signatures::local_definition(text, offset, &tokens)
    }

    /// The built-in function call around the caret, for signature help.
    fn signature_help(&self, cx: &App) -> Option<signatures::CallSite> {
        if !self.is_code() || !self.selected_range.is_empty() || self.marked_range.is_some() {
            return None;
        }
        let text = self.value.read(cx);
        let cursor = self.cursor_offset_internal();
        let mut cache = self.paint_cache.borrow_mut();
        cache.refresh(text, self.language);
        let tokens = cache.sql_tokens(cursor.saturating_sub(signatures::CALL_SCAN_LIMIT)..cursor);
        signatures::call_at(text, cursor, &tokens)
    }

    fn render_signature_help(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        if !self.focus_handle.is_focused(window) {
            return None;
        }
        let call = self.signature_help(cx)?;
        let bounds = self.last_bounds?;
        let text = self.value.read(cx);
        let (line, column) = line_and_column(text, call.name.start);
        let line_height = bounds.size.height / self.last_layout.count.max(1) as f32;
        let x = self
            .last_layout
            .get(line)
            .map_or(dbx_ui_components::theme::ui_px(0.), |shaped| {
                shaped.x_for_index(column)
            });
        let position = point(
            bounds.left() + x,
            bounds.top() + self.last_layout.top(line, line_height),
        );
        let signature = call.signature;
        let active = signature.active_param(call.argument);
        let mut parts = vec![(format!("{}(", signature.name), false)];
        for (index, param) in signature.params.iter().enumerate() {
            if index > 0 {
                parts.push((", ".into(), false));
            }
            parts.push(((*param).into(), active == Some(index)));
        }
        if signature.variadic {
            parts.push((", …".into(), false));
        }
        parts.push((format!(") → {}", signature.returns), false));
        let card = div()
            .id("dbx-editor-signature")
            .debug_selector(|| "dbx-editor-signature".into())
            .max_w(dbx_ui_components::theme::ui_px(480.))
            .px(dbx_ui_components::theme::ui_px(9.))
            .py(dbx_ui_components::theme::ui_px(6.))
            .rounded(dbx_ui_components::theme::ui_px(7.))
            .border_1()
            .border_color(theme().border_strong)
            .bg(theme().panel_raised)
            .text_size(dbx_ui_components::theme::text_px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .text_color(theme().text_muted)
                    .children(parts.into_iter().map(|(part, active)| {
                        div()
                            .when(active, |part| {
                                part.text_color(theme().text)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                            })
                            .child(part)
                    })),
            )
            .child(
                div()
                    .pt(dbx_ui_components::theme::ui_px(3.))
                    .text_size(dbx_ui_components::theme::text_px(11.))
                    .text_color(theme().text_muted)
                    .child(signature.summary),
            );
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .position(
                        position
                            - point(
                                dbx_ui_components::theme::ui_px(9.),
                                dbx_ui_components::theme::ui_px(4.),
                            ),
                    )
                    .snap_to_window_with_margin(dbx_ui_components::theme::ui_px(8.))
                    .child(card),
            )
            .with_priority(30)
            .into_any_element(),
        )
    }

    fn update_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let info = self
            .text_index_at(position, cx)
            .and_then(|offset| self.hover_info_at(offset, cx));
        let Some(info) = info else {
            self.clear_hover(cx);
            return;
        };
        if self.hover.as_ref().is_some_and(|hover| hover.info == info) {
            return;
        }
        let was_visible = self.hover.as_ref().is_some_and(|hover| hover.visible);
        self.hover = Some(EditorHover {
            info,
            position,
            visible: false,
        });
        if was_visible {
            cx.notify();
        }
        self._hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            let _ = this.update(cx, |editor, cx| {
                if let Some(hover) = editor.hover.as_mut() {
                    hover.visible = true;
                    cx.notify();
                }
            });
        }));
    }

    fn clear_hover(&mut self, cx: &mut Context<Self>) {
        self._hover_task = None;
        if self.hover.take().is_some_and(|hover| hover.visible) {
            cx.notify();
        }
    }

    /// The text offset under `position`, only when the pointer is over a
    /// line's text rather than the empty space after it.
    fn text_index_at(&self, position: Point<Pixels>, cx: &App) -> Option<usize> {
        let bounds = self.last_bounds?;
        if self.last_layout.count == 0 || !bounds.contains(&position) {
            return None;
        }
        let line_height = bounds.size.height / self.last_layout.count as f32;
        let line = self
            .last_layout
            .line_at(position.y - bounds.top(), line_height);
        let shaped = self.last_layout.get(line)?;
        if position.x - bounds.left() > shaped.width {
            return None;
        }
        let offset = self.index_for_mouse_position(position, cx);
        // The hit test rounds to the nearest boundary; hover the character
        // under the pointer instead.
        let text = self.value.read(cx);
        let x = shaped.x_for_index(offset - nth_line_start(text, line));
        Some(if x > position.x - bounds.left() {
            previous_boundary(text, offset)
        } else {
            offset
        })
    }

    fn render_hover(&self) -> Option<AnyElement> {
        let hover = self.hover.as_ref().filter(|hover| hover.visible)?;
        let card = div()
            .id("dbx-editor-hover")
            .debug_selector(|| "dbx-editor-hover".into())
            .max_w(dbx_ui_components::theme::ui_px(420.))
            .px(dbx_ui_components::theme::ui_px(9.))
            .py(dbx_ui_components::theme::ui_px(6.))
            .rounded(dbx_ui_components::theme::ui_px(7.))
            .border_1()
            .border_color(theme().border_strong)
            .bg(theme().panel_raised)
            .text_size(dbx_ui_components::theme::text_px(12.))
            .text_color(theme().text)
            .child(hover.info.title.clone())
            .children(hover.info.detail.clone().map(|detail| {
                div()
                    .pt(dbx_ui_components::theme::ui_px(3.))
                    .text_size(dbx_ui_components::theme::text_px(11.))
                    .text_color(theme().text_muted)
                    .child(detail)
            }));
        Some(
            deferred(
                anchored()
                    .position(
                        hover.position
                            + point(
                                dbx_ui_components::theme::ui_px(0.),
                                dbx_ui_components::theme::ui_px(18.),
                            ),
                    )
                    .snap_to_window_with_margin(dbx_ui_components::theme::ui_px(8.))
                    .child(card),
            )
            .with_priority(40)
            .into_any_element(),
        )
    }

    /// Select a UTF-8 byte range and scroll it into view.
    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let range = clamp_range(&self.text(cx), range);
        self.extra_selections.clear();
        self.selected_range = range;
        self.selection_reversed = false;
        self.reveal_requested.set(true);
        cx.notify();
    }

    /// Move the caret to a UTF-8 byte offset, collapsing any selection.
    pub fn move_cursor_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to(offset, cx);
    }

    fn cursor_offset_internal(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.extra_selections.clear();
        let offset = clamp_boundary(&self.text(cx), offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.extra_selections.clear();
        let offset = clamp_boundary(&self.text(cx), offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, inserted: &str, cx: &mut Context<Self>) {
        self.typing_run = None;
        self.extra_selections.clear();
        self.replace_recording(range, inserted, true, cx);
    }

    fn replace_recording(
        &mut self,
        range: Range<usize>,
        inserted: &str,
        record: bool,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let text = self.text(cx);
        let range = clamp_range(&text, range);
        let inserted = normalize_value(inserted, self.multiline);
        if text[range.clone()] == inserted {
            return;
        }
        if record {
            self.record_history(cx);
        }
        let next = replace_selection(&text, range.clone(), &inserted);
        let cursor = range.start + inserted.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.set_text_without_selection(next, cx);
        cx.notify();
    }

    fn set_text_without_selection(&mut self, text: String, cx: &mut Context<Self>) {
        self.value.update(cx, |value, cx| {
            *value = text;
            cx.notify();
        });
    }

    fn record_history(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.vim.as_ref().is_some_and(|vim| {
            vim.mode == VimMode::Insert && self.undo_history.len() > vim.insert_undo_start
        }) {
            self.redo_history.clear();
            return;
        }
        let snapshot = EditorSnapshot::from_editor(self, cx);
        if self.undo_history.last() != Some(&snapshot) {
            self.undo_history.push(snapshot);
            if self.undo_history.len() > HISTORY_LIMIT {
                self.undo_history.remove(0);
            }
        }
        self.redo_history.clear();
    }

    fn restore_snapshot(&mut self, snapshot: EditorSnapshot, cx: &mut Context<Self>) {
        self.extra_selections.clear();
        self.selected_range = clamp_range(&snapshot.text, snapshot.selection);
        self.selection_reversed = snapshot.selection_reversed;
        self.marked_range = None;
        self.set_text_without_selection(snapshot.text, cx);
        cx.notify();
    }

    /// Undo the most recent local text replacement, including IME commits.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.undo_history.pop() else {
            return;
        };
        self.redo_history
            .push(EditorSnapshot::from_editor(self, cx));
        self.restore_snapshot(snapshot, cx);
    }

    /// Redo a replacement reversed by [`Self::undo`].
    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.redo_history.pop() else {
            return;
        };
        self.undo_history
            .push(EditorSnapshot::from_editor(self, cx));
        self.restore_snapshot(snapshot, cx);
    }

    fn undo_action(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo(cx);
    }

    fn redo_action(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.redo(cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(previous_boundary(&self.text(cx), self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(next_boundary(&self.text(cx), self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(previous_boundary(&self.text(cx), self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_boundary(&self.text(cx), self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    /// Select the whole value, so typing replaces it.
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.extra_selections.clear();
        self.move_to(0, cx);
        self.select_to(self.text(cx).len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(line_start(&self.text(cx), self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(line_end(&self.text(cx), self.cursor_offset()), cx);
    }

    fn component_home(&mut self, _: &MoveHome, window: &mut Window, cx: &mut Context<Self>) {
        self.home(&Home, window, cx);
    }

    fn component_end(&mut self, _: &MoveEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.end(&End, window, cx);
    }

    fn component_select_home(
        &mut self,
        _: &SelectToStartOfLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.text(cx);
        self.select_to(line_start(&text, self.cursor_offset()), cx);
    }

    fn component_select_end(
        &mut self,
        _: &SelectToEndOfLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.text(cx);
        self.select_to(line_end(&text, self.cursor_offset()), cx);
    }

    fn component_previous_word(
        &mut self,
        _: &MoveToPreviousWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            let text = self.text(cx);
            self.move_to(previous_word_boundary(&text, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn component_next_word(&mut self, _: &MoveToNextWord, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let text = self.text(cx);
            self.move_to(next_word_boundary(&text, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn component_select_previous_word(
        &mut self,
        _: &SelectToPreviousWordStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.text(cx);
        self.select_to(previous_word_boundary(&text, self.cursor_offset()), cx);
    }

    fn component_select_next_word(
        &mut self,
        _: &SelectToNextWordEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.text(cx);
        self.select_to(next_word_boundary(&text, self.cursor_offset()), cx);
    }

    fn vertical(&mut self, direction: isize, cx: &mut Context<Self>) {
        if !self.multiline {
            return;
        }
        let target = self.vertical_target(direction, cx);
        self.move_to(target, cx);
    }

    /// The offset one line up or down at the same column; past the first or
    /// last line this is the start or end of the text.
    fn vertical_target(&self, direction: isize, cx: &App) -> usize {
        let text = self.value.read(cx);
        let cursor = self.cursor_offset();
        if !self.multiline {
            return cursor;
        }
        let (line, column) = line_and_column(text, cursor);
        // Step over folded lines: move by painted row.
        let rows = folding::RowMap::new(&self.folded);
        let target = rows.row(line) as isize + direction;
        let line_count = text.split('\n').count();
        if target < 0 {
            return 0;
        }
        let target = target as usize;
        if target >= rows.rows(line_count) {
            return text.len();
        }
        let target = rows.line(target);
        let start = nth_line_start(text, target);
        let target_line = &text[start..line_end(text, start)];
        start + clamp_boundary(target_line, column.min(target_line.len()))
    }

    /// Move the cursor between lines without requiring the caller to depend
    /// on the editor's private action types.
    pub fn move_vertical(&mut self, direction: isize, cx: &mut Context<Self>) {
        self.vertical(direction, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if !self.extra_selections.is_empty() {
            self.edit_all_selections(
                |text, selections| commands::delete_at_all(text, selections, true),
                cx,
            );
            return;
        }
        if self.is_code()
            && let Some(edit) = commands::delete_pair(&self.text(cx), &self.selected_range)
        {
            self.apply_edit(edit, cx);
            return;
        }
        if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            let previous = previous_boundary(&self.text(cx), cursor);
            if cursor == previous {
                return;
            }
            self.select_to(previous, cx);
        }
        self.replace(self.selected_range.clone(), "", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if !self.extra_selections.is_empty() {
            self.edit_all_selections(
                |text, selections| commands::delete_at_all(text, selections, false),
                cx,
            );
            return;
        }
        if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            let next = next_boundary(&self.text(cx), cursor);
            if cursor == next {
                return;
            }
            self.select_to(next, cx);
        }
        self.replace(self.selected_range.clone(), "", cx);
    }

    /// The multiline SQL editor gets code-editing behaviour; filters, cell
    /// editors and connection fields stay plain text fields.
    fn is_code(&self) -> bool {
        self.multiline && matches!(self.language, EditorLanguage::Sql(_))
    }

    /// Replace the whole text and selection as one undo step.
    fn apply_edit(&mut self, edit: commands::Edit, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        self.typing_run = None;
        self.extra_selections.clear();
        if edit.text != *self.value.read(cx) {
            self.record_history(cx);
            self.marked_range = None;
            self.set_text_without_selection(edit.text, cx);
        }
        self.selected_range = edit.selection;
        self.selection_reversed = false;
        self.reveal_requested.set(true);
        cx.notify();
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.indent_selection(cx);
    }

    pub fn indent_selection(&mut self, cx: &mut Context<Self>) {
        if !self.is_code() {
            cx.propagate();
            return;
        }
        let edit = commands::indent(&self.text(cx), self.selected_range.clone());
        self.apply_edit(edit, cx);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_code() {
            cx.propagate();
            return;
        }
        if let Some(edit) = commands::outdent(&self.text(cx), self.selected_range.clone()) {
            self.apply_edit(edit, cx);
        }
    }

    fn toggle_line_comment(
        &mut self,
        _: &ToggleLineComment,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_code() {
            let edit = commands::toggle_line_comment(&self.text(cx), self.selected_range.clone());
            self.apply_edit(edit, cx);
        }
    }

    fn duplicate_lines(&mut self, _: &DuplicateLines, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_code() {
            let edit = commands::duplicate_lines(&self.text(cx), self.selected_range.clone());
            self.apply_edit(edit, cx);
        }
    }

    fn move_lines_up(&mut self, _: &MoveLinesUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_lines(-1, cx);
    }

    fn move_lines_down(&mut self, _: &MoveLinesDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_lines(1, cx);
    }

    fn move_lines(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.is_code()
            && let Some(edit) =
                commands::move_lines(&self.text(cx), self.selected_range.clone(), direction)
        {
            self.apply_edit(edit, cx);
        }
    }

    fn delete_lines(&mut self, _: &DeleteLines, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_code() {
            let edit = commands::delete_lines(&self.text(cx), self.selected_range.clone());
            self.apply_edit(edit, cx);
        }
    }

    fn select_line(&mut self, _: &SelectLine, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let lines = commands::selected_lines(&text, &self.selected_range);
        // Repeating extends the selection by the next line.
        let end = if self.selected_range == (lines.start..(lines.end + 1).min(text.len())) {
            line_end(&text, (lines.end + 1).min(text.len()))
        } else {
            lines.end
        };
        let end = if end < text.len() { end + 1 } else { end };
        self.extra_selections.clear();
        self.selected_range = lines.start..end;
        self.selection_reversed = false;
        self.reveal_requested.set(true);
        cx.notify();
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(-1, cx);
        self.select_to(target, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(1, cx);
        self.select_to(target, cx);
    }

    fn move_to_start(&mut self, _: &MoveToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text(cx).len(), cx);
    }

    fn select_to_start(&mut self, _: &SelectToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text(cx).len(), cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            let text = self.text(cx);
            self.select_to(previous_word_boundary(&text, self.cursor_offset()), cx);
        }
        self.replace(self.selected_range.clone(), "", cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            let text = self.text(cx);
            self.select_to(line_start(&text, self.cursor_offset()), cx);
        }
        self.replace(self.selected_range.clone(), "", cx);
    }

    fn newline(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.is_code() {
            self.insert_newline(cx);
        } else if self.multiline {
            self.replace(self.selected_range.clone(), "\n", cx);
        } else {
            // A single-line field has no newline; let an enclosing form bind
            // Enter to its submit action.
            cx.propagate();
        }
    }

    /// Insert a newline for a shell-level Enter handler. The SQL editor keeps
    /// the line's indentation and opens bracketed blocks.
    pub fn insert_newline(&mut self, cx: &mut Context<Self>) {
        if !self.extra_selections.is_empty() {
            self.edit_all_selections(
                |text, selections| commands::insert_at_all(text, selections, "\n"),
                cx,
            );
        } else if self.is_code() {
            let edit = commands::newline(&self.text(cx), self.selected_range.clone());
            self.apply_edit(edit, cx);
        } else if self.multiline {
            self.replace(self.selected_range.clone(), "\n", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if self.extra_selections.is_empty() {
                self.replace(self.selected_range.clone(), &text, cx);
            } else {
                let text = normalize_value(&text, self.multiline);
                self.edit_all_selections(
                    |content, selections| commands::insert_at_all(content, selections, &text),
                    cx,
                );
            }
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text(cx)[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
        if !self.selected_range.is_empty() {
            self.replace(self.selected_range.clone(), "", cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>, cx: &App) -> usize {
        let text = self.value.read(cx);
        let bounds = self.last_bounds;
        let (Some(bounds), false) = (bounds, self.last_layout.count == 0) else {
            return 0;
        };
        if position.y <= bounds.top() {
            return 0;
        }
        if position.y >= bounds.bottom() {
            return text.len();
        }

        let line_height = bounds.size.height / self.last_layout.count.max(1) as f32;
        let line = self
            .last_layout
            .line_at(position.y - bounds.top(), line_height);
        let local_x = position.x - bounds.left();
        let offset = nth_line_start(text, line);
        // Only visible lines are laid out; the pointer is over one of them.
        let Some(shaped) = self.last_layout.get(line) else {
            return offset;
        };
        let display_index = shaped.closest_index_for_x(local_x);
        let source_index = if self.password {
            password_source_offset(&text[offset..line_end(text, offset)], display_index)
        } else {
            display_index
        };
        clamp_boundary(text, offset + source_index)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.clear_hover(cx);
        self.extra_selections.clear();
        if event.modifiers.secondary()
            && !event.modifiers.shift
            && let Some(offset) = self.text_index_at(event.position, cx)
        {
            let text = self.text(cx);
            if let Some(definition) = self.local_definition(&text, offset) {
                self.select_range(definition, cx);
                return;
            }
            if let Some(host) = self.language_host.clone()
                && host.go_to_definition(&text, offset, window, cx)
            {
                return;
            }
        }
        let index = self.index_for_mouse_position(event.position, cx);
        self.selection_pointer = Some(event.position);
        self.is_selecting = true;
        self.typing_run = None;
        if event.modifiers.shift {
            self.select_to(index, cx);
        } else if event.click_count == 2 {
            let text = self.text(cx);
            let word = commands::word_at(&text, index);
            self.selected_range = word;
            self.selection_reversed = false;
            self.is_selecting = false;
            cx.notify();
        } else if event.click_count >= 3 {
            let text = self.text(cx);
            let start = line_start(&text, index);
            let end = line_end(&text, index);
            self.selected_range = start..(end + 1).min(text.len());
            self.selection_reversed = false;
            self.is_selecting = false;
            cx.notify();
        } else {
            self.move_to(index, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
        self.selection_pointer = None;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Focusing on mouse-down can rerender this element before the matching
        // mouse-up listener clears `is_selecting`. Never let that stale local
        // flag turn an ordinary hover into a selection/caret move: the
        // platform event is the source of truth for whether a drag is active.
        self.is_selecting = mouse_selection_active(self.is_selecting, event.dragging());
        if self.is_selecting {
            self.selection_pointer = Some(event.position);
            self.select_at_pointer(cx);
            cx.notify();
        } else {
            self.selection_pointer = None;
            if self.is_code() {
                self.update_hover(event.position, cx);
            }
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Wheel events only reach us while hovered. A trackpad gesture must
        // also have *started* here: otherwise a sidebar scroll that slides this
        // value under a stationary pointer would be hijacked mid-gesture.
        // Discrete mouse wheels have no phases, so hovering is enough.
        match event.touch_phase {
            gpui::TouchPhase::Started => self.wheel_gesture = true,
            gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                let owned = self.wheel_gesture;
                self.wheel_gesture = false;
                if !owned {
                    return;
                }
            }
            gpui::TouchPhase::Moved => {}
        }
        let discrete = matches!(event.delta, gpui::ScrollDelta::Lines(_));
        if !discrete && !self.wheel_gesture {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height());
        let current = self.scroll_handle.offset();
        let maximum = self.scroll_handle.max_offset();
        let next = point(
            (current.x
                + if event.modifiers.shift && delta.x == px(0.) {
                    delta.y
                } else {
                    delta.x
                })
            .clamp(-maximum.x, px(0.)),
            if self.multiline && !event.modifiers.shift {
                (current.y + delta.y).clamp(-maximum.y, px(0.))
            } else {
                current.y
            },
        );
        if next != current {
            self.scroll_handle.set_offset(next);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn select_at_pointer(&mut self, cx: &mut Context<Self>) {
        let (Some(pointer), Some(viewport)) = (self.selection_pointer, self.last_viewport) else {
            return;
        };
        // Hit-test only painted lines. Crossing an edge scrolls incrementally
        // instead of jumping to an unshaped line or the end of the document.
        let position = point(
            pointer
                .x
                .clamp(viewport.left(), viewport.right().max(viewport.left())),
            pointer.y.clamp(
                viewport.top(),
                (viewport.bottom() - px(1.)).max(viewport.top()),
            ),
        );
        let index = self.index_for_mouse_position(position, cx);
        if index != self.cursor_offset_internal() {
            self.select_to(index, cx);
        }
    }

    fn scroll_selection(&mut self, cx: &mut Context<Self>) {
        if !self.is_selecting {
            return;
        }
        self.select_at_pointer(cx);
        let (Some(pointer), Some(viewport)) = (self.selection_pointer, self.last_viewport) else {
            return;
        };
        let current = self.scroll_handle.offset();
        let next = drag_scroll_offset(
            current,
            self.scroll_handle.max_offset(),
            viewport,
            pointer,
            self.multiline,
        );
        if next != current {
            self.scroll_handle.set_offset(next);
            cx.notify();
        }
    }
}

/// Bounded edge scrolling keeps selection predictable even far outside a field.
fn drag_scroll_offset(
    current: Point<Pixels>,
    maximum: Point<Pixels>,
    viewport: Bounds<Pixels>,
    pointer: Point<Pixels>,
    multiline: bool,
) -> Point<Pixels> {
    let step = |position: Pixels, start: Pixels, end: Pixels| {
        if position < start {
            (start - position).min(px(16.))
        } else if position > end {
            (end - position).max(px(-16.))
        } else {
            px(0.)
        }
    };
    point(
        (current.x + step(pointer.x, viewport.left(), viewport.right())).clamp(-maximum.x, px(0.)),
        if multiline {
            (current.y + step(pointer.y, viewport.top(), viewport.bottom()))
                .clamp(-maximum.y, px(0.))
        } else {
            current.y
        },
    )
}

fn completion_anchor(
    text: &str,
    cursor: usize,
    bounds: Option<Bounds<Pixels>>,
    layout: &LineLayouts,
) -> Point<Pixels> {
    let Some(bounds) = bounds else {
        return point(
            dbx_ui_components::theme::ui_px(0.),
            dbx_ui_components::theme::ui_px(0.),
        );
    };
    let cursor = clamp_boundary(text, cursor);
    let (line, column) = line_and_column(text, cursor);
    let line_height = bounds.size.height / layout.count.max(1) as f32;
    let x = layout
        .get(line)
        .map(|line| line.x_for_index(column))
        .unwrap_or(dbx_ui_components::theme::ui_px(0.));
    point(
        bounds.left() + x,
        bounds.top() + layout.top(line, line_height) + line_height,
    )
}

/// The lines laid out in the last frame: the visible ones and the caret's.
#[derive(Default)]
struct LineLayouts {
    /// Painted rows: the document's lines less folded ones, laid out or not.
    count: usize,
    /// Shaped lines by line index, in ascending order.
    lines: Vec<(usize, ShapedLine)>,
    /// Which lines folding hid when these were laid out.
    rows: folding::RowMap,
}

impl LineLayouts {
    /// The document line painted at the row under `y`, from the top.
    fn line_at(&self, y: Pixels, line_height: Pixels) -> usize {
        let row = ((y / line_height).max(0.) as usize).min(self.count.saturating_sub(1));
        self.rows.line(row)
    }

    /// The top of `line`'s row.
    fn top(&self, line: usize, line_height: Pixels) -> Pixels {
        line_height * self.rows.row(line) as f32
    }

    fn get(&self, line: usize) -> Option<&ShapedLine> {
        self.lines
            .binary_search_by_key(&line, |(index, _)| *index)
            .ok()
            .map(|index| &self.lines[index].1)
    }
}

/// Per-text analysis that painting reuses until the text changes. Comparing
/// the stored copy is a memcmp, far cheaper than lexing or re-shaping.
#[derive(Default)]
struct PaintCache {
    text: String,
    language: EditorLanguage,
    valid: bool,
    tokens: Vec<HighlightToken>,
    line_starts: Vec<usize>,
    /// The lines with the most characters. The widest painted line is among
    /// them, so only these are measured for the content width.
    longest_lines: Vec<usize>,
    /// The measured width and the font size it was measured at.
    width: Option<(Pixels, SharedString, Pixels)>,
    /// The caret offset and the uses of the identifier under it.
    occurrences: Option<(usize, Vec<Range<usize>>)>,
    /// Blocks that can fold, computed when first needed.
    fold_regions: Option<Vec<folding::FoldRegion>>,
}

impl PaintCache {
    const MEASURED_LINES: usize = 4;

    fn refresh(&mut self, text: &str, language: EditorLanguage) {
        if self.valid && self.language == language && self.text == text {
            return;
        }
        self.text.clear();
        self.text.push_str(text);
        self.language = language;
        self.valid = true;
        self.width = None;
        self.occurrences = None;
        self.fold_regions = None;
        self.tokens = match language {
            EditorLanguage::PlainText => Vec::new(),
            EditorLanguage::Sql(dialect) => lex_sql_for(text, dialect)
                .into_iter()
                .map(HighlightToken::from)
                .collect(),
            EditorLanguage::Redis => lex_redis(text)
                .into_iter()
                .map(HighlightToken::from)
                .collect(),
            EditorLanguage::Json => lex_json(text)
                .into_iter()
                .map(HighlightToken::from)
                .collect(),
        };
        self.line_starts.clear();
        self.line_starts.push(0);
        self.line_starts
            .extend(text.match_indices('\n').map(|(index, _)| index + 1));
        let mut lengths = (0..self.line_starts.len())
            .map(|line| (self.line(text, line).chars().count(), line))
            .collect::<Vec<_>>();
        let keep = Self::MEASURED_LINES.min(lengths.len());
        if keep > 0 && keep < lengths.len() {
            lengths.select_nth_unstable_by(keep - 1, |left, right| right.cmp(left));
        }
        self.longest_lines = lengths
            .into_iter()
            .take(keep)
            .map(|(_, line)| line)
            .collect();
    }

    fn line_range(&self, text: &str, line: usize) -> Range<usize> {
        let start = self.line_starts[line];
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(text.len(), |next| next - 1);
        start..end
    }

    fn line<'a>(&self, text: &'a str, line: usize) -> &'a str {
        &text[self.line_range(text, line)]
    }

    /// (line, column) of a byte offset, by binary search.
    fn line_and_column(&self, offset: usize) -> (usize, usize) {
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        (line, offset - self.line_starts[line])
    }

    /// Uses of the identifier at `cursor` in its statement, reused while
    /// neither the text nor the caret changes.
    fn occurrences(&mut self, text: &str, cursor: usize, dialect: SqlDialect) -> Vec<Range<usize>> {
        if let Some((cached, ranges)) = &self.occurrences
            && *cached == cursor
        {
            return ranges.clone();
        }
        let touching = self
            .sql_tokens(cursor.saturating_sub(1)..cursor)
            .iter()
            .any(|token| {
                token.kind == SqlTokenKind::Identifier
                    && token.range.start <= cursor
                    && cursor <= token.range.end
            });
        let ranges = if touching {
            let statement = sql_statement_range(text, cursor, dialect);
            let tokens = self.sql_tokens(statement.clone());
            signatures::occurrences(text, cursor, &tokens, statement)
        } else {
            Vec::new()
        };
        self.occurrences = Some((cursor, ranges.clone()));
        ranges
    }

    /// Blocks that can fold, by opening offset.
    fn fold_regions(&mut self, text: &str) -> &[folding::FoldRegion] {
        if self.fold_regions.is_none() {
            let tokens = self.sql_tokens(0..text.len());
            self.fold_regions = Some(folding::fold_regions(text, &tokens, &self.line_starts));
        }
        self.fold_regions.as_deref().unwrap_or_default()
    }

    /// SQL tokens overlapping `range`, in document order.
    fn sql_tokens(&self, range: Range<usize>) -> Vec<SqlToken> {
        self.tokens_from(range.start)
            .iter()
            .take_while(|token| token.range.start <= range.end)
            .filter_map(|token| match token.kind {
                HighlightTokenKind::Sql(kind) => Some(SqlToken {
                    kind,
                    range: token.range.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Tokens from the first one that can touch `line_start`.
    fn tokens_from(&self, line_start: usize) -> &[HighlightToken] {
        let first = self
            .tokens
            .partition_point(|token| token.range.end <= line_start);
        &self.tokens[first..]
    }
}

fn mouse_selection_active(is_selecting: bool, dragging: bool) -> bool {
    is_selecting && dragging
}

fn caret_reveal_allowed(focused: bool, is_selecting: bool) -> bool {
    // During a drag, changing the scroll offset moves the content origin used
    // by hit testing underneath the pointer and feeds back into selection.
    focused && !is_selecting
}

impl Focusable for TextEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextEditor {
    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.accepts_insert_input()
    }
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.text(cx);
        let range = clamp_range(
            &text,
            utf16_to_utf8(&text, range.start)..utf16_to_utf8(&text, range.end),
        );
        *actual = Some(utf8_to_utf16(&text, range.start)..utf8_to_utf16(&text, range.end));
        Some(text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let text = self.text(cx);
        Some(UTF16Selection {
            range: utf8_to_utf16(&text, self.selected_range.start)
                ..utf8_to_utf16(&text, self.selected_range.end),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| {
            let text = self.value.read(cx);
            utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end)
        })
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_insert_input() {
            return;
        }
        let content = self.text(cx);
        let range = range
            .map(|range| {
                clamp_range(
                    &content,
                    utf16_to_utf8(&content, range.start)..utf16_to_utf8(&content, range.end),
                )
            })
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let typing = self.marked_range.is_none() && range == self.selected_range;
        if typing && !self.extra_selections.is_empty() {
            self.edit_all_selections(
                |content, selections| commands::insert_at_all(content, selections, text),
                cx,
            );
            return;
        }
        if !typing {
            self.extra_selections.clear();
        }
        if typing
            && self.is_code()
            && let Some(edit) = commands::type_character(&content, range.clone(), text)
        {
            self.apply_edit(edit, cx);
            return;
        }
        // Consecutive word characters typed at the caret share an undo step.
        let word = text.chars().count() == 1
            && text
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_');
        let continues = typing && word && range.is_empty() && self.typing_run == Some(range.start);
        self.typing_run = None;
        self.replace_recording(range.clone(), text, !continues, cx);
        if typing && word {
            self.typing_run = Some(range.start + text.len());
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_insert_input() {
            return;
        }
        self.extra_selections.clear();
        let content = self.text(cx);
        let range = range
            .map(|range| {
                clamp_range(
                    &content,
                    utf16_to_utf8(&content, range.start)..utf16_to_utf8(&content, range.end),
                )
            })
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let inserted = normalize_value(text, self.multiline);
        let start = range.start;
        if content[range.clone()] != inserted {
            self.record_history(cx);
        }
        let next = replace_selection(&content, range.clone(), &inserted);
        self.set_text_without_selection(next, cx);
        self.marked_range = (!inserted.is_empty()).then_some(start..start + inserted.len());

        if let Some(selected) = selected {
            let relative =
                utf16_to_utf8(&inserted, selected.start)..utf16_to_utf8(&inserted, selected.end);
            self.selected_range = start + relative.start..start + relative.end;
        } else {
            self.selected_range = start + inserted.len()..start + inserted.len();
        }
        self.selection_reversed = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        viewport_bounds: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let text = self.text(cx);
        let range = clamp_range(
            &text,
            utf16_to_utf8(&text, range.start)..utf16_to_utf8(&text, range.end),
        );
        let (start_line, start_col) = line_and_column(&text, range.start);
        let (end_line, end_col) = line_and_column(&text, range.end);
        let bounds = self.last_bounds.unwrap_or(viewport_bounds);
        let line_height = bounds.size.height / self.last_layout.count.max(1) as f32;
        let start = self.last_layout.get(start_line)?;
        let end = self.last_layout.get(end_line)?;
        let start_col = if self.password {
            password_display_offset(&text[nth_line_start(&text, start_line)..], start_col)
        } else {
            start_col
        };
        let end_col = if self.password {
            password_display_offset(&text[nth_line_start(&text, end_line)..], end_col)
        } else {
            end_col
        };
        Some(Bounds::from_corners(
            point(
                bounds.left() + start.x_for_index(start_col),
                bounds.top() + self.last_layout.top(start_line, line_height),
            ),
            point(
                bounds.left() + end.x_for_index(end_col),
                bounds.top() + self.last_layout.top(end_line, line_height) + line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let text = self.text(cx);
        Some(utf8_to_utf16(
            &text,
            self.index_for_mouse_position(point, cx),
        ))
    }
}

impl Render for TextEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Row fields follow the row preference, including JSON. Other code
        // fields use the editor font for both text and the line-number gutter,
        // including single-line SQL expressions without a gutter.
        let code_font = if self.row_value && !self.password {
            Some(dbx_ui_components::theme::row_font_family())
        } else {
            (!self.password && self.language != EditorLanguage::PlainText)
                .then(|| cx.theme().mono_font_family.clone())
        };
        if self.is_code() {
            self.reconcile_folds(cx);
        }
        // Refresh only while a screen reader is connected. Never copy a
        // password into a snapshot, even while the bridge is inactive.
        *self.accessible_text.borrow_mut() = if window.is_a11y_active() && !self.password {
            accessibility::AccessibleText::new(self.text(cx))
        } else {
            Default::default()
        };
        let editor = div()
            .id(gpui::SharedString::from(format!(
                "dbx-text-editor-scroll-{:?}",
                cx.entity().entity_id()
            )))
            .size_full()
            .when_some(code_font.clone(), |view, family| view.font_family(family))
            // Keep the scrollable editor from contributing its intrinsic text
            // width to a flex ancestor. The input shell owns the available
            // width; this child may be wider only inside its scroll viewport.
            .min_w_0()
            .track_scroll(&self.scroll_handle)
            .child(TextEditorText {
                editor: cx.entity(),
            });

        if self.is_code() {
            div()
                .flex()
                .size_full()
                .when_some(code_font, |view, family| view.font_family(family))
                .min_w_0()
                .when_some(self.vim_mode(), |view, mode| {
                    view.track_focus(&self.focus_handle)
                        .key_context(if mode == VimMode::Insert {
                            vim::INSERT_CONTEXT
                        } else {
                            vim::NORMAL_CONTEXT
                        })
                        .on_action(cx.listener(Self::vim_key))
                        .on_key_down(cx.listener(Self::vim_unbound_key))
                })
                .child(LineNumberGutter {
                    editor: cx.entity(),
                })
                .child(editor.overflow_scroll())
                .children(self.render_signature_help(window, cx))
                .children(self.render_hover())
                .into_any_element()
        } else if self.row_value {
            editor
                .overflow_hidden()
                .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
                .into_any_element()
        } else if self.multiline {
            editor.overflow_scroll().into_any_element()
        } else {
            // The compact field's content area can be fractionally shorter
            // than one painted line. Never let caret reveal chase that
            // unavoidable vertical clipping; single-line editors only need
            // horizontal scrolling.
            editor.overflow_x_scroll().into_any_element()
        }
    }
}

/// Line numbers beside the SQL editor. The gutter sits outside the scroll
/// area so it never scrolls horizontally; it follows the vertical offset.
struct LineNumberGutter {
    editor: Entity<TextEditor>,
}

impl IntoElement for LineNumberGutter {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for LineNumberGutter {
    type RequestLayoutState = ();
    type PrepaintState = gpui::Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let editor = self.editor.read(cx);
        let line_count = editor
            .value
            .read(cx)
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let digits = line_count.to_string().len().max(2);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let sample = "0".repeat(digits);
        let width = window
            .text_system()
            .shape_line(
                sample.clone().into(),
                font_size,
                &[TextRun {
                    len: sample.len(),
                    font: style.font(),
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width;
        let mut layout = Style::default();
        layout.size.width = (width + px(GUTTER_PADDING * 2. + FOLD_COLUMN)).into();
        layout.size.height = gpui::relative(1.).into();
        layout.flex_shrink = 0.;
        (window.request_layout(layout, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> gpui::Hitbox {
        window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut gpui::Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let editor = self.editor.read(cx);
        let text = editor.value.read(cx);
        let line_count = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let (cursor_line, _) = line_and_column(text, editor.cursor_offset());
        let focused = editor.focus_handle.is_focused(window);
        let offset = editor.scroll_handle.offset().y;
        let line_height = window.line_height();

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let rows = folding::RowMap::new(&editor.folded);
        let row_count = rows.rows(line_count);
        let first = ((-offset) / line_height).floor().max(0.) as usize;
        let visible = (bounds.size.height / line_height).ceil() as usize + 1;
        let hovered = hitbox.is_hovered(window);
        // Fold starts: folded ones always show; the rest while hovered.
        let folded_starts = editor
            .folded
            .iter()
            .map(|region| region.start_line)
            .collect::<Vec<_>>();
        let fold_starts = if hovered {
            let mut cache = editor.paint_cache.borrow_mut();
            cache.refresh(text, editor.language);
            cache
                .fold_regions(text)
                .iter()
                .map(|region| region.start_line)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let shape = |window: &mut Window, label: &str, color: gpui::Hsla| {
            window.text_system().shape_line(
                label.to_owned().into(),
                font_size,
                &[TextRun {
                    len: label.len(),
                    font: style.font(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
        };
        let numbers_right = bounds.right() - px(GUTTER_PADDING + FOLD_COLUMN);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for row in first..(first + visible).min(row_count) {
                let line = rows.line(row);
                let number = (line + 1).to_string();
                let color = if focused && line == cursor_line {
                    theme().text
                } else {
                    theme().text_muted
                };
                let shaped = shape(window, &number, color.into());
                let y = bounds.top() + offset + line_height * row as f32;
                let _ = shaped.paint(
                    point(numbers_right - shaped.width, y),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                let chevron = if folded_starts.contains(&line) {
                    Some("▸")
                } else if fold_starts.contains(&line) {
                    Some("▾")
                } else {
                    None
                };
                if let Some(chevron) = chevron {
                    let shaped = shape(window, chevron, theme().text_muted.into());
                    let x = numbers_right + px(FOLD_COLUMN / 2. + 2.) - shaped.width / 2.;
                    let _ =
                        shaped.paint(point(x, y), line_height, TextAlign::Left, None, window, cx);
                }
            }
        });

        let fold_area =
            Bounds::from_corners(point(numbers_right, bounds.top()), bounds.bottom_right());
        if hovered {
            window.set_cursor_style(gpui::CursorStyle::Arrow, hitbox);
        }
        let editor = self.editor.clone();
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, _| {
            if phase == gpui::DispatchPhase::Bubble && hitbox.is_hovered(window) != hovered {
                window.refresh();
            }
        });
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !fold_area.contains(&event.position)
            {
                return;
            }
            let row = ((event.position.y - bounds.top() - offset) / line_height).max(0.) as usize;
            if row >= row_count {
                return;
            }
            let line = rows.line(row);
            editor.update(cx, |editor, cx| editor.toggle_fold_at(line, cx));
            window.refresh();
            cx.stop_propagation();
        });
    }
}

const GUTTER_PADDING: f32 = 8.;
/// Room for fold chevrons between the line numbers and the text.
const FOLD_COLUMN: f32 = 12.;

struct TextEditorText {
    editor: Entity<TextEditor>,
}

struct PrepaintState {
    lines: LineLayouts,
    /// Current-line and matching-bracket highlights, painted under the text.
    decorations: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    /// Carets of the selections beyond the primary one.
    extra_cursors: Vec<PaintQuad>,
    /// `⋯` after each folded block's opening line.
    fold_markers: Vec<(Point<Pixels>, ShapedLine)>,
    selections: Vec<PaintQuad>,
    /// The child bounds after applying any caret-reveal delta calculated for
    /// this frame. Painting with these bounds keeps the text and scroll
    /// offset in sync instead of waiting for the next frame.
    paint_bounds: Bounds<Pixels>,
    scroll_offset: Option<Point<Pixels>>,
    viewport: Bounds<Pixels>,
}

impl IntoElement for TextEditorText {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextEditorText {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let editor = self.editor.read(cx);
        let text = editor.value.read(cx);
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let shape = |window: &mut Window, line: &str| {
            window
                .text_system()
                .shape_line(
                    line.to_owned().into(),
                    font_size,
                    &[TextRun {
                        len: line.len(),
                        font: text_style.font(),
                        color: text_style.color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
                .width
        };
        let (measured_width, line_count) = if editor.password {
            // Password fields are single short lines; mask and measure them.
            let painted = password_mask(text);
            let width = painted
                .split('\n')
                .map(|line| shape(window, line))
                .max()
                .unwrap_or(dbx_ui_components::theme::ui_px(1.));
            (width, painted.split('\n').count())
        } else {
            let mut cache = editor.paint_cache.borrow_mut();
            cache.refresh(text, editor.language);
            let width = match &cache.width {
                Some((size, family, width))
                    if *size == font_size && *family == text_style.font_family =>
                {
                    *width
                }
                _ => {
                    let width = cache
                        .longest_lines
                        .clone()
                        .into_iter()
                        .map(|line| shape(window, cache.line(text, line)))
                        .max()
                        .unwrap_or(dbx_ui_components::theme::ui_px(1.));
                    cache.width = Some((font_size, text_style.font_family.clone(), width));
                    width
                }
            };
            let rows = folding::RowMap::new(&editor.folded);
            (width, rows.rows(cache.line_starts.len()))
        };

        let mut style = Style::default();
        style.size.width = measured_width
            .max(dbx_ui_components::theme::ui_px(1.))
            .into();
        style.size.height = (window.line_height() * line_count.max(1) as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let editor = self.editor.read(cx);
        let text = editor.value.read(cx);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut cache = editor.paint_cache.borrow_mut();
        cache.refresh(text, editor.language);
        // Password fields paint a mask whose byte offsets differ per line.
        let masked = editor.password.then(|| password_mask(text));
        let masked_lines = masked
            .as_deref()
            .map(|masked| masked.split('\n').collect::<Vec<_>>());
        let line_count = cache.line_starts.len();
        let rows = folding::RowMap::new(&editor.folded);
        let row_count = rows.rows(line_count);

        let shape = |window: &mut Window, line: usize| {
            let line_start = cache.line_starts[line];
            let source = cache.line(text, line);
            let painted = masked_lines
                .as_ref()
                .and_then(|lines| lines.get(line).copied())
                .unwrap_or(source);
            let marked_range = editor.marked_range.as_ref().and_then(|range| {
                let range = marked_slice(source.len(), line_start, range)?;
                let range = if editor.password {
                    password_display_offset(source, range.start)
                        ..password_display_offset(source, range.end)
                } else {
                    range
                };
                Some(line_start + range.start..line_start + range.end)
            });
            let runs = editor_text_runs(
                painted,
                line_start,
                &style,
                editor.language,
                cache.tokens_from(line_start),
                marked_range.as_ref(),
                &editor.diagnostics,
            );
            window
                .text_system()
                .shape_line(painted.to_owned().into(), font_size, &runs, None)
        };
        let display_column = |line: usize, column: usize| {
            if editor.password {
                password_display_offset(cache.line(text, line), column)
            } else {
                column
            }
        };

        let cursor = clamp_boundary(
            text,
            editor
                .vim
                .as_ref()
                .filter(|vim| matches!(vim.mode, VimMode::Visual | VimMode::VisualLine))
                .map_or_else(|| editor.cursor_offset(), |vim| vim.cursor),
        );
        let (cursor_line, cursor_col) = cache.line_and_column(cursor);
        let cursor_shaped = shape(window, cursor_line);
        let block_caret = editor
            .vim_mode()
            .is_some_and(|mode| mode != VimMode::Insert);
        let caret_width = if block_caret {
            let line = cache.line(text, cursor_line);
            let measured = cursor_shaped.x_for_index(next_boundary(line, cursor_col))
                - cursor_shaped.x_for_index(cursor_col);
            if measured > dbx_ui_components::theme::ui_px(0.) {
                measured
            } else {
                dbx_ui_components::theme::ui_px(8.)
            }
        } else {
            dbx_ui_components::theme::ui_px(2.)
        };
        let cursor_position = point(
            cursor_shaped.x_for_index(display_column(cursor_line, cursor_col)),
            line_height * rows.row(cursor_line) as f32,
        );
        let focused = editor.focus_handle.is_focused(window);
        let reveal = editor.reveal_requested.replace(false);
        let cursor_changed = editor.last_revealed_cursor.replace(Some(cursor)) != Some(cursor);
        let (paint_bounds, scroll_offset) =
            if caret_reveal_allowed((focused && cursor_changed) || reveal, editor.is_selecting) {
                let current_offset = editor.scroll_handle.offset();
                let next_offset = scroll_offset_for_cursor(
                    current_offset,
                    editor.scroll_handle.max_offset().into(),
                    editor.scroll_handle.bounds().size,
                    cursor_position,
                    size(caret_width, line_height),
                    editor.multiline,
                );
                (bounds + (next_offset - current_offset), Some(next_offset))
            } else {
                (bounds, None)
            };

        // Lay out only the lines inside the scroll viewport, plus the caret's
        // line for hit testing and input-method bounds.
        let viewport = window.content_mask().bounds;
        let first_row = ((viewport.top() - paint_bounds.top()) / line_height)
            .floor()
            .max(0.) as usize;
        let last_row = (((viewport.bottom() - paint_bounds.top()) / line_height)
            .ceil()
            .max(0.) as usize)
            .min(row_count);
        let mut cursor_shaped = Some(cursor_shaped);
        let first = rows.line(first_row);
        let mut lines = Vec::with_capacity(last_row.saturating_sub(first_row) + 1);
        if cursor_line < first {
            lines.push((cursor_line, cursor_shaped.take().unwrap()));
        }
        for line in (first_row..last_row.max(first_row)).map(|row| rows.line(row)) {
            let shaped = if line == cursor_line {
                cursor_shaped.take().unwrap()
            } else {
                shape(window, line)
            };
            lines.push((line, shaped));
        }
        if let Some(shaped) = cursor_shaped {
            lines.push((cursor_line, shaped));
        }

        let extra_cursors = if focused {
            editor
                .extra_selections
                .iter()
                .filter_map(|range| {
                    let (line, column) = cache.line_and_column(clamp_boundary(text, range.end));
                    let (_, shaped) = lines.iter().find(|(shaped_line, _)| *shaped_line == line)?;
                    Some(fill(
                        Bounds::new(
                            point(
                                paint_bounds.left()
                                    + shaped.x_for_index(display_column(line, column)),
                                paint_bounds.top() + line_height * rows.row(line) as f32,
                            ),
                            size(dbx_ui_components::theme::ui_px(2.), line_height),
                        ),
                        dbx_ui_components::theme::theme().focus_ring,
                    ))
                })
                .collect()
        } else {
            Vec::new()
        };
        // A read-only view keeps selection but has no insertion point.
        let cursor_quad = (focused && !editor.read_only).then(|| {
            fill(
                Bounds::new(
                    point(
                        paint_bounds.left() + cursor_position.x,
                        paint_bounds.top() + cursor_position.y,
                    ),
                    size(dbx_ui_components::theme::ui_px(2.), line_height),
                ),
                // Pure blue vanishes on the dark canvas; the focus ring
                // colour is tuned for contrast in both appearances.
                {
                    let mut color = dbx_ui_components::theme::theme().focus_ring;
                    if block_caret {
                        color.a = 0.4;
                    }
                    color
                },
            )
        });

        let mut selections = Vec::new();
        let painted_selections = std::iter::once(&editor.selected_range)
            .chain(&editor.extra_selections)
            .filter(|range| !range.is_empty());
        for selected in painted_selections {
            let (start_line, start_col) =
                cache.line_and_column(clamp_boundary(text, selected.start));
            let (end_line, end_col) = cache.line_and_column(clamp_boundary(text, selected.end));
            for (line, shaped_line) in lines
                .iter()
                .filter(|(line, _)| (start_line..=end_line).contains(line))
            {
                let line = *line;
                let start = if line == start_line {
                    shaped_line.x_for_index(display_column(line, start_col))
                } else {
                    dbx_ui_components::theme::ui_px(0.)
                };
                let end = if line == end_line {
                    shaped_line.x_for_index(display_column(line, end_col))
                } else {
                    shaped_line.x_for_index(shaped_line.text.len())
                };
                selections.push(fill(
                    Bounds::from_corners(
                        point(
                            paint_bounds.left() + start,
                            paint_bounds.top() + line_height * rows.row(line) as f32,
                        ),
                        point(
                            paint_bounds.left() + end,
                            paint_bounds.top() + line_height * (rows.row(line) + 1) as f32,
                        ),
                    ),
                    theme().selection,
                ));
            }
        }

        let mut decorations = Vec::new();
        if editor.is_code() && focused {
            if editor.selected_range.is_empty() {
                decorations.push(fill(
                    Bounds::from_corners(
                        point(
                            viewport.left(),
                            paint_bounds.top() + line_height * rows.row(cursor_line) as f32,
                        ),
                        point(
                            viewport.right().max(paint_bounds.right()),
                            paint_bounds.top() + line_height * (rows.row(cursor_line) + 1) as f32,
                        ),
                    ),
                    theme().grid_alternate,
                ));
            }
            if editor.selected_range.is_empty() {
                for range in cache.occurrences(text, cursor, editor.sql_dialect()) {
                    let (line, start) = cache.line_and_column(range.start);
                    let (end_line, end) = cache.line_and_column(range.end);
                    let Some((_, shaped)) = lines
                        .iter()
                        .find(|(shaped_line, _)| *shaped_line == line && line == end_line)
                    else {
                        continue;
                    };
                    decorations.push(fill(
                        Bounds::from_corners(
                            point(
                                paint_bounds.left() + shaped.x_for_index(start),
                                paint_bounds.top() + line_height * rows.row(line) as f32,
                            ),
                            point(
                                paint_bounds.left() + shaped.x_for_index(end),
                                paint_bounds.top() + line_height * (rows.row(line) + 1) as f32,
                            ),
                        ),
                        theme().selection.alpha(0.45),
                    ));
                }
            }
            let near_bracket = [Some(cursor), cursor.checked_sub(1)]
                .into_iter()
                .flatten()
                .any(|offset| {
                    text.as_bytes()
                        .get(offset)
                        .is_some_and(|byte| b"()[]{}".contains(byte))
                });
            if near_bracket {
                let tokens = cache
                    .tokens
                    .iter()
                    .filter_map(|token| match token.kind {
                        HighlightTokenKind::Sql(kind) => Some(SqlToken {
                            kind,
                            range: token.range.clone(),
                        }),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if let Some((first, second)) = commands::matching_bracket(text, cursor, &tokens) {
                    for offset in [first, second] {
                        let (line, column) = cache.line_and_column(offset);
                        if let Some((_, shaped)) =
                            lines.iter().find(|(shaped_line, _)| *shaped_line == line)
                        {
                            let left = shaped.x_for_index(column);
                            let right = shaped.x_for_index(column + 1);
                            decorations.push(fill(
                                Bounds::from_corners(
                                    point(
                                        paint_bounds.left() + left,
                                        paint_bounds.top() + line_height * rows.row(line) as f32,
                                    ),
                                    point(
                                        paint_bounds.left() + right,
                                        paint_bounds.top()
                                            + line_height * (rows.row(line) + 1) as f32,
                                    ),
                                ),
                                theme().accent_soft,
                            ));
                        }
                    }
                }
            }
        }

        let mut fold_markers = Vec::new();
        for region in &editor.folded {
            let Some((_, shaped)) = lines.iter().find(|(line, _)| *line == region.start_line)
            else {
                continue;
            };
            let marker = window.text_system().shape_line(
                "⋯".into(),
                font_size,
                &[TextRun {
                    len: "⋯".len(),
                    font: style.font(),
                    color: dbx_ui_components::theme::theme().text_muted.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let top = paint_bounds.top() + line_height * rows.row(region.start_line) as f32;
            let left = paint_bounds.left() + shaped.width + dbx_ui_components::theme::ui_px(6.);
            decorations.push(
                fill(
                    Bounds::new(
                        point(left, top + dbx_ui_components::theme::ui_px(2.)),
                        size(
                            marker.width + dbx_ui_components::theme::ui_px(8.),
                            line_height - dbx_ui_components::theme::ui_px(4.),
                        ),
                    ),
                    dbx_ui_components::theme::theme().grid_alternate,
                )
                .corner_radii(dbx_ui_components::theme::ui_px(3.)),
            );
            fold_markers.push((
                point(left + dbx_ui_components::theme::ui_px(4.), top),
                marker,
            ));
        }
        let lines = LineLayouts {
            count: row_count,
            lines,
            rows,
        };
        if window.is_a11y_active() && !editor.password {
            editor.accessible_text.borrow_mut().update_geometry(
                &cache.line_starts,
                &lines,
                paint_bounds,
                line_height,
                window.scale_factor(),
            );
        }
        PrepaintState {
            lines,
            decorations,
            cursor: cursor_quad,
            extra_cursors,
            fold_markers,
            selections,
            paint_bounds,
            scroll_offset,
            viewport: viewport.intersect(&editor.scroll_handle.bounds()),
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus_handle.clone();

        window.handle_input(
            &focus,
            // The input handler's fallback bounds must share the text
            // origin used for painting and hit testing, including a reveal
            // adjustment applied in this frame.
            ElementInputHandler::new(state.paint_bounds, self.editor.clone()),
            cx,
        );

        for decoration in state.decorations.drain(..) {
            window.paint_quad(decoration);
        }
        for selection in state.selections.drain(..) {
            window.paint_quad(selection);
        }
        for (line, shaped_line) in &state.lines.lines {
            shaped_line
                .paint(
                    point(
                        state.paint_bounds.left(),
                        state.paint_bounds.top() + state.lines.top(*line, window.line_height()),
                    ),
                    window.line_height(),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .unwrap();
        }
        if let Some(cursor) = state.cursor.take() {
            window.paint_quad(cursor);
        }
        for cursor in state.extra_cursors.drain(..) {
            window.paint_quad(cursor);
        }
        for (origin, marker) in state.fold_markers.drain(..) {
            let _ = marker.paint(
                origin,
                window.line_height(),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        // A text selection owns pointer movement until release, including
        // movement outside the input shell and its clipped ancestors.
        // Register even while idle: mouse-down and the first drag movement
        // can arrive before another frame is painted.
        let editor = self.editor.downgrade();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == gpui::DispatchPhase::Bubble
                && let Some(editor) = editor.upgrade()
                && editor.read(cx).is_selecting
            {
                editor.update(cx, |editor, cx| editor.on_mouse_move(event, window, cx));
            }
        });
        let editor = self.editor.downgrade();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == gpui::DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && let Some(editor) = editor.upgrade()
                && editor.read(cx).is_selecting
            {
                editor.update(cx, |editor, cx| editor.on_mouse_up(event, window, cx));
            }
        });
        self.editor.update(cx, |editor, _| {
            if let Some(scroll_offset) = state.scroll_offset
                && editor.scroll_handle.offset() != scroll_offset
            {
                editor.scroll_handle.set_offset(scroll_offset);
                // The line-number gutter painted earlier this frame with the
                // old offset.
                if editor.is_code() {
                    // A refresh during painting is cleared at the end of the
                    // current draw. Schedule the follow-up after this frame so
                    // the gutter repaints with the committed offset.
                    window.on_next_frame(|window, _| window.refresh());
                }
            }
            // The prepaint state is no longer needed after painting. Move the
            // shaped lines into the editor so the hit-test/completion cache
            // does not allocate and clone the layout every frame.
            editor.last_layout = std::mem::take(&mut state.lines);
            editor.last_bounds = Some(state.paint_bounds);
            editor.last_viewport = Some(state.viewport);
            if editor.is_selecting
                && editor
                    .selection_pointer
                    .is_some_and(|pointer| !state.viewport.contains(&pointer))
            {
                let editor = self.editor.downgrade();
                window.on_next_frame(move |_, cx| {
                    let _ = editor.update(cx, |editor, cx| editor.scroll_selection(cx));
                });
            }
        });
    }
}

/// Render an editor with the standard DBX field treatment.
///
/// The `multiline` argument is kept explicit so callers can build an editor
/// before deciding where it is placed, and to make the intended row/editor
/// mode obvious at call sites.  It should match the mode passed to
/// [`TextEditor::new`].
pub fn input(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
    cx: &App,
) -> impl IntoElement {
    input_with_context(editor, focus, multiline, TEXT_EDITOR_CONTEXT, false, cx)
}

/// Render a multiline field so its editable viewport fills a containing pane.
pub fn input_fill(editor: Entity<TextEditor>, focus: FocusHandle, cx: &App) -> impl IntoElement {
    input_with_context(editor, focus, true, TEXT_EDITOR_CONTEXT, true, cx)
}

/// A standard field whose viewport is limited by a containing form. Keep the
/// editor and accessible node unchanged while allowing multiline controls to
/// fit a short scrolling pane. Single-line fields must still fit a full line
/// at the current font size, including padding and borders.
pub fn input_with_height(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
    height: Pixels,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let height = if multiline {
        height
    } else {
        height.max(single_line_input_height(&editor, cx))
    };
    input_with_context(editor, focus, multiline, TEXT_EDITOR_CONTEXT, false, cx).h(height)
}

/// Render a standard DBX field with an additional shell-owned key context.
///
/// The caller supplies the complete combined context so a focused editor can
/// participate in a containing workflow's shortcuts at the same dispatch
/// depth as the editor's own bindings.
pub fn input_with_key_context(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
    key_context: &'static str,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    input_with_context(editor, focus, multiline, key_context, false, cx)
}

/// Render a single-line editor without its own chrome, sized to `height`.
///
/// The caller supplies the surrounding field (border, background, icon) so the
/// input can sit flush with sibling controls of the same height.
pub fn bare_input(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    height: Pixels,
    cx: &App,
) -> impl IntoElement {
    input_with_context(editor, focus, false, TEXT_EDITOR_CONTEXT, false, cx)
        .h(height)
        .px(dbx_ui_components::theme::ui_px(0.))
        .py(dbx_ui_components::theme::ui_px(0.))
        .pt(dbx_ui_components::theme::ui_px(3.))
        .bg(gpui::transparent_black())
        .border_0()
}

/// Render the SQL editor with both its text-editing and completion contexts.
#[allow(dead_code)]
pub fn sql_input(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
    cx: &App,
) -> impl IntoElement {
    input_with_context(editor, focus, multiline, SQL_TEXT_EDITOR_CONTEXT, false, cx)
}

/// Render the query SQL editor so it fills a resizable parent pane.
///
/// This intentionally does not alter [`sql_input`]: row and JSON editors
/// retain their compact, fixed multiline height.
pub fn sql_input_fill(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    cx: &App,
) -> impl IntoElement {
    input_with_context(editor, focus, true, SQL_TEXT_EDITOR_CONTEXT, true, cx)
}

fn accessible_input_shell(editor: Entity<TextEditor>, cx: &App) -> gpui::Stateful<gpui::Div> {
    let state = editor.read(cx);
    let password = state.password;
    let read_only = state.read_only;
    let multiline = state.multiline;
    let invalid = state
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
    let description = if password {
        // Diagnostic text can contain input excerpts. Never publish them for
        // a protected field, even if a future validator supplies diagnostics.
        if invalid {
            "Invalid value".to_owned()
        } else {
            String::new()
        }
    } else {
        state
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let severity = match diagnostic.severity {
                    DiagnosticSeverity::Error => "Error",
                    DiagnosticSeverity::Warning => "Warning",
                };
                format!("{severity}: {}", diagnostic.message)
            })
            .collect::<Vec<_>>()
            .join(". ")
    };
    let selection = state.selected_range.clone();
    let reversed = state.selection_reversed;
    let label = state.accessibility_label.clone();
    let accessible_text = state.accessible_text.clone();
    let entity = editor.clone();
    div()
        .id(gpui::SharedString::from(format!(
            "dbx-text-editor-{:?}",
            editor.entity_id()
        )))
        .role(if password {
            gpui::Role::PasswordInput
        } else if multiline {
            gpui::Role::MultilineTextInput
        } else {
            gpui::Role::TextInput
        })
        .aria_label(label)
        .aria_description(description)
        .a11y_synthetic_children(move |builder| {
            let node = builder.parent_node();
            if invalid {
                node.set_invalid(gpui::accesskit::Invalid::True);
            }
            if read_only {
                node.set_read_only();
            }
            if password {
                return;
            }
            let mut snapshot = accessible_text.borrow_mut();
            if snapshot.runs.is_empty() {
                return;
            }
            for (index, run) in snapshot.runs.iter_mut().enumerate() {
                run.id = builder.synthetic_node_id(index);
            }
            for run in &snapshot.runs {
                builder.push_child(run.id, snapshot.node(run));
            }
            builder
                .parent_node()
                .set_text_selection(snapshot.selection(selection, reversed));
        })
        .map(|mut view| {
            for action in [
                gpui::AccessibleAction::SetValue,
                gpui::AccessibleAction::ReplaceSelectedText,
                gpui::AccessibleAction::SetTextSelection,
                gpui::AccessibleAction::ScrollIntoView,
            ] {
                if password && action == gpui::AccessibleAction::SetTextSelection {
                    continue;
                }
                if read_only
                    && matches!(
                        action,
                        gpui::AccessibleAction::SetValue
                            | gpui::AccessibleAction::ReplaceSelectedText
                    )
                {
                    continue;
                }
                let entity = entity.clone();
                view = view.on_a11y_action(action, move |data, window, cx| {
                    entity.update(cx, |editor, cx| {
                        editor.accessible_action(action, data, window, cx)
                    });
                });
            }
            view
        })
}

fn input_font_size(editor: &Entity<TextEditor>, cx: &App) -> Pixels {
    if editor.read(cx).row_value {
        dbx_ui_components::theme::row_text_px(12.)
    } else if editor.read(cx).is_code() {
        dbx_ui_components::theme::mono_text_px(12.) * dbx_ui_components::theme::editor_scale()
    } else {
        dbx_ui_components::theme::text_px(12.)
    }
}

/// The height of a standard single-line field, for sibling controls that
/// must line up with it.
pub fn single_line_input_height(editor: &Entity<TextEditor>, cx: &App) -> Pixels {
    let text = input_font_size(editor, cx);
    // Match GPUI's default line height and pixel rounding. Both lengths are
    // already in pixels, so the rem size is unused here.
    let line_height = gpui::phi().to_pixels(text.into(), text).round();
    // Seven design pixels of padding on each side, plus two unscaled borders.
    dbx_ui_components::theme::ui_px(32.)
        .max(line_height + dbx_ui_components::theme::ui_px(14.) + px(2.))
}

fn input_with_context(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
    key_context: &'static str,
    fill_height: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    accessible_input_shell(editor.clone(), cx)
        .key_context(key_context)
        .track_focus(&focus)
        // GPUI 0.2.2 invalidates the owning view whenever a cursor-bearing
        // element is entered or left. This shared shell has no hover style,
        // so that invalidation only re-runs intrinsic text layout and makes
        // compact inputs visibly jump under the pointer.
        .w_full()
        .min_w_0()
        .when(fill_height, |this| this.flex_1().min_h_0().h_full())
        .when(!fill_height, |this| {
            this.h(if multiline {
                dbx_ui_components::theme::ui_px(204.)
            } else {
                single_line_input_height(&editor, cx)
            })
        })
        .p(if multiline {
            dbx_ui_components::theme::ui_px(10.)
        } else {
            dbx_ui_components::theme::ui_px(7.)
        })
        .overflow_hidden()
        .bg(theme().canvas)
        .border_1()
        .border_color(theme().input_border)
        // Colour only: a width change would shift neighbouring controls.
        .focus(|style| style.border_color(theme().focus_ring))
        .rounded(dbx_ui_components::theme::ui_px(5.))
        .text_size(input_font_size(&editor, cx))
        .when(!multiline, |this| this.line_height(gpui::phi()))
        .text_color(theme().text)
        .on_action({
            let editor = editor.clone();
            move |action: &Backspace, window, cx| {
                editor.update(cx, |editor, cx| editor.backspace(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Delete, window, cx| {
                editor.update(cx, |editor, cx| editor.delete(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Left, window, cx| {
                editor.update(cx, |editor, cx| editor.left(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Right, window, cx| {
                editor.update(cx, |editor, cx| editor.right(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectLeft, window, cx| {
                editor.update(cx, |editor, cx| editor.select_left(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectRight, window, cx| {
                editor.update(cx, |editor, cx| editor.select_right(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectAll, window, cx| {
                editor.update(cx, |editor, cx| editor.select_all(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Home, window, cx| {
                editor.update(cx, |editor, cx| editor.home(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &End, window, cx| {
                editor.update(cx, |editor, cx| editor.end(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveHome, window, cx| {
                editor.update(cx, |editor, cx| editor.component_home(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveEnd, window, cx| {
                editor.update(cx, |editor, cx| editor.component_end(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectToStartOfLine, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.component_select_home(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectToEndOfLine, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.component_select_end(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveToPreviousWord, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.component_previous_word(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveToNextWord, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.component_next_word(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectToPreviousWordStart, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.component_select_previous_word(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectToNextWordEnd, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.component_select_next_word(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Up, window, cx| {
                editor.update(cx, |editor, cx| editor.up(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Down, window, cx| {
                editor.update(cx, |editor, cx| editor.down(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Enter, window, cx| {
                editor.update(cx, |editor, cx| editor.newline(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Paste, window, cx| {
                editor.update(cx, |editor, cx| editor.paste(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Cut, window, cx| {
                editor.update(cx, |editor, cx| editor.cut(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Copy, window, cx| {
                editor.update(cx, |editor, cx| editor.copy(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Undo, window, cx| {
                editor.update(cx, |editor, cx| editor.undo_action(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Redo, window, cx| {
                editor.update(cx, |editor, cx| editor.redo_action(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &ShowCharacterPalette, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.show_character_palette(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Indent, window, cx| {
                editor.update(cx, |editor, cx| editor.indent(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Outdent, window, cx| {
                editor.update(cx, |editor, cx| editor.outdent(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &ToggleLineComment, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.toggle_line_comment(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &DuplicateLines, window, cx| {
                editor.update(cx, |editor, cx| editor.duplicate_lines(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveLinesUp, window, cx| {
                editor.update(cx, |editor, cx| editor.move_lines_up(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveLinesDown, window, cx| {
                editor.update(cx, |editor, cx| editor.move_lines_down(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &DeleteLines, window, cx| {
                editor.update(cx, |editor, cx| editor.delete_lines(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectUp, window, cx| {
                editor.update(cx, |editor, cx| editor.select_up(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectDown, window, cx| {
                editor.update(cx, |editor, cx| editor.select_down(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveToStart, window, cx| {
                editor.update(cx, |editor, cx| editor.move_to_start(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &MoveToEnd, window, cx| {
                editor.update(cx, |editor, cx| editor.move_to_end(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectToStart, window, cx| {
                editor.update(cx, |editor, cx| editor.select_to_start(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectToEnd, window, cx| {
                editor.update(cx, |editor, cx| editor.select_to_end(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &DeleteWordLeft, window, cx| {
                editor.update(cx, |editor, cx| editor.delete_word_left(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &DeleteToLineStart, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.delete_to_line_start(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectLine, window, cx| {
                editor.update(cx, |editor, cx| editor.select_line(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectNextOccurrence, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.select_next_occurrence(action, window, cx)
                });
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Fold, window, cx| {
                editor.update(cx, |editor, cx| editor.fold(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &Unfold, window, cx| {
                editor.update(cx, |editor, cx| editor.unfold(action, window, cx));
            }
        })
        .on_action({
            let editor = editor.clone();
            move |action: &SelectAllOccurrences, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.select_all_occurrences(action, window, cx)
                });
            }
        })
        .on_mouse_down(MouseButton::Left, {
            let editor = editor.clone();
            move |event, window, cx| {
                editor.update(cx, |editor, cx| editor.on_mouse_down(event, window, cx));
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let editor = editor.clone();
            move |event, window, cx| {
                editor.update(cx, |editor, cx| editor.on_mouse_up(event, window, cx));
            }
        })
        .on_mouse_up_out(MouseButton::Left, {
            let editor = editor.clone();
            move |event, window, cx| {
                editor.update(cx, |editor, cx| editor.on_mouse_up(event, window, cx));
            }
        })
        .child(editor)
}

fn editor_text_runs(
    line: &str,
    line_start: usize,
    style: &gpui::TextStyle,
    language: EditorLanguage,
    tokens: &[HighlightToken],
    marked_range: Option<&Range<usize>>,
    diagnostics: &[EditorDiagnostic],
) -> Vec<TextRun> {
    let base = style.to_run(line.len());
    let runs = match language {
        EditorLanguage::PlainText => vec![base],
        EditorLanguage::Sql(_) | EditorLanguage::Redis | EditorLanguage::Json => {
            syntax_runs(line, line_start, tokens, &base)
        }
    };
    let runs = apply_marked_runs(line.len(), line_start, marked_range, runs);
    apply_diagnostic_runs(line.len(), line_start, diagnostics, runs)
}

fn syntax_runs(
    line: &str,
    line_start: usize,
    tokens: &[HighlightToken],
    base: &TextRun,
) -> Vec<TextRun> {
    let line_end = line_start + line.len();
    let mut runs = Vec::new();
    let mut offset = line_start;

    for token in tokens {
        if token.range.end <= line_start {
            continue;
        }
        if token.range.start >= line_end {
            break;
        }

        let start = token.range.start.max(line_start).min(line_end);
        let end = token.range.end.max(line_start).min(line_end);
        if end <= start {
            continue;
        }
        if start > offset {
            runs.push(TextRun {
                len: start - offset,
                ..base.clone()
            });
        }
        runs.push(TextRun {
            len: end - start,
            color: token.color(),
            ..base.clone()
        });
        offset = end;
    }

    if offset < line_end {
        runs.push(TextRun {
            len: line_end - offset,
            ..base.clone()
        });
    }
    if runs.is_empty() {
        runs.push(base.clone());
    }
    runs
}

fn apply_marked_runs(
    line_len: usize,
    line_start: usize,
    marked_range: Option<&Range<usize>>,
    runs: Vec<TextRun>,
) -> Vec<TextRun> {
    let Some(marked_range) =
        marked_range.and_then(|range| marked_slice(line_len, line_start, range))
    else {
        return runs;
    };

    let mut result = Vec::with_capacity(runs.len() + 2);
    let mut offset = 0;
    for run in runs {
        let run_start = offset;
        let run_end = offset + run.len;
        let start = marked_range.start.max(run_start).min(run_end);
        let end = marked_range.end.max(run_start).min(run_end);

        if start > run_start {
            result.push(TextRun {
                len: start - run_start,
                ..run.clone()
            });
        }
        if end > start {
            result.push(TextRun {
                len: end - start,
                underline: Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: dbx_ui_components::theme::ui_px(1.),
                    wavy: false,
                }),
                ..run.clone()
            });
        }
        if run_end > end {
            result.push(TextRun {
                len: run_end - end,
                ..run
            });
        }
        offset = run_end;
    }
    result
}

/// Split runs so every diagnostic range on this line carries a wavy error
/// underline. Applied after IME marking so a composing range keeps its own
/// treatment while the diagnostic underline remains visible underneath.
fn apply_diagnostic_runs(
    line_len: usize,
    line_start: usize,
    diagnostics: &[EditorDiagnostic],
    mut runs: Vec<TextRun>,
) -> Vec<TextRun> {
    for diagnostic in diagnostics {
        let Some(range) = marked_slice(line_len, line_start, &diagnostic.range) else {
            continue;
        };
        let color = match diagnostic.severity {
            DiagnosticSeverity::Error => theme().danger,
            DiagnosticSeverity::Warning => theme().warning,
        };
        let mut result = Vec::with_capacity(runs.len() + 2);
        let mut offset = 0;
        for run in runs.drain(..) {
            let run_start = offset;
            let run_end = offset + run.len;
            let start = range.start.max(run_start).min(run_end);
            let end = range.end.max(run_start).min(run_end);

            if start > run_start {
                result.push(TextRun {
                    len: start - run_start,
                    ..run.clone()
                });
            }
            if end > start {
                result.push(TextRun {
                    len: end - start,
                    underline: Some(UnderlineStyle {
                        color: Some(color.into()),
                        thickness: dbx_ui_components::theme::ui_px(1.),
                        wavy: true,
                    }),
                    ..run.clone()
                });
            }
            if run_end > end {
                result.push(TextRun {
                    len: run_end - end,
                    ..run
                });
            }
            offset = run_end;
        }
        runs = result;
    }
    runs
}

fn sql_token_color(kind: SqlTokenKind) -> gpui::Hsla {
    match kind {
        SqlTokenKind::Keyword => theme().sql_keyword.into(),
        SqlTokenKind::String => theme().sql_string.into(),
        SqlTokenKind::Comment => theme().sql_comment.into(),
        SqlTokenKind::Number => theme().sql_number.into(),
        SqlTokenKind::Parameter => theme().sql_parameter.into(),
        SqlTokenKind::Identifier => theme().sql_identifier.into(),
        SqlTokenKind::Type => theme().sql_type.into(),
    }
}

fn redis_token_color(kind: RedisTokenKind) -> gpui::Hsla {
    match kind {
        RedisTokenKind::Command => theme().sql_keyword.into(),
        RedisTokenKind::Option => theme().sql_type.into(),
        RedisTokenKind::String => theme().sql_string.into(),
        RedisTokenKind::Number => theme().sql_number.into(),
        RedisTokenKind::Identifier => theme().sql_identifier.into(),
    }
}

pub fn json_token_color(kind: JsonTokenKind) -> gpui::Hsla {
    match kind {
        JsonTokenKind::Property => theme().sql_identifier.into(),
        JsonTokenKind::String => theme().sql_string.into(),
        JsonTokenKind::Number => theme().sql_number.into(),
        JsonTokenKind::Boolean | JsonTokenKind::Null => theme().sql_keyword.into(),
    }
}

fn normalize_value(text: &str, multiline: bool) -> String {
    if multiline {
        text.to_owned()
    } else {
        text.chars()
            .map(|character| match character {
                '\n' | '\r' => ' ',
                character => character,
            })
            .collect()
    }
}

/// Return the text painted by a password editor.
///
/// A bullet is emitted for every Unicode scalar so the rendered string has a
/// deterministic byte-index mapping back to the stored value. Newlines stay
/// visible so multiline editor geometry remains unchanged.
fn password_mask(text: &str) -> String {
    text.chars()
        .map(|character| if character == '\n' { '\n' } else { '•' })
        .collect()
}

/// Translate a source byte offset to the equivalent masked-text byte offset.
fn password_display_offset(text: &str, source_offset: usize) -> usize {
    let source_offset = clamp_boundary(text, source_offset);
    text.char_indices()
        .take_while(|(offset, _)| *offset < source_offset)
        .map(|(_, character)| {
            if character == '\n' {
                1
            } else {
                '•'.len_utf8()
            }
        })
        .sum()
}

/// Translate a masked-text byte offset back to the corresponding source byte
/// offset. This is used only for pointer hit testing; selections and input
/// protocol ranges always remain offsets into the stored value.
fn password_source_offset(text: &str, display_offset: usize) -> usize {
    let mut displayed = 0;
    for (source_offset, character) in text.char_indices() {
        let width = if character == '\n' {
            1
        } else {
            '•'.len_utf8()
        };
        if display_offset < displayed + width {
            return source_offset;
        }
        displayed += width;
    }
    text.len()
}

fn clamp_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn clamp_range(text: &str, range: Range<usize>) -> Range<usize> {
    clamp_boundary(text, range.start)..clamp_boundary(text, range.end)
}

fn previous_boundary(text: &str, offset: usize) -> usize {
    let offset = clamp_boundary(text, offset);
    text.grapheme_indices(true)
        .rev()
        .find_map(|(index, _)| (index < offset).then_some(index))
        .unwrap_or(0)
}

fn next_boundary(text: &str, offset: usize) -> usize {
    let offset = clamp_boundary(text, offset);
    text.grapheme_indices(true)
        .find_map(|(index, _)| (index > offset).then_some(index))
        .unwrap_or(text.len())
}

/// Match gpui-component's word navigation: left lands at the start of the
/// previous word and right lands at the end of the next word. Keeping the
/// implementation here preserves that behavior for DBX's custom renderer,
/// including its UTF-8/grapheme-safe selection model.
fn previous_word_boundary(text: &str, offset: usize) -> usize {
    let offset = clamp_boundary(text, offset);
    UnicodeSegmentation::split_word_bound_indices(&text[..offset])
        .rfind(|(_, segment)| !segment.trim_start().is_empty())
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_word_boundary(text: &str, offset: usize) -> usize {
    let offset = clamp_boundary(text, offset);
    let right = &text[offset..];
    UnicodeSegmentation::split_word_bound_indices(right)
        .find(|(_, segment)| !segment.trim_start().is_empty())
        .map(|(index, segment)| offset + index + segment.len())
        .unwrap_or(text.len())
}

fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    let mut bytes = 0;
    let mut units = 0;
    for character in text.chars() {
        if units >= offset {
            break;
        }
        units += character.len_utf16();
        bytes += character.len_utf8();
    }
    bytes.min(text.len())
}

fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    let offset = clamp_boundary(text, offset);
    let mut bytes = 0;
    let mut units = 0;
    for character in text.chars() {
        if bytes >= offset {
            break;
        }
        bytes += character.len_utf8();
        units += character.len_utf16();
    }
    units
}

fn line_start(text: &str, cursor: usize) -> usize {
    let cursor = clamp_boundary(text, cursor);
    text[..cursor].rfind('\n').map_or(0, |index| index + 1)
}

fn line_end(text: &str, cursor: usize) -> usize {
    let cursor = clamp_boundary(text, cursor);
    text[cursor..]
        .find('\n')
        .map_or(text.len(), |index| cursor + index)
}

fn line_and_column(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = clamp_boundary(text, cursor);
    let start = line_start(text, cursor);
    (
        text[..start].bytes().filter(|byte| *byte == b'\n').count(),
        cursor - start,
    )
}

fn nth_line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 1)
        .map_or(text.len(), |(index, _)| index + 1)
}

fn replace_selection(text: &str, range: Range<usize>, inserted: &str) -> String {
    format!("{}{}{}", &text[..range.start], inserted, &text[range.end..])
}

fn marked_slice(
    line_len: usize,
    line_start: usize,
    marked_range: &Range<usize>,
) -> Option<Range<usize>> {
    let start = marked_range.start.saturating_sub(line_start).min(line_len);
    let end = marked_range.end.saturating_sub(line_start).min(line_len);
    (start < end).then_some(start..end)
}

fn reveal_offset(
    current: Pixels,
    maximum: Pixels,
    viewport: Pixels,
    target_start: Pixels,
    target_end: Pixels,
) -> Pixels {
    let mut next = current.clamp(
        dbx_ui_components::theme::ui_px(0.),
        maximum.max(dbx_ui_components::theme::ui_px(0.)),
    );
    // If the target itself is taller/wider than the viewport, neither edge
    // can be revealed at once. Keeping the existing offset avoids a frame to
    // frame top/bottom oscillation.
    if target_end - target_start >= viewport {
        return next;
    }
    if target_start < next {
        next = target_start;
    } else if target_end > next + viewport {
        next = target_end - viewport;
    }
    next.clamp(
        dbx_ui_components::theme::ui_px(0.),
        maximum.max(dbx_ui_components::theme::ui_px(0.)),
    )
}

fn scroll_offset_for_cursor(
    current: Point<Pixels>,
    maximum: Size<Pixels>,
    viewport: Size<Pixels>,
    cursor_position: Point<Pixels>,
    cursor_size: Size<Pixels>,
    reveal_vertical: bool,
) -> Point<Pixels> {
    let x = reveal_offset(
        -current.x,
        maximum.width,
        viewport.width,
        cursor_position.x,
        cursor_position.x + cursor_size.width,
    );
    let y = if reveal_vertical {
        reveal_offset(
            -current.y,
            maximum.height,
            viewport.height,
            cursor_position.y,
            cursor_position.y + cursor_size.height,
        )
    } else {
        -current.y
    };
    point(-x, -y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::DatabaseKind;

    struct ScrollTestView {
        editor: Entity<TextEditor>,
        scroll: ScrollHandle,
        multiline: bool,
    }

    impl Render for ScrollTestView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .id("test-sidebar")
                    .w(px(240.))
                    .h(px(160.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(input_with_height(
                                self.editor.clone(),
                                self.editor.read(cx).focus_handle(),
                                self.multiline,
                                px(80.),
                                cx,
                            ))
                            .child(div().h(px(600.)).flex_none()),
                    ),
            )
        }
    }

    fn scroll_test_view(
        multiline: bool,
        window: &mut Window,
        cx: &mut Context<ScrollTestView>,
    ) -> ScrollTestView {
        let value = cx.new(|_| {
            if multiline {
                (0..80)
                    .map(|n| format!("row {n}: café 🙂 {}\n", "long text ".repeat(12)))
                    .collect()
            } else {
                "café 🙂 long text ".repeat(80)
            }
        });
        let editor = cx.new(|cx| {
            TextEditor::new(value, multiline, window, cx)
                .read_only()
                .row_value()
        });
        ScrollTestView {
            editor,
            scroll: ScrollHandle::new(),
            multiline,
        }
    }

    #[gpui::test]
    fn hovered_values_scroll_only_for_gestures_that_start_over_them(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (view, cx) = cx.add_window_view(|window, cx| scroll_test_view(true, window, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let editor = view.read_with(cx, |view, _| view.editor.clone());
        let position = editor.read_with(cx, |editor, _| editor.scroll_handle.bounds().center());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        view.read_with(cx, |view, cx| {
            assert!(
                view.scroll.offset().y < px(0.),
                "a gesture that began elsewhere keeps scrolling the sidebar"
            );
            assert_eq!(view.editor.read(cx).scroll_handle.offset().y, px(0.));
        });
        view.update(cx, |view, cx| {
            view.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // No focus needed: a gesture starting over the value scrolls it.
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-40.))),
            touch_phase: gpui::TouchPhase::Started,
            ..Default::default()
        });
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.refresh();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, cx| {
            assert_eq!(view.scroll.offset().y, px(0.));
            assert!(
                view.editor.read(cx).scroll_handle.offset().y < px(0.),
                "focused wheel scroll must not snap back to the caret"
            );
        });
        // Once the value reaches its end, let the sidebar continue naturally.
        editor.update(cx, |editor, cx| {
            editor
                .scroll_handle
                .set_offset(point(px(0.), -editor.scroll_handle.max_offset().y));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        view.read_with(cx, |view, _| assert!(view.scroll.offset().y < px(0.)));
    }

    #[gpui::test]
    fn dragging_beyond_a_field_reveals_text_and_stops_on_release(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        for multiline in [false, true] {
            let (view, cx) =
                cx.add_window_view(|window, cx| scroll_test_view(multiline, window, cx));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let editor = view.read_with(cx, |view, _| view.editor.clone());
            let bounds = editor.read_with(cx, |editor, _| editor.scroll_handle.bounds());
            let start = point(bounds.left() + px(2.), bounds.top() + px(8.));
            let outside = if multiline {
                point(start.x + px(40.), bounds.bottom() + px(25.))
            } else {
                point(bounds.right() + px(25.), start.y)
            };
            cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
            // A fast drag may move before the mouse-down notification paints.
            cx.simulate_mouse_move(outside, MouseButton::Left, Default::default());
            editor.read_with(cx, |editor, _| assert!(!editor.selected_range.is_empty()));
            let first_selection = editor.read_with(cx, |editor, _| editor.selected_range.end);
            // Exercise frame callbacks while the pointer is held stationary.
            for _ in 0..5 {
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                    window.refresh();
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            let (offset, selection) = editor.read_with(cx, |editor, cx| {
                let offset = editor.scroll_handle.offset();
                assert!(if multiline {
                    offset.y < px(0.)
                } else {
                    offset.x < px(0.)
                });
                assert!(
                    editor.selected_range.end > first_selection,
                    "holding beyond the edge must keep extending the selection"
                );
                assert!(editor.text(cx).is_char_boundary(editor.selected_range.end));
                (offset, editor.selected_range.clone())
            });
            cx.simulate_mouse_up(outside, MouseButton::Left, Default::default());
            for _ in 0..3 {
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                    window.refresh();
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            editor.read_with(cx, |editor, _| {
                assert!(!editor.is_selecting);
                assert_eq!(editor.scroll_handle.offset(), offset);
                assert_eq!(editor.selected_range, selection);
            });
        }
    }

    #[test]
    fn sql_execution_uses_the_statement_at_the_caret() {
        let text = "SELECT 1;\nSELECT 🦀 FROM users;\nSELECT 3";
        let cursor = text.find("🦀").unwrap();
        let range = execution_range(
            text,
            0..0,
            cursor,
            QueryExecutionScope::SelectionOrStatement,
            None,
        );
        assert_eq!(&text[range], "\nSELECT 🦀 FROM users;");

        let selection = text.find("SELECT 3").unwrap()..text.len();
        let range = execution_range(
            text,
            selection,
            cursor,
            QueryExecutionScope::SelectionOrStatement,
            None,
        );
        assert_eq!(&text[range], "SELECT 3");
    }

    #[test]
    fn statements_and_tokens_follow_the_connection_dialect() {
        let postgres = Some(DatabaseKind::PostgreSQL);
        let mysql = Some(DatabaseKind::MySQL);
        // `#` is an operator in PostgreSQL and a comment in MySQL.
        let text = "SELECT doc #>> '{a}' FROM t; SELECT 2";
        assert!(
            !lex_sql_for(text, postgres)
                .iter()
                .any(|token| token.kind == SqlTokenKind::Comment)
        );
        assert_eq!(sql_statement_count(text, postgres), 2);
        let commented = "SELECT 1; # a;b\nSELECT 2";
        let second = commented.find("SELECT 2").unwrap();
        assert_eq!(
            &commented[sql_statement_range(commented, second, mysql)],
            " # a;b\nSELECT 2"
        );
        // PostgreSQL standard strings keep backslashes literal.
        let path = "SELECT 'C:\\'; SELECT 2";
        assert_eq!(sql_statement_count(path, postgres), 2);
        // `$$` is a MySQL delimiter, not a string.
        let routine = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END $$\nDELIMITER ;";
        assert!(
            lex_sql_for(routine, mysql)
                .iter()
                .any(|token| token.kind == SqlTokenKind::Keyword
                    && &routine[token.range.clone()] == "CREATE")
        );
        let body = routine.find("SELECT 1").unwrap();
        assert_eq!(sql_statement_delimiter(routine, body, mysql), ";");
        let routines = "DELIMITER $$\nCREATE PROCEDURE a() BEGIN SELECT 1; END $$\nCREATE PROCEDURE b() BEGIN SELECT 2; END $$\nDELIMITER ;";
        let second_body = routines.find("SELECT 2").unwrap();
        assert_eq!(sql_statement_delimiter(routines, second_body, mysql), "$$");
        // Danger classification sees past PostgreSQL `#` operators.
        assert_eq!(
            sql_execution_kind("UPDATE t SET flags = flags # 4 WHERE id = 1", postgres),
            SqlExecutionKind::MutationRisk
        );
    }

    #[test]
    fn schema_change_detection_is_lexer_aware_and_conservative() {
        assert!(sql_may_change_schema(
            "SELECT 1; CREATE TABLE audit_log (id int)",
            None
        ));
        assert!(sql_may_change_schema(
            "ALTER TABLE users ADD COLUMN active boolean",
            None
        ));
        assert!(sql_may_change_schema("CALL install_schema()", None));
        assert!(!sql_may_change_schema(
            "SELECT 'CREATE TABLE decoy'; -- DROP TABLE decoy\nSELECT 2",
            None
        ));
        assert!(!sql_may_change_schema(
            "SELECT $$ALTER TABLE decoy ADD COLUMN value int$$",
            None
        ));
    }

    #[test]
    fn execution_kind_is_conservative_about_writes() {
        assert_eq!(sql_execution_kind("SELECT 1", None), SqlExecutionKind::Read);
        assert_eq!(
            sql_execution_kind("SELECT 1; UPDATE users SET active = TRUE", None),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("DELETE FROM users", None),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("DELETE FROM users WHERE id = 1", None),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind("UPDATE users SET active = TRUE", None),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("UPDATE users SET active = TRUE WHERE id = 1", None),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind(
                "WITH doomed AS (SELECT id FROM users) DELETE FROM users",
                None
            ),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind(
                "WITH changed AS (SELECT id FROM users) UPDATE users SET active = TRUE",
                None
            ),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind(
                "WITH matched AS (SELECT id FROM users) DELETE FROM users WHERE id = 1",
                None
            ),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind(
                "WITH deleted AS (DELETE FROM users RETURNING *) SELECT * FROM deleted",
                None
            ),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind(
                "WITH changed AS (UPDATE users SET active = TRUE WHERE id = 1 RETURNING *) SELECT * FROM changed",
                None
            ),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind(
                "WITH selected AS (SELECT id FROM users WHERE active = TRUE) SELECT * FROM selected",
                None
            ),
            SqlExecutionKind::Read
        );
        assert_eq!(
            sql_execution_kind(
                "WITH selected AS (SELECT 'DELETE FROM users', $$UPDATE users$$) /* DELETE */ SELECT * FROM selected",
                None
            ),
            SqlExecutionKind::Read
        );
        assert_eq!(
            sql_execution_kind(
                "MERGE INTO users USING incoming ON users.id = incoming.id",
                None
            ),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("REPLACE INTO users(id) VALUES (1)", None),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("DROP TABLE users", None),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("EXPLAIN SELECT 1", None),
            SqlExecutionKind::MutationRisk
        );
    }
}
