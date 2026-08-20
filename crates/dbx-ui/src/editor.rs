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

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InteractiveElement as _,
    IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ScrollHandle, ShapedLine, Size, StatefulInteractiveElement as _,
    Style, Subscription, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill,
    point, prelude::*, px, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::THEME;

/// Key context placed on the editor's root element.
///
/// DBX can bind the actions returned from [`default_key_bindings`] globally;
/// this context keeps those bindings scoped to text editors.
pub const TEXT_EDITOR_CONTEXT: &str = "DbxTextEditor";

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
        ShowCharacterPalette,
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
        KeyBinding::new("home", Home, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("end", End, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("up", Up, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("down", Down, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new("enter", Enter, Some(TEXT_EDITOR_CONTEXT)),
        KeyBinding::new(
            "ctrl-cmd-space",
            ShowCharacterPalette,
            Some(TEXT_EDITOR_CONTEXT),
        ),
    ]
}

/// The language used when painting an editor's text.
///
/// Editors remain plain text by default.  SQL highlighting is opt-in so the
/// same editor can continue to be used for connection strings, filters, and
/// editable cells without paying for a lexer pass or changing their colors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorLanguage {
    #[default]
    PlainText,
    Sql,
}

/// The lexical categories understood by the built-in SQL highlighter.
///
/// This is intentionally a lexer rather than a parser.  It is safe to use
/// while a query is incomplete, as it is while the user is typing, and every
/// returned range is a UTF-8 character boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlTokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Parameter,
    Identifier,
    Type,
}

/// A token returned by [`lex_sql`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlToken {
    pub kind: SqlTokenKind,
    pub range: Range<usize>,
}

