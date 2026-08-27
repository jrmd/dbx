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
    Style, Subscription, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, point, prelude::*, px, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::theme;
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
    Redis,
    Json,
}

/// The source used when executing text from an editor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)]
pub enum QueryExecutionScope {
    /// Run a non-empty selection, otherwise the SQL statement around the caret.
    #[default]
    SelectionOrStatement,
    /// Run the whole document exactly as written.
    Document,
    /// Run a non-empty selection, otherwise the line containing the caret.
    /// This is suitable for Redis and other line-oriented command editors.
    SelectionOrCurrentLine,
}

/// Conservative safety classification for SQL execution.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[allow(dead_code)]
pub enum SqlExecutionKind {
    /// A query that is safe to run without modifying database state.
    Read,
    /// A statement that may modify state and should be described honestly.
    MutationRisk,
    /// A broad or irreversible statement that should require confirmation.
    Destructive,
}

/// Return the execution range selected by the user, or the appropriate
/// fallback for the requested scope. All ranges are valid UTF-8 byte ranges.
#[allow(dead_code)]
pub fn execution_range(
    text: &str,
    selection: Range<usize>,
    cursor: usize,
    scope: QueryExecutionScope,
) -> Range<usize> {
    let selection = clamp_range(text, selection);
    if !selection.is_empty() {
        return selection;
    }
    match scope {
        QueryExecutionScope::Document => 0..text.len(),
        QueryExecutionScope::SelectionOrStatement => sql_statement_range(text, cursor),
        QueryExecutionScope::SelectionOrCurrentLine => {
            let cursor = clamp_boundary(text, cursor);
            line_start(text, cursor)..line_end(text, cursor)
        }
    }
}

/// Return the SQL statement surrounding `cursor`, ignoring terminators in
/// quoted strings, comments, and PostgreSQL dollar-quoted bodies.
#[allow(dead_code)]
pub fn sql_statement_range(text: &str, cursor: usize) -> Range<usize> {
    let cursor = clamp_boundary(text, cursor);
    let boundaries = sql_statement_boundaries(text);
    // A caret immediately after a terminator belongs to the next statement;
    // a caret on the terminator itself remains with the preceding statement.
    // If there is no next statement (only trailing whitespace), keep the last
    // executable statement active instead of returning an empty range.
    let end_index = boundaries.partition_point(|boundary| *boundary <= cursor);
    let start = boundaries[end_index.saturating_sub(1)];
    let end = boundaries.get(end_index).copied().unwrap_or(text.len());
    let candidate = start..end;
    if !text[candidate.clone()].trim().is_empty() {
        return candidate;
    }

    boundaries[..end_index]
        .windows(2)
        .rev()
        .map(|pair| pair[0]..pair[1])
        .find(|range| !text[range.clone()].trim().is_empty())
        .unwrap_or(candidate)
}

/// Count non-empty SQL statements while ignoring semicolons in lexical
/// regions such as quoted strings, comments, and dollar-quoted bodies.
#[allow(dead_code)]
pub fn sql_statement_count(text: &str) -> usize {
    sql_statement_ranges(text)
        .into_iter()
        .filter(|range| {
            lex_sql(&text[range.clone()])
                .iter()
                .any(|token| token.kind != SqlTokenKind::Comment)
        })
        .count()
}

/// Classify a SQL script conservatively. Any risky statement determines the
/// script's result; unfamiliar syntax is deliberately treated as mutation
/// risk rather than read-only.
#[allow(dead_code)]
pub fn sql_execution_kind(text: &str) -> SqlExecutionKind {
    sql_statement_ranges(text)
        .into_iter()
        .filter_map(|range| sql_statement_kind(&text[range]))
        .max()
        .unwrap_or(SqlExecutionKind::MutationRisk)
}

/// Whether successful execution may have changed the relational catalogue.
///
/// This intentionally errs on the side of refreshing schema-derived UI. The
/// lexical pass ignores comments, strings, quoted identifiers, and
/// dollar-quoted bodies, so examples or procedure bodies do not spuriously
/// invalidate an open database diagram.
#[allow(dead_code)]
pub fn sql_may_change_schema(text: &str) -> bool {
    sql_statement_ranges(text).into_iter().any(|range| {
        sql_words_with_depth(&text[range]).iter().any(|(word, _)| {
            matches!(
                word.as_str(),
                "CREATE"
                    | "ALTER"
                    | "DROP"
                    | "RENAME"
                    | "ATTACH"
                    | "DETACH"
                    | "DO"
                    | "CALL"
                    | "EXEC"
                    | "EXECUTE"
            )
        })
    })
}

fn sql_statement_ranges(text: &str) -> Vec<Range<usize>> {
    let boundaries = sql_statement_boundaries(text);
    boundaries
        .windows(2)
        .map(|pair| pair[0]..pair[1])
        .filter(|range| !text[range.clone()].trim().is_empty())
        .collect()
}

fn sql_statement_kind(text: &str) -> Option<SqlExecutionKind> {
    let words = sql_words_with_depth(text);
    let top_level_words: Vec<_> = words
        .iter()
        .filter(|(_, depth)| *depth == 0)
        .map(|(word, _)| word.as_str())
        .collect();
    let first = top_level_words.first()?;
    if words
        .iter()
        .any(|(word, _)| matches!(word.as_str(), "DROP" | "TRUNCATE" | "ALTER"))
    {
        return Some(SqlExecutionKind::Destructive);
    }
    // MySQL's REPLACE deletes a conflicting row before inserting; MERGE/CALL
    // and EXEC may run arbitrary write paths. Require confirmation rather
    // than attempting dialect-specific parser completeness here.
    if words.iter().any(|(word, _)| {
        matches!(
            word.as_str(),
            "MERGE" | "REPLACE" | "CALL" | "EXEC" | "EXECUTE"
        )
    }) {
        return Some(SqlExecutionKind::Destructive);
    }
    for (index, (word, depth)) in words.iter().enumerate() {
        if matches!(word.as_str(), "DELETE" | "UPDATE") {
            return Some(
                if words[index + 1..]
                    .iter()
                    .any(|(word, where_depth)| where_depth == depth && word == "WHERE")
                {
                    SqlExecutionKind::MutationRisk
                } else {
                    SqlExecutionKind::Destructive
                },
            );
        }
    }
    if words.iter().any(|(word, _)| word == "INSERT") {
        return Some(SqlExecutionKind::MutationRisk);
    }
    if matches!(*first, "SELECT" | "VALUES")
        || (*first == "WITH"
            && top_level_words
                .iter()
                .skip(1)
                .any(|word| matches!(*word, "SELECT" | "VALUES")))
    {
        return Some(SqlExecutionKind::Read);
    }
    Some(SqlExecutionKind::MutationRisk)
}

/// Return identifier-like tokens with their parenthesis depth. `lex_sql` has
/// already protected strings, comments, and dollar-quoted bodies; the depth
/// pass applies the same protection while tracking only real SQL grouping.
fn sql_words_with_depth(text: &str) -> Vec<(String, usize)> {
    lex_sql(text)
        .into_iter()
        .filter(|token| token.kind != SqlTokenKind::Comment)
        .map(|token| {
            (
                text[token.range.clone()].to_ascii_uppercase(),
                sql_parenthesis_depth_at(text, token.range.start),
            )
        })
        .collect()
}

