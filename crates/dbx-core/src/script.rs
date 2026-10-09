//! SQL script splitting shared by the console, imports and the editor.
//!
//! One scanner decides where statements end, so the statement the editor
//! highlights and runs is exactly the statement the engine executes.

use std::{
    collections::VecDeque,
    io::{self, BufRead, Read},
    ops::Range,
};

use crate::{DatabaseKind, DbxError, Result};

fn io_error(error: io::Error) -> DbxError {
    DbxError::Io(error.to_string())
}

/// Split a SQL script into individual statements.
///
/// Understands single/double-quoted strings and backtick identifiers,
/// `--`/`#` line comments, nested block comments, PostgreSQL dollar-quoted
/// strings, and MySQL-style `DELIMITER` directives so routine bodies from
/// real-world dumps survive intact. Trailing content without a terminator is
/// returned as one final statement when it is not blank. The function never
/// fails: malformed scripts simply produce statements the engine will
/// reject with its own diagnostics.
pub fn split_sql_statements(script: &str) -> Vec<String> {
    checked_split_sql_statements(script).unwrap_or_else(|_| vec![script.to_owned()])
}
/// [`split_sql_statements`] using one engine's comment and quoting rules.
pub(crate) fn split_sql_statements_for(kind: DatabaseKind, script: &str) -> Vec<String> {
    checked_split_sql_for(Some(kind), script).unwrap_or_else(|_| vec![script.to_owned()])
}
pub(crate) fn checked_split_sql_statements(script: &str) -> Result<Vec<String>> {
    checked_split_sql_for(None, script)
}
pub fn checked_split_sql_for(kind: Option<DatabaseKind>, script: &str) -> Result<Vec<String>> {
    if kind == Some(DatabaseKind::SqlServer) {
        return split_sql_server_script(script);
    }
    let mut reader = SqlScriptReader::with_kind(script.as_bytes(), kind);
    let mut statements = Vec::new();
    while let Some(statement) = reader.next_statement().map_err(io_error)? {
        statements.push(statement);
    }
    Ok(statements)
}

/// Whether `#` starts a line comment. PostgreSQL uses it for operators
/// (`#`, `#>`, `#>>`, `#-`), T-SQL for temporary tables, and SQLite, DuckDB
/// and Snowflake have no such comment; treating it as one there would drop
/// the rest of the line from the executed statement.
pub fn sql_hash_comments(kind: Option<DatabaseKind>) -> bool {
    matches!(
        kind.map(DatabaseKind::dialect),
        None | Some(DatabaseKind::MySQL | DatabaseKind::BigQuery | DatabaseKind::ClickHouse)
    )
}

/// Whether `$tag$ ... $tag$` quotes a string. MySQL scripts commonly use
/// `$$` as a `DELIMITER`, and SQLite and T-SQL have no such strings.
pub fn sql_dollar_quotes(kind: Option<DatabaseKind>) -> bool {
    matches!(
        kind.map(DatabaseKind::dialect),
        None | Some(
            DatabaseKind::PostgreSQL
                | DatabaseKind::DuckDB
                | DatabaseKind::Snowflake
                | DatabaseKind::ClickHouse
        )
    )
}

/// Whether a single-quoted string starting after `prefix` treats backslash
/// as an escape. PostgreSQL and DuckDB do so only for `E'...'` strings.
pub fn sql_backslash_escapes(kind: Option<DatabaseKind>, prefix: &str) -> bool {
    match kind.map(DatabaseKind::dialect) {
        None
        | Some(
            DatabaseKind::MySQL
            | DatabaseKind::BigQuery
            | DatabaseKind::ClickHouse
            | DatabaseKind::Snowflake,
        ) => true,
        Some(DatabaseKind::PostgreSQL | DatabaseKind::DuckDB) => {
            let bytes = prefix.as_bytes();
            bytes.last().is_some_and(|byte| matches!(byte, b'e' | b'E'))
                && (bytes.len() < 2
                    || !(bytes[bytes.len() - 2].is_ascii_alphanumeric()
                        || bytes[bytes.len() - 2] == b'_'))
        }
        _ => false,
    }
}