/// Lex SQL into byte ranges suitable for syntax highlighting.
///
/// The lexer is deliberately tolerant: malformed or unfinished strings and
/// comments are highlighted through the end of the input, which keeps the
/// query editor useful while a statement is being written.  Whitespace and
/// punctuation are omitted from the result and should use the editor's base
/// color.
pub fn lex_sql(text: &str) -> Vec<SqlToken> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let character = chars[index].1;
        let start = chars[index].0;

        if character.is_whitespace() {
            index += 1;
            continue;
        }

        if character == '-' && next_char(&chars, index) == Some('-') {
            let mut end = index + 2;
            while end < chars.len() && chars[end].1 != '\n' {
                end += 1;
            }
            push_sql_token(
                &mut tokens,
                SqlTokenKind::Comment,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        // MySQL also accepts # comments.  Keeping this here is harmless for
        // the other dialects and makes the editor useful across all engines.
        if character == '#' {
            let mut end = index + 1;
            while end < chars.len() && chars[end].1 != '\n' {
                end += 1;
            }
            push_sql_token(
                &mut tokens,
                SqlTokenKind::Comment,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        if character == '/' && next_char(&chars, index) == Some('*') {
            let mut end = index + 2;
            while end + 1 < chars.len() && !(chars[end].1 == '*' && chars[end + 1].1 == '/') {
                end += 1;
            }
            if end + 1 < chars.len() {
                end += 2;
            } else {
                end = chars.len();
            }
            push_sql_token(
                &mut tokens,
                SqlTokenKind::Comment,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        if character == '\'' {
            let end = consume_quoted(&chars, index, character);
            push_sql_token(
                &mut tokens,
                SqlTokenKind::String,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        if character == '"' || character == '`' {
            let end = consume_quoted(&chars, index, character);
            push_sql_token(
                &mut tokens,
                SqlTokenKind::Identifier,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        if character == '$' {
            if let Some(end) = consume_dollar_quoted(&chars, text, index) {
                push_sql_token(
                    &mut tokens,
                    SqlTokenKind::String,
                    start,
                    byte_end(&chars, end, text.len()),
                );
                index = end;
                continue;
            }

            if let Some(end) = consume_parameter(&chars, index) {
                push_sql_token(
                    &mut tokens,
                    SqlTokenKind::Parameter,
                    start,
                    byte_end(&chars, end, text.len()),
                );
                index = end;
                continue;
            }
        }

        // PostgreSQL's cast operator is punctuation, not a named parameter.
        // Consume it as a pair so the type following `::` is still lexed.
        if character == ':' && next_char(&chars, index) == Some(':') {
            index += 2;
            continue;
        }

        if (character == ':' || character == '@' || character == '?')
            && consume_parameter(&chars, index).is_some()
        {
            let end = consume_parameter(&chars, index).unwrap_or(index + 1);
            push_sql_token(
                &mut tokens,
                SqlTokenKind::Parameter,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        if character.is_ascii_digit()
            || (character == '.'
                && next_char(&chars, index).is_some_and(|next| next.is_ascii_digit()))
        {
            let end = consume_number(&chars, index);
            push_sql_token(
                &mut tokens,
                SqlTokenKind::Number,
                start,
                byte_end(&chars, end, text.len()),
            );
            index = end;
            continue;
        }

        if is_identifier_start(character) {
            let end = consume_identifier(&chars, index);
            let word_end = byte_end(&chars, end, text.len());
            let word = &text[start..word_end];
            let kind = if is_sql_keyword(word) {
                SqlTokenKind::Keyword
            } else if is_sql_type(word) {
                SqlTokenKind::Type
            } else {
                SqlTokenKind::Identifier
            };
            push_sql_token(&mut tokens, kind, start, word_end);
            index = end;
            continue;
        }

        index += 1;
    }

    tokens
}

/// A native single-line or multiline text editor.
pub struct TextEditor {
    /// The surrounding view owns this entity and can observe it for changes.
    value: Entity<String>,
    /// A frame-local copy used while painting.  Keeping this copy avoids
    /// borrowing the value entity throughout custom element layout/paint.
    content: String,
    focus_handle: FocusHandle,
    multiline: bool,
    language: EditorLanguage,
    /// Paint text as bullets while retaining the backing value for input and
    /// connection-string construction.
    password: bool,
    /// UTF-8 byte ranges at grapheme boundaries.
    selected_range: Range<usize>,
    selection_reversed: bool,
    /// The currently composing IME text, as a UTF-8 byte range.
    marked_range: Option<Range<usize>>,
    scroll_handle: ScrollHandle,
    last_layout: Vec<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// The painted text origin at the start of a pointer selection. A
    /// selection updates this view on every pointer move; retaining the
    /// original origin keeps those updates from changing the coordinates used
    /// to resolve the next character under the pointer.
    selection_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    _subscriptions: Vec<Subscription>,
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
        let focus_handle = cx.focus_handle();
        let content = value.read(cx).clone();
        let observed = cx.observe(&value, |this, value, cx| {
            let text = value.read(cx);
            this.content = text.clone();
            this.selected_range = clamp_range(text, this.selected_range.clone());
            this.marked_range = this
                .marked_range
                .take()
                .map(|range| clamp_range(text, range));
            cx.notify();
        });
        let focus = cx.on_focus(&focus_handle, window, |_, _, cx| cx.notify());
        let blur = cx.on_blur(&focus_handle, window, |_, _, cx| cx.notify());

        Self {
            value,
            content,
            focus_handle,
            multiline,
            language,
            password: false,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            scroll_handle: ScrollHandle::new(),
            last_layout: Vec::new(),
            last_bounds: None,
            selection_bounds: None,
            is_selecting: false,
            _subscriptions: vec![observed, focus, blur],
        }
    }

    /// Create a multiline SQL editor backed by an existing value entity.
    pub fn new_sql(value: Entity<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_language(value, true, EditorLanguage::Sql, window, cx)
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
        let text = normalize_value(&text.into(), self.multiline);
        let cursor = text.len();
        self.content = text.clone();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.value.update(cx, |value, cx| {
            *value = text;
            cx.notify();
        });
        cx.notify();
    }

    /// The current selection as UTF-8 byte offsets.
    #[allow(dead_code)]
    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = clamp_boundary(&self.text(cx), offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
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
        let text = self.text(cx);
        let range = clamp_range(&text, range);
        let inserted = normalize_value(inserted, self.multiline);
        let next = replace_selection(&text, range.clone(), &inserted);
        let cursor = range.start + inserted.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.set_text_without_selection(next, cx);
        cx.notify();
    }

    fn set_text_without_selection(&mut self, text: String, cx: &mut Context<Self>) {
        self.content = text.clone();
        self.value.update(cx, |value, cx| {
            *value = text;
            cx.notify();
        });
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
        self.move_to(0, cx);
        self.select_to(self.text(cx).len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(line_start(&self.text(cx), self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(line_end(&self.text(cx), self.cursor_offset()), cx);
    }

    fn vertical(&mut self, direction: isize, cx: &mut Context<Self>) {
        if !self.multiline {
            return;
        }

        let text = self.text(cx);
        let cursor = self.cursor_offset();
        let (line, column) = line_and_column(&text, cursor);
        let target = (line as isize + direction).max(0) as usize;
        let lines: Vec<_> = text.split('\n').collect();
        if target >= lines.len() {
            self.move_to(text.len(), cx);
            return;
        }
        let start = nth_line_start(&text, target);
        let target_column = clamp_boundary(lines[target], column).min(lines[target].len());
        self.move_to(start + target_column, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
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

    fn newline(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace(self.selected_range.clone(), "\n", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(self.selected_range.clone(), &text, cx);
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

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let bounds =
            selection_hit_bounds(self.is_selecting, self.selection_bounds, self.last_bounds);
        let (Some(bounds), false) = (bounds, self.last_layout.is_empty()) else {
            return 0;
        };
        if position.y <= bounds.top() {
            return 0;
        }
        if position.y >= bounds.bottom() {
            return self.content.len();
        }

        let line_height = bounds.size.height / self.last_layout.len().max(1) as f32;
        let line = ((position.y - bounds.top()) / line_height) as usize;
        let line = line.min(self.last_layout.len() - 1);
        let local_x = position.x - bounds.left();
        let text = self.text_unchecked();
        let offset = nth_line_start(text, line);
        let display_index = self.last_layout[line].closest_index_for_x(local_x);
        let source_index = if self.password {
            password_source_offset(&text[offset..line_end(text, offset)], display_index)
        } else {
            display_index
        };
        clamp_boundary(text, offset + source_index)
    }

    fn text_unchecked(&self) -> &str {
        &self.content
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let index = self.index_for_mouse_position(event.position);
        // Snapshot after resolving the initial click, before the focus and
        // selection notifications can cause another paint pass.
        self.selection_bounds = self.last_bounds;
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(index, cx);
        } else {
            self.move_to(index, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
        self.selection_bounds = None;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Focusing on mouse-down can rerender this element before the matching
        // mouse-up listener clears `is_selecting`. Never let that stale local
        // flag turn an ordinary hover into a selection/caret move: the
        // platform event is the source of truth for whether a drag is active.
        self.is_selecting = mouse_selection_active(self.is_selecting, event.dragging());
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.selection_bounds = None;
        }
    }
}

fn selection_hit_bounds(
    is_selecting: bool,
    selection_bounds: Option<Bounds<Pixels>>,
    last_bounds: Option<Bounds<Pixels>>,
) -> Option<Bounds<Pixels>> {
    if is_selecting {
        selection_bounds.or(last_bounds)
    } else {
        last_bounds
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

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| {
            let text = self.text_unchecked();
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
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let line_height = bounds.size.height / self.last_layout.len().max(1) as f32;
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
                bounds.top() + line_height * start_line as f32,
            ),
            point(
                bounds.left() + end.x_for_index(end_col),
                bounds.top() + line_height * (end_line + 1) as f32,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(utf8_to_utf16(
            &self.text(cx),
            self.index_for_mouse_position(point),
        ))
    }
}

impl Render for TextEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = div()
            .id(gpui::SharedString::from(format!(
                "dbx-text-editor-scroll-{:?}",
                cx.entity().entity_id()
            )))
            .size_full()
            // Keep the scrollable editor from contributing its intrinsic text
            // width to a flex ancestor. The input shell owns the available
            // width; this child may be wider only inside its scroll viewport.
            .min_w_0()
            .track_scroll(&self.scroll_handle)
            .child(TextEditorText {
                editor: cx.entity(),
            });

        if self.multiline {
            editor.overflow_scroll()
        } else {
            // The compact field's content area can be fractionally shorter
            // than one painted line. Never let caret reveal chase that
            // unavoidable vertical clipping; single-line editors only need
            // horizontal scrolling.
            editor.overflow_x_scroll()
        }
    }
}

struct TextEditorText {
    editor: Entity<TextEditor>,
}

struct PrepaintState {
    lines: Vec<ShapedLine>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    /// The child bounds after applying any caret-reveal delta calculated for
    /// this frame. Painting with these bounds keeps the text and scroll
    /// offset in sync instead of waiting for the next frame.
    paint_bounds: Bounds<Pixels>,
    scroll_offset: Option<Point<Pixels>>,
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
        let text = editor.text(cx);
        let display = editor.password.then(|| password_mask(&text));
        let painted_text = display.as_deref().unwrap_or(&text);
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let measured_width = painted_text
            .split('\n')
            .map(|line| {
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
            })
            .max()
            .unwrap_or(px(1.));

        let mut style = Style::default();
        style.size.width = measured_width.max(px(1.)).into();
        style.size.height =
            (window.line_height() * painted_text.split('\n').count().max(1) as f32).into();
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
        let text = editor.text(cx);
        let display = editor.password.then(|| password_mask(&text));
        let painted_text = display.as_deref().unwrap_or(&text);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut lines = Vec::new();
        let mut line_start = 0;
        let sql_tokens = (editor.language == EditorLanguage::Sql).then(|| lex_sql(&text));

        for (line, painted_line) in text.split('\n').zip(painted_text.split('\n')) {
            let marked_range = editor.marked_range.as_ref().and_then(|range| {
                let range = marked_slice(line.len(), line_start, range)?;
                if editor.password {
                    Some(
                        password_display_offset(line, range.start)
                            ..password_display_offset(line, range.end),
                    )
                } else {
                    Some(range)
                }
            });
            let runs = editor_text_runs(
                painted_line,
                0,
                &style,
                editor.language,
                sql_tokens.as_deref().unwrap_or(&[]),
                marked_range.as_ref(),
            );
            lines.push(window.text_system().shape_line(
                painted_line.to_owned().into(),
                font_size,
                &runs,
                None,
            ));
            line_start += line.len() + 1;
        }

        let line_height = window.line_height();
        let cursor = editor.cursor_offset();
        let (cursor_line, cursor_col) = line_and_column(&text, cursor);
        let cursor_position = point(
            lines[cursor_line].x_for_index(if editor.password {
                password_display_offset(&text[nth_line_start(&text, cursor_line)..], cursor_col)
            } else {
                cursor_col
            }),
            line_height * cursor_line as f32,
        );
        let focused = editor.focus_handle.is_focused(window);
        let (paint_bounds, scroll_offset) = if caret_reveal_allowed(focused, editor.is_selecting) {
            let current_offset = editor.scroll_handle.offset();
            let next_offset = scroll_offset_for_cursor(
                current_offset,
                editor.scroll_handle.max_offset(),
                editor.scroll_handle.bounds().size,
                cursor_position,
                size(px(2.), line_height),
                editor.multiline,
            );
            (bounds + (next_offset - current_offset), Some(next_offset))
        } else {
            (bounds, None)
        };
        let cursor_quad = focused.then(|| {
            fill(
                Bounds::new(
                    point(
                        paint_bounds.left() + cursor_position.x,
                        paint_bounds.top() + cursor_position.y,
                    ),
                    size(px(2.), line_height),
                ),
                gpui::blue(),
            )
        });

        let mut selections = Vec::new();
        if !editor.selected_range.is_empty() {
            let (start_line, start_col) = line_and_column(&text, editor.selected_range.start);
            let (end_line, end_col) = line_and_column(&text, editor.selected_range.end);
            for (line, shaped_line) in lines
                .iter()
                .enumerate()
                .skip(start_line)
                .take(end_line - start_line + 1)
            {
                let start = if line == start_line {
                    shaped_line.x_for_index(if editor.password {
                        password_display_offset(&text[nth_line_start(&text, line)..], start_col)
                    } else {
                        start_col
                    })
                } else {
                    px(0.)
                };
                let end = if line == end_line {
                    shaped_line.x_for_index(if editor.password {
                        password_display_offset(&text[nth_line_start(&text, line)..], end_col)
                    } else {
                        end_col
                    })
                } else {
                    shaped_line.x_for_index(shaped_line.text.len())
                };
                selections.push(fill(
                    Bounds::from_corners(
                        point(
                            paint_bounds.left() + start,
                            paint_bounds.top() + line_height * line as f32,
                        ),
                        point(
                            paint_bounds.left() + end,
                            paint_bounds.top() + line_height * (line + 1) as f32,
                        ),
                    ),
                    rgba(0x3311ff30),
                ));
            }
        }

        PrepaintState {
            lines,
            cursor: cursor_quad,
            selections,
            paint_bounds,
            scroll_offset,
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

        for selection in state.selections.drain(..) {
            window.paint_quad(selection);
        }
        for (line, shaped_line) in state.lines.iter().enumerate() {
            shaped_line
                .paint(
                    point(
                        state.paint_bounds.left(),
                        state.paint_bounds.top() + window.line_height() * line as f32,
                    ),
                    window.line_height(),
                    window,
                    cx,
                )
                .unwrap();
        }
        if let Some(cursor) = state.cursor.take() {
            window.paint_quad(cursor);
        }

        self.editor.update(cx, |editor, _| {
            if let Some(scroll_offset) = state.scroll_offset
                && editor.scroll_handle.offset() != scroll_offset
            {
                editor.scroll_handle.set_offset(scroll_offset);
            }
            editor.last_layout = state.lines.clone();
            editor.last_bounds = Some(state.paint_bounds);
        });
    }
}

/// Render an editor with the standard DBX field treatment.
///
/// The `multiline` argument is kept explicit so callers can build an editor
/// before deciding where it is placed, and to make the intended row/editor
/// mode obvious at call sites.  It should match the mode passed to
/// [`TextEditor::new`].
pub fn input(editor: Entity<TextEditor>, focus: FocusHandle, multiline: bool) -> impl IntoElement {
    div()
        .id(gpui::SharedString::from(format!(
            "dbx-text-editor-{:?}",
            editor.entity_id()
        )))
        .key_context(TEXT_EDITOR_CONTEXT)
        .track_focus(&focus)
        // GPUI 0.2.2 invalidates the owning view whenever a cursor-bearing
        // element is entered or left. This shared shell has no hover style,
        // so that invalidation only re-runs intrinsic text layout and makes
        // compact inputs visibly jump under the pointer.
        .w_full()
        .min_w_0()
        .h(if multiline { px(204.) } else { px(32.) })
        .p(if multiline { px(10.) } else { px(7.) })
        .overflow_hidden()
        .bg(THEME.canvas)
        .border_1()
        .border_color(THEME.border_strong)
        .rounded(px(5.))
        .text_size(px(12.))
        .text_color(THEME.text)
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
            move |action: &ShowCharacterPalette, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.show_character_palette(action, window, cx)
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
        .on_mouse_move({
            let editor = editor.clone();
            move |event, window, cx| {
                // Avoid touching the editor entity for ordinary pointer
                // motion. A stale selection flag can still be cleared, but
                // only an active left-button drag may update selection.
                if editor.read(cx).is_selecting {
                    editor.update(cx, |editor, cx| editor.on_mouse_move(event, window, cx));
                }
            }
        })
        .child(editor)
}

fn editor_text_runs(
    line: &str,
    line_start: usize,
    style: &gpui::TextStyle,
    language: EditorLanguage,
    tokens: &[SqlToken],
    marked_range: Option<&Range<usize>>,
) -> Vec<TextRun> {
    let base = style.to_run(line.len());
    let runs = match language {
        EditorLanguage::PlainText => vec![base],
        EditorLanguage::Sql => sql_runs(line, line_start, tokens, &base),
    };
    apply_marked_runs(line.len(), line_start, marked_range, runs)
}

fn sql_runs(line: &str, line_start: usize, tokens: &[SqlToken], base: &TextRun) -> Vec<TextRun> {
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
            color: sql_token_color(token.kind),
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
                    thickness: px(1.),
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

fn sql_token_color(kind: SqlTokenKind) -> gpui::Hsla {
    match kind {
        // DBX's dark editor palette: restrained purple keywords, cyan types,
        // green strings, warm numeric/parameter literals, and blue names.
        SqlTokenKind::Keyword => gpui::rgb(0xc792ea).into(),
        SqlTokenKind::String => gpui::rgb(0xc3e88d).into(),
        SqlTokenKind::Comment => gpui::rgb(0x6b7482).into(),
        SqlTokenKind::Number => gpui::rgb(0xf78c6c).into(),
        SqlTokenKind::Parameter => gpui::rgb(0xffcb6b).into(),
        SqlTokenKind::Identifier => gpui::rgb(0x82aaff).into(),
        SqlTokenKind::Type => gpui::rgb(0x89ddff).into(),
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

fn next_char(chars: &[(usize, char)], index: usize) -> Option<char> {
    chars.get(index + 1).map(|(_, character)| *character)
}

fn byte_end(chars: &[(usize, char)], index: usize, text_len: usize) -> usize {
    chars.get(index).map_or(text_len, |(offset, _)| *offset)
}

fn push_sql_token(tokens: &mut Vec<SqlToken>, kind: SqlTokenKind, start: usize, end: usize) {
    if start < end {
        tokens.push(SqlToken {
            kind,
            range: start..end,
        });
    }
}

fn consume_quoted(chars: &[(usize, char)], mut index: usize, quote: char) -> usize {
    index += 1;
    while index < chars.len() {
        let character = chars[index].1;
        if character == '\\' {
            // Backslash escaping is accepted by SQLite/MySQL and is also a
            // useful tolerant fallback for an unfinished query.
            index = (index + 2).min(chars.len());
            continue;
        }
        if character == quote {
            if next_char(chars, index) == Some(quote) {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }
    index
}

fn consume_dollar_quoted(chars: &[(usize, char)], text: &str, index: usize) -> Option<usize> {
    let mut tag_end = index + 1;
    if chars
        .get(tag_end)
        .is_some_and(|(_, character)| *character != '$')
    {
        let first = chars[tag_end].1;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }
        tag_end += 1;
        while tag_end < chars.len() && chars[tag_end].1 != '$' {
            let character = chars[tag_end].1;
            if !(character == '_' || character.is_ascii_alphanumeric()) {
                return None;
            }
            tag_end += 1;
        }
    }
    if tag_end >= chars.len() || chars[tag_end].1 != '$' {
        return None;
    }

    let delimiter_end = byte_end(chars, tag_end + 1, text.len());
    let delimiter = &text[chars[index].0..delimiter_end];
    let close = text[delimiter_end..].find(delimiter);
    let end_byte = close.map_or(text.len(), |offset| {
        delimiter_end + offset + delimiter.len()
    });
    Some(char_index_at_or_after(chars, end_byte))
}

fn consume_parameter(chars: &[(usize, char)], index: usize) -> Option<usize> {
    let character = chars.get(index)?.1;
    if character == '?' {
        return Some(index + 1);
    }

    let next = next_char(chars, index)?;
    if character == '$' && !next.is_ascii_digit() && !is_identifier_start(next) {
        return None;
    }
    if character != '$' && character != ':' && character != '@' {
        return None;
    }
    if !(next.is_ascii_alphanumeric() || next == '_') {
        return None;
    }

    let mut end = index + 2;
    while end < chars.len() {
        let character = chars[end].1;
        if !(character.is_ascii_alphanumeric() || character == '_') {
            break;
        }
        end += 1;
    }
    Some(end)
}

fn consume_number(chars: &[(usize, char)], mut index: usize) -> usize {
    if chars[index].1 == '0' && matches!(next_char(chars, index), Some('x' | 'X')) {
        index += 2;
        while index < chars.len() && (chars[index].1.is_ascii_hexdigit() || chars[index].1 == '_') {
            index += 1;
        }
        return index;
    }

    while index < chars.len() && (chars[index].1.is_ascii_digit() || chars[index].1 == '_') {
        index += 1;
    }
    if chars
        .get(index)
        .is_some_and(|(_, character)| *character == '.')
        && chars
            .get(index + 1)
            .is_some_and(|(_, character)| character.is_ascii_digit())
    {
        index += 1;
        while index < chars.len() && (chars[index].1.is_ascii_digit() || chars[index].1 == '_') {
            index += 1;
        }
    }

    if chars
        .get(index)
        .is_some_and(|(_, character)| *character == 'e' || *character == 'E')
    {
        let mut exponent = index + 1;
        if chars
            .get(exponent)
            .is_some_and(|(_, character)| *character == '+' || *character == '-')
        {
            exponent += 1;
        }
        let digits_start = exponent;
        while exponent < chars.len()
            && (chars[exponent].1.is_ascii_digit() || chars[exponent].1 == '_')
        {
            exponent += 1;
        }
        if exponent > digits_start {
            index = exponent;
        }
    }
    index
}

fn consume_identifier(chars: &[(usize, char)], mut index: usize) -> usize {
    index += 1;
    while index < chars.len() && is_identifier_continue(chars[index].1) {
        index += 1;
    }
    index
}

fn char_index_at_or_after(chars: &[(usize, char)], byte_offset: usize) -> usize {
    chars
        .binary_search_by_key(&byte_offset, |(offset, _)| *offset)
        .unwrap_or_else(|index| index)
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character == '$' || character.is_alphanumeric()
}

fn is_sql_keyword(word: &str) -> bool {
    SQL_KEYWORDS
        .iter()
        .any(|keyword| keyword.eq_ignore_ascii_case(word))
}

fn is_sql_type(word: &str) -> bool {
    SQL_TYPES
        .iter()
        .any(|sql_type| sql_type.eq_ignore_ascii_case(word))
}

const SQL_KEYWORDS: &[&str] = &[
    "ALL",
    "ALTER",
    "ANALYZE",
    "AND",
    "AS",
    "ASC",
    "ATTACH",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASE",
    "CASCADE",
    "CHECK",
    "COLLATE",
    "COMMIT",
    "CONFLICT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "DATABASE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DETACH",
    "DISTINCT",
    "DO",
    "DROP",
    "ELSE",
    "END",
    "ESCAPE",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FOREIGN",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "IF",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "MATCH",
    "NATURAL",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "PRIMARY",
    "PRAGMA",
    "REFERENCES",
    "REINDEX",
    "RELEASE",
    "RENAME",
    "REPLACE",
    "RESTRICT",
    "RETURNING",
    "RIGHT",
    "ROLLBACK",
    "SAVEPOINT",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TO",
    "TRANSACTION",
    "TRIGGER",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VACUUM",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
    "WITHOUT",
    "WRITE",
    "TRUE",
    "FALSE",
];

const SQL_TYPES: &[&str] = &[
    "ARRAY",
    "BIGINT",
    "BIGSERIAL",
    "BLOB",
    "BOOL",
    "BOOLEAN",
    "BYTEA",
    "CHAR",
    "CLOB",
    "DATE",
    "DATETIME",
    "DECIMAL",
    "DOUBLE",
    "ENUM",
    "FLOAT",
    "INTEGER",
    "INT",
    "INT2",
    "INT4",
    "INT8",
    "JSON",
    "JSONB",
    "MEDIUMINT",
    "MONEY",
    "NCHAR",
    "NUMERIC",
    "REAL",
    "SERIAL",
    "SMALLINT",
    "TEXT",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYINT",
    "UUID",
    "VARBINARY",
    "VARCHAR",
    "XML",
];

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
    let mut next = current.clamp(px(0.), maximum.max(px(0.)));
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
    next.clamp(px(0.), maximum.max(px(0.)))
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

    #[test]
    fn grapheme_navigation_never_splits_emoji_or_combining_marks() {
        let text = "d🦀e\u{301}f";
        assert_eq!(previous_boundary(text, text.len()), 8);
        assert_eq!(next_boundary(text, 1), 5);
    }

    #[test]
    fn utf16_conversion_round_trips_astral_characters() {
        let text = "d🦀e\u{301}f";
        for offset in [0, 1, 5, 8, 9] {
            assert_eq!(utf16_to_utf8(text, utf8_to_utf16(text, offset)), offset);
        }
    }

    #[test]
    fn line_navigation_is_line_aware() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_start(text, 6), 4);
        assert_eq!(line_end(text, 6), 7);
        assert_eq!(nth_line_start(text, 2), 8);
    }

    #[test]
    fn ranges_clamp_to_utf8_boundaries() {
        assert_eq!(clamp_range("🦀", 1..3), 0..0);
    }

    #[test]
    fn single_line_values_replace_newlines_without_changing_other_text() {
        assert_eq!(normalize_value("a\nb\r\nc", false), "a b  c");
        assert_eq!(normalize_value("a\nb", true), "a\nb");
    }

    #[test]
    fn password_mask_hides_source_text_and_preserves_offset_mapping() {
        let text = "a🦀e\u{301}\nb";
        let masked = password_mask(text);

        assert_eq!(masked, "••••\n•");
        assert!(!masked.contains('a'));
        assert!(!masked.contains('🦀'));
        for offset in [0, 1, 5, 6, 8, 9] {
            let display = password_display_offset(text, offset);
            assert_eq!(password_source_offset(text, display), offset);
        }
    }

    #[test]
    fn selection_replacement_preserves_unicode_boundaries() {
        let text = "d🦀f";
        assert_eq!(replace_selection(text, 1..5, ""), "df");
        assert_eq!(replace_selection("title", 5..5, "df"), "titledf");
    }

    #[test]
    fn marked_text_is_clipped_to_its_line() {
        assert_eq!(marked_slice(5, 10, &(12..14)), Some(2..4));
        assert_eq!(marked_slice(5, 10, &(15..18)), None);
    }

    #[test]
    fn reveal_offset_moves_only_far_enough_to_show_the_caret() {
        assert_eq!(
            reveal_offset(px(0.), px(200.), px(100.), px(120.), px(122.)),
            px(22.)
        );
        assert_eq!(
            reveal_offset(px(50.), px(200.), px(100.), px(20.), px(22.)),
            px(20.)
        );
        assert_eq!(
            reveal_offset(px(50.), px(200.), px(100.), px(80.), px(82.)),
            px(50.)
        );
        assert_eq!(
            reveal_offset(px(0.), px(20.), px(100.), px(120.), px(122.)),
            px(20.)
        );
    }

    #[test]
    fn scroll_offset_for_cursor_returns_the_offset_used_for_painting() {
        assert_eq!(
            scroll_offset_for_cursor(
                point(px(-20.), px(-5.)),
                size(px(200.), px(100.)),
                size(px(100.), px(40.)),
                point(px(120.), px(20.)),
                size(px(2.), px(18.)),
                true,
            ),
            point(px(-22.), px(-5.))
        );
    }

    #[test]
    fn scroll_offset_for_cursor_preserves_a_visible_caret() {
        let current = point(px(-20.), px(-5.));
        assert_eq!(
            scroll_offset_for_cursor(
                current,
                size(px(200.), px(100.)),
                size(px(100.), px(40.)),
                point(px(40.), px(20.)),
                size(px(2.), px(18.)),
                true,
            ),
            current
        );
    }

    #[test]
    fn single_line_caret_reveal_never_changes_the_vertical_offset() {
        let current = point(px(-20.), px(-2.));
        let next = scroll_offset_for_cursor(
            current,
            size(px(200.), px(2.)),
            size(px(100.), px(16.)),
            point(px(40.), px(0.)),
            size(px(2.), px(18.)),
            false,
        );

        assert_eq!(next, current);
        assert_eq!(
            scroll_offset_for_cursor(
                next,
                size(px(200.), px(2.)),
                size(px(100.), px(16.)),
                point(px(40.), px(0.)),
                size(px(2.), px(18.)),
                false,
            ),
            next
        );
    }

    #[test]
    fn oversized_caret_reveal_keeps_the_current_offset_stable() {
        let current = point(px(0.), px(-2.));
        let next = scroll_offset_for_cursor(
            current,
            size(px(0.), px(20.)),
            size(px(100.), px(16.)),
            point(px(0.), px(0.)),
            size(px(2.), px(18.)),
            true,
        );

        assert_eq!(next, current);
        assert_eq!(
            scroll_offset_for_cursor(
                next,
                size(px(0.), px(20.)),
                size(px(100.), px(16.)),
                point(px(0.), px(0.)),
                size(px(2.), px(18.)),
                true,
            ),
            next
        );
    }

    #[test]
    fn focused_hover_clears_stale_mouse_selection_without_moving_the_caret() {
        assert!(!mouse_selection_active(true, false));
        assert!(!mouse_selection_active(false, true));
        assert!(mouse_selection_active(true, true));
    }

    #[test]
    fn drag_selection_does_not_move_the_scroll_anchor() {
        assert!(!caret_reveal_allowed(true, true));
        assert!(caret_reveal_allowed(true, false));
        assert!(!caret_reveal_allowed(false, false));
    }

    #[test]
    fn drag_selection_uses_the_bounds_from_mouse_down() {
        let initial = Bounds::new(point(px(12.), px(20.)), size(px(160.), px(18.)));
        let repainted = Bounds::new(point(px(-28.), px(20.)), size(px(160.), px(18.)));

        assert_eq!(
            selection_hit_bounds(true, Some(initial), Some(repainted)),
            Some(initial)
        );
        assert_eq!(
            selection_hit_bounds(false, Some(initial), Some(repainted)),
            Some(repainted)
        );
        assert_eq!(
            selection_hit_bounds(true, None, Some(repainted)),
            Some(repainted)
        );
    }

    #[test]
    fn editor_bindings_are_scoped_to_the_text_editor_context() {
        let bindings = default_key_bindings();
        assert_eq!(bindings.len(), 20);
        assert!(bindings.iter().all(|binding| binding.predicate().is_some()));
        assert_eq!(TEXT_INPUT_CONTEXT, TEXT_EDITOR_CONTEXT);
    }

    #[test]
    fn sql_lexer_covers_common_query_tokens() {
        let text = "SELECT id, amount::NUMERIC, 'O''Reilly', 12.50, $1, :name\n-- note\n/* multi\ncomment */ FROM public.users WHERE active = TRUE";
        let tokens = lex_sql(text);
        let values: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect();

        assert_eq!(
            values,
            vec![
                (SqlTokenKind::Keyword, "SELECT"),
                (SqlTokenKind::Identifier, "id"),
                (SqlTokenKind::Identifier, "amount"),
                (SqlTokenKind::Type, "NUMERIC"),
                (SqlTokenKind::String, "'O''Reilly'"),
                (SqlTokenKind::Number, "12.50"),
                (SqlTokenKind::Parameter, "$1"),
                (SqlTokenKind::Parameter, ":name"),
                (SqlTokenKind::Comment, "-- note"),
                (SqlTokenKind::Comment, "/* multi\ncomment */"),
                (SqlTokenKind::Keyword, "FROM"),
                (SqlTokenKind::Identifier, "public"),
                (SqlTokenKind::Identifier, "users"),
                (SqlTokenKind::Keyword, "WHERE"),
                (SqlTokenKind::Identifier, "active"),
                (SqlTokenKind::Keyword, "TRUE"),
            ]
        );
    }

    #[test]
    fn sql_lexer_is_unicode_safe_and_multiline_aware() {
        let text = "SELECT café FROM \"таблица\"\nWHERE city = 'Łódź\n第二行' AND score >= .5e+1";
        let tokens = lex_sql(text);

        assert!(tokens.iter().any(|token| {
            token.kind == SqlTokenKind::Identifier && &text[token.range.clone()] == "café"
        }));
        assert!(tokens.iter().any(|token| {
            token.kind == SqlTokenKind::Identifier && &text[token.range.clone()] == "\"таблица\""
        }));
        assert!(tokens.iter().any(|token| {
            token.kind == SqlTokenKind::String && &text[token.range.clone()] == "'Łódź\n第二行'"
        }));
        assert!(tokens.iter().any(|token| {
            token.kind == SqlTokenKind::Number && &text[token.range.clone()] == ".5e+1"
        }));
        assert!(
            tokens
                .iter()
                .all(|token| text.is_char_boundary(token.range.start)
                    && text.is_char_boundary(token.range.end))
        );
    }

    #[test]
    fn sql_lexer_does_not_parse_inside_strings_or_comments() {
        let text = "'SELECT 1' -- UPDATE users\n/* DELETE FROM users */ SELECT 2";
        let tokens = lex_sql(text);
        let values: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect();

        assert_eq!(
            values,
            vec![
                (SqlTokenKind::String, "'SELECT 1'"),
                (SqlTokenKind::Comment, "-- UPDATE users"),
                (SqlTokenKind::Comment, "/* DELETE FROM users */"),
                (SqlTokenKind::Keyword, "SELECT"),
                (SqlTokenKind::Number, "2"),
            ]
        );
    }

    #[test]
    fn sql_lexer_supports_postgres_dollar_quoted_strings() {
        let text = "SELECT $$BEGIN; SELECT 1; END$$, $body$UPDATE users$body$, $2";
        let tokens = lex_sql(text);
        let values: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect();

        assert_eq!(
            values,
            vec![
                (SqlTokenKind::Keyword, "SELECT"),
                (SqlTokenKind::String, "$$BEGIN; SELECT 1; END$$"),
                (SqlTokenKind::String, "$body$UPDATE users$body$"),
                (SqlTokenKind::Parameter, "$2"),
            ]
        );
    }
}