fn sql_parenthesis_depth_at(text: &str, target: usize) -> usize {
    let bytes = text.as_bytes();
    let target = clamp_boundary(text, target);
    let mut depth = 0usize;
    let mut index = 0;
    while index < target {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == quote {
                        index += 1;
                        if index < bytes.len() && bytes[index] == quote {
                            index += 1;
                        } else {
                            break;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'$' => {
                index = dollar_quoted_end(text, index).unwrap_or(index + 1);
            }
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    depth
}

fn sql_statement_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries = vec![0];
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == quote {
                        index += 1;
                        if index < bytes.len() && bytes[index] == quote {
                            index += 1;
                        } else {
                            break;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'$' => {
                if let Some(end) = dollar_quoted_end(text, index) {
                    index = end;
                } else {
                    index += 1;
                }
            }
            b';' => {
                boundaries.push(index + 1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    if boundaries.last().copied() != Some(text.len()) {
        boundaries.push(text.len());
    }
    boundaries
}

#[allow(dead_code)]
fn dollar_quoted_end(text: &str, start: usize) -> Option<usize> {
    let remainder = &text[start..];
    let tag_end = remainder[1..].find('$')? + 1;
    let delimiter = &remainder[..=tag_end];
    let tag = &delimiter[1..delimiter.len() - 1];
    if !tag.is_empty()
        && !(tag.starts_with(|character: char| character == '_' || character.is_ascii_alphabetic())
            && tag
                .chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric()))
    {
        return None;
    }
    remainder[delimiter.len()..]
        .find(delimiter)
        .map(|offset| start + delimiter.len() + offset + delimiter.len())
        .or(Some(text.len()))
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

/// The lexical categories used by the Redis command editor. Redis input is
/// line-oriented, so the first token on every line is a command and later
/// tokens are classified without requiring a complete or valid command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedisTokenKind {
    Command,
    Option,
    String,
    Number,
    Identifier,
}

/// A UTF-8-safe token returned by [`lex_redis`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisToken {
    pub kind: RedisTokenKind,
    pub range: Range<usize>,
}

/// The lexical categories understood by the built-in JSON highlighter.
///
/// JSON values are lexed, rather than parsed, so a partially typed document
/// remains highlighted and completely editable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonTokenKind {
    Property,
    String,
    Number,
    Boolean,
    Null,
}

/// A token returned by [`lex_json`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonToken {
    pub kind: JsonTokenKind,
    pub range: Range<usize>,
}

#[derive(Clone, Debug)]
struct HighlightToken {
    range: Range<usize>,
    kind: HighlightTokenKind,
}

#[derive(Clone, Copy, Debug)]
enum HighlightTokenKind {
    Sql(SqlTokenKind),
    Redis(RedisTokenKind),
    Json(JsonTokenKind),
}

impl From<SqlToken> for HighlightToken {
    fn from(token: SqlToken) -> Self {
        Self {
            range: token.range,
            kind: HighlightTokenKind::Sql(token.kind),
        }
    }
}

impl From<RedisToken> for HighlightToken {
    fn from(token: RedisToken) -> Self {
        Self {
            range: token.range,
            kind: HighlightTokenKind::Redis(token.kind),
        }
    }
}

impl From<JsonToken> for HighlightToken {
    fn from(token: JsonToken) -> Self {
        Self {
            range: token.range,
            kind: HighlightTokenKind::Json(token.kind),
        }
    }
}

impl HighlightToken {
    fn color(&self) -> gpui::Hsla {
        match self.kind {
            HighlightTokenKind::Sql(kind) => sql_token_color(kind),
            HighlightTokenKind::Redis(kind) => redis_token_color(kind),
            HighlightTokenKind::Json(kind) => json_token_color(kind),
        }
    }
}

/// The part of a SQL statement that a completion menu should search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlCompletionTarget {
    Any,
    Table,
    Column,
}

/// The lexical context used by DBX's schema-aware SQL completion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlCompletionContext {
    pub target: SqlCompletionTarget,
    pub prefix: String,
    pub qualifier: Option<String>,
    /// The quote delimiter immediately before the editable prefix, when the
    /// user is completing inside a quoted identifier.
    pub quote: Option<char>,
    pub replacement_range: Range<usize>,
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

/// Lex Redis command text into UTF-8-safe ranges suitable for highlighting.
/// Quoted and backslash-escaped arguments remain one token even while they are
/// incomplete, matching the command parser's editing-time behavior.
pub fn lex_redis(text: &str) -> Vec<RedisToken> {
    let mut tokens = Vec::new();
    let mut line_start = 0;
    for line_with_ending in text.split_inclusive('\n') {
        let line = line_with_ending
            .strip_suffix('\n')
            .unwrap_or(line_with_ending);
        lex_redis_line(line, line_start, &mut tokens);
        line_start += line_with_ending.len();
    }
    tokens
}

fn lex_redis_line(line: &str, line_start: usize, tokens: &mut Vec<RedisToken>) {
    let mut cursor = 0;
    let mut token_index = 0;
    while cursor < line.len() {
        while let Some(character) = line[cursor..].chars().next()
            && character.is_whitespace()
        {
            cursor += character.len_utf8();
        }
        if cursor == line.len() {
            break;
        }

        let start = cursor;
        let mut end = line.len();
        let mut quote = None;
        let mut escaped = false;
        for (relative, character) in line[start..].char_indices() {
            let index = start + relative;
            if escaped {
                escaped = false;
                continue;
            }
            if character == '\\' {
                escaped = true;
                continue;
            }
            if let Some(expected) = quote {
                if character == expected {
                    quote = None;
                }
                continue;
            }
            if matches!(character, '\'' | '"') {
                quote = Some(character);
            } else if character.is_whitespace() {
                end = index;
                break;
            }
        }

        let raw = &line[start..end];
        tokens.push(RedisToken {
            kind: redis_token_kind(raw, token_index),
            range: line_start + start..line_start + end,
        });
        token_index += 1;
        cursor = end;
    }
}

fn redis_token_kind(raw: &str, token_index: usize) -> RedisTokenKind {
    const OPTIONS: &[&str] = &[
        "AGGREGATE",
        "ASYNC",
        "BLOCK",
        "BYLEX",
        "BYSCORE",
        "CH",
        "COUNT",
        "ENTRIESREAD",
        "EX",
        "EXAT",
        "FREQ",
        "FULL",
        "GET",
        "HARD",
        "IDLETIME",
        "INCR",
        "KEEPTTL",
        "LIMIT",
        "MATCH",
        "MAX",
        "MIN",
        "MKSTREAM",
        "NOACK",
        "NOMKSTREAM",
        "NX",
        "PX",
        "PXAT",
        "RESET",
        "REV",
        "SAMPLES",
        "SOFT",
        "STORE",
        "STREAMS",
        "SUM",
        "SYNC",
        "TYPE",
        "WEIGHTS",
        "WITHSCORES",
        "XX",
    ];

    if token_index == 0 {
        RedisTokenKind::Command
    } else if raw.starts_with(['\'', '"']) {
        RedisTokenKind::String
    } else if raw.parse::<f64>().is_ok() {
        RedisTokenKind::Number
    } else if OPTIONS
        .iter()
        .any(|option| raw.eq_ignore_ascii_case(option))
    {
        RedisTokenKind::Option
    } else {
        RedisTokenKind::Identifier
    }
}

/// Lex JSON into UTF-8-safe ranges suitable for syntax highlighting.
///
/// This deliberately accepts incomplete strings and partially written values:
/// the editor needs useful feedback while a JSON document is still being
/// composed, not only after it is valid. Object keys are recognised by a
/// following colon; punctuation and unknown text retain the base colour.
pub fn lex_json(text: &str) -> Vec<JsonToken> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let start = chars[index].0;
        match chars[index].1 {
            '"' => {
                let end = consume_json_string(&chars, index);
                let end_byte = byte_end(&chars, end, text.len());
                let mut next = end;
                while next < chars.len() && chars[next].1.is_whitespace() {
                    next += 1;
                }
                let kind = if next < chars.len() && chars[next].1 == ':' {
                    JsonTokenKind::Property
                } else {
                    JsonTokenKind::String
                };
                push_json_token(&mut tokens, kind, start, end_byte);
                index = end;
            }
            '-' | '0'..='9' if consume_json_number(&chars, index).is_some() => {
                let end = consume_json_number(&chars, index).unwrap_or(index + 1);
                push_json_token(
                    &mut tokens,
                    JsonTokenKind::Number,
                    start,
                    byte_end(&chars, end, text.len()),
                );
                index = end;
            }
            character if character.is_ascii_alphabetic() => {
                let mut end = index + 1;
                while end < chars.len() && chars[end].1.is_ascii_alphabetic() {
                    end += 1;
                }
                let end_byte = byte_end(&chars, end, text.len());
                let kind = match &text[start..end_byte] {
                    "true" | "false" => Some(JsonTokenKind::Boolean),
                    "null" => Some(JsonTokenKind::Null),
                    _ => None,
                };
                if let Some(kind) = kind {
                    push_json_token(&mut tokens, kind, start, end_byte);
                }
                index = end;
            }
            _ => index += 1,
        }
    }

    tokens
}

/// Return the completion context at a UTF-8 cursor offset.
///
/// This is intentionally a small, tolerant lexer-side context detector rather
/// than a full SQL parser. It is safe while a statement is incomplete and
/// understands the contexts that matter most for a database workbench:
/// tables after `FROM`/`JOIN`/`UPDATE`/`INTO`, columns after projection and
/// predicate keywords, and qualified names after a dot.
pub fn sql_completion_context(text: &str, cursor: usize) -> Option<SqlCompletionContext> {
    let cursor = clamp_boundary(text, cursor);
    if cursor == 0 {
        return None;
    }

    let tokens = lex_sql(text);
    if tokens.iter().any(|token| {
        matches!(token.kind, SqlTokenKind::String | SqlTokenKind::Comment)
            && token.range.start <= cursor
            && cursor <= token.range.end
    }) {
        return None;
    }

    // A cursor immediately after a closed identifier is between tokens, not
    // inside an editable prefix. Do not let the generic path reinterpret its
    // closing delimiter as a fresh opening quote.
    if tokens.iter().any(|token| {
        token.kind == SqlTokenKind::Identifier
            && cursor == token.range.end
            && text[token.range.clone()]
                .chars()
                .next()
                .is_some_and(|character| matches!(character, '"' | '`'))
            && text[token.range.start..]
                .chars()
                .next()
                .is_some_and(|quote| quoted_identifier_is_closed(&text[token.range.clone()], quote))
    }) {
        return None;
    }

    let quoted_identifier = quoted_completion_identifier(text, cursor, &tokens);
    let (start, prefix, quote, replacement_end, qualifier_start) = quoted_identifier
        .map(|quoted| {
            (
                quoted.range.start,
                quoted.prefix,
                Some(quoted.quote),
                quoted.range.end,
                quoted.qualifier_start,
            )
        })
        .unwrap_or_else(|| {
            let start = completion_identifier_start(text, cursor);
            (
                start,
                text[start..cursor].to_owned(),
                text[..start]
                    .chars()
                    .next_back()
                    .filter(|character| matches!(character, '"' | '`')),
                completion_identifier_end(text, cursor),
                start,
            )
        });
    let qualifier = completion_qualifier(text, qualifier_start);
    let previous_position = qualifier.as_ref().map_or_else(
        || quote.map_or(start, |_| qualifier_start),
        |(_, qualifier_start)| *qualifier_start,
    );
    let previous_word = previous_sql_word(text, previous_position);
    let target = if qualifier.is_some() {
        if previous_word
            .as_deref()
            .is_some_and(is_table_completion_keyword)
        {
            SqlCompletionTarget::Table
        } else {
            SqlCompletionTarget::Column
        }
    } else if previous_word
        .as_deref()
        .is_some_and(is_table_completion_keyword)
    {
        SqlCompletionTarget::Table
    } else if previous_word
        .as_deref()
        .is_some_and(is_column_completion_keyword)
    {
        SqlCompletionTarget::Column
    } else {
        SqlCompletionTarget::Any
    };

    let after_list_separator = text[..start]
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|character| matches!(character, '(' | ','));
    if prefix.is_empty() && qualifier.is_none() && previous_word.is_none() && !after_list_separator
    {
        return None;
    }

    Some(SqlCompletionContext {
        target,
        prefix,
        qualifier: qualifier.map(|(qualifier, _)| qualifier),
        quote,
        replacement_range: start..replacement_end,
    })
}

