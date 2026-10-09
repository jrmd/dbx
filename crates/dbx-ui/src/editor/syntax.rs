//! Syntax, statement selection, formatting and completion analysis.
use super::*;
use dbx_core::DatabaseKind;

/// The connected engine whose comment, quoting and statement rules apply.
/// `None` is permissive: it accepts every dialect's comments and quotes.
pub type SqlDialect = Option<DatabaseKind>;

/// The language used when painting an editor's text.
///
/// Editors remain plain text by default.  SQL highlighting is opt-in so the
/// same editor can continue to be used for connection strings, filters, and
/// editable cells without paying for a lexer pass or changing their colors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorLanguage {
    #[default]
    PlainText,
    /// SQL lexed with the connection's dialect rules.
    Sql(SqlDialect),
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
    dialect: SqlDialect,
) -> Range<usize> {
    let selection = clamp_range(text, selection);
    if !selection.is_empty() {
        return selection;
    }
    match scope {
        QueryExecutionScope::Document => 0..text.len(),
        QueryExecutionScope::SelectionOrStatement => sql_statement_range(text, cursor, dialect),
        QueryExecutionScope::SelectionOrCurrentLine => {
            let cursor = clamp_boundary(text, cursor);
            line_start(text, cursor)..line_end(text, cursor)
        }
    }
}

/// Return the SQL statement surrounding `cursor`. Statements are the
/// engine's own spans for `dialect`, so the range is exactly what it runs.
#[allow(dead_code)]
pub fn sql_statement_range(text: &str, cursor: usize, dialect: SqlDialect) -> Range<usize> {
    let cursor = clamp_boundary(text, cursor);
    let spans = dbx_core::sql_statement_spans(dialect, text);
    // A caret immediately after a terminator belongs to the next statement;
    // a caret on the terminator itself remains with the preceding statement.
    // If there is no next statement (only trailing whitespace), keep the last
    // executable statement active instead of returning an empty range.
    let index = spans
        .iter()
        .position(|span| cursor < span.range.end)
        .unwrap_or(spans.len().saturating_sub(1));
    let candidate = spans
        .get(index)
        .map_or(0..text.len(), |span| span.range.clone());
    if !text[candidate.clone()].trim().is_empty() {
        return candidate;
    }
    spans[..index]
        .iter()
        .rev()
        .map(|span| span.range.clone())
        .find(|range| !text[range.clone()].trim().is_empty())
        .unwrap_or(candidate)
}

/// The statement delimiter in force at `position`: `;` unless a MySQL
/// `DELIMITER` directive changed it. Running a statement from inside such a
/// block needs the directive repeated in front of it.
pub fn sql_statement_delimiter(text: &str, position: usize, dialect: SqlDialect) -> String {
    dbx_core::sql_statement_spans(dialect, text)
        .into_iter()
        .find(|span| position < span.range.end)
        .map_or_else(|| ";".to_owned(), |span| span.delimiter)
}

/// Count non-empty SQL statements while ignoring semicolons in lexical
/// regions such as quoted strings, comments, and dollar-quoted bodies.
#[allow(dead_code)]
pub fn sql_statement_count(text: &str, dialect: SqlDialect) -> usize {
    sql_statement_ranges(text, dialect)
        .into_iter()
        .filter(|range| {
            lex_sql_for(&text[range.clone()], dialect)
                .iter()
                .any(|token| token.kind != SqlTokenKind::Comment)
        })
        .count()
}