/// One statement's source within a script: its byte range, including the
/// whitespace and comments before it and its terminator, and the delimiter
/// in force where it starts (MySQL `DELIMITER` scripts).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptSpan {
    pub range: Range<usize>,
    pub delimiter: String,
}

impl ScriptSpan {
    /// The span's text as a standalone script. A statement inside a
    /// `DELIMITER` block keeps its delimiter so routine bodies stay whole.
    pub fn executable_text<'a>(&self, script: &'a str) -> std::borrow::Cow<'a, str> {
        let text = &script[self.range.clone()];
        if self.delimiter == ";" {
            text.into()
        } else {
            format!("DELIMITER {}\n{text}", self.delimiter).into()
        }
    }
}

/// Partition `script` into consecutive statement spans using `kind`'s
/// comment, quoting, block-body and batch rules. The spans cover the whole
/// script; the last one holds any unterminated remainder. This never fails:
/// unsplittable input is returned as a single span.
pub fn sql_statement_spans(kind: Option<DatabaseKind>, script: &str) -> Vec<ScriptSpan> {
    let mut spans = Vec::new();
    if kind == Some(DatabaseKind::SqlServer) {
        sql_server_spans(script, &mut spans);
    } else {
        collect_spans(kind, script, 0, &mut spans);
    }
    spans
}

fn collect_spans(
    kind: Option<DatabaseKind>,
    script: &str,
    base: usize,
    spans: &mut Vec<ScriptSpan>,
) {
    let mut reader = SqlScriptReader::with_kind(script.as_bytes(), kind);
    reader.lenient = true;
    reader.boundaries = Some(Vec::new());
    while let Ok(Some(_)) = reader.next_statement() {}
    let mut start = 0;
    let mut delimiter = ";".to_owned();
    for (end, next) in reader.boundaries.take().unwrap_or_default() {
        if end <= start || end > script.len() {
            continue;
        }
        spans.push(ScriptSpan {
            range: base + start..base + end,
            delimiter: std::mem::replace(&mut delimiter, next),
        });
        start = end;
    }
    if start < script.len() || spans.is_empty() {
        spans.push(ScriptSpan {
            range: base + start..base + script.len(),
            delimiter,
        });
    }
}

/// T-SQL batches end at `GO` lines; module batches are single statements.
fn sql_server_spans(script: &str, spans: &mut Vec<ScriptSpan>) {
    let mut batch_start = 0;
    let mut line_start = 0;
    let push_batch =
        |batch_start: usize, batch_end: usize, end: usize, spans: &mut Vec<ScriptSpan>| {
            let batch = &script[batch_start..batch_end];
            if sql_server_module(batch) {
                spans.push(ScriptSpan {
                    range: batch_start..end,
                    delimiter: ";".into(),
                });
                return;
            }
            let first = spans.len();
            collect_spans(Some(DatabaseKind::SqlServer), batch, batch_start, spans);
            // The `GO` line, and blank space before it, ends the last statement.
            if spans.len() > first + 1
                && script[spans[spans.len() - 1].range.clone()]
                    .trim()
                    .is_empty()
            {
                spans.pop();
            }
            if spans.len() > first
                && let Some(last) = spans.last_mut()
            {
                last.range.end = end;
            }
        };
    for line in script.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if line.trim().eq_ignore_ascii_case("go") {
            push_batch(batch_start, line_start, line_end, spans);
            batch_start = line_end;
        }
        line_start = line_end;
    }
    if batch_start < script.len() || spans.is_empty() {
        push_batch(batch_start, script.len(), script.len(), spans);
    }
}

/// T-SQL module bodies (procedures, functions, triggers, views) contain
/// semicolons but must reach the server as one batch, ended by `GO`.
fn split_sql_server_script(script: &str) -> Result<Vec<String>> {
    let mut batches = vec![String::new()];
    for line in script.split_inclusive('\n') {
        if line.trim().eq_ignore_ascii_case("go") {
            batches.push(String::new());
        } else if let Some(batch) = batches.last_mut() {
            batch.push_str(line);
        }
    }
    let mut statements = Vec::new();
    for batch in batches {
        if sql_server_module(&batch) {
            let trimmed = batch.trim();
            if !trimmed.is_empty() {
                statements.push(trimmed.to_owned());
            }
            continue;
        }
        let mut reader =
            SqlScriptReader::with_kind(batch.as_bytes(), Some(DatabaseKind::SqlServer));
        while let Some(statement) = reader.next_statement().map_err(io_error)? {
            statements.push(statement);
        }
    }
    Ok(statements)
}