/// The keyword vocabulary used by the syntax highlighter and completion menu.
pub fn sql_completion_keywords() -> &'static [&'static str] {
    SQL_KEYWORDS
}

/// The common SQL type vocabulary used for DDL completion.
pub fn sql_completion_types() -> &'static [&'static str] {
    SQL_TYPES
}

/// The common built-in function vocabulary used for completion.
pub fn sql_completion_functions() -> &'static [&'static str] {
    SQL_FUNCTIONS
}

/// Locate the part of `query` that a database error message is most likely
/// pointing at.
///
/// Drivers surface the same failure in dialect-specific prose, so this uses a
/// chain of tolerant strategies instead of one strict grammar:
///
/// 1. PostgreSQL's trailing `POSITION: n` marker.
/// 2. A quoted token after `near` / `at or near` (PostgreSQL syntax errors,
///    SQLite `near "x": syntax error`).
/// 3. MySQL's `near 'fragment' at line n`, matching the longest exact prefix
///    of the fragment because MySQL quotes the *remainder* of the statement.
/// 4. Missing-column phrasings (`column "x" does not exist`,
///    `Unknown column 'x'`).
///
/// Returns a UTF-8 byte range into `query`, expanded to whole identifier
/// boundaries, or `None` when nothing in the message can be located. The
/// result is advisory: it only drives an underline in the query editor.
pub fn sql_error_range(message: &str, query: &str) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }

    let lowered = message.to_ascii_lowercase();

    if let Some(range) = sql_error_position_range(&lowered, query) {
        return Some(range);
    }
    if let Some(needle) = quoted_after(&lowered, &["at or near", "near"]) {
        if let Some(range) = find_word_in_query(query, &needle) {
            return Some(range);
        }
        // MySQL quotes the remaining text rather than the offending token;
        // its first word usually is that token.
        if let Some(first) = needle.split_whitespace().next()
            && first.len() >= 2
            && let Some(range) = find_word_in_query(query, first)
        {
            return Some(range);
        }
    }
    if let Some(needle) = missing_column_name(&lowered)
        && let Some(range) = find_word_in_query(query, &needle)
    {
        return Some(range);
    }
    // Last resort: any quoted identifier-ish snippet that actually appears
    // in the query (`relation "x" does not exist`, duplicate-key values,
    // driver-specific phrasings).
    for needle in quoted_snippets(&lowered) {
        if let Some(range) = find_word_in_query(query, &needle) {
            return Some(range);
        }
    }
    None
}

/// Every `"…"` / `'…'` snippet in a lowercased message that plausibly names a
/// database object (letters, digits, `_ $ .` only).
fn quoted_snippets(lowered_message: &str) -> Vec<String> {
    let mut snippets = Vec::new();
    let bytes = lowered_message.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !matches!(bytes[index], b'"' | b'\'') {
            index += 1;
            continue;
        }
        let quote = bytes[index];
        let Some(close) = lowered_message[index + 1..].find(quote as char) else {
            break;
        };
        let inner = &lowered_message[index + 1..index + 1 + close];
        if !inner.is_empty()
            && inner.chars().all(|character| {
                character.is_alphanumeric() || matches!(character, '_' | '$' | '.')
            })
        {
            snippets.push(inner.to_owned());
        }
        index += inner.len() + 2;
    }
    snippets
}

fn sql_error_position_range(lowered_message: &str, query: &str) -> Option<Range<usize>> {
    let marker = lowered_message.find("position")?;
    let digits = lowered_message[marker + "position".len()..]
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    // PostgreSQL positions are 1-based character offsets.
    let position: usize = digits.parse().ok()?;
    let position = position.checked_sub(1)?;
    let offset = clamp_boundary(
        query,
        query
            .char_indices()
            .nth(position)
            .map_or(query.len(), |(offset, _)| offset),
    );
    expand_to_identifier(query, offset)
}
fn quoted_after(lowered_message: &str, markers: &[&str]) -> Option<String> {
    let quote = ['"', '\''];
    for marker in markers {
        let Some(start) = lowered_message.find(marker) else {
            continue;
        };
        let tail = &lowered_message[start + marker.len()..];
        let quote_index = tail.find(quote)?;
        let open = tail[quote_index..].chars().next()?;
        let rest = &tail[quote_index + open.len_utf8()..];
        let end = rest.find(open)?;
        let inner = rest[..end].trim();
        // Skip empty and degenerate quotes; they carry no location signal.
        if !inner.is_empty()
            && inner.chars().all(|character| {
                character.is_alphanumeric()
                    || character == '_'
                    || character == '$'
                    || character == '.'
                    || character == ' '
            })
        {
            return Some(inner.to_owned());
        }
        return None;
    }
    None
}

fn missing_column_name(lowered_message: &str) -> Option<String> {
    for (marker, terminator) in [
        ("column \"", '"'),
        ("unknown column '", '\''),
        ("column '", '\''),
        ("no such column: ", ' '),
    ] {
        let Some(start) = lowered_message.find(marker) else {
            continue;
        };
        let rest = &lowered_message[start + marker.len()..];
        let end = rest
            .find(terminator)
            .unwrap_or_else(|| rest.find(['\n', ',']).unwrap_or(rest.len()));
        let name = rest[..end].trim();
        let name = name.rsplit('.').next().unwrap_or(name);
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}

/// Case-insensitively locate `needle` as a standalone word inside `query` and
/// return its range expanded to full identifier boundaries.
fn find_word_in_query(query: &str, needle: &str) -> Option<Range<usize>> {
    let lowered = query.to_ascii_lowercase();
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    let mut search_from = 0;
    while let Some(found) = lowered[search_from..].find(needle) {
        let start = search_from + found;
        let end = start + needle.len();
        let bounded_before = lowered[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_identifier_continue(character));
        let bounded_after = lowered[end..]
            .chars()
            .next()
            .is_none_or(|character| !is_identifier_continue(character));
        if bounded_before && bounded_after {
            return expand_to_identifier(query, start);
        }
        search_from = start + needle.len().max(1);
        if search_from >= lowered.len() {
            break;
        }
    }
    // Fall back to any occurrence when the word never appears standalone
    // (for example `users.id` reported as `id`).
    lowered
        .find(needle)
        .and_then(|start| expand_to_identifier(query, start))
}

/// Expand the offset to full identifier boundaries. Returns `None` when the
/// offset does not sit inside an identifier-like run.
fn expand_to_identifier(text: &str, at: usize) -> Option<Range<usize>> {
    let at = clamp_boundary(text, at);
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, character)| is_identifier_continue(*character))
        .map(|(index, _)| index)
        .last()
        .unwrap_or(at);
    let end = text[at..]
        .char_indices()
        .take_while(|(_, character)| is_identifier_continue(*character))
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    (end > 0).then_some(start..at + end)
}

/// Format a SQL string with DBX's tolerant pretty-printer.
///
/// The formatter is token-driven on top of [`lex_sql`], so it is safe on
/// incomplete statements and never rewrites strings, comments, parameters, or
/// identifier casing. It uppercases keywords and types, breaks major clauses
/// onto indented lines, puts top-level projection/VALUES list items on their
/// own lines, and separates statements with a blank line.
pub fn format_sql(text: &str) -> String {
    SqlFormatter::new(text).run()
}

/// Format `text` and map `cursor` to the equivalent offset in the output.
///
/// The formatter preserves every token exactly once and in order, so offsets
/// are remapped by aligning the two lexings instead of diffing strings.
pub fn format_sql_at_cursor(text: &str, cursor: usize) -> (String, usize) {
    let formatted = format_sql(text);
    let old_tokens = lex_sql(text);
    let new_tokens = lex_sql(&formatted);
    let cursor = clamp_boundary(text, cursor);

    let mapped = match old_tokens
        .iter()
        .position(|token| token.range.start <= cursor && cursor <= token.range.end)
    {
        Some(index) => {
            let delta = cursor - old_tokens[index].range.start;
            new_tokens
                .get(index)
                .map(|token| token.range.start + delta.min(token.range.len()))
        }
        None => old_tokens
            .iter()
            .position(|token| token.range.start >= cursor)
            .and_then(|index| new_tokens.get(index))
            .map(|token| token.range.start),
    }
    .unwrap_or(formatted.len());
    let mapped = clamp_boundary(&formatted, mapped);

    (formatted, mapped)
}

#[derive(Clone, Copy, PartialEq)]
enum AtomKind<'a> {
    Token(SqlTokenKind),
    /// A punctuation atom. Grouping characters (`(` `)` `,` `;` `.`) stand
    /// alone; every other punctuation run merges into one operator slice so
    /// `::`, `<=`, and `||` survive formatting intact.
    Punct(&'a str),
}

enum ClauseBreak {
    /// Break onto a new line at the enclosing indent.
    Clause,
    /// Break with one extra indent level (AND/OR predicates).
    Predicate,
}

/// Split the whitespace-only gap between tokens into punctuation atoms,
/// keeping operator runs together.
fn push_gap_atoms<'a>(atoms: &mut Vec<(AtomKind<'a>, &'a str)>, gap: &'a str) {
    const GROUPING: [char; 5] = ['(', ')', ',', ';', '.'];
    let mut iter = gap.char_indices().peekable();
    while let Some((start, character)) = iter.next() {
        if character.is_whitespace() {
            continue;
        }
        if GROUPING.contains(&character) {
            atoms.push((
                AtomKind::Punct(&gap[start..start + character.len_utf8()]),
                "",
            ));
            continue;
        }
        let mut end = start + character.len_utf8();
        while let Some(&(next, more)) = iter.peek() {
            if more.is_whitespace() || GROUPING.contains(&more) {
                break;
            }
            end = next + more.len_utf8();
            iter.next();
        }
        atoms.push((AtomKind::Punct(&gap[start..end]), ""));
    }
}

