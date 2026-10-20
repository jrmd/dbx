//! Modal editing for SQL documents. Text mutation and history stay in TextEditor.
//! Motions use graphemes and UTF-8 ranges, just like the native input path.

use super::*;

pub(super) const NORMAL_CONTEXT: &str = "DbxVimNormal";
pub(super) const INSERT_CONTEXT: &str = "DbxVimInsert";
const MAX_COUNT: usize = 10_000;

#[derive(Clone, PartialEq, serde::Deserialize, gpui::Action)]
#[action(namespace = dbx_vim, no_json)]
pub(super) struct Key {
    key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl VimMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
            Self::VisualLine => "VISUAL LINE",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Operator {
    Delete,
    Change,
    Yank,
}

pub(super) struct VimState {
    pub mode: VimMode,
    pub(super) cursor: usize,
    anchor: usize,
    count: usize,
    operator: Option<(Operator, usize)>,
    g_pending: bool,
    replace_pending: bool,
    register: Option<(String, bool)>,
    column: Option<usize>,
    pub(super) insert_undo_start: usize,
}

impl Default for VimState {
    fn default() -> Self {
        Self {
            mode: VimMode::Normal,
            cursor: 0,
            anchor: 0,
            count: 0,
            operator: None,
            g_pending: false,
            replace_pending: false,
            register: None,
            column: None,
            insert_undo_start: 0,
        }
    }
}

pub(super) fn bindings() -> Vec<KeyBinding> {
    let mut keys = Vec::new();
    // Each key is dispatched immediately. The state machine handles counts and
    // operators without GPUI's chord timeout swallowing ordinary Insert text.
    for key in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
        let key = key.to_string();
        keys.push(KeyBinding::new(
            &key,
            Key { key: key.clone() },
            Some(NORMAL_CONTEXT),
        ));
    }
    for (binding, key) in [
        ("shift-a", "A"),
        ("shift-b", "B"),
        ("shift-c", "C"),
        ("shift-d", "D"),
        ("shift-e", "E"),
        ("shift-g", "G"),
        ("shift-i", "I"),
        ("shift-o", "O"),
        ("shift-p", "P"),
        ("shift-v", "V"),
        ("shift-w", "W"),
        ("shift-x", "X"),
        ("shift-4", "$"),
        ("shift-6", "^"),
        ("left", "h"),
        ("right", "l"),
        ("up", "k"),
        ("down", "j"),
        ("home", "0"),
        ("end", "$"),
        ("space", " "),
        ("backspace", "h"),
        ("delete", "x"),
        ("enter", "j"),
        ("ctrl-r", "redo"),
        ("ctrl-u", "page-up"),
        ("ctrl-d", "page-down"),
        ("tab", "tab"),
        ("shift-tab", "shift-tab"),
    ] {
        keys.push(KeyBinding::new(
            binding,
            Key { key: key.into() },
            Some(NORMAL_CONTEXT),
        ));
    }
    for context in [NORMAL_CONTEXT, INSERT_CONTEXT] {
        for binding in ["escape", "ctrl-["] {
            keys.push(KeyBinding::new(
                binding,
                Key {
                    key: "escape".into(),
                },
                Some(context),
            ));
        }
    }
    keys
}

impl TextEditor {
    pub fn set_vim_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        // Single-line fields and other query languages retain native editing.
        let enabled = enabled && self.multiline && matches!(self.language, EditorLanguage::Sql(_));
        if enabled == self.vim.is_some() {
            return;
        }
        self.vim = enabled.then(VimState::default);
        self.marked_range = None;
        self.typing_run = None;
        if enabled {
            self.move_to(normal_cursor(&self.text(cx), self.cursor_offset()), cx);
        }
        cx.notify();
    }

    pub fn vim_mode(&self) -> Option<VimMode> {
        self.vim.as_ref().map(|vim| vim.mode)
    }

    pub fn accepts_insert_input(&self) -> bool {
        !self.read_only && self.vim_mode().is_none_or(|mode| mode == VimMode::Insert)
    }

    pub(super) fn vim_key(&mut self, action: &Key, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut vim) = self.vim.take() else {
            return;
        };
        vim.execute(&action.key, self, window, cx);
        self.vim = Some(vim);
        self.reveal_requested.set(true);
        cx.notify();
    }

    pub(super) fn vim_unbound_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if self.accepts_insert_input()
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
        {
            return;
        }
        cx.stop_propagation();
        if self.vim.as_ref().is_some_and(|vim| vim.replace_pending) {
            let key = event
                .keystroke
                .key_char
                .as_deref()
                .unwrap_or(&event.keystroke.key);
            self.vim_key(&Key { key: key.into() }, window, cx);
        } else if let Some(vim) = &mut self.vim {
            vim.clear_pending();
        }
    }
}