fn sql_server_module(batch: &str) -> bool {
    let words = sql_words_for(batch);
    matches!(words.first().map(String::as_str), Some("CREATE" | "ALTER"))
        && words
            .iter()
            .skip(1)
            .find(|word| !matches!(word.as_str(), "OR" | "ALTER"))
            .is_some_and(|word| {
                matches!(
                    word.as_str(),
                    "PROCEDURE" | "PROC" | "FUNCTION" | "TRIGGER" | "VIEW"
                )
            })
}

/// The first few upper-cased words of a batch, skipping leading comments.
fn sql_words_for(batch: &str) -> Vec<String> {
    let mut text = batch.trim_start();
    loop {
        if let Some(rest) = text.strip_prefix("--") {
            text = rest
                .split_once('\n')
                .map_or("", |(_, rest)| rest)
                .trim_start();
        } else if let Some(rest) = text.strip_prefix("/*") {
            text = rest
                .split_once("*/")
                .map_or("", |(_, rest)| rest)
                .trim_start();
        } else {
            break;
        }
    }
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .take(4)
        .map(str::to_ascii_uppercase)
        .collect()
}

#[derive(Clone)]
enum ScriptState {
    Normal,
    LineComment,
    BlockComment(usize, bool),
    SingleQuote(bool),
    DoubleQuote,
    Backtick,
    Dollar(String),
}