struct SqlFormatter<'a> {
    atoms: Vec<(AtomKind<'a>, &'a str)>,
    out: String,
    line: String,
    /// One entry per unclosed parenthesis; `true` marks a subquery body
    /// (an opening paren directly followed by SELECT/WITH). Raw length
    /// drives comma breaking, the `true` count drives indentation.
    parens: Vec<bool>,
    /// Raw paren depth of the most recent clause keyword; top-level commas
    /// break onto new lines at this depth.
    clause_depth: usize,
    /// Set while a JOIN phrase (LEFT OUTER JOIN…) is being emitted so later
    /// modifiers do not each force another line break.
    join_phrase_open: bool,
    /// Extra indent units owed to the line currently being built (AND/OR
    /// predicates sit one level deeper than their clause keyword).
    line_extra: usize,
    /// Subquery indent units captured when the current line started. A
    /// closing paren may appear later on the same line, so indentation must
    /// not be recomputed at flush time.
    line_base: usize,
    pending_blank_line: bool,
}

impl<'a> SqlFormatter<'a> {
    fn new(text: &'a str) -> Self {
        let mut atoms = Vec::new();
        let mut previous_end = 0;
        for token in lex_sql(text) {
            push_gap_atoms(&mut atoms, &text[previous_end..token.range.start]);
            atoms.push((AtomKind::Token(token.kind), &text[token.range.clone()]));
            previous_end = token.range.end;
        }
        push_gap_atoms(&mut atoms, &text[previous_end..]);

        Self {
            atoms,
            out: String::with_capacity(text.len() + 32),
            line: String::new(),
            parens: Vec::new(),
            clause_depth: 0,
            join_phrase_open: false,
            line_extra: 0,
            line_base: 0,
            pending_blank_line: false,
        }
    }

    fn raw_depth(&self) -> usize {
        self.parens.len()
    }

    /// Indent units contributed by enclosing subqueries.
    fn subquery_depth(&self) -> usize {
        self.parens.iter().filter(|open| **open).count()
    }

    fn flush(&mut self) {
        let has_content = !self.line.trim().is_empty();
        if has_content {
            if self.pending_blank_line {
                self.out.push('\n');
                self.pending_blank_line = false;
            }
            for _ in 0..self.line_base + self.line_extra {
                self.out.push_str("  ");
            }
            self.out.push_str(self.line.trim_end());
            self.out.push('\n');
        }
        self.line.clear();
        // Whatever comes next starts at the indentation live right now.
        // Empty flushes still refresh this: a carried `(` or a comment can
        // clear the line without going through a real break.
        self.line_base = self.subquery_depth();
        self.line_extra = 0;
    }

    fn previous_atom(&self, index: usize) -> Option<AtomKind<'_>> {
        self.atoms.get(index.wrapping_sub(1)).map(|(kind, _)| *kind)
    }

    /// Decide whether the keyword at `index` forces a line break and how deep
    /// that line sits. Returns the extra indent units, if any.
    fn break_before(
        &mut self,
        lower: &str,
        index: usize,
        case_stack: &mut Vec<usize>,
    ) -> Option<usize> {
        if lower == "case" {
            // CASE stays glued to its clause (`SELECT CASE …`, `sum(case
            // …)`); only its WHEN/ELSE/END arms break, so just track nesting.
            case_stack.push(self.raw_depth());
            return None;
        }

        if let Some(clause) = clause_break_kind(lower) {
            if let ClauseBreak::Predicate = clause {
                self.join_phrase_open = false;
                return Some(1);
            }
            if is_join_modifier(lower) || lower == "join" {
                // JOIN phrases occupy a single line; later modifiers continue
                // it. Words like LEFT/RIGHT only break when they really head
                // a join, never when they are function calls.
                if !join_word_starts_phrase(&self.atoms, index) {
                    return None;
                }
                let brk = (!self.join_phrase_open).then_some(0);
                self.join_phrase_open = true;
                return brk;
            }
            // REPLACE doubles as a statement starter and a common string
            // function; a following open paren means the function call.
            if lower == "replace" && followed_by_open_paren(&self.atoms, index) {
                return None;
            }
            self.join_phrase_open = false;
            return Some(0);
        }

        self.join_phrase_open = false;
        let case_top = case_stack.last().copied();
        let at_clause_level = self.raw_depth() == self.clause_depth;
        match lower {
            "when" | "else"
                if case_top.is_some_and(|depth| depth == self.raw_depth()) && at_clause_level =>
            {
                Some(1)
            }
            "end" if case_top.is_some_and(|depth| depth == self.raw_depth()) => {
                case_stack.pop();
                at_clause_level.then_some(0)
            }
            _ => None,
        }
    }

    fn run(mut self) -> String {
        let mut case_stack: Vec<usize> = Vec::new();
        let mut index = 0;
        while index < self.atoms.len() {
            let (kind, raw) = self.atoms[index];
            match kind {
                AtomKind::Token(kind) => {
                    let lower = raw.to_ascii_lowercase();
                    let is_keyword = matches!(kind, SqlTokenKind::Keyword | SqlTokenKind::Type);

                    if is_keyword
                        && let Some(extra) = self.break_before(&lower, index, &mut case_stack)
                    {
                        // A paren already appended to this line belongs to
                        // the clause that follows (`FROM (`), so carry it.
                        let mut carried_open = false;
                        if self.line.trim_end().ends_with('(') {
                            while self.line.ends_with(char::is_whitespace) {
                                self.line.pop();
                            }
                            self.line.pop();
                            while self.line.ends_with(' ') {
                                self.line.pop();
                            }
                            carried_open = true;
                        }
                        // The line being built keeps its own indent; the
                        // break's extra indent belongs to the next line.
                        self.flush();
                        self.line_extra = extra;
                        self.clause_depth = self.raw_depth();
                        if carried_open {
                            self.line.push('(');
                        }
                    }

                    // Spacing against whatever is already on the line.
                    // Operators and closing punctuation manage their own
                    // trailing spaces, so only genuinely missing gaps are
                    // filled here.
                    if !self.line.is_empty() {
                        let glued = match self.previous_atom(index) {
                            Some(AtomKind::Punct(open))
                                if open == "(" || open == "." || open.ends_with(':') =>
                            {
                                true
                            }
                            _ => self.line.ends_with(' '),
                        };
                        if !glued {
                            self.line.push(' ');
                        }
                    }
                    let word = if is_keyword {
                        raw.to_ascii_uppercase()
                    } else {
                        raw.to_owned()
                    };
                    self.line.push_str(&word);

                    if kind == SqlTokenKind::Comment && raw.starts_with("--") {
                        self.flush();
                    }
                }
                AtomKind::Punct("(") => {
                    // A paren opened directly before SELECT/WITH begins a
                    // subquery body: mark it now so the body's lines indent.
                    let opens_subquery = matches!(
                        self.atoms.get(index + 1),
                        Some((AtomKind::Token(_), raw))
                            if raw.eq_ignore_ascii_case("select")
                                || raw.eq_ignore_ascii_case("with")
                    );
                    if !self.line.is_empty() {
                        let glued = match self.previous_atom(index) {
                            Some(AtomKind::Punct(open)) => open == "(" || open == ".",
                            Some(AtomKind::Token(
                                SqlTokenKind::Identifier | SqlTokenKind::Parameter,
                            )) => true,
                            _ => false,
                        };
                        if !glued {
                            self.line.push(' ');
                        }
                    }
                    self.line.push('(');
                    self.parens.push(opens_subquery);
                }
                AtomKind::Punct(")") => {
                    self.parens.pop();
                    self.clause_depth = self.clause_depth.min(self.raw_depth());
                    while self.line.ends_with(' ') {
                        self.line.pop();
                    }
                    self.line.push(')');
                }
                AtomKind::Punct(",") => {
                    while self.line.ends_with(' ') {
                        self.line.pop();
                    }
                    self.line.push(',');
                    if self.raw_depth() <= self.clause_depth {
                        self.flush();
                        self.line_extra = 0;
                    }
                }
                AtomKind::Punct(";") => {
                    while self.line.ends_with(' ') {
                        self.line.pop();
                    }
                    self.line.push(';');
                    self.flush();
                    self.line_extra = 0;
                    self.parens.clear();
                    self.clause_depth = 0;
                    self.join_phrase_open = false;
                    self.pending_blank_line = true;
                }
                AtomKind::Punct(".") => {
                    while self.line.ends_with(' ') {
                        self.line.pop();
                    }
                    self.line.push('.');
                }
                AtomKind::Punct(operator) => {
                    self.append_operator(index, operator);
                }
            }
            index += 1;
        }
        self.flush();
        self.out.trim_end().to_owned()
    }

    /// Append an operator atom with even spacing, gluing against grouping
    /// punctuation and unary signs (`-5`, `count(*)`, `(a)::text`).
    fn append_operator(&mut self, index: usize, operator: &str) {
        let previous = self.previous_atom(index).and_then(|kind| match kind {
            AtomKind::Punct(text) => Some(text),
            AtomKind::Token(_) => None,
        });
        // `None`: the next atom is a word token rather than punctuation.
        let next = self.atoms.get(index + 1).and_then(|(kind, _)| match kind {
            AtomKind::Punct(text) => Some(Some(*text)),
            AtomKind::Token(_) => None,
        });

        let unary = (operator == "-" || operator == "+")
            && operator.len() == 1
            && matches!(
                previous,
                None | Some("(" | "," | "=" | "<" | ">" | "!" | "*" | "/" | "-" | "+" | "::")
            );
        // PostgreSQL casts glue on both sides: `a.x::text`.
        let cast = operator == "::";
        let glue_before =
            unary || cast || previous.is_some_and(|previous| previous == "(" || previous == ".");
        let glue_after =
            unary || cast || next.is_some_and(|next| matches!(next, Some(")" | "," | ";" | ".")));

        if !glue_before && !self.line.is_empty() && !self.line.ends_with(' ') {
            self.line.push(' ');
        }
        self.line.push_str(operator);
        if !glue_after {
            self.line.push(' ');
        }
    }
}