impl VimState {
    fn clear_pending(&mut self) {
        self.count = 0;
        self.operator = None;
        self.g_pending = false;
        self.replace_pending = false;
    }

    fn visual(&self) -> bool {
        matches!(self.mode, VimMode::Visual | VimMode::VisualLine)
    }

    fn select(&self, editor: &mut TextEditor, cx: &mut Context<TextEditor>) {
        let text = editor.text(cx);
        let range = if self.mode == VimMode::VisualLine {
            line_start(&text, self.anchor.min(self.cursor))
                ..after_line(&text, self.anchor.max(self.cursor))
        } else {
            self.anchor.min(self.cursor)..next_boundary(&text, self.anchor.max(self.cursor))
        };
        editor.select_range(range, cx);
        editor.selection_reversed = self.cursor < self.anchor;
    }

    fn execute(
        &mut self,
        key: &str,
        editor: &mut TextEditor,
        window: &mut Window,
        cx: &mut Context<TextEditor>,
    ) {
        let text = editor.text(cx);
        if !self.visual() {
            self.cursor = normal_cursor(&text, editor.cursor_offset());
        }
        self.cursor = normal_cursor(&text, self.cursor);
        if self.visual() && key == "o" {
            std::mem::swap(&mut self.cursor, &mut self.anchor);
            self.select(editor, cx);
            return;
        }
        if key == "escape" {
            if self.mode == VimMode::Insert {
                let cursor = editor.cursor_offset();
                self.cursor = if cursor > line_start(&text, cursor) {
                    previous_boundary(&text, cursor)
                } else {
                    cursor
                };
            }
            self.mode = VimMode::Normal;
            self.clear_pending();
            self.column = None;
            editor.marked_range = None;
            editor.typing_run = None;
            editor.move_to(normal_cursor(&text, self.cursor), cx);
            return;
        }
        if key == "tab" || key == "shift-tab" {
            self.clear_pending();
            if key == "tab" {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
            return;
        }
        if self.mode == VimMode::Insert {
            return;
        }
        if self.replace_pending {
            self.replace_pending = false;
            if key.graphemes(true).count() == 1 && !text.is_empty() {
                let count = self.count.max(1);
                let end = advance_in_line(&text, self.cursor, count);
                editor.replace(
                    self.cursor..end,
                    &key.repeat(count.min(text[self.cursor..end].graphemes(true).count())),
                    cx,
                );
                editor.move_to(self.cursor, cx);
            }
            self.clear_pending();
            return;
        }
        if let Ok(digit) = key.parse::<usize>()
            && key.len() == 1
            && (digit != 0 || self.count > 0)
        {
            self.count = (self.count.saturating_mul(10) + digit).min(MAX_COUNT);
            return;
        }
        let explicit_count = self.count;
        let count = self.count.max(1);
        if key == "g" && !self.g_pending {
            self.g_pending = true;
            return;
        }
        let key = if self.g_pending && key == "g" {
            "gg"
        } else {
            key
        };
        if self.g_pending && key != "gg" {
            self.clear_pending();
            return;
        }
        self.g_pending = false;

        let op = match key {
            "d" => Some(Operator::Delete),
            "c" => Some(Operator::Change),
            "y" => Some(Operator::Yank),
            _ => None,
        };
        if let Some(op) = op {
            if self.visual() {
                self.operate(
                    op,
                    editor.selected_range.clone(),
                    self.mode == VimMode::VisualLine,
                    editor,
                    cx,
                );
            } else if let Some((pending, before)) = self.operator {
                if pending == op {
                    let range = lines_range(
                        &text,
                        self.cursor,
                        count.saturating_mul(before).min(MAX_COUNT),
                    );
                    self.operate(op, range, true, editor, cx);
                }
                self.clear_pending();
            } else {
                self.operator = Some((op, count));
                self.count = 0;
            }
            return;
        }

        let total = self.operator.map_or(count, |(_, before)| {
            count.saturating_mul(before).min(MAX_COUNT)
        });
        let motion_key = if self.operator.is_some_and(|(op, _)| op == Operator::Change)
            && matches!(key, "w" | "W")
            && text[self.cursor..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
        {
            if key == "W" { "E" } else { "e" }
        } else {
            key
        };
        if let Some((to, inclusive, linewise)) = motion(
            &text,
            self.cursor,
            motion_key,
            total,
            explicit_count,
            &mut self.column,
        ) {
            if let Some((op, _)) = self.operator {
                let mut range = if linewise {
                    line_start(&text, self.cursor.min(to))..after_line(&text, self.cursor.max(to))
                } else {
                    self.cursor.min(to)..if inclusive {
                        next_boundary(&text, self.cursor.max(to))
                    } else {
                        self.cursor.max(to)
                    }
                };
                // An operator's final w/W stops at the last word's end when
                // the next word begins on another line.
                if matches!(motion_key, "w" | "W") && range.end > range.start {
                    let last = text[range.clone()]
                        .grapheme_indices(true)
                        .rev()
                        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace));
                    if let Some((offset, grapheme)) = last {
                        let end = range.start + offset + grapheme.len();
                        if text[end..range.end].contains('\n') {
                            range.end = end;
                        }
                    }
                }
                self.operate(op, range, linewise, editor, cx);
            } else {
                self.cursor = normal_cursor(&text, to);
                if self.visual() {
                    self.select(editor, cx);
                } else {
                    editor.move_to(self.cursor, cx);
                }
            }
            self.clear_pending();
            return;
        }
        if self.operator.is_some() {
            self.clear_pending();
            return;
        }
        self.count = 0;
        self.column = None;
        match key {
            "i" | "a" | "I" | "A" => {
                self.insert_undo_start = editor.undo_history.len();
                let to = match key {
                    "a" => next_boundary(&text, self.cursor).min(line_end(&text, self.cursor)),
                    "I" => first_nonblank(&text, self.cursor),
                    "A" => line_end(&text, self.cursor),
                    _ => self.cursor,
                };
                self.mode = VimMode::Insert;
                editor.move_to(to, cx);
                editor.typing_run = None;
            }
            "o" | "O" => {
                self.insert_undo_start = editor.undo_history.len();
                let start = line_start(&text, self.cursor);
                let indent = &text[start..first_nonblank(&text, self.cursor)];
                let (at, inserted, cursor) = if key == "o" {
                    let at = line_end(&text, self.cursor);
                    (at, format!("\n{indent}"), at + 1 + indent.len())
                } else {
                    (start, format!("{indent}\n"), start + indent.len())
                };
                editor.replace(at..at, &inserted, cx);
                editor.move_to(cursor, cx);
                self.mode = VimMode::Insert;
            }
            "v" | "V" => {
                let mode = if key == "v" {
                    VimMode::Visual
                } else {
                    VimMode::VisualLine
                };
                if self.mode == mode {
                    self.mode = VimMode::Normal;
                    editor.move_to(self.cursor, cx);
                } else {
                    if !self.visual() {
                        self.anchor = self.cursor;
                    }
                    self.mode = mode;
                    self.select(editor, cx);
                }
            }
            "x" | "X" | "D" | "C" => {
                let range = if self.visual() {
                    editor.selected_range.clone()
                } else if key == "X" {
                    let mut start = self.cursor;
                    for _ in 0..count {
                        start = previous_boundary(&text, start).max(line_start(&text, self.cursor));
                    }
                    start..self.cursor
                } else if matches!(key, "D" | "C") {
                    self.cursor..line_end(&text, self.cursor)
                } else {
                    self.cursor..advance_in_line(&text, self.cursor, count)
                };
                self.operate(
                    if key == "C" {
                        Operator::Change
                    } else {
                        Operator::Delete
                    },
                    range,
                    self.mode == VimMode::VisualLine,
                    editor,
                    cx,
                );
            }
            "p" | "P" => {
                if let Some((value, linewise)) = self.register.clone().or_else(|| {
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .map(|text| {
                            let linewise = text.ends_with('\n');
                            (text, linewise)
                        })
                }) {
                    if self.visual() {
                        let range = editor.selected_range.clone();
                        editor.replace(range.clone(), &value.repeat(count), cx);
                        self.cursor = range.start;
                    } else if linewise {
                        let at = if key == "P" {
                            line_start(&text, self.cursor)
                        } else {
                            after_line(&text, self.cursor)
                        };
                        let value = value.repeat(count);
                        let inserted = if at == text.len()
                            && !text.ends_with('\n')
                            && !text.is_empty()
                            && key == "p"
                        {
                            format!("\n{}", value.strip_suffix('\n').unwrap_or(&value))
                        } else {
                            value
                        };
                        editor.replace(at..at, &inserted, cx);
                        self.cursor = if inserted.starts_with('\n') {
                            at + 1
                        } else {
                            at
                        };
                    } else {
                        let at = if key == "P" {
                            self.cursor
                        } else {
                            next_boundary(&text, self.cursor).min(line_end(&text, self.cursor))
                        };
                        let value = value.repeat(count);
                        editor.replace(at..at, &value, cx);
                        self.cursor = previous_boundary(&editor.text(cx), at + value.len());
                    }
                    self.mode = VimMode::Normal;
                    editor.move_to(normal_cursor(&editor.text(cx), self.cursor), cx);
                }
            }
            "u" | "redo" => {
                for _ in 0..count {
                    if key == "u" {
                        editor.undo(cx);
                    } else {
                        editor.redo(cx);
                    }
                }
                self.mode = VimMode::Normal;
                editor.move_to(normal_cursor(&editor.text(cx), editor.cursor_offset()), cx);
            }
            "r" => {
                self.replace_pending = true;
                self.count = count;
            }
            _ => self.clear_pending(),
        }
    }