/// Classify a SQL script conservatively. Any risky statement determines the
/// script's result; unfamiliar syntax is deliberately treated as mutation
/// risk rather than read-only.
#[allow(dead_code)]
pub fn sql_execution_kind(text: &str, dialect: SqlDialect) -> SqlExecutionKind {
    sql_statement_ranges(text, dialect)
        .into_iter()
        .filter_map(|range| sql_statement_kind(&text[range], dialect))
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
pub fn sql_may_change_schema(text: &str, dialect: SqlDialect) -> bool {
    sql_statement_ranges(text, dialect)
        .into_iter()
        .any(|range| {
            sql_words_with_depth(&text[range], dialect)
                .iter()
                .any(|(word, _)| {
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

pub(super) fn sql_statement_ranges(text: &str, dialect: SqlDialect) -> Vec<Range<usize>> {
    dbx_core::sql_statement_spans(dialect, text)
        .into_iter()
        .map(|span| span.range)
        .filter(|range| !text[range.clone()].trim().is_empty())
        .collect()
}

pub(super) fn sql_statement_kind(text: &str, dialect: SqlDialect) -> Option<SqlExecutionKind> {
    let words = sql_words_with_depth(text, dialect);
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

/// Return identifier-like tokens with their parenthesis depth. Parentheses
/// only count between tokens, so strings, comments, quoted identifiers and
/// dollar-quoted bodies never change the depth.
pub(super) fn sql_words_with_depth(text: &str, dialect: SqlDialect) -> Vec<(String, usize)> {
    let mut depth = 0usize;
    let mut previous_end = 0;
    let mut words = Vec::new();
    for token in lex_sql_for(text, dialect) {
        for byte in text[previous_end..token.range.start].bytes() {
            match byte {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        previous_end = token.range.end;
        if token.kind != SqlTokenKind::Comment {
            words.push((text[token.range].to_ascii_uppercase(), depth));
        }
    }
    words
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
pub(super) struct HighlightToken {
    pub(super) range: Range<usize>,
    pub(super) kind: HighlightTokenKind,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum HighlightTokenKind {
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
    pub(super) fn color(&self) -> gpui::Hsla {
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
#[cfg(test)]
pub fn lex_sql(text: &str) -> Vec<SqlToken> {
    lex_sql_for(text, None)
}

/// Lex SQL using `dialect`'s comment and string rules, so `#` is an operator
/// in PostgreSQL and `$$` is a MySQL `DELIMITER` rather than a string.
pub fn lex_sql_for(text: &str, dialect: SqlDialect) -> Vec<SqlToken> {
    let hash_comments = dbx_core::sql_hash_comments(dialect);
    let dollar_quotes = dbx_core::sql_dollar_quotes(dialect);
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

        if character == '#' && hash_comments {
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
            let escapes = dbx_core::sql_backslash_escapes(dialect, &text[..start]);
            let end = consume_quoted(&chars, index, character, escapes);
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
            // MySQL reads double quotes as escaped strings unless ANSI_QUOTES
            // is set; either way a backslash cannot end the quoted run there.
            let escapes = character == '"'
                && dialect.is_some_and(|kind| kind.dialect() == DatabaseKind::MySQL);
            let end = consume_quoted(&chars, index, character, escapes);
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
            if dollar_quotes && let Some(end) = consume_dollar_quoted(&chars, text, index) {
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

pub(super) fn lex_redis_line(line: &str, line_start: usize, tokens: &mut Vec<RedisToken>) {
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

pub(super) fn redis_token_kind(raw: &str, token_index: usize) -> RedisTokenKind {
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
#[cfg(test)]
pub fn sql_completion_context(text: &str, cursor: usize) -> Option<SqlCompletionContext> {
    sql_completion_context_for(text, cursor, None)
}

/// [`sql_completion_context`] with `dialect`'s comment and string rules.
pub fn sql_completion_context_for(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
) -> Option<SqlCompletionContext> {
    let cursor = clamp_boundary(text, cursor);
    if cursor == 0 {
        return None;
    }

    let tokens = lex_sql_for(text, dialect);
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
pub(super) fn quoted_snippets(lowered_message: &str) -> Vec<String> {
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

pub(super) fn sql_error_position_range(lowered_message: &str, query: &str) -> Option<Range<usize>> {
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
pub(super) fn quoted_after(lowered_message: &str, markers: &[&str]) -> Option<String> {
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

pub(super) fn missing_column_name(lowered_message: &str) -> Option<String> {
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
pub(super) fn find_word_in_query(query: &str, needle: &str) -> Option<Range<usize>> {
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
pub(super) fn expand_to_identifier(text: &str, at: usize) -> Option<Range<usize>> {
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
#[cfg(test)]
pub fn format_sql(text: &str) -> String {
    format_sql_for(text, None)
}

/// [`format_sql`] with `dialect`'s comment and string rules.
pub fn format_sql_for(text: &str, dialect: SqlDialect) -> String {
    SqlFormatter::new(text, dialect).run()
}

/// Format `text` and map `cursor` to the equivalent offset in the output.
///
/// The formatter preserves every token exactly once and in order, so offsets
/// are remapped by aligning the two lexings instead of diffing strings.
pub fn format_sql_at_cursor(text: &str, cursor: usize, dialect: SqlDialect) -> (String, usize) {
    let formatted = format_sql_for(text, dialect);
    let old_tokens = lex_sql_for(text, dialect);
    let new_tokens = lex_sql_for(&formatted, dialect);
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
pub(super) enum AtomKind<'a> {
    Token(SqlTokenKind),
    /// A punctuation atom. Grouping characters (`(` `)` `,` `;` `.`) stand
    /// alone; every other punctuation run merges into one operator slice so
    /// `::`, `<=`, and `||` survive formatting intact.
    Punct(&'a str),
}

pub(super) enum ClauseBreak {
    /// Break onto a new line at the enclosing indent.
    Clause,
    /// Break with one extra indent level (AND/OR predicates).
    Predicate,
}

/// Split the whitespace-only gap between tokens into punctuation atoms,
/// keeping operator runs together.
pub(super) fn push_gap_atoms<'a>(atoms: &mut Vec<(AtomKind<'a>, &'a str)>, gap: &'a str) {
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

pub(super) struct SqlFormatter<'a> {
    pub(super) atoms: Vec<(AtomKind<'a>, &'a str)>,
    pub(super) out: String,
    pub(super) line: String,
    /// One entry per unclosed parenthesis; `true` marks a subquery body
    /// (an opening paren directly followed by SELECT/WITH). Raw length
    /// drives comma breaking, the `true` count drives indentation.
    pub(super) parens: Vec<bool>,
    /// Raw paren depth of the most recent clause keyword; top-level commas
    /// break onto new lines at this depth.
    pub(super) clause_depth: usize,
    /// Set while a JOIN phrase (LEFT OUTER JOIN…) is being emitted so later
    /// modifiers do not each force another line break.
    pub(super) join_phrase_open: bool,
    /// Extra indent units owed to the line currently being built (AND/OR
    /// predicates sit one level deeper than their clause keyword).
    pub(super) line_extra: usize,
    /// Subquery indent units captured when the current line started. A
    /// closing paren may appear later on the same line, so indentation must
    /// not be recomputed at flush time.
    pub(super) line_base: usize,
    pub(super) pending_blank_line: bool,
}

impl<'a> SqlFormatter<'a> {
    pub(super) fn new(text: &'a str, dialect: SqlDialect) -> Self {
        let mut atoms = Vec::new();
        let mut previous_end = 0;
        for token in lex_sql_for(text, dialect) {
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

    pub(super) fn raw_depth(&self) -> usize {
        self.parens.len()
    }

    /// Indent units contributed by enclosing subqueries.
    pub(super) fn subquery_depth(&self) -> usize {
        self.parens.iter().filter(|open| **open).count()
    }

    pub(super) fn flush(&mut self) {
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

    pub(super) fn previous_atom(&self, index: usize) -> Option<AtomKind<'_>> {
        self.atoms.get(index.wrapping_sub(1)).map(|(kind, _)| *kind)
    }

    /// Decide whether the keyword at `index` forces a line break and how deep
    /// that line sits. Returns the extra indent units, if any.
    pub(super) fn break_before(
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

    pub(super) fn run(mut self) -> String {
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
    pub(super) fn append_operator(&mut self, index: usize, operator: &str) {
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
pub(super) fn followed_by_open_paren(atoms: &[(AtomKind<'_>, &str)], index: usize) -> bool {
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
pub(super) fn join_word_starts_phrase(atoms: &[(AtomKind, &str)], index: usize) -> bool {
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

pub(super) fn is_join_modifier(word: &str) -> bool {
    matches!(
        word,
        "inner" | "outer" | "left" | "right" | "full" | "cross" | "natural"
    )
}

pub(super) fn clause_break_kind(word: &str) -> Option<ClauseBreak> {
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

pub(super) const SQL_FUNCTIONS: &[&str] = &[
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

pub(super) fn completion_identifier_start(text: &str, cursor: usize) -> usize {
    let cursor = clamp_boundary(text, cursor);
    text[..cursor]
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!is_identifier_continue(character)).then_some(index + character.len_utf8())
        })
        .unwrap_or(0)
}

pub(super) fn completion_identifier_end(text: &str, cursor: usize) -> usize {
    let cursor = clamp_boundary(text, cursor);
    text[cursor..]
        .char_indices()
        .find_map(|(index, character)| {
            (!is_identifier_continue(character)).then_some(cursor + index)
        })
        .unwrap_or(text.len())
}

pub(super) struct QuotedCompletionIdentifier {
    pub(super) prefix: String,
    pub(super) quote: char,
    pub(super) qualifier_start: usize,
    pub(super) range: Range<usize>,
}

/// Return the quoted identifier contents around `cursor`. Completion
/// candidates intentionally insert raw identifier text when `quote` is set,
/// so the delimiters stay outside the replacement range.
///
/// An unfinished identifier deliberately extends through the lexer token. The
/// tolerant lexer keeps it available while the user is still typing, and a
/// selected candidate retains the existing incomplete-quote behavior.
pub(super) fn quoted_completion_identifier(
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

pub(super) fn quoted_identifier_is_closed(raw: &str, quote: char) -> bool {
    let mut characters = raw.chars();
    let opening_quote = characters.next();
    debug_assert_eq!(opening_quote, Some(quote));
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

pub(super) fn completion_qualifier(text: &str, start: usize) -> Option<(String, usize)> {
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

pub(super) fn completion_qualifier_segment(text: &str, end: usize) -> Option<(usize, String)> {
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

pub(super) fn previous_sql_word(text: &str, before: usize) -> Option<String> {
    let before = clamp_boundary(text, before);
    let end = text[..before]
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())
        .map_or(before, |(index, character)| index + character.len_utf8());
    let start = completion_identifier_start(text, end);
    (start < end).then(|| text[start..end].to_ascii_lowercase())
}

pub(super) fn is_table_completion_keyword(word: &str) -> bool {
    matches!(
        word,
        "from" | "join" | "update" | "into" | "table" | "view" | "references"
    )
}

pub(super) fn is_column_completion_keyword(word: &str) -> bool {
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

pub(super) fn next_char(chars: &[(usize, char)], index: usize) -> Option<char> {
    chars.get(index + 1).map(|(_, character)| *character)
}

pub(super) fn byte_end(chars: &[(usize, char)], index: usize, text_len: usize) -> usize {
    chars.get(index).map_or(text_len, |(offset, _)| *offset)
}

pub(super) fn push_sql_token(
    tokens: &mut Vec<SqlToken>,
    kind: SqlTokenKind,
    start: usize,
    end: usize,
) {
    if start < end {
        tokens.push(SqlToken {
            kind,
            range: start..end,
        });
    }
}

pub(super) fn push_json_token(
    tokens: &mut Vec<JsonToken>,
    kind: JsonTokenKind,
    start: usize,
    end: usize,
) {
    if start < end {
        tokens.push(JsonToken {
            kind,
            range: start..end,
        });
    }
}

pub(super) fn consume_json_string(chars: &[(usize, char)], mut index: usize) -> usize {
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

pub(super) fn consume_json_number(chars: &[(usize, char)], mut index: usize) -> Option<usize> {
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

pub(super) fn consume_quoted(
    chars: &[(usize, char)],
    mut index: usize,
    quote: char,
    escapes: bool,
) -> usize {
    index += 1;
    while index < chars.len() {
        let character = chars[index].1;
        if character == '\\' && escapes {
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

pub(super) fn consume_dollar_quoted(
    chars: &[(usize, char)],
    text: &str,
    index: usize,
) -> Option<usize> {
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

pub(super) fn consume_parameter(chars: &[(usize, char)], index: usize) -> Option<usize> {
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

pub(super) fn consume_number(chars: &[(usize, char)], mut index: usize) -> usize {
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

pub(super) fn consume_identifier(chars: &[(usize, char)], mut index: usize) -> usize {
    index += 1;
    while index < chars.len() && is_identifier_continue(chars[index].1) {
        index += 1;
    }
    index
}

pub(super) fn char_index_at_or_after(chars: &[(usize, char)], byte_offset: usize) -> usize {
    chars
        .binary_search_by_key(&byte_offset, |(offset, _)| *offset)
        .unwrap_or_else(|index| index)
}

pub(super) fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

pub(super) fn is_identifier_continue(character: char) -> bool {
    character == '_' || character == '$' || character.is_alphanumeric()
}

pub(super) fn is_sql_keyword(word: &str) -> bool {
    SQL_KEYWORDS
        .iter()
        .any(|keyword| keyword.eq_ignore_ascii_case(word))
}

pub(super) fn is_sql_type(word: &str) -> bool {
    SQL_TYPES
        .iter()
        .any(|sql_type| sql_type.eq_ignore_ascii_case(word))
}

pub(super) const SQL_KEYWORDS: &[&str] = &[
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

pub(super) const SQL_TYPES: &[&str] = &[
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