/// Whether the next significant atom after `index` is an open paren, which
/// distinguishes function-call keywords (`REPLACE(…)`) from statement
/// starters.
fn followed_by_open_paren(atoms: &[(AtomKind<'_>, &str)], index: usize) -> bool {
    for atom in &atoms[index + 1..] {
        match atom {
            (AtomKind::Punct("("), _) => return true,
            (AtomKind::Punct(_), _) => continue,
            (AtomKind::Token(_), _) => return false,
        }
    }
    false
}

/// True when the keyword at `index` participates in a JOIN phrase that should
/// occupy one line: either `JOIN` itself or a modifier whose next word token
/// leads to `JOIN` (so `LEFT(name, 3)` stays a function call).
fn join_word_starts_phrase(atoms: &[(AtomKind, &str)], index: usize) -> bool {
    let word = |atom: &(AtomKind, &str)| match atom {
        (AtomKind::Token(_), raw) => Some(raw.to_ascii_lowercase()),
        (AtomKind::Punct(_), _) => None,
    };
    let Some(current) = word(&atoms[index]) else {
        return false;
    };
    if current == "join" {
        return true;
    }
    let mut ahead = index + 1;
    while ahead < atoms.len() {
        match &atoms[ahead] {
            (AtomKind::Punct(_), _) => ahead += 1,
            (AtomKind::Token(_), raw) => {
                let candidate = raw.to_ascii_lowercase();
                if candidate == "join" {
                    return true;
                }
                if is_join_modifier(&candidate) {
                    ahead += 1;
                    continue;
                }
                return false;
            }
        }
    }
    false
}

fn is_join_modifier(word: &str) -> bool {
    matches!(
        word,
        "inner" | "outer" | "left" | "right" | "full" | "cross" | "natural"
    )
}

fn clause_break_kind(word: &str) -> Option<ClauseBreak> {
    match word {
        "select" | "from" | "where" | "group" | "order" | "having" | "limit" | "offset"
        | "returning" | "set" | "values" | "window" | "union" | "except" | "intersect"
        | "insert" | "update" | "delete" | "create" | "alter" | "drop" | "truncate" | "begin"
        | "commit" | "rollback" | "with" | "explain" | "pragma" | "vacuum" | "replace" | "on"
        | "join" | "inner" | "outer" | "left" | "right" | "full" | "cross" | "natural" => {
            Some(ClauseBreak::Clause)
        }
        "and" | "or" => Some(ClauseBreak::Predicate),
        _ => None,
    }
}

const SQL_FUNCTIONS: &[&str] = &[
    "ABS",
    "AVG",
    "CEIL",
    "CHAR_LENGTH",
    "COALESCE",
    "CONCAT",
    "COUNT",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "DATE_TRUNC",
    "EXTRACT",
    "FLOOR",
    "GREATEST",
    "GROUP_CONCAT",
    "IFNULL",
    "LEAST",
    "LENGTH",
    "LOWER",
    "LPAD",
    "LTRIM",
    "MAX",
    "MIN",
    "MOD",
    "NOW",
    "NULLIF",
    "POWER",
    "RANDOM",
    "REPLACE",
    "ROUND",
    "RPAD",
    "RTRIM",
    "STRING_AGG",
    "SUBSTR",
    "SUBSTRING",
    "SUM",
    "TO_CHAR",
    "TO_TIMESTAMP",
    "TRIM",
    "UPPER",
];

fn completion_identifier_start(text: &str, cursor: usize) -> usize {
    let cursor = clamp_boundary(text, cursor);
    text[..cursor]
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!is_identifier_continue(character)).then_some(index + character.len_utf8())
        })
        .unwrap_or(0)
}

fn completion_identifier_end(text: &str, cursor: usize) -> usize {
    let cursor = clamp_boundary(text, cursor);
    text[cursor..]
        .char_indices()
        .find_map(|(index, character)| {
            (!is_identifier_continue(character)).then_some(cursor + index)
        })
        .unwrap_or(text.len())
}

struct QuotedCompletionIdentifier {
    prefix: String,
    quote: char,
    qualifier_start: usize,
    range: Range<usize>,
}

/// Return the quoted identifier contents around `cursor`. Completion
/// candidates intentionally insert raw identifier text when `quote` is set,
/// so the delimiters stay outside the replacement range.
///
/// An unfinished identifier deliberately extends through the lexer token. The
/// tolerant lexer keeps it available while the user is still typing, and a
/// selected candidate retains the existing incomplete-quote behavior.
fn quoted_completion_identifier(
    text: &str,
    cursor: usize,
    tokens: &[SqlToken],
) -> Option<QuotedCompletionIdentifier> {
    let token = tokens.iter().find(|token| {
        token.kind == SqlTokenKind::Identifier
            && token.range.start < cursor
            && cursor <= token.range.end
            && text[token.range.clone()]
                .chars()
                .next()
                .is_some_and(|character| matches!(character, '"' | '`'))
    })?;
    let quote = text[token.range.start..].chars().next()?;
    let closed = quoted_identifier_is_closed(&text[token.range.clone()], quote);
    let content_start = token.range.start + quote.len_utf8();
    let prefix =
        text[content_start..cursor].replace(&format!("{quote}{quote}"), &quote.to_string());
    let content_end = if closed {
        token.range.end - quote.len_utf8()
    } else {
        cursor
    };

    Some(QuotedCompletionIdentifier {
        prefix,
        quote,
        qualifier_start: token.range.start,
        range: content_start..content_end,
    })
}

fn quoted_identifier_is_closed(raw: &str, quote: char) -> bool {
    let mut characters = raw.chars();
    debug_assert_eq!(characters.next(), Some(quote));
    while let Some(character) = characters.next() {
        if character != quote {
            continue;
        }
        if characters.next() != Some(quote) {
            return true;
        }
    }
    false
}

fn completion_qualifier(text: &str, start: usize) -> Option<(String, usize)> {
    let (dot_start, dot) = text[..start]
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())?;
    if dot != '.' {
        return None;
    }

    let mut segment_end = text[..dot_start].trim_end().len();
    let (mut segment_start, segment) = completion_qualifier_segment(text, segment_end)?;
    let mut segments = vec![segment];
    while segment_start > 0 {
        let (separator_start, separator) = text[..segment_start]
            .char_indices()
            .rev()
            .find(|(_, character)| !character.is_whitespace())?;
        if separator != '.' {
            break;
        }
        segment_end = text[..separator_start].trim_end().len();
        let (previous_start, previous_segment) = completion_qualifier_segment(text, segment_end)?;
        segments.push(previous_segment);
        segment_start = previous_start;
    }

    segments.reverse();
    Some((segments.join("."), segment_start))
}

fn completion_qualifier_segment(text: &str, end: usize) -> Option<(usize, String)> {
    let end = text[..end].trim_end().len();
    if end == 0 {
        return None;
    }

    let bytes = text.as_bytes();
    if end >= 2 && matches!(bytes[end - 1], b'"' | b'`') {
        let quote = bytes[end - 1];
        let mut index = end.saturating_sub(2);
        loop {
            if bytes[index] == quote {
                if index > 1 && bytes[index - 1] == quote {
                    index -= 2;
                    continue;
                }
                let raw = &text[index..end];
                let quote = char::from(quote);
                let segment =
                    raw[1..raw.len() - 1].replace(&format!("{quote}{quote}"), &quote.to_string());
                return (!segment.is_empty()).then_some((index, segment));
            }
            if index == 0 {
                break;
            }
            index -= 1;
        }
        return None;
    }

    let start = completion_identifier_start(text, end);
    (start < end).then(|| (start, text[start..end].to_owned()))
}

fn previous_sql_word(text: &str, before: usize) -> Option<String> {
    let before = clamp_boundary(text, before);
    let end = text[..before]
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())
        .map_or(before, |(index, character)| index + character.len_utf8());
    let start = completion_identifier_start(text, end);
    (start < end).then(|| text[start..end].to_ascii_lowercase())
}

fn is_table_completion_keyword(word: &str) -> bool {
    matches!(
        word,
        "from" | "join" | "update" | "into" | "table" | "view" | "references"
    )
}

fn is_column_completion_keyword(word: &str) -> bool {
    matches!(
        word,
        "select"
            | "where"
            | "and"
            | "or"
            | "on"
            | "by"
            | "group"
            | "order"
            | "having"
            | "set"
            | "returning"
            | "values"
    )
}