    fn operate(
        &mut self,
        op: Operator,
        mut range: Range<usize>,
        linewise: bool,
        editor: &mut TextEditor,
        cx: &mut Context<TextEditor>,
    ) {
        let text = editor.text(cx);
        let range_original = clamp_range(&text, range.clone());
        range = range_original.clone();
        let mut copied = text[range.clone()].to_owned();
        if linewise && !copied.ends_with('\n') {
            copied.push('\n');
        }
        if !copied.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
            self.register = Some((copied, linewise));
        }
        if op == Operator::Yank {
            self.mode = VimMode::Normal;
            editor.move_to(normal_cursor(&text, range.start), cx);
        } else {
            if op == Operator::Change {
                self.insert_undo_start = editor.undo_history.len();
            }
            if op == Operator::Delete
                && linewise
                && range.end == text.len()
                && range.start > 0
                && !text[range.clone()].ends_with('\n')
            {
                range.start -= 1;
            }
            let replacement = if op == Operator::Change && linewise {
                let indent_end = first_nonblank(&text, range.start).min(range.end);
                format!(
                    "{}{}",
                    &text[range.start..indent_end],
                    if text[range.clone()].ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    }
                )
            } else {
                String::new()
            };
            editor.replace(range.clone(), &replacement, cx);
            self.mode = if op == Operator::Change {
                VimMode::Insert
            } else {
                VimMode::Normal
            };
            let cursor = if op == Operator::Change {
                range.start + replacement.trim_end_matches('\n').len()
            } else {
                normal_cursor(&editor.text(cx), range.start)
            };
            editor.move_to(cursor, cx);
        }
        self.cursor = editor.cursor_offset();
        self.column = None;
        self.clear_pending();
    }
}