pub(crate) const MAX_TRANSFER_RECORD_BYTES: usize = 64 * 1024 * 1024;
pub(crate) struct SqlScriptReader<R> {
    kind: Option<DatabaseKind>,
    input: R,
    state: ScriptState,
    delimiter: String,
    /// Upper-cased leading words of the current statement, enough to
    /// recognise a routine or trigger header.
    words: Vec<String>,
    /// Whether the current statement is a routine/trigger whose body is a
    /// `BEGIN ... END` block with its own `;` terminators.
    block: bool,
    block_depth: usize,
    /// The previous word was `END`; `END IF`/`END LOOP`/... close constructs
    /// that never opened a counted block.
    after_end: bool,
    /// Byte offset of the line being fed, and the end offset of every
    /// terminated statement with the delimiter that follows it.
    offset: usize,
    boundaries: Option<Vec<(usize, String)>>,
    /// Treat executable MySQL comments as comments instead of refusing them;
    /// only boundaries are wanted, nothing is executed.
    lenient: bool,
    current: String,
    ready: VecDeque<String>,
    at_line_start: bool,
    first_line: bool,
    ended: bool,
}
impl<R: BufRead> SqlScriptReader<R> {
    pub(crate) fn with_kind(input: R, kind: Option<DatabaseKind>) -> Self {
        Self {
            kind,
            input,
            state: ScriptState::Normal,
            delimiter: ";".into(),
            words: Vec::new(),
            block: false,
            block_depth: 0,
            after_end: false,
            offset: 0,
            boundaries: None,
            lenient: false,
            current: String::new(),
            ready: VecDeque::new(),
            at_line_start: true,
            first_line: true,
            ended: false,
        }
    }
    pub(crate) fn next_statement(&mut self) -> io::Result<Option<String>> {
        loop {
            if let Some(statement) = self.ready.pop_front() {
                return Ok(Some(statement));
            }
            if self.ended {
                return Ok(None);
            }
            let mut line = String::new();
            let count = (&mut self.input)
                .take((MAX_TRANSFER_RECORD_BYTES + 1) as u64)
                .read_line(&mut line)?;
            if line.len() > MAX_TRANSFER_RECORD_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SQL line exceeds the 64 MiB transfer budget",
                ));
            }
            if count == 0 {
                self.ended = true;
                let remaining = std::mem::take(&mut self.current);
                if !remaining.trim().is_empty() {
                    return Ok(Some(remaining.trim().to_owned()));
                }
                return Ok(None);
            }
            if self.first_line {
                let length = line.len();
                line = line.trim_start_matches('\u{feff}').to_owned();
                self.offset += length - line.len();
                self.first_line = false;
            }
            self.feed(&line)?;
            self.offset += line.len();
        }
    }
    fn end_statement(&mut self, end: usize, delimiter: &str) {
        self.words.clear();
        self.block = false;
        self.block_depth = 0;
        self.after_end = false;
        if let Some(boundaries) = &mut self.boundaries {
            boundaries.push((end, delimiter.to_owned()));
        }
    }

    /// Follow `BEGIN ... END` nesting inside routine and trigger bodies so
    /// their inner terminators stay part of the statement.
    fn track_word(&mut self, word: String) {
        if !self.block && self.words.len() < 8 {
            self.words.push(word.clone());
            self.block = block_statement(self.kind, &self.words);
        }
        if !self.block {
            return;
        }
        if std::mem::take(&mut self.after_end) {
            match word.as_str() {
                // `END IF`, `END LOOP`, ... close constructs that were never
                // counted, so undo the plain `END`.
                "IF" | "LOOP" | "WHILE" | "REPEAT" | "FOR" => self.block_depth += 1,
                // `END CASE` closes the counted `CASE` statement.
                "CASE" => {}
                _ => self.track_block_word(&word),
            }
        } else {
            self.track_block_word(&word);
        }
    }

    fn track_block_word(&mut self, word: &str) {
        match word {
            "BEGIN" | "CASE" => self.block_depth += 1,
            "END" => {
                self.block_depth = self.block_depth.saturating_sub(1);
                self.after_end = true;
            }
            _ => {}
        }
    }

    fn feed(&mut self, script: &str) -> io::Result<()> {
        let characters: Vec<char> = script.chars().collect();
        let byte_at = script
            .char_indices()
            .map(|(position, _)| position)
            .chain([script.len()])
            .collect::<Vec<_>>();
        let mut statements = Vec::new();
        let mut current = std::mem::take(&mut self.current);
        let mut delimiter = self.delimiter.clone();
        let mut state = self.state.clone();
        let mut at_line_start = self.at_line_start;
        let mut index = 0usize;
        while index < characters.len() {
            if current.len() > MAX_TRANSFER_RECORD_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SQL statement exceeds the 64 MiB transfer budget",
                ));
            }
            let character = characters[index];
            let matches_here = |needle: &str, at: usize| {
                characters[at..].starts_with(needle.chars().collect::<Vec<_>>().as_slice())
            };
            match state.clone() {
                ScriptState::Normal => {
                    if at_line_start
                        && current.trim().is_empty()
                        && matches_here_ci(&characters, index, "delimiter")
                        && characters
                            .get(index + "delimiter".len())
                            .is_some_and(|next| next.is_whitespace())
                    {
                        index += "delimiter".len();
                        let mut token = String::new();
                        while let Some(&next) = characters.get(index) {
                            if next == '\n' || next == '\r' {
                                break;
                            }
                            token.push(next);
                            index += 1;
                        }
                        let trimmed = token.trim();
                        if !trimmed.is_empty() {
                            delimiter = trimmed.to_owned();
                        }
                        continue;
                    }
                    // T-SQL clients separate batches with a line holding
                    // only GO; it is not SQL the server understands.
                    if self.kind == Some(DatabaseKind::SqlServer)
                        && matches_here_ci(&characters, index, "go")
                        && characters[..index].iter().all(|c| c.is_whitespace())
                        && characters[index + 2..].iter().all(|c| c.is_whitespace())
                    {
                        let trimmed = current.trim();
                        if !trimmed.is_empty() {
                            statements.push(trimmed.to_owned());
                        }
                        current.clear();
                        self.end_statement(self.offset + script.len(), &delimiter);
                        at_line_start = true;
                        index = characters.len();
                        continue;
                    }
                    if character == '-' && matches_here("--", index) {
                        state = ScriptState::LineComment;
                        index += 2;
                        continue;
                    }
                    if character == '#' && sql_hash_comments(self.kind) {
                        state = ScriptState::LineComment;
                        index += 1;
                        continue;
                    }
                    if character == '/' && matches_here("/*", index) {
                        if matches_here("/*!", index) && !self.lenient {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Expand executable MySQL version comments before running or importing a script",
                            ));
                        }
                        let preserve = matches_here("/*+", index);
                        if preserve {
                            current.push_str("/*");
                        } else {
                            current.push(' ');
                        }
                        state = ScriptState::BlockComment(1, preserve);
                        index += 2;
                        continue;
                    }
                    if delimiter == ";" && character.is_ascii_alphabetic() {
                        let start = index;
                        while characters.get(index).is_some_and(|character| {
                            character.is_ascii_alphanumeric() || *character == '_'
                        }) {
                            index += 1;
                        }
                        let word = characters[start..index].iter().collect::<String>();
                        self.track_word(word.to_ascii_uppercase());
                        current.push_str(&word);
                        at_line_start = false;
                        continue;
                    }
                    if matches_here(&delimiter, index) && (!self.block || self.block_depth == 0) {
                        let trimmed = current.trim();
                        if !trimmed.is_empty() {
                            statements.push(trimmed.to_owned());
                        }
                        current.clear();
                        index += delimiter.chars().count();
                        self.end_statement(self.offset + byte_at[index], &delimiter);
                        at_line_start = true;
                        continue;
                    }
                    if !character.is_whitespace() {
                        self.after_end = false;
                    }
                    match character {
                        '\'' => {
                            state = ScriptState::SingleQuote(sql_backslash_escapes(
                                self.kind, &current,
                            ));
                            current.push(character);
                            index += 1;
                        }
                        '"' => {
                            state = ScriptState::DoubleQuote;
                            current.push(character);
                            index += 1;
                        }
                        '`' => {
                            state = ScriptState::Backtick;
                            current.push(character);
                            index += 1;
                        }
                        '$' if sql_dollar_quotes(self.kind) => {
                            if let Some(tag) = parse_dollar_tag(&characters[index..]) {
                                let token_length = tag.chars().count() + 2;
                                current.extend(characters[index..index + token_length].iter());
                                index += token_length;
                                state = ScriptState::Dollar(tag);
                            } else {
                                current.push(character);
                                index += 1;
                            }
                        }
                        _ => {
                            if !character.is_whitespace() {
                                at_line_start = false;
                            }
                            current.push(character);
                            index += 1;
                        }
                    }
                }
                ScriptState::LineComment => {
                    if character == '\n' {
                        state = ScriptState::Normal;
                        at_line_start = true;
                        current.push(character);
                    }
                    index += 1;
                }
                ScriptState::BlockComment(depth, preserve) => {
                    if character == '/' && matches_here("/*", index) {
                        if preserve {
                            current.push_str("/*");
                        }
                        state = ScriptState::BlockComment(depth + 1, preserve);
                        index += 2;
                    } else if character == '*' && matches_here("*/", index) {
                        if preserve {
                            current.push_str("*/");
                        }
                        state = if depth <= 1 {
                            ScriptState::Normal
                        } else {
                            ScriptState::BlockComment(depth - 1, preserve)
                        };
                        index += 2;
                    } else {
                        if preserve {
                            current.push(character);
                        }
                        index += 1;
                    }
                }
                ScriptState::SingleQuote(escape) => {
                    current.push(character);
                    if character == '\\' && escape {
                        if let Some(&next) = characters.get(index + 1) {
                            current.push(next);
                            index += 2;
                            continue;
                        }
                    } else if character == '\'' {
                        if characters.get(index + 1) == Some(&'\'') {
                            current.push('\'');
                            index += 2;
                            continue;
                        }
                        state = ScriptState::Normal;
                    }
                    index += 1;
                }
                ScriptState::DoubleQuote => {
                    current.push(character);
                    if character == '"' {
                        if characters.get(index + 1) == Some(&'"') {
                            current.push('"');
                            index += 2;
                            continue;
                        }
                        state = ScriptState::Normal;
                    }
                    index += 1;
                }
                ScriptState::Backtick => {
                    current.push(character);
                    if character == '`' {
                        if characters.get(index + 1) == Some(&'`') {
                            current.push('`');
                            index += 2;
                            continue;
                        }
                        state = ScriptState::Normal;
                    }
                    index += 1;
                }
                ScriptState::Dollar(tag) => {
                    let closing = format!("${tag}$");
                    if matches_here(&closing, index) {
                        current.push_str(&closing);
                        index += closing.chars().count();
                        state = ScriptState::Normal;
                    } else {
                        current.push(character);
                        index += 1;
                    }
                }
            }
        }

        self.current = current;
        self.state = state;
        self.delimiter = delimiter;
        self.at_line_start = at_line_start;
        self.ready.extend(statements);
        Ok(())
    }
}