/// A native single-line or multiline text editor.
const HISTORY_LIMIT: usize = 100;

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
    /// UTF-8 byte ranges painted with a wavy error underline. The SQL shell
    /// derives these from failed-query messages; they are advisory paint only
    /// and never participate in selection or input handling.
    diagnostics: Vec<Range<usize>>,
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
    undo_history: Vec<EditorSnapshot>,
    redo_history: Vec<EditorSnapshot>,
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
            diagnostics: Vec::new(),
            selection_reversed: false,
            marked_range: None,
            scroll_handle: ScrollHandle::new(),
            last_layout: Vec::new(),
            last_bounds: None,
            selection_bounds: None,
            is_selecting: false,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            _subscriptions: vec![observed, focus, blur],
        }
    }

    /// Create a multiline SQL editor backed by an existing value entity.
    pub fn new_sql(value: Entity<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_language(value, true, EditorLanguage::Sql, window, cx)
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
        self.record_history(cx);
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

    /// The selected text, if there is a non-empty UTF-8 selection.
    #[allow(dead_code)]
    pub fn selected_text(&self, cx: &App) -> Option<String> {
        let text = self.text(cx);
        (!self.selected_range.is_empty()).then(|| text[self.selected_range.clone()].to_owned())
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
        )
    }

    /// Return the painted location immediately below the caret for a
    /// completion popover. Before the editor has been laid out this safely
    /// falls back to the origin, allowing callers to render without a fixed
    /// query-editor-specific offset.
    #[allow(dead_code)]
    pub fn completion_anchor(&self) -> Point<Pixels> {
        completion_anchor(
            &self.content,
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
    pub fn set_diagnostics(&mut self, ranges: Vec<Range<usize>>, cx: &mut Context<Self>) {
        if self.diagnostics == ranges {
            return;
        }
        self.diagnostics = clamp_ranges(&self.text(cx), ranges);
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
        if text[range.clone()] == inserted {
            return;
        }
        self.record_history(cx);
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

    fn record_history(&mut self, cx: &mut Context<Self>) {
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
        self.content = snapshot.text.clone();
        self.selected_range = clamp_range(&snapshot.text, snapshot.selection);
        self.selection_reversed = snapshot.selection_reversed;
        self.marked_range = None;
        self.set_text_without_selection(snapshot.text, cx);
        cx.notify();
    }

    /// Undo the most recent local text replacement, including IME commits.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.undo_history.pop() else {
            return;
        };
        self.redo_history
            .push(EditorSnapshot::from_editor(self, cx));
        self.restore_snapshot(snapshot, cx);
    }

    /// Redo a replacement reversed by [`Self::undo`].
    pub fn redo(&mut self, cx: &mut Context<Self>) {
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

    /// Insert a newline for a shell-level Enter handler.
    pub fn insert_newline(&mut self, cx: &mut Context<Self>) {
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
        self.focus_handle.focus(window, cx);
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

fn completion_anchor(
    text: &str,
    cursor: usize,
    bounds: Option<Bounds<Pixels>>,
    layout: &[ShapedLine],
) -> Point<Pixels> {
    let Some(bounds) = bounds else {
        return point(px(0.), px(0.));
    };
    let cursor = clamp_boundary(text, cursor);
    let (line, column) = line_and_column(text, cursor);
    let line_height = bounds.size.height / layout.len().max(1) as f32;
    let x = layout
        .get(line)
        .map(|line| line.x_for_index(column))
        .unwrap_or(px(0.));
    point(
        bounds.left() + x,
        bounds.top() + line_height * (line + 1).min(layout.len().max(1)) as f32,
    )
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
        let syntax_tokens = match editor.language {
            EditorLanguage::PlainText => Vec::new(),
            EditorLanguage::Sql => lex_sql(&text)
                .into_iter()
                .map(HighlightToken::from)
                .collect(),
            EditorLanguage::Redis => lex_redis(&text)
                .into_iter()
                .map(HighlightToken::from)
                .collect(),
            EditorLanguage::Json => lex_json(&text)
                .into_iter()
                .map(HighlightToken::from)
                .collect(),
        };

        for (line, painted_line) in text.split('\n').zip(painted_text.split('\n')) {
            let marked_range = editor.marked_range.as_ref().and_then(|range| {
                let range = marked_slice(line.len(), line_start, range)?;
                let range = if editor.password {
                    password_display_offset(line, range.start)
                        ..password_display_offset(line, range.end)
                } else {
                    range
                };
                Some(line_start + range.start..line_start + range.end)
            });
            let runs = editor_text_runs(
                painted_line,
                line_start,
                &style,
                editor.language,
                &syntax_tokens,
                marked_range.as_ref(),
                &editor.diagnostics,
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
                editor.scroll_handle.max_offset().into(),
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
                    theme().selection,
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
    input_with_context(editor, focus, multiline, TEXT_EDITOR_CONTEXT, false)
}

/// Render the SQL editor with both its text-editing and completion contexts.
#[allow(dead_code)]
pub fn sql_input(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
) -> impl IntoElement {
    input_with_context(editor, focus, multiline, SQL_TEXT_EDITOR_CONTEXT, false)
}

/// Render the query SQL editor so it fills a resizable parent pane.
///
/// This intentionally does not alter [`sql_input`]: row and JSON editors
/// retain their compact, fixed multiline height.
pub fn sql_input_fill(editor: Entity<TextEditor>, focus: FocusHandle) -> impl IntoElement {
    input_with_context(editor, focus, true, SQL_TEXT_EDITOR_CONTEXT, true)
}

fn input_with_context(
    editor: Entity<TextEditor>,
    focus: FocusHandle,
    multiline: bool,
    key_context: &'static str,
    fill_height: bool,
) -> impl IntoElement {
    div()
        .id(gpui::SharedString::from(format!(
            "dbx-text-editor-{:?}",
            editor.entity_id()
        )))
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
            this.h(if multiline { px(204.) } else { px(32.) })
        })
        .p(if multiline { px(10.) } else { px(7.) })
        .overflow_hidden()
        .bg(theme().canvas)
        .border_1()
        .border_color(theme().border_strong)
        .rounded(px(5.))
        .text_size(px(12.))
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
    tokens: &[HighlightToken],
    marked_range: Option<&Range<usize>>,
    diagnostics: &[Range<usize>],
) -> Vec<TextRun> {
    let base = style.to_run(line.len());
    let runs = match language {
        EditorLanguage::PlainText => vec![base],
        EditorLanguage::Sql | EditorLanguage::Redis | EditorLanguage::Json => {
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

/// Split runs so every diagnostic range on this line carries a wavy error
/// underline. Applied after IME marking so a composing range keeps its own
/// treatment while the diagnostic underline remains visible underneath.
fn apply_diagnostic_runs(
    line_len: usize,
    line_start: usize,
    diagnostics: &[Range<usize>],
    mut runs: Vec<TextRun>,
) -> Vec<TextRun> {
    for range in diagnostics {
        let Some(range) = marked_slice(line_len, line_start, range) else {
            continue;
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
                        color: Some(theme().danger.into()),
                        thickness: px(1.),
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

fn json_token_color(kind: JsonTokenKind) -> gpui::Hsla {
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

fn push_json_token(tokens: &mut Vec<JsonToken>, kind: JsonTokenKind, start: usize, end: usize) {
    if start < end {
        tokens.push(JsonToken {
            kind,
            range: start..end,
        });
    }
}

fn consume_json_string(chars: &[(usize, char)], mut index: usize) -> usize {
    index += 1;
    while index < chars.len() {
        match chars[index].1 {
            '\\' => index = (index + 2).min(chars.len()),
            '"' => return index + 1,
            _ => index += 1,
        }
    }
    index
}

fn consume_json_number(chars: &[(usize, char)], mut index: usize) -> Option<usize> {
    if chars.get(index)?.1 == '-' {
        index += 1;
    }
    let first = chars.get(index)?.1;
    if first == '0' {
        index += 1;
    } else if first.is_ascii_digit() {
        index += 1;
        while chars
            .get(index)
            .is_some_and(|(_, character)| character.is_ascii_digit())
        {
            index += 1;
        }
    } else {
        return None;
    }

    if chars
        .get(index)
        .is_some_and(|(_, character)| *character == '.')
    {
        let fraction_start = index + 1;
        index = fraction_start;
        while chars
            .get(index)
            .is_some_and(|(_, character)| character.is_ascii_digit())
        {
            index += 1;
        }
        // Retain a trailing decimal point while the value is being edited.
        if index == fraction_start {
            index = fraction_start;
        }
    }

    if chars
        .get(index)
        .is_some_and(|(_, character)| matches!(*character, 'e' | 'E'))
    {
        let exponent_start = index;
        index += 1;
        if chars
            .get(index)
            .is_some_and(|(_, character)| matches!(*character, '+' | '-'))
        {
            index += 1;
        }
        while chars
            .get(index)
            .is_some_and(|(_, character)| character.is_ascii_digit())
        {
            index += 1;
        }
        // Highlight an incomplete exponent too; validity remains the
        // database's concern at save time.
        if index == exponent_start + 1 {
            index = exponent_start + 1;
        }
    }

    Some(index)
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

fn clamp_ranges(text: &str, ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges
        .into_iter()
        .map(|range| clamp_range(text, range))
        .filter(|range| !range.is_empty())
        .collect()
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
    fn component_word_navigation_lands_on_word_edges() {
        let text = "one two";
        assert_eq!(previous_word_boundary(text, text.len()), 4);
        assert_eq!(previous_word_boundary(text, 4), 0);
        assert_eq!(next_word_boundary(text, 0), 3);
        assert_eq!(next_word_boundary(text, 3), text.len());
    }

    #[test]
    fn ranges_clamp_to_utf8_boundaries() {
        assert_eq!(clamp_range("🦀", 1..3), 0..0);
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
        );
        assert_eq!(&text[range], "\nSELECT 🦀 FROM users;");

        let selection = text.find("SELECT 3").unwrap()..text.len();
        let range = execution_range(
            text,
            selection,
            cursor,
            QueryExecutionScope::SelectionOrStatement,
        );
        assert_eq!(&text[range], "SELECT 3");
    }

    #[test]
    fn sql_execution_keeps_the_last_statement_after_its_terminator() {
        let statement = "SELECT * FROM missing_table;";
        assert_eq!(
            sql_statement_range(statement, statement.len()),
            0..statement.len()
        );

        let with_trailing_whitespace = "SELECT 1;\n  ";
        assert_eq!(
            &with_trailing_whitespace
                [sql_statement_range(with_trailing_whitespace, with_trailing_whitespace.len(),)],
            "SELECT 1;"
        );
    }

    #[test]
    fn sql_execution_ignores_terminators_inside_lexical_regions() {
        let text = "SELECT ';', $$BEGIN; END$$ /* ; */ -- ;\n; SELECT 2;";
        let first_end = text.find(" SELECT 2").unwrap();
        assert_eq!(sql_statement_range(text, 10), 0..first_end);
        let second = text.find("SELECT 2").unwrap();
        assert_eq!(&text[sql_statement_range(text, second)], " SELECT 2;");
    }

    #[test]
    fn statement_count_ignores_lexical_semicolons_and_empty_statements() {
        assert_eq!(
            sql_statement_count("; SELECT ';'; $$BEGIN; END$$; -- ;\n"),
            2
        );
        assert_eq!(
            sql_statement_count("-- comment only\n/* still comment */"),
            0
        );
    }

    #[test]
    fn schema_change_detection_is_lexer_aware_and_conservative() {
        assert!(sql_may_change_schema(
            "SELECT 1; CREATE TABLE audit_log (id int)"
        ));
        assert!(sql_may_change_schema(
            "ALTER TABLE users ADD COLUMN active boolean"
        ));
        assert!(sql_may_change_schema("CALL install_schema()"));
        assert!(!sql_may_change_schema(
            "SELECT 'CREATE TABLE decoy'; -- DROP TABLE decoy\nSELECT 2"
        ));
        assert!(!sql_may_change_schema(
            "SELECT $$ALTER TABLE decoy ADD COLUMN value int$$"
        ));
    }

    #[test]
    fn execution_kind_is_conservative_about_writes() {
        assert_eq!(sql_execution_kind("SELECT 1"), SqlExecutionKind::Read);
        assert_eq!(
            sql_execution_kind("SELECT 1; UPDATE users SET active = TRUE"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("DELETE FROM users"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("DELETE FROM users WHERE id = 1"),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind("UPDATE users SET active = TRUE"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("UPDATE users SET active = TRUE WHERE id = 1"),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind("WITH doomed AS (SELECT id FROM users) DELETE FROM users"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind(
                "WITH changed AS (SELECT id FROM users) UPDATE users SET active = TRUE"
            ),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind(
                "WITH matched AS (SELECT id FROM users) DELETE FROM users WHERE id = 1"
            ),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind(
                "WITH deleted AS (DELETE FROM users RETURNING *) SELECT * FROM deleted"
            ),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind(
                "WITH changed AS (UPDATE users SET active = TRUE WHERE id = 1 RETURNING *) SELECT * FROM changed"
            ),
            SqlExecutionKind::MutationRisk
        );
        assert_eq!(
            sql_execution_kind(
                "WITH selected AS (SELECT id FROM users WHERE active = TRUE) SELECT * FROM selected"
            ),
            SqlExecutionKind::Read
        );
        assert_eq!(
            sql_execution_kind(
                "WITH selected AS (SELECT 'DELETE FROM users', $$UPDATE users$$) /* DELETE */ SELECT * FROM selected"
            ),
            SqlExecutionKind::Read
        );
        assert_eq!(
            sql_execution_kind("MERGE INTO users USING incoming ON users.id = incoming.id"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("REPLACE INTO users(id) VALUES (1)"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("DROP TABLE users"),
            SqlExecutionKind::Destructive
        );
        assert_eq!(
            sql_execution_kind("EXPLAIN SELECT 1"),
            SqlExecutionKind::MutationRisk
        );
    }

    #[test]
    fn execution_scope_supports_document_and_line_or_selection_commands() {
        let text = "SET one\nGET two\nDEL three";
        assert_eq!(
            execution_range(text, 0..0, 9, QueryExecutionScope::SelectionOrCurrentLine),
            8..15
        );
        assert_eq!(
            execution_range(text, 0..0, 9, QueryExecutionScope::Document),
            0..text.len()
        );
    }

    #[test]
    fn history_is_bounded_and_keeps_utf8_snapshots() {
        let mut history = Vec::new();
        for index in 0..=HISTORY_LIMIT {
            history.push(EditorSnapshot {
                text: format!("{index}🦀"),
                selection: 0..0,
                selection_reversed: false,
            });
            if history.len() > HISTORY_LIMIT {
                history.remove(0);
            }
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history.first().unwrap().text, "1🦀");
        assert!(
            history
                .iter()
                .all(|snapshot| snapshot.text.is_char_boundary(snapshot.selection.end))
        );
    }

    #[test]
    fn completion_anchor_has_a_safe_pre_layout_fallback() {
        assert_eq!(
            completion_anchor("SELECT 🦀", 8, None, &[]),
            point(px(0.), px(0.))
        );
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
    fn later_line_runs_use_document_offsets_for_syntax_diagnostics_and_ime_marks() {
        let text = "SELECT\nFROM users";
        let tokens: Vec<_> = lex_sql(text)
            .into_iter()
            .map(HighlightToken::from)
            .collect();
        let style = gpui::TextStyle::default();
        let second_line_start = "SELECT\n".len();
        let diagnostics = std::iter::once(second_line_start + 5..text.len()).collect::<Vec<_>>();

        let runs = editor_text_runs(
            "FROM users",
            second_line_start,
            &style,
            EditorLanguage::Sql,
            &tokens,
            Some(&(second_line_start..second_line_start + 4)),
            &diagnostics,
        );

        assert_eq!(
            runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            [4, 1, 5]
        );
        assert_eq!(runs[0].color, sql_token_color(SqlTokenKind::Keyword));
        assert!(runs[0].underline.as_ref().is_some_and(|style| !style.wavy));
        assert!(runs[2].underline.as_ref().is_some_and(|style| style.wavy));
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
        assert_eq!(bindings.len(), 32);
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
    fn redis_query_text_must_not_use_the_plain_text_paint_pipeline() {
        let text = "GET \"user:name\" 42";
        let tokens: Vec<_> = lex_redis(text)
            .into_iter()
            .map(HighlightToken::from)
            .collect();
        let style = gpui::TextStyle::default();

        let runs = editor_text_runs(text, 0, &style, EditorLanguage::Redis, &tokens, None, &[]);

        assert!(
            runs.len() > 1,
            "Redis commands, strings, and numbers need semantic runs"
        );
    }

    #[test]
    fn redis_lexer_is_line_aware_and_tolerates_quoted_unicode() {
        let text = "get \"user:東京\" 42 NX\nSCAN 0 MATCH user:* COUNT 100";
        let tokens = lex_redis(text);
        let values = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            [
                (RedisTokenKind::Command, "get"),
                (RedisTokenKind::String, "\"user:東京\""),
                (RedisTokenKind::Number, "42"),
                (RedisTokenKind::Option, "NX"),
                (RedisTokenKind::Command, "SCAN"),
                (RedisTokenKind::Number, "0"),
                (RedisTokenKind::Option, "MATCH"),
                (RedisTokenKind::Identifier, "user:*"),
                (RedisTokenKind::Option, "COUNT"),
                (RedisTokenKind::Number, "100"),
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

    #[test]
    fn json_lexer_highlights_properties_escapes_and_unicode() {
        let text = r#"{"café": "say \"hi\" to 🦀", "nested": {"城市": "東京"}}"#;
        let tokens = lex_json(text);
        let values: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect();

        assert_eq!(
            values,
            vec![
                (JsonTokenKind::Property, "\"café\""),
                (JsonTokenKind::String, "\"say \\\"hi\\\" to 🦀\""),
                (JsonTokenKind::Property, "\"nested\""),
                (JsonTokenKind::Property, "\"城市\""),
                (JsonTokenKind::String, "\"東京\""),
            ]
        );
        assert!(tokens.iter().all(|token| {
            text.is_char_boundary(token.range.start) && text.is_char_boundary(token.range.end)
        }));
    }

    #[test]
    fn json_lexer_highlights_numbers_and_literals() {
        let text = r#"[-0, 42, 3.14, -2.5e+3, 6E-2, true, false, null]"#;
        let tokens = lex_json(text);
        let values: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect();

        assert_eq!(
            values,
            vec![
                (JsonTokenKind::Number, "-0"),
                (JsonTokenKind::Number, "42"),
                (JsonTokenKind::Number, "3.14"),
                (JsonTokenKind::Number, "-2.5e+3"),
                (JsonTokenKind::Number, "6E-2"),
                (JsonTokenKind::Boolean, "true"),
                (JsonTokenKind::Boolean, "false"),
                (JsonTokenKind::Null, "null"),
            ]
        );
    }

    #[test]
    fn json_lexer_keeps_incomplete_strings_highlighted() {
        let text = r#"{"draft": "still typing 🦀"#;
        let tokens = lex_json(text);
        let values: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, &text[token.range.clone()]))
            .collect();

        assert_eq!(
            values,
            vec![
                (JsonTokenKind::Property, "\"draft\""),
                (JsonTokenKind::String, "\"still typing 🦀"),
            ]
        );
    }

    #[test]
    fn completion_context_detects_table_and_column_positions() {
        let table = sql_completion_context("SELECT * FROM pu", "SELECT * FROM pu".len()).unwrap();
        assert_eq!(table.target, SqlCompletionTarget::Table);
        assert_eq!(table.prefix, "pu");
        assert_eq!(table.replacement_range, 14..16);

        let column = sql_completion_context("SELECT us", "SELECT us".len()).unwrap();
        assert_eq!(column.target, SqlCompletionTarget::Column);
        assert_eq!(column.prefix, "us");

        let qualified = sql_completion_context("SELECT users.", "SELECT users.".len()).unwrap();
        assert_eq!(qualified.target, SqlCompletionTarget::Column);
        assert_eq!(qualified.qualifier.as_deref(), Some("users"));
        assert_eq!(qualified.quote, None);
        assert_eq!(qualified.replacement_range, 13..13);

        let schema_qualified =
            sql_completion_context("SELECT analytics.users.", "SELECT analytics.users.".len())
                .unwrap();
        assert_eq!(
            schema_qualified.qualifier.as_deref(),
            Some("analytics.users")
        );

        let quoted_qualified = sql_completion_context(
            "SELECT \"analytics\".\"users\".",
            "SELECT \"analytics\".\"users\".".len(),
        )
        .unwrap();
        assert_eq!(
            quoted_qualified.qualifier.as_deref(),
            Some("analytics.users")
        );

        let quoted =
            sql_completion_context("SELECT * FROM \"us", "SELECT * FROM \"us".len()).unwrap();
        assert_eq!(quoted.target, SqlCompletionTarget::Table);
        assert_eq!(quoted.quote, Some('"'));
        assert_eq!(quoted.prefix, "us");
        assert_eq!(quoted.replacement_range, 15..17);

        let quoted_qualified = sql_completion_context(
            "SELECT \"analytics\".\"us",
            "SELECT \"analytics\".\"us".len(),
        )
        .unwrap();
        assert_eq!(quoted_qualified.target, SqlCompletionTarget::Column);
        assert_eq!(quoted_qualified.qualifier.as_deref(), Some("analytics"));
        assert_eq!(quoted_qualified.replacement_range, 20..22);

        assert!(sql_completion_context("\"\"\".\"", "\"\"\".\"".len()).is_none());
    }

    #[test]
    fn completion_context_replaces_the_whole_identifier_around_the_caret() {
        let text = "SELECT user_name FROM users";
        let cursor = "SELECT user".len();
        let context = sql_completion_context(text, cursor).expect("column completion");

        assert_eq!(context.prefix, "user");
        assert_eq!(context.replacement_range, 7..16);
    }

    #[test]
    fn completion_context_replaces_a_quoted_identifier_with_spaces() {
        let text = "SELECT \"order items\" FROM orders";
        let cursor = "SELECT \"order".len();
        let context = sql_completion_context(text, cursor).expect("column completion");

        assert_eq!(context.prefix, "order");
        assert_eq!(context.quote, Some('"'));
        assert_eq!(context.replacement_range, 8..19);
        assert_eq!(
            format!(
                "{}customer order{}",
                &text[..context.replacement_range.start],
                &text[context.replacement_range.end..]
            ),
            "SELECT \"customer order\" FROM orders"
        );
    }

    #[test]
    fn completion_context_replaces_an_escaped_quoted_identifier() {
        let text = "SELECT \"customer\"\" name\" FROM orders";
        let cursor = "SELECT \"customer\"\"".len();
        let context = sql_completion_context(text, cursor).expect("column completion");

        assert_eq!(context.prefix, "customer\"");
        assert_eq!(context.quote, Some('"'));
        assert_eq!(context.replacement_range, 8..23);
        assert_eq!(
            format!(
                "{}customer id{}",
                &text[..context.replacement_range.start],
                &text[context.replacement_range.end..]
            ),
            "SELECT \"customer id\" FROM orders"
        );
    }

    #[test]
    fn completion_context_replaces_a_unicode_quoted_identifier() {
        let text = "SELECT \"東京 orders\" FROM orders";
        let cursor = "SELECT \"東京".len();
        let context = sql_completion_context(text, cursor).expect("column completion");

        assert_eq!(context.prefix, "東京");
        assert_eq!(context.quote, Some('"'));
        assert_eq!(context.replacement_range, 8..21);
        assert_eq!(
            format!(
                "{}東京 customer{}",
                &text[..context.replacement_range.start],
                &text[context.replacement_range.end..]
            ),
            "SELECT \"東京 customer\" FROM orders"
        );
    }

    #[test]
    fn completion_context_keeps_unclosed_identifier_text_after_the_caret() {
        let text = "SELECT \"us FROM users";
        let cursor = "SELECT \"us".len();
        let context = sql_completion_context(text, cursor).expect("column completion");

        assert_eq!(context.prefix, "us");
        assert_eq!(context.replacement_range, 8..10);
        assert_eq!(
            format!(
                "{}accounts{}",
                &text[..context.replacement_range.start],
                &text[context.replacement_range.end..]
            ),
            "SELECT \"accounts FROM users"
        );
    }

    #[test]
    fn completion_context_handles_backtick_quoted_identifiers() {
        let text = "SELECT `order items` FROM orders";
        let cursor = "SELECT `order".len();
        let context = sql_completion_context(text, cursor).expect("column completion");

        assert_eq!(context.prefix, "order");
        assert_eq!(context.quote, Some('`'));
        assert_eq!(context.replacement_range, 8..19);
    }

    #[test]
    fn completion_context_is_not_opened_after_a_closed_quoted_identifier() {
        let text = "SELECT \"users\"";
        assert!(sql_completion_context(text, text.len()).is_none());
    }

    #[test]
    fn completion_context_ignores_strings_and_comments() {
        assert!(sql_completion_context("SELECT 'users", "SELECT 'users".len()).is_none());
        assert!(sql_completion_context("SELECT 1 -- users", "SELECT 1 -- users".len()).is_none());
    }

    #[test]
    fn formatter_breaks_clauses_and_uppercases_keywords() {
        let formatted =
            format_sql("select id, name from users where id = 1 order by id desc limit 5;");
        assert_eq!(
            formatted,
            "SELECT id,\nname\nFROM users\nWHERE id = 1\nORDER BY id DESC\nLIMIT 5;"
        );
    }

    #[test]
    fn formatter_keeps_function_calls_and_operators_tight() {
        let formatted = format_sql("select count(*), coalesce(a.b, 0) as total, a.x::text from t");
        assert_eq!(
            formatted,
            "SELECT count(*),\ncoalesce(a.b, 0) AS total,\na.x::TEXT\nFROM t"
        );
    }

    #[test]
    fn formatter_breaks_joins_and_predicates() {
        let formatted = format_sql(
            "select u.id from users u left outer join orders o on o.user_id = u.id where u.active = true and o.total > 10",
        );
        assert_eq!(
            formatted,
            "SELECT u.id\nFROM users u\nLEFT OUTER JOIN orders o\nON o.user_id = u.id\nWHERE u.active = TRUE\n  AND o.total > 10"
        );
    }

    #[test]
    fn formatter_indents_subqueries_and_breaks_projection_commas_only_at_top_level() {
        let formatted = format_sql(
            "select id, (select count(*) from orders o where o.uid = u.id) n, concat(first, ' ', last) from users u",
        );
        assert_eq!(
            formatted,
            "SELECT id,\n  (SELECT count(*)\n  FROM orders o\n  WHERE o.uid = u.id) n,\nconcat(first, ' ', last)\nFROM users u"
        );
    }

    #[test]
    fn formatter_formats_case_blocks_and_insert_values() {
        let formatted = format_sql("select case when a then 1 else 0 end from t");
        assert_eq!(
            formatted,
            "SELECT CASE\n  WHEN a THEN 1\n  ELSE 0\nEND\nFROM t"
        );

        let inserted = format_sql("insert into t(a,b) values(1,2),(3,4)");
        assert_eq!(inserted, "INSERT INTO t(a, b)\nVALUES (1, 2),\n(3, 4)");
    }

    #[test]
    fn formatter_preserves_strings_comments_identifiers_and_statements() {
        let formatted = format_sql(
            "select 'a  b' as s, -- trailing note\nMyCol /* keep me */ from \"Weird Table\"; select 2;",
        );
        assert_eq!(
            formatted,
            "SELECT 'a  b' AS s,\n-- trailing note\nMyCol /* keep me */\nFROM \"Weird Table\";\n\nSELECT 2;"
        );
    }

    #[test]
    fn formatter_is_safe_on_incomplete_input() {
        assert_eq!(format_sql(""), "");
        assert_eq!(format_sql("   \n  "), "");
        assert_eq!(format_sql("select"), "SELECT");
        assert_eq!(format_sql("select 'unterminated"), "SELECT 'unterminated");
    }

    #[test]
    fn format_cursor_stays_on_the_same_token() {
        let text = "select id,name from users";
        let (formatted, cursor) = format_sql_at_cursor(text, 7);
        assert_eq!(&formatted[cursor..cursor + 2], "id");
        // Cursor in the gap before `name` maps to that token's start.
        let (_, gap_cursor) = format_sql_at_cursor(text, 10);
        assert!(formatted[gap_cursor..].starts_with("name"));
        // End of input clamps to the end of the output.
        let (_, end_cursor) = format_sql_at_cursor(text, text.len());
        assert_eq!(end_cursor, formatted.len());
    }

    #[test]
    fn error_range_reads_postgres_position_markers() {
        let query = "SELECT FORM users";
        let message = "error returned from database: syntax error at or near \"FORM\"\nPOSITION: 8";
        assert_eq!(sql_error_range(message, query), Some(7..11));
    }

    #[test]
    fn error_range_finds_quoted_near_tokens() {
        let query = "SELECT * FROM userss ORDER BY id";
        let message = "error returned from database: relation \"userss\" does not exist";
        assert_eq!(sql_error_range(message, query), Some(14..20));

        let sqlite_query = "SELEC * FROM t";
        let sqlite_message = "near \"SELEC\": syntax error";
        assert_eq!(sql_error_range(sqlite_message, sqlite_query), Some(0..5));
    }

    #[test]
    fn error_range_matches_mysql_remainder_prefixes_and_missing_columns() {
        let query = "SELECT FORM LIMIT 1 FROM users";
        let message = "You have an error in your SQL syntax; check the manual for the right syntax to use near 'FORM LIMIT 1 FROM users' at line 1";
        assert_eq!(sql_error_range(message, query), Some(7..11));

        let pg_column = "SELECT usr FROM users";
        let pg_message = "error returned from database: column \"usr\" does not exist";
        assert_eq!(sql_error_range(pg_message, pg_column), Some(7..10));

        let mysql_column = "SELECT usr FROM users";
        let mysql_message = "Unknown column 'usr' in 'field list'";
        assert_eq!(sql_error_range(mysql_message, mysql_column), Some(7..10));
    }

    #[test]
    fn error_range_returns_none_when_nothing_matches() {
        assert_eq!(sql_error_range("connection refused", "SELECT 1"), None);
        assert_eq!(sql_error_range("syntax error", ""), None);
    }
}