fn normal_cursor(text: &str, offset: usize) -> usize {
    let offset = clamp_boundary(text, offset);
    let end = line_end(text, offset);
    if offset == end && offset > line_start(text, offset) {
        previous_boundary(text, offset)
    } else {
        offset
    }
}

fn after_line(text: &str, cursor: usize) -> usize {
    let end = line_end(text, cursor);
    if end < text.len() { end + 1 } else { end }
}

fn first_nonblank(text: &str, cursor: usize) -> usize {
    let start = line_start(text, cursor);
    let end = line_end(text, cursor);
    start
        + text[start..end]
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(end - start)
}

fn lines_range(text: &str, cursor: usize, count: usize) -> Range<usize> {
    let start = line_start(text, cursor);
    let mut end = start;
    for _ in 0..count {
        end = after_line(text, end);
    }
    start..end
}

fn advance_in_line(text: &str, cursor: usize, count: usize) -> usize {
    let end = line_end(text, cursor);
    let mut to = cursor;
    for _ in 0..count {
        to = next_boundary(text, to).min(end);
    }
    to
}

fn class(grapheme: &str, big: bool) -> u8 {
    if grapheme.chars().all(char::is_whitespace) {
        0
    } else if big || grapheme.chars().any(|c| c.is_alphanumeric() || c == '_') {
        1
    } else {
        2
    }
}