/// Whether a statement's leading words make it a routine or trigger whose
/// body is a `BEGIN ... END` block in this dialect. The object keyword is
/// the first one after `CREATE` and its modifiers, before the object's name.
fn block_statement(kind: Option<DatabaseKind>, words: &[String]) -> bool {
    if words.first().map(String::as_str) != Some("CREATE") {
        return false;
    }
    let Some(object) = words[1..].iter().map(String::as_str).find(|word| {
        matches!(
            *word,
            "TABLE"
                | "VIEW"
                | "INDEX"
                | "UNIQUE"
                | "DATABASE"
                | "SCHEMA"
                | "USER"
                | "ROLE"
                | "SEQUENCE"
                | "TYPE"
                | "DOMAIN"
                | "EXTENSION"
                | "MATERIALIZED"
                | "POLICY"
                | "RULE"
                | "PROCEDURE"
                | "FUNCTION"
                | "TRIGGER"
                | "EVENT"
        )
    }) else {
        return false;
    };
    match kind.map(DatabaseKind::dialect) {
        // SQLite triggers; PostgreSQL SQL-standard `BEGIN ATOMIC` bodies;
        // MySQL and BigQuery compound statements without `DELIMITER`.
        Some(DatabaseKind::SQLite) => object == "TRIGGER",
        Some(DatabaseKind::PostgreSQL) => matches!(object, "FUNCTION" | "PROCEDURE"),
        Some(DatabaseKind::MySQL) => {
            matches!(object, "PROCEDURE" | "FUNCTION" | "TRIGGER" | "EVENT")
        }
        Some(DatabaseKind::BigQuery) => object == "PROCEDURE",
        None => matches!(object, "PROCEDURE" | "FUNCTION" | "TRIGGER" | "EVENT"),
        _ => false,
    }
}

fn matches_here_ci(characters: &[char], index: usize, needle: &str) -> bool {
    needle.chars().enumerate().all(|(offset, expected)| {
        characters
            .get(index + offset)
            .is_some_and(|found| found.eq_ignore_ascii_case(&expected))
    })
}

/// Parse `$tag$` starting at `characters[0]`, returning the inner tag when
/// the shape matches. The empty tag (`$$`) is valid PostgreSQL syntax.
fn parse_dollar_tag(characters: &[char]) -> Option<String> {
    let mut tag = String::new();
    for &character in characters.iter().skip(1) {
        match character {
            '$' => return Some(tag),
            _ if character.is_ascii_alphanumeric() || character == '_' => tag.push(character),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(kind: Option<DatabaseKind>, script: &str) -> Vec<String> {
        sql_statement_spans(kind, script)
            .iter()
            .map(|span| script[span.range.clone()].to_owned())
            .collect()
    }

    #[test]
    fn spans_partition_the_script_at_byte_offsets() {
        let script = "\u{feff}SELECT 'é;🙂'; -- ;\nSELECT 2;\n  ";
        let spans = sql_statement_spans(Some(DatabaseKind::PostgreSQL), script);
        assert_eq!(spans.first().unwrap().range.start, 0);
        assert_eq!(spans.last().unwrap().range.end, script.len());
        assert!(
            spans
                .windows(2)
                .all(|pair| pair[0].range.end == pair[1].range.start)
        );
        assert_eq!(
            texts(Some(DatabaseKind::PostgreSQL), script),
            ["\u{feff}SELECT 'é;🙂';", " -- ;\nSELECT 2;", "\n  "]
        );
        assert_eq!(sql_statement_spans(None, "").len(), 1);
    }

    #[test]
    fn routine_and_trigger_bodies_keep_their_inner_terminators() {
        let mysql = "CREATE DEFINER=`root`@`%` PROCEDURE p()\nBEGIN\n  DECLARE n INT;\n  lbl: BEGIN SELECT 1; END lbl;\n  IF n > 0 THEN SELECT 2; END IF;\n  CASE n WHEN 1 THEN SELECT 3; END CASE;\n  WHILE n < 3 DO SET n = n + 1; END WHILE;\nEND;\nSELECT 4;";
        assert_eq!(
            checked_split_sql_for(Some(DatabaseKind::MySQL), mysql)
                .unwrap()
                .len(),
            2
        );
        // `END;` followed by an `IF` statement is not `END IF`.
        let nested = "CREATE PROCEDURE p() BEGIN BEGIN SELECT 1; END; IF 1 THEN SELECT 2; END IF; END; SELECT 3";
        assert_eq!(
            checked_split_sql_for(Some(DatabaseKind::MySQL), nested).unwrap(),
            [
                "CREATE PROCEDURE p() BEGIN BEGIN SELECT 1; END; IF 1 THEN SELECT 2; END IF; END",
                "SELECT 3"
            ]
        );
        let postgres = "CREATE OR REPLACE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT CASE WHEN true THEN 2 END; END; SELECT 3";
        assert_eq!(
            checked_split_sql_for(Some(DatabaseKind::PostgreSQL), postgres)
                .unwrap()
                .len(),
            2
        );
        let sqlite =
            "CREATE TEMP TRIGGER t AFTER INSERT ON x BEGIN UPDATE y SET a = 1; END; SELECT 2";
        assert_eq!(
            texts(Some(DatabaseKind::SQLite), sqlite),
            [
                "CREATE TEMP TRIGGER t AFTER INSERT ON x BEGIN UPDATE y SET a = 1; END;",
                " SELECT 2"
            ]
        );
        // Ordinary statements and transactions are unaffected.
        for kind in [
            DatabaseKind::MySQL,
            DatabaseKind::PostgreSQL,
            DatabaseKind::SQLite,
        ] {
            assert_eq!(
                checked_split_sql_for(Some(kind), "BEGIN; CREATE TABLE trigger_log (id int); END;")
                    .unwrap()
                    .len(),
                3
            );
        }
    }

    #[test]
    fn delimiter_spans_run_as_standalone_scripts() {
        let script = "DELIMITER $$\nCREATE PROCEDURE a() BEGIN SELECT 1; END $$\nCREATE PROCEDURE b() BEGIN SELECT 2; END $$\nDELIMITER ;\nSELECT 3;";
        let spans = sql_statement_spans(Some(DatabaseKind::MySQL), script);
        let second = spans
            .iter()
            .find(|span| script[span.range.clone()].contains("b()"))
            .unwrap();
        assert_eq!(second.delimiter, "$$");
        assert_eq!(
            checked_split_sql_for(Some(DatabaseKind::MySQL), &second.executable_text(script))
                .unwrap(),
            ["CREATE PROCEDURE b() BEGIN SELECT 2; END"]
        );
        let last = spans.last().unwrap();
        assert_eq!(&script[last.range.clone()], "\nDELIMITER ;\nSELECT 3;");
    }

    #[test]
    fn sql_server_spans_follow_go_batches() {
        let script =
            "CREATE PROCEDURE p AS BEGIN SELECT 1; SELECT 2; END\nGO\nSELECT 3; SELECT 4;\nGO\n";
        assert_eq!(
            texts(Some(DatabaseKind::SqlServer), script),
            [
                "CREATE PROCEDURE p AS BEGIN SELECT 1; SELECT 2; END\nGO\n",
                "SELECT 3;",
                " SELECT 4;\nGO\n"
            ]
        );
    }

    #[test]
    fn executable_mysql_comments_do_not_stop_span_discovery() {
        let script = "/*!40101 SET NAMES utf8 */; SELECT 1;";
        assert_eq!(texts(Some(DatabaseKind::MySQL), script).len(), 2);
        assert!(checked_split_sql_for(Some(DatabaseKind::MySQL), script).is_err());
    }
}