fn word_motion(text: &str, cursor: usize, key: &str, count: usize) -> usize {
    let parts: Vec<_> = text.grapheme_indices(true).collect();
    if parts.is_empty() {
        return 0;
    }
    let mut ix = parts
        .partition_point(|(offset, _)| *offset < cursor)
        .min(parts.len() - 1);
    let big = matches!(key, "W" | "B" | "E");
    for _ in 0..count {
        match key {
            "w" | "W" => {
                let initial = class(parts[ix].1, big);
                if initial != 0 {
                    while ix < parts.len() && class(parts[ix].1, big) == initial {
                        ix += 1;
                    }
                }
                while ix < parts.len() && class(parts[ix].1, big) == 0 {
                    ix += 1;
                }
                if ix == parts.len() {
                    return text.len();
                }
            }
            "b" | "B" => {
                ix = ix.saturating_sub(1);
                while ix > 0 && class(parts[ix].1, big) == 0 {
                    ix -= 1;
                }
                let initial = class(parts[ix].1, big);
                while ix > 0 && class(parts[ix - 1].1, big) == initial {
                    ix -= 1;
                }
            }
            _ => {
                ix = (ix + 1).min(parts.len() - 1);
                while ix + 1 < parts.len() && class(parts[ix].1, big) == 0 {
                    ix += 1;
                }
                let initial = class(parts[ix].1, big);
                while ix + 1 < parts.len() && class(parts[ix + 1].1, big) == initial {
                    ix += 1;
                }
            }
        }
    }
    parts[ix].0
}

fn motion(
    text: &str,
    cursor: usize,
    key: &str,
    count: usize,
    explicit: usize,
    column: &mut Option<usize>,
) -> Option<(usize, bool, bool)> {
    let mut to = cursor;
    let mut inclusive = false;
    let mut linewise = false;
    match key {
        "h" => {
            for _ in 0..count {
                to = previous_boundary(text, to).max(line_start(text, cursor));
            }
        }
        "l" | " " => to = advance_in_line(text, cursor, count),
        "0" => to = line_start(text, cursor),
        "^" => to = first_nonblank(text, cursor),
        "$" => {
            to = normal_cursor(text, line_end(text, cursor));
            inclusive = true;
        }
        "w" | "W" | "b" | "B" | "e" | "E" => {
            to = word_motion(text, cursor, key, count);
            inclusive = matches!(key, "e" | "E");
        }
        "j" | "k" | "page-up" | "page-down" => {
            let preferred = *column.get_or_insert_with(|| {
                text[line_start(text, cursor)..cursor]
                    .graphemes(true)
                    .count()
            });
            let (line, _) = line_and_column(text, cursor);
            let down = matches!(key, "j" | "page-down");
            let count = if key.starts_with("page-") {
                count.saturating_mul(20)
            } else {
                count
            };
            let target = if down {
                line.saturating_add(count)
            } else {
                line.saturating_sub(count)
            };
            let last_line = text.bytes().filter(|b| *b == b'\n').count();
            let start = nth_line_start(text, target.min(last_line));
            let end = line_end(text, start);
            to = start
                + text[start..end]
                    .grapheme_indices(true)
                    .nth(preferred)
                    .map_or(end - start, |(ix, _)| ix);
            linewise = true;
        }
        "gg" | "G" => {
            let target = if explicit > 0 {
                explicit - 1
            } else if key == "gg" {
                0
            } else {
                text.bytes().filter(|b| *b == b'\n').count()
            };
            let last_line = text.bytes().filter(|b| *b == b'\n').count();
            to = first_nonblank(text, nth_line_start(text, target.min(last_line)));
            linewise = true;
        }
        _ => return None,
    }
    if !matches!(key, "j" | "k" | "page-up" | "page-down") {
        *column = None;
    }
    Some((
        if matches!(key, "w" | "W" | "l") {
            to
        } else {
            normal_cursor(text, to)
        },
        inclusive,
        linewise,
    ))
}
