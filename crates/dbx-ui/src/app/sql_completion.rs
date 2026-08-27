use std::collections::{HashMap, HashSet};

use dbx_core::{ColumnInfo, DatabaseKind, EntityKind, QueryResult, TableInfo, TableRef};

use crate::editor::{self, SqlCompletionTarget};

fn table_ref(table: &TableInfo) -> TableRef {
    TableRef {
        schema: table.schema.clone(),
        name: table.name.clone(),
    }
}

fn table_ref_label(table: &TableRef) -> String {
    table.schema.as_ref().map_or_else(
        || table.name.clone(),
        |schema| format!("{schema}.{}", table.name),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompletionItemKind {
    Keyword,
    Type,
    Table,
    Column,
    Function,
    Command,
    Key,
}

impl CompletionItemKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Table => "table",
            Self::Column => "column",
            Self::Function => "function",
            Self::Command => "command",
            Self::Key => "key",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SqlCompletionItem {
    pub(super) label: String,
    pub(super) insert_text: String,
    pub(super) detail: String,
    search_text: String,
    pub(super) kind: CompletionItemKind,
}

impl SqlCompletionItem {
    pub(super) fn new(
        label: impl Into<String>,
        insert_text: impl Into<String>,
        detail: impl Into<String>,
        search_text: impl Into<String>,
        kind: CompletionItemKind,
    ) -> Self {
        Self {
            label: label.into(),
            insert_text: insert_text.into(),
            detail: detail.into(),
            search_text: search_text.into(),
            kind,
        }
    }
}

pub(super) struct SqlCompletionRequest<'a> {
    pub(super) database_kind: DatabaseKind,
    pub(super) tables: &'a [TableInfo],
    pub(super) completion_columns: &'a HashMap<String, Vec<ColumnInfo>>,
    pub(super) selected_table: Option<&'a TableRef>,
    pub(super) active_columns: &'a [ColumnInfo],
    pub(super) result: Option<&'a QueryResult>,
    pub(super) active_schema_filter: Option<&'a str>,
}

#[derive(Clone, Debug)]
struct SqlQueryToken {
    raw: String,
    text: String,
    kind: editor::SqlTokenKind,
    start: usize,
    end: usize,
    depth: usize,
}

#[derive(Clone, Debug)]
struct SqlQuerySource {
    relation: String,
    schema: Option<String>,
    alias: Option<String>,
    columns: Vec<ColumnInfo>,
    depth: usize,
    scope_start: usize,
    scope_end: usize,
}

impl SqlQuerySource {
    fn display_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.relation)
    }

    fn matches_qualifier(&self, qualifier: &str) -> bool {
        self.alias
            .as_deref()
            .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
            || self.relation.eq_ignore_ascii_case(qualifier)
            || self.schema.as_deref().is_some_and(|schema| {
                format!("{schema}.{}", self.relation).eq_ignore_ascii_case(qualifier)
            })
    }
}

#[derive(Clone, Debug, Default)]
struct SqlQueryIndex {
    sources: Vec<SqlQuerySource>,
    ctes: Vec<SqlQuerySource>,
    projection_aliases: Vec<ColumnInfo>,
    insert_columns: HashSet<String>,
    current_depth: usize,
    current_scope_start: usize,
    current_scope_end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SqlCompletionArea {
    General,
    Table,
    Column,
    Type,
}

const MAX_SQL_COMPLETIONS: usize = 14;

pub(super) fn sql_completion_items(
    query_text: &str,
    cursor: usize,
    context: &editor::SqlCompletionContext,
    sources: SqlCompletionRequest<'_>,
) -> Vec<SqlCompletionItem> {
    let index = infer_sql_query_index(query_text, cursor, &sources);
    let area = sql_completion_area(query_text, cursor, context, &index);
    let SqlCompletionRequest {
        database_kind,
        tables,
        completion_columns,
        selected_table,
        active_columns,
        result,
        active_schema_filter,
    } = sources;
    let prefix = context.prefix.trim_matches(['"', '`']).to_ascii_lowercase();
    let append_separator = completion_needs_separator(query_text, context.replacement_range.end);
    let offer_keywords =
        !prefix.is_empty() && context.quote.is_none() && context.qualifier.is_none();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let mut push = |item: SqlCompletionItem| {
        // Insertion order doubles as context priority: candidates pushed
        // earlier come from tighter scopes (visible sources before fallback
        // tables, for example).
        let order = items.len();
        let score = completion_match_score(&item.search_text, &prefix);
        let key = item.insert_text.to_ascii_lowercase();
        if let Some(score) = score
            && seen.insert(key)
        {
            items.push((score, order, item));
        }
    };

    match area {
        SqlCompletionArea::General => {
            push_sql_keywords(&mut push, append_separator);
            if sql_is_create_table_columns(query_text, cursor) {
                push_sql_types(&mut push, append_separator);
            }
            push_sql_functions(&mut push);
        }
        SqlCompletionArea::Type => {
            push_sql_types(&mut push, append_separator);
            if offer_keywords {
                push_sql_keywords(&mut push, append_separator);
            }
        }
        SqlCompletionArea::Table => {
            push_table_candidates(
                &mut push,
                &index,
                tables,
                database_kind,
                context,
                active_schema_filter,
            );
            if offer_keywords {
                push_sql_keywords(&mut push, append_separator);
            }
        }
        SqlCompletionArea::Column => {
            if context.qualifier.is_none() {
                push_columns(
                    &mut push,
                    &index.projection_aliases,
                    "query alias",
                    database_kind,
                    context.quote,
                    None,
                );
            }

            let visible_sources = index
                .sources
                .iter()
                .filter(|source| source.depth <= index.current_depth)
                .filter(|source| {
                    source.scope_start <= index.current_scope_start
                        && index.current_scope_end <= source.scope_end
                })
                .collect::<Vec<_>>();
            let mut added_source = false;
            if let Some(qualifier) = context.qualifier.as_deref() {
                let matching_sources = visible_sources
                    .iter()
                    .filter(|source| source.matches_qualifier(qualifier))
                    .collect::<Vec<_>>();
                let nearest_depth = matching_sources.iter().map(|source| source.depth).max();
                for source in matching_sources
                    .into_iter()
                    .filter(|source| Some(source.depth) == nearest_depth)
                {
                    push_columns(
                        &mut push,
                        &source.columns,
                        source.display_name(),
                        database_kind,
                        context.quote,
                        Some(&index.insert_columns),
                    );
                    added_source = true;
                }
            } else {
                for source in visible_sources {
                    push_columns(
                        &mut push,
                        &source.columns,
                        source.display_name(),
                        database_kind,
                        context.quote,
                        Some(&index.insert_columns),
                    );
                    added_source = true;
                }
            }

            if !added_source && let Some(qualifier) = context.qualifier.as_deref() {
                for table in tables
                    .iter()
                    .filter(|table| completion_table_matches_qualifier(table, qualifier))
                {
                    let table_ref = table_ref(table);
                    if let Some(columns) = completion_columns.get(&completion_table_key(&table_ref))
                    {
                        push_columns(
                            &mut push,
                            columns,
                            &table_ref_label(&table_ref),
                            database_kind,
                            context.quote,
                            Some(&index.insert_columns),
                        );
                        added_source = true;
                    }
                }
            }

            if !added_source {
                if let Some(selected_table) = selected_table {
                    push_columns(
                        &mut push,
                        active_columns,
                        &table_ref_label(selected_table),
                        database_kind,
                        context.quote,
                        Some(&index.insert_columns),
                    );
                } else {
                    push_columns(
                        &mut push,
                        active_columns,
                        "active table",
                        database_kind,
                        context.quote,
                        Some(&index.insert_columns),
                    );
                }
                if let Some(result) = result {
                    push_columns(
                        &mut push,
                        &result.columns,
                        "query result",
                        database_kind,
                        context.quote,
                        Some(&index.insert_columns),
                    );
                }
            }

            // Functions rank below real schema columns: after a qualifier dot
            // they make no sense at all.
            if context.qualifier.is_none() {
                push_sql_functions(&mut push);
            }
            if offer_keywords {
                push_sql_keywords(&mut push, append_separator);
            }
        }
    }

    items.sort_by_key(|(score, order, _)| (*score, *order));
    items.truncate(MAX_SQL_COMPLETIONS);
    items.into_iter().map(|(_, _, item)| item).collect()
}

/// Relevance rank for a completion candidate against the typed prefix. Lower
/// sorts first; `None` drops the candidate entirely.
///
/// 0 exact word, 1 candidate starts with the prefix, 2 any whitespace word in
/// the search text starts with it (so `analytics.users users` still matches on
/// its bare name), 3 substring hit (`mail` finds `email`), 4 untyped — used
/// for an empty prefix where source order alone decides.
fn completion_match_score(search_text: &str, prefix: &str) -> Option<u8> {
    if prefix.is_empty() {
        return Some(4);
    }
    let lowered = search_text.to_ascii_lowercase();
    if lowered == prefix {
        return Some(0);
    }
    if lowered.starts_with(prefix) {
        return Some(1);
    }
    if lowered
        .split_whitespace()
        .any(|word| word.starts_with(prefix))
    {
        return Some(2);
    }
    lowered.contains(prefix).then_some(3)
}

fn infer_sql_query_index(
    query_text: &str,
    cursor: usize,
    sources: &SqlCompletionRequest<'_>,
) -> SqlQueryIndex {
    let cursor = cursor.min(query_text.len());
    let (statement_start, statement_end) = sql_statement_bounds(query_text, cursor);
    let statement_text = &query_text[statement_start..statement_end];
    let statement_cursor = cursor
        .saturating_sub(statement_start)
        .min(statement_text.len());
    let query_prefix = &statement_text[..statement_text.floor_char_boundary(statement_cursor)];
    let (_, current_depth) = sql_query_tokens(query_prefix);
    let scopes = sql_parenthesis_scopes(statement_text);
    let (current_scope_start, current_scope_end) =
        sql_scope_for_position(statement_cursor, &scopes, statement_text.len());
    let (tokens, _) = sql_query_tokens(statement_text);
    let ctes = infer_sql_ctes(statement_text, &tokens, sources, &scopes);
    let mut index = SqlQueryIndex {
        ctes,
        current_depth,
        current_scope_start,
        current_scope_end,
        ..SqlQueryIndex::default()
    };

    let mut token_index = 0;
    while token_index < tokens.len() {
        let token = &tokens[token_index];
        let is_relation_keyword =
            matches!(token.text.as_str(), "from" | "join" | "update" | "into");
        if !is_relation_keyword || !sql_token_is_word(token) {
            token_index += 1;
            continue;
        }

        let mut relation_index = token_index + 1;
        while let Some((source, next_index)) = parse_sql_source(
            statement_text,
            &tokens,
            relation_index,
            token,
            &index.ctes,
            sources,
            &scopes,
        ) {
            index.sources.push(source);

            let Some(next_word) = sql_next_word(&tokens, next_index) else {
                break;
            };
            let previous_end = tokens
                .get(next_word.saturating_sub(1))
                .map_or(token.end, |token| token.end);
            if sql_gap_contains(query_text, previous_end, tokens[next_word].start, ',') {
                relation_index = next_word;
                continue;
            }
            break;
        }
        token_index += 1;
    }

    index.projection_aliases = infer_projection_columns(statement_text, sources);
    index.insert_columns = infer_insert_columns(query_prefix);
    index
}

fn sql_completion_area(
    query_text: &str,
    cursor: usize,
    context: &editor::SqlCompletionContext,
    index: &SqlQueryIndex,
) -> SqlCompletionArea {
    let cursor = cursor.min(query_text.len());
    let (statement_start, statement_end) = sql_statement_bounds(query_text, cursor);
    let statement_text = &query_text[statement_start..statement_end];
    let statement_cursor = cursor
        .saturating_sub(statement_start)
        .min(statement_text.len());

    if sql_is_insert_column_list(statement_text, statement_cursor) {
        return SqlCompletionArea::Column;
    }
    if sql_is_insert_values_list(statement_text, statement_cursor) {
        return SqlCompletionArea::General;
    }
    if sql_is_create_table_columns(statement_text, statement_cursor) {
        return SqlCompletionArea::Type;
    }
    if sql_is_ddl_type_context(statement_text, statement_cursor) {
        return SqlCompletionArea::Type;
    }
    if context.target == SqlCompletionTarget::Table {
        return SqlCompletionArea::Table;
    }
    if context.target == SqlCompletionTarget::Column {
        return SqlCompletionArea::Column;
    }

    let query_prefix = &statement_text[..statement_text.floor_char_boundary(statement_cursor)];
    let (tokens, _) = sql_query_tokens(query_prefix);
    let Some(keyword) = tokens
        .iter()
        .rev()
        .find(|token| token.kind == editor::SqlTokenKind::Keyword)
        .map(|token| token.text.as_str())
    else {
        return SqlCompletionArea::General;
    };

    match keyword {
        "from" | "join" | "update" | "into" | "table" | "view" | "references" => {
            SqlCompletionArea::Table
        }
        "select" | "where" | "and" | "or" | "on" | "by" | "group" | "order" | "having" | "set"
        | "returning" | "values" => SqlCompletionArea::Column,
        _ if !index.sources.is_empty() => SqlCompletionArea::Column,
        _ => SqlCompletionArea::General,
    }
}

fn sql_query_tokens(text: &str) -> (Vec<SqlQueryToken>, usize) {
    let mut depth = 0;
    let mut offset = 0;
    let mut tokens = Vec::new();
    for token in editor::lex_sql(text) {
        sql_update_parenthesis_depth(&mut depth, &text[offset..token.range.start]);
        let raw = text[token.range.clone()].to_owned();
        tokens.push(SqlQueryToken {
            text: sql_identifier_text(&raw).to_ascii_lowercase(),
            raw,
            kind: token.kind,
            start: token.range.start,
            end: token.range.end,
            depth,
        });
        offset = token.range.end;
    }
    sql_update_parenthesis_depth(&mut depth, &text[offset..]);
    (tokens, depth)
}

fn sql_statement_bounds(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(text.len());
    let mut separators = Vec::new();
    let mut offset = 0;
    for token in editor::lex_sql(text) {
        sql_collect_statement_separators(text, offset, token.range.start, &mut separators);
        offset = token.range.end;
    }
    sql_collect_statement_separators(text, offset, text.len(), &mut separators);

    let start = separators
        .iter()
        .rev()
        .find(|separator| **separator < cursor)
        .map_or(0, |separator| separator.saturating_add(1));
    let end = separators
        .iter()
        .find(|separator| **separator >= cursor)
        .copied()
        .unwrap_or(text.len());
    (start, end)
}

fn sql_collect_statement_separators(
    text: &str,
    start: usize,
    end: usize,
    separators: &mut Vec<usize>,
) {
    separators.extend(
        text[start..end]
            .char_indices()
            .filter_map(|(offset, character)| (character == ';').then_some(start + offset)),
    );
}

fn sql_parenthesis_scopes(text: &str) -> Vec<(usize, usize)> {
    let tokens = editor::lex_sql(text);
    let mut scopes = Vec::new();
    let mut open_positions = Vec::new();
    let mut offset = 0;
    for token in tokens {
        sql_collect_parenthesis_scopes(
            text,
            offset,
            token.range.start,
            &mut open_positions,
            &mut scopes,
        );
        offset = token.range.end;
    }
    sql_collect_parenthesis_scopes(text, offset, text.len(), &mut open_positions, &mut scopes);
    scopes.extend(open_positions.into_iter().map(|open| (open, text.len())));
    scopes
}

fn sql_collect_parenthesis_scopes(
    text: &str,
    start: usize,
    end: usize,
    open_positions: &mut Vec<usize>,
    scopes: &mut Vec<(usize, usize)>,
) {
    for (offset, character) in text[start..end].char_indices() {
        let position = start + offset;
        match character {
            '(' => open_positions.push(position),
            ')' => {
                if let Some(open) = open_positions.pop() {
                    scopes.push((open, position));
                }
            }
            _ => {}
        }
    }
}

fn sql_scope_for_position(
    position: usize,
    scopes: &[(usize, usize)],
    text_len: usize,
) -> (usize, usize) {
    scopes
        .iter()
        .filter(|(open, close)| *open < position && position <= *close)
        .max_by_key(|(open, _)| *open)
        .map_or((0, text_len.saturating_add(1)), |(open, close)| {
            (open.saturating_add(1), *close)
        })
}

fn sql_update_parenthesis_depth(depth: &mut usize, text: &str) {
    for character in text.chars() {
        match character {
            '(' => *depth += 1,
            ')' => *depth = depth.saturating_sub(1),
            _ => {}
        }
    }
}

fn sql_identifier_text(raw: &str) -> String {
    let raw = raw.trim();
    let Some(quote) = raw.chars().next() else {
        return String::new();
    };
    if matches!(quote, '"' | '`') && raw.ends_with(quote) && raw.len() >= 2 {
        raw[quote.len_utf8()..raw.len() - quote.len_utf8()]
            .replace(&format!("{quote}{quote}"), &quote.to_string())
    } else {
        raw.to_owned()
    }
}

fn sql_token_is_word(token: &SqlQueryToken) -> bool {
    matches!(
        token.kind,
        editor::SqlTokenKind::Keyword
            | editor::SqlTokenKind::Identifier
            | editor::SqlTokenKind::Type
    )
}

fn sql_next_word(tokens: &[SqlQueryToken], start: usize) -> Option<usize> {
    (start..tokens.len()).find(|index| sql_token_is_word(&tokens[*index]))
}

fn sql_gap_contains(text: &str, start: usize, end: usize, expected: char) -> bool {
    text.get(start..end)
        .is_some_and(|gap| gap.chars().any(|character| character == expected))
}

fn sql_gap_is_only(text: &str, start: usize, end: usize, expected: char) -> bool {
    let Some(gap) = text.get(start..end) else {
        return false;
    };
    let mut found = false;
    for character in gap.chars() {
        if character.is_whitespace() {
            continue;
        }
        if character != expected || found {
            return false;
        }
        found = true;
    }
    found
}

fn sql_current_open_parenthesis(query_text: &str, cursor: usize) -> Option<usize> {
    let cursor = cursor.min(query_text.len());
    let prefix = &query_text[..query_text.floor_char_boundary(cursor)];
    let scopes = sql_parenthesis_scopes(prefix);
    let (scope_start, _) = sql_scope_for_position(prefix.len(), &scopes, prefix.len());
    let open = scope_start.checked_sub(1)?;
    (prefix.as_bytes().get(open) == Some(&b'(')).then_some(open)
}

fn sql_is_insert_column_list(query_text: &str, cursor: usize) -> bool {
    let prefix = &query_text[..query_text.floor_char_boundary(cursor.min(query_text.len()))];
    let Some(open) = sql_current_open_parenthesis(query_text, cursor) else {
        return false;
    };
    let (tokens, _) = sql_query_tokens(&prefix[..open]);
    let Some(into_index) = tokens
        .iter()
        .rposition(|token| token.text == "into" && token.kind == editor::SqlTokenKind::Keyword)
    else {
        return false;
    };
    let has_insert = tokens[..into_index]
        .iter()
        .rev()
        .any(|token| token.text == "insert" && token.kind == editor::SqlTokenKind::Keyword);
    has_insert
        && !tokens[into_index + 1..].iter().any(|token| {
            matches!(
                token.text.as_str(),
                "select" | "values" | "default" | "on" | "conflict" | "returning"
            )
        })
}

fn sql_is_insert_values_list(query_text: &str, cursor: usize) -> bool {
    let prefix = &query_text[..query_text.floor_char_boundary(cursor.min(query_text.len()))];
    let Some(open) = sql_current_open_parenthesis(query_text, cursor) else {
        return false;
    };
    let (tokens, _) = sql_query_tokens(&prefix[..open]);
    let Some(values_index) = tokens
        .iter()
        .rposition(|token| token.text == "values" && token.kind == editor::SqlTokenKind::Keyword)
    else {
        return false;
    };
    tokens[..values_index]
        .iter()
        .rev()
        .any(|token| token.text == "insert" && token.kind == editor::SqlTokenKind::Keyword)
}

fn sql_is_create_table_columns(query_text: &str, cursor: usize) -> bool {
    let prefix = &query_text[..query_text.floor_char_boundary(cursor.min(query_text.len()))];
    let Some(open) = sql_current_open_parenthesis(query_text, cursor) else {
        return false;
    };
    let (tokens, depth) = sql_query_tokens(&prefix[..open]);
    let Some(create_index) = tokens.iter().rposition(|token| {
        token.text == "create"
            && token.kind == editor::SqlTokenKind::Keyword
            && token.depth == depth
    }) else {
        return false;
    };
    let has_table = (create_index + 1..tokens.len()).any(|index| {
        tokens[index].text == "table"
            && tokens[index].kind == editor::SqlTokenKind::Keyword
            && tokens[index].depth == depth
    });
    has_table
        && !tokens
            .last()
            .is_some_and(|token| matches!(token.text.as_str(), "check" | "constraint"))
}

fn sql_is_ddl_type_context(query_text: &str, cursor: usize) -> bool {
    let prefix = &query_text[..query_text.floor_char_boundary(cursor.min(query_text.len()))];
    let (tokens, _) = sql_query_tokens(prefix);
    let Some(alter_index) = tokens
        .iter()
        .rposition(|token| token.text == "alter" && token.kind == editor::SqlTokenKind::Keyword)
    else {
        return false;
    };
    let Some(table_index) = (alter_index + 1..tokens.len()).find(|index| {
        tokens[*index].text == "table" && tokens[*index].kind == editor::SqlTokenKind::Keyword
    }) else {
        return false;
    };
    let after_table = &tokens[table_index + 1..];
    let Some(add_index) = after_table.iter().position(|token| token.text == "add") else {
        return after_table.last().is_some_and(|token| token.text == "type");
    };
    let after_add = &after_table[add_index + 1..];
    let next = after_add.first().map(|token| token.text.as_str());
    !matches!(
        next,
        Some("constraint" | "primary" | "unique" | "foreign" | "check")
    )
}

fn infer_sql_ctes(
    query_text: &str,
    tokens: &[SqlQueryToken],
    sources: &SqlCompletionRequest<'_>,
    scopes: &[(usize, usize)],
) -> Vec<SqlQuerySource> {
    let mut ctes = Vec::new();
    for (with_index, with_token) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.text == "with" && token.kind == editor::SqlTokenKind::Keyword)
    {
        let mut next_name_index = sql_next_word(tokens, with_index + 1);
        if next_name_index.is_some_and(|index| tokens[index].text == "recursive") {
            next_name_index = sql_next_word(tokens, next_name_index.unwrap_or(with_index) + 1);
        }

        while let Some(name_index) = next_name_index {
            let name_token = &tokens[name_index];
            if name_token.depth != with_token.depth || name_token.text == "select" {
                break;
            }
            let Some(as_index) = (name_index + 1..tokens.len()).find(|index| {
                tokens[*index].depth == name_token.depth
                    && tokens[*index].text == "as"
                    && tokens[*index].kind == editor::SqlTokenKind::Keyword
            }) else {
                break;
            };
            let declared_columns =
                sql_cte_column_names(query_text, name_token.end, tokens[as_index].start);
            let Some(mut body_first) = sql_next_word(tokens, as_index + 1) else {
                break;
            };
            if tokens[body_first].text == "not"
                && sql_next_word(tokens, body_first + 1)
                    .is_some_and(|index| tokens[index].text == "materialized")
            {
                body_first = sql_next_word(tokens, body_first + 2).unwrap_or(body_first);
            } else if tokens[body_first].text == "materialized" {
                body_first = sql_next_word(tokens, body_first + 1).unwrap_or(body_first);
            }
            let Some(open) = query_text[tokens[as_index].end..tokens[body_first].start]
                .rfind('(')
                .map(|offset| tokens[as_index].end + offset)
            else {
                break;
            };
            let close = (body_first + 1..tokens.len()).find_map(|index| {
                let previous = tokens.get(index.saturating_sub(1))?;
                (tokens[index].depth <= name_token.depth
                    && sql_gap_contains(query_text, previous.end, tokens[index].start, ')'))
                .then_some(index)
            });
            let body_end = close.map_or(query_text.len(), |index| {
                let previous = &tokens[index.saturating_sub(1)];
                query_text[previous.end..tokens[index].start]
                    .find(')')
                    .map_or(tokens[index].start, |offset| previous.end + offset)
            });
            let body = query_text.get(open + 1..body_end).unwrap_or_default();
            let inferred_columns = infer_projection_columns(body, sources);
            let (scope_start, scope_end) =
                sql_scope_for_position(with_token.start, scopes, query_text.len());
            ctes.push(SqlQuerySource {
                relation: name_token.text.clone(),
                schema: None,
                alias: None,
                columns: declared_columns
                    .map(|names| rename_projection_columns(names, &inferred_columns))
                    .unwrap_or(inferred_columns),
                depth: name_token.depth,
                scope_start,
                scope_end,
            });

            let Some(close_index) = close else {
                break;
            };
            let next = sql_next_word(tokens, close_index);
            let has_next_cte = next.is_some_and(|next| {
                sql_gap_contains(
                    query_text,
                    tokens[close_index.saturating_sub(1)].end,
                    tokens[next].start,
                    ',',
                )
            });
            if !has_next_cte {
                break;
            }
            next_name_index = next;
        }
    }
    ctes
}

fn sql_cte_column_names(query_text: &str, start: usize, end: usize) -> Option<Vec<String>> {
    let declaration = query_text.get(start..end)?;
    let open = declaration.find('(')?;
    let close = declaration.rfind(')')?;
    if close <= open {
        return None;
    }
    let (tokens, _) = sql_query_tokens(&declaration[open + 1..close]);
    let names = tokens
        .into_iter()
        .filter(sql_token_is_word)
        .map(|token| token.text)
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some(names)
}

fn rename_projection_columns(
    names: Vec<String>,
    inferred_columns: &[ColumnInfo],
) -> Vec<ColumnInfo> {
    names
        .into_iter()
        .enumerate()
        .map(|(ordinal, name)| {
            inferred_columns
                .get(ordinal)
                .cloned()
                .map(|mut column| {
                    column.name = name.clone();
                    column.ordinal = ordinal;
                    column
                })
                .unwrap_or_else(|| ColumnInfo::result(name, ordinal, "unknown"))
        })
        .collect()
}

fn parse_sql_source(
    query_text: &str,
    tokens: &[SqlQueryToken],
    start: usize,
    relation_keyword: &SqlQueryToken,
    ctes: &[SqlQuerySource],
    sources: &SqlCompletionRequest<'_>,
    scopes: &[(usize, usize)],
) -> Option<(SqlQuerySource, usize)> {
    let keyword_end = relation_keyword.end;
    let depth = relation_keyword.depth;
    let mut first_index = sql_next_word(tokens, start)?;
    if tokens[first_index].text == "lateral" || tokens[first_index].text == "only" {
        first_index = sql_next_word(tokens, first_index + 1)?;
    }
    let first = &tokens[first_index];
    let has_subquery =
        sql_gap_contains(query_text, keyword_end, first.start, '(') || first.depth > depth;
    if has_subquery {
        let open = query_text[keyword_end..first.start]
            .rfind('(')
            .map_or(keyword_end, |offset| keyword_end + offset);
        let close = (first_index + 1..tokens.len()).find_map(|index| {
            let previous = tokens.get(index.saturating_sub(1))?;
            (tokens[index].depth <= depth
                && sql_gap_contains(query_text, previous.end, tokens[index].start, ')'))
            .then_some(index)
        });
        let body_end = close.map_or(query_text.len(), |index| {
            let previous = &tokens[index.saturating_sub(1)];
            query_text[previous.end..tokens[index].start]
                .find(')')
                .map_or(tokens[index].start, |offset| previous.end + offset)
        });
        let body = query_text.get(open + 1..body_end).unwrap_or_default();
        let (alias, next_index) = parse_sql_alias(
            query_text,
            tokens,
            close.unwrap_or(tokens.len()),
            depth,
            body_end + usize::from(close.is_some()),
        );
        let relation = alias.clone().unwrap_or_else(|| "subquery".into());
        let (scope_start, scope_end) =
            sql_scope_for_position(keyword_end, scopes, query_text.len());
        return Some((
            SqlQuerySource {
                relation,
                schema: None,
                alias,
                columns: infer_projection_columns(body, sources),
                depth,
                scope_start,
                scope_end,
            },
            next_index,
        ));
    }
    if first.depth != depth {
        return None;
    }

    let mut relation_index = first_index;
    let mut schema = None;
    let mut relation = first.text.clone();
    if let Some(second_index) = sql_next_word(tokens, first_index + 1)
        && tokens[second_index].depth == depth
        && sql_gap_is_only(query_text, first.end, tokens[second_index].start, '.')
    {
        schema = Some(relation.clone());
        relation = tokens[second_index].text.clone();
        relation_index = second_index;
    }
    let relation_end = tokens[relation_index].end;
    let (alias, next_index) =
        parse_sql_alias(query_text, tokens, relation_index + 1, depth, relation_end);
    let cte = ctes
        .iter()
        .find(|cte| schema.is_none() && cte.relation.eq_ignore_ascii_case(&relation));
    let table = cte
        .is_none()
        .then(|| resolve_completion_table(&relation, schema.as_deref(), sources))
        .flatten();
    let columns = cte
        .map(|cte| cte.columns.clone())
        .or_else(|| {
            table
                .as_ref()
                .map(|table_ref| completion_columns_for_table(table_ref, sources))
        })
        .unwrap_or_default();
    let (scope_start, scope_end) = sql_scope_for_position(first.start, scopes, query_text.len());
    Some((
        SqlQuerySource {
            relation,
            schema,
            alias,
            columns,
            depth,
            scope_start,
            scope_end,
        },
        next_index,
    ))
}

fn parse_sql_alias(
    query_text: &str,
    tokens: &[SqlQueryToken],
    start: usize,
    depth: usize,
    previous_end: usize,
) -> (Option<String>, usize) {
    let Some(candidate_index) = sql_next_word(tokens, start) else {
        return (None, tokens.len());
    };
    let candidate = &tokens[candidate_index];
    if candidate.depth != depth
        || sql_gap_contains(query_text, previous_end, candidate.start, ',')
        || sql_gap_contains(query_text, previous_end, candidate.start, ')')
    {
        return (None, start);
    }
    if candidate.text == "as" {
        let Some(alias_index) = sql_next_word(tokens, candidate_index + 1) else {
            return (None, candidate_index + 1);
        };
        if tokens[alias_index].depth == depth
            && tokens[alias_index].kind == editor::SqlTokenKind::Identifier
        {
            return (Some(tokens[alias_index].text.clone()), alias_index + 1);
        }
        return (None, candidate_index + 1);
    }
    if candidate.kind == editor::SqlTokenKind::Identifier && !sql_is_clause_word(&candidate.text) {
        return (Some(candidate.text.clone()), candidate_index + 1);
    }
    (None, start)
}

fn sql_is_clause_word(word: &str) -> bool {
    matches!(
        word,
        "from"
            | "join"
            | "where"
            | "on"
            | "group"
            | "order"
            | "having"
            | "limit"
            | "offset"
            | "union"
            | "except"
            | "intersect"
            | "set"
            | "returning"
            | "values"
            | "using"
    )
}

fn resolve_completion_table(
    name: &str,
    schema: Option<&str>,
    sources: &SqlCompletionRequest<'_>,
) -> Option<TableRef> {
    let exact = sources.tables.iter().find(|table| {
        table.name.eq_ignore_ascii_case(name)
            && schema.is_none_or(|schema| {
                table
                    .schema
                    .as_deref()
                    .is_some_and(|table_schema| table_schema.eq_ignore_ascii_case(schema))
            })
    });
    if let Some(table) = exact {
        return Some(table_ref(table));
    }
    if schema.is_some() {
        return None;
    }
    if let Some(active_schema) = sources.active_schema_filter
        && let Some(table) = sources.tables.iter().find(|table| {
            table.name.eq_ignore_ascii_case(name)
                && table
                    .schema
                    .as_deref()
                    .is_some_and(|schema| schema.eq_ignore_ascii_case(active_schema))
        })
    {
        return Some(table_ref(table));
    }
    if let Some(selected) = sources.selected_table
        && selected.name.eq_ignore_ascii_case(name)
    {
        return Some(selected.clone());
    }
    sources
        .tables
        .iter()
        .find(|table| table.name.eq_ignore_ascii_case(name))
        .map(table_ref)
}

fn completion_columns_for_table(
    table: &TableRef,
    sources: &SqlCompletionRequest<'_>,
) -> Vec<ColumnInfo> {
    sources
        .completion_columns
        .get(&completion_table_key(table))
        .cloned()
        .or_else(|| {
            sources
                .selected_table
                .filter(|selected| *selected == table)
                .map(|_| sources.active_columns.to_vec())
        })
        .unwrap_or_default()
}

fn infer_projection_columns(
    query_text: &str,
    sources: &SqlCompletionRequest<'_>,
) -> Vec<ColumnInfo> {
    let (tokens, _) = sql_query_tokens(query_text);
    let Some(select_depth) = tokens
        .iter()
        .filter(|token| token.text == "select" && token.kind == editor::SqlTokenKind::Keyword)
        .map(|token| token.depth)
        .min()
    else {
        return Vec::new();
    };
    let Some(select_index) = tokens.iter().enumerate().rev().find_map(|(index, token)| {
        (token.text == "select"
            && token.kind == editor::SqlTokenKind::Keyword
            && token.depth == select_depth)
            .then_some(index)
    }) else {
        return Vec::new();
    };
    let projection_end = (select_index + 1..tokens.len())
        .find(|index| {
            tokens[*index].text == "from"
                && tokens[*index].kind == editor::SqlTokenKind::Keyword
                && tokens[*index].depth == tokens[select_index].depth
        })
        .unwrap_or(tokens.len());
    let depth = tokens[select_index].depth;
    let mut segment_start = select_index + 1;
    let mut columns = Vec::new();
    for index in select_index + 1..projection_end {
        if tokens[index].depth == depth
            && index > segment_start
            && sql_gap_contains(query_text, tokens[index - 1].end, tokens[index].start, ',')
        {
            if let Some(column) = projection_column(&tokens, segment_start, index, sources) {
                columns.push(with_column_ordinal(column, columns.len()));
            }
            segment_start = index;
        }
    }
    if segment_start < projection_end
        && let Some(column) = projection_column(&tokens, segment_start, projection_end, sources)
    {
        columns.push(with_column_ordinal(column, columns.len()));
    }
    columns
}

fn with_column_ordinal(mut column: ColumnInfo, ordinal: usize) -> ColumnInfo {
    column.ordinal = ordinal;
    column
}

fn projection_column(
    tokens: &[SqlQueryToken],
    start: usize,
    end: usize,
    sources: &SqlCompletionRequest<'_>,
) -> Option<ColumnInfo> {
    let words = (start..end)
        .filter(|index| sql_token_is_word(&tokens[*index]))
        .collect::<Vec<_>>();
    if words.is_empty() {
        return None;
    }
    let alias = words.windows(2).find_map(|window| {
        (tokens[window[0]].text == "as").then(|| tokens[window[1]].text.clone())
    });
    let name = alias.or_else(|| {
        let last = *words.last()?;
        (tokens[last].kind == editor::SqlTokenKind::Identifier
            && !tokens[start..end]
                .iter()
                .any(|token| token.raw.contains('(') || token.raw.contains('*')))
        .then(|| tokens[last].text.clone())
    })?;
    let known = sources
        .active_columns
        .iter()
        .chain(sources.completion_columns.values().flatten())
        .chain(
            sources
                .result
                .into_iter()
                .flat_map(|result| result.columns.iter()),
        )
        .find(|column| column.name.eq_ignore_ascii_case(&name));
    Some(
        known
            .cloned()
            .unwrap_or_else(|| ColumnInfo::result(name, 0, "unknown")),
    )
}

fn infer_insert_columns(query_text: &str) -> HashSet<String> {
    if !sql_is_insert_column_list(query_text, query_text.len()) {
        return HashSet::new();
    }
    let Some(open) = query_text.rfind('(') else {
        return HashSet::new();
    };
    let (tokens, _) = sql_query_tokens(&query_text[open + 1..]);
    tokens
        .into_iter()
        .filter(|token| token.kind == editor::SqlTokenKind::Identifier)
        .map(|token| token.text)
        .collect()
}

fn completion_needs_separator(query_text: &str, replacement_end: usize) -> bool {
    query_text
        .get(replacement_end..)
        .and_then(|suffix| suffix.chars().next())
        .is_none_or(|character| {
            !character.is_whitespace() && !matches!(character, ',' | ')' | ';' | '.')
        })
}

fn push_sql_keywords(push: &mut impl FnMut(SqlCompletionItem), append_separator: bool) {
    for keyword in editor::sql_completion_keywords() {
        push(SqlCompletionItem {
            label: (*keyword).into(),
            insert_text: format!("{keyword}{}", if append_separator { " " } else { "" }),
            detail: "SQL keyword".into(),
            search_text: (*keyword).into(),
            kind: CompletionItemKind::Keyword,
        });
    }
}

fn push_sql_types(push: &mut impl FnMut(SqlCompletionItem), append_separator: bool) {
    for sql_type in editor::sql_completion_types() {
        push(SqlCompletionItem {
            label: (*sql_type).into(),
            insert_text: format!("{sql_type}{}", if append_separator { " " } else { "" }),
            detail: "SQL type".into(),
            search_text: (*sql_type).into(),
            kind: CompletionItemKind::Type,
        });
    }
}

/// Offer DBX's cross-engine function vocabulary. Value-style constants keep
/// their bare form; everything else inserts an open paren ready for arguments.
fn push_sql_functions(push: &mut impl FnMut(SqlCompletionItem)) {
    const BARE_FORMS: &[&str] = &["CURRENT_DATE", "CURRENT_TIMESTAMP"];
    for function in editor::sql_completion_functions() {
        let insert_text = if BARE_FORMS.contains(function) {
            (*function).to_owned()
        } else {
            format!("{function}(")
        };
        push(SqlCompletionItem {
            label: (*function).into(),
            insert_text,
            detail: "function".into(),
            search_text: (*function).into(),
            kind: CompletionItemKind::Function,
        });
    }
}

fn push_table_candidates(
    push: &mut impl FnMut(SqlCompletionItem),
    index: &SqlQueryIndex,
    tables: &[TableInfo],
    database_kind: DatabaseKind,
    context: &editor::SqlCompletionContext,
    active_schema_filter: Option<&str>,
) {
    for table in tables.iter().filter(|table| {
        matches!(table.kind, EntityKind::Table | EntityKind::View)
            && context.qualifier.as_deref().is_none_or(|qualifier| {
                table.schema.as_deref().is_some_and(|schema| {
                    !qualifier.contains('.') && schema.eq_ignore_ascii_case(qualifier)
                })
            })
    }) {
        let schema = table.schema.as_deref();
        let qualified = schema.is_some_and(|schema| Some(schema) != active_schema_filter);
        let label = if context.qualifier.is_some() || !qualified {
            table.name.clone()
        } else {
            format!("{}.{}", schema.unwrap_or_default(), table.name)
        };
        let raw_insert_text = if context.qualifier.is_some() || !qualified {
            table.name.clone()
        } else {
            format!("{}.{}", schema.unwrap_or_default(), table.name)
        };
        let insert_text = completion_identifier(database_kind, &raw_insert_text, context.quote);
        let entity = match table.kind {
            EntityKind::View => "view",
            _ => "table",
        };
        let detail = schema
            .map(|schema| format!("{entity} · {schema}"))
            .unwrap_or_else(|| entity.into());
        let search_text = schema
            .map(|schema| format!("{schema}.{} {}", table.name, table.name))
            .unwrap_or_else(|| table.name.clone());
        push(SqlCompletionItem {
            label,
            insert_text,
            detail,
            search_text,
            kind: CompletionItemKind::Table,
        });
    }

    if context.qualifier.is_none() {
        for cte in &index.ctes {
            push(SqlCompletionItem {
                label: cte.relation.clone(),
                insert_text: completion_identifier(database_kind, &cte.relation, context.quote),
                detail: format!(
                    "CTE · {} column{}",
                    cte.columns.len(),
                    if cte.columns.len() == 1 { "" } else { "s" }
                ),
                search_text: cte.relation.clone(),
                kind: CompletionItemKind::Table,
            });
        }
    }
}

fn push_columns(
    push: &mut impl FnMut(SqlCompletionItem),
    columns: &[ColumnInfo],
    source: &str,
    database_kind: DatabaseKind,
    quote: Option<char>,
    excluded: Option<&HashSet<String>>,
) {
    for column in columns {
        if excluded.is_some_and(|excluded| excluded.contains(&column.name.to_ascii_lowercase())) {
            continue;
        }
        push(SqlCompletionItem {
            label: column.name.clone(),
            insert_text: completion_identifier(database_kind, &column.name, quote),
            detail: format!("column · {source} · {}", column.data_type),
            search_text: column.name.clone(),
            kind: CompletionItemKind::Column,
        });
    }
}

fn completion_table_matches_qualifier(table: &TableInfo, qualifier: &str) -> bool {
    let parts = qualifier.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        [name] => {
            table.name.eq_ignore_ascii_case(name)
                || table
                    .schema
                    .as_deref()
                    .is_some_and(|schema| schema.eq_ignore_ascii_case(name))
        }
        [schema, name] => {
            table.name.eq_ignore_ascii_case(name)
                && table
                    .schema
                    .as_deref()
                    .is_some_and(|table_schema| table_schema.eq_ignore_ascii_case(schema))
        }
        _ => false,
    }
}

fn completion_identifier(kind: DatabaseKind, identifier: &str, quote: Option<char>) -> String {
    if let Some(quote) = quote {
        let mut escaped = String::with_capacity(identifier.len());
        for character in identifier.chars() {
            if character == quote {
                escaped.push(quote);
            }
            escaped.push(character);
        }
        escaped
    } else {
        dbx_core::quote_identifier(kind, identifier).unwrap_or_else(|_| identifier.to_owned())
    }
}

pub(super) fn completion_table_key(table: &TableRef) -> String {
    format!(
        "{}\u{0}{}",
        table.schema.as_deref().unwrap_or_default(),
        table.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_completion_uses_schema_and_cached_columns() {
        let tables = vec![
            TableInfo::table("users", Some("public".into())),
            TableInfo::table("events", Some("analytics".into())),
        ];
        let users = TableRef::in_schema("public", "users");
        let mut columns = HashMap::new();
        columns.insert(
            completion_table_key(&users),
            vec![ColumnInfo::result("user_id", 0, "INTEGER")],
        );
        let events = TableRef::in_schema("analytics", "events");
        columns.insert(
            completion_table_key(&events),
            vec![ColumnInfo::result("event_id", 0, "INTEGER")],
        );

        let table_context = editor::sql_completion_context("SELECT * FROM pu", 16).unwrap();
        let table_items = sql_completion_items(
            "SELECT * FROM pu",
            16,
            &table_context,
            SqlCompletionRequest {
                database_kind: DatabaseKind::PostgreSQL,
                tables: &tables,
                completion_columns: &columns,
                selected_table: Some(&users),
                active_columns: &[],
                result: None,
                active_schema_filter: Some("public"),
            },
        );
        assert!(table_items.iter().any(|item| {
            item.kind == CompletionItemKind::Table
                && item.label == "users"
                && item.detail.contains("public")
        }));

        let column_context = editor::sql_completion_context("SELECT user", 11).unwrap();
        let column_items = sql_completion_items(
            "SELECT user",
            11,
            &column_context,
            SqlCompletionRequest {
                database_kind: DatabaseKind::PostgreSQL,
                tables: &tables,
                completion_columns: &columns,
                selected_table: Some(&users),
                active_columns: &[ColumnInfo::result("user_id", 0, "INTEGER")],
                result: None,
                active_schema_filter: Some("public"),
            },
        );
        assert!(
            column_items
                .iter()
                .any(|item| { item.kind == CompletionItemKind::Column && item.label == "user_id" })
        );
        assert_eq!(
            table_items
                .iter()
                .find(|item| item.label == "users")
                .map(|item| item.insert_text.as_str()),
            Some("\"users\"")
        );
        assert_eq!(
            column_items
                .iter()
                .find(|item| item.label == "user_id")
                .map(|item| item.insert_text.as_str()),
            Some("\"user_id\"")
        );

        let qualified_context = editor::sql_completion_context(
            "SELECT analytics.events.",
            "SELECT analytics.events.".len(),
        )
        .unwrap();
        let qualified_items = sql_completion_items(
            "SELECT analytics.events.",
            "SELECT analytics.events.".len(),
            &qualified_context,
            SqlCompletionRequest {
                database_kind: DatabaseKind::PostgreSQL,
                tables: &tables,
                completion_columns: &columns,
                selected_table: Some(&users),
                active_columns: &[],
                result: None,
                active_schema_filter: Some("public"),
            },
        );
        assert!(qualified_items.iter().any(|item| item.label == "event_id"));
        assert!(!qualified_items.iter().any(|item| item.label == "user_id"));
    }

    #[test]
    fn sql_completion_quotes_identifiers_per_dialect_and_preserves_open_quote() {
        assert_eq!(
            completion_identifier(DatabaseKind::PostgreSQL, "display name", None),
            "\"display name\""
        );
        assert_eq!(
            completion_identifier(DatabaseKind::MySQL, "display name", None),
            "`display name`"
        );
        assert_eq!(
            completion_identifier(DatabaseKind::PostgreSQL, "display\"name", None),
            "\"display\"\"name\""
        );
        assert_eq!(
            completion_identifier(DatabaseKind::PostgreSQL, "display name", Some('"')),
            "display name"
        );
    }

    #[test]
    fn keyword_completion_inserts_exactly_one_separator_when_needed() {
        let tables = Vec::new();
        let columns = HashMap::new();
        let items_for = |query: &str, cursor: usize| {
            let context = editor::sql_completion_context(query, cursor).unwrap();
            sql_completion_items(
                query,
                cursor,
                &context,
                SqlCompletionRequest {
                    database_kind: DatabaseKind::PostgreSQL,
                    tables: &tables,
                    completion_columns: &columns,
                    selected_table: None,
                    active_columns: &[],
                    result: None,
                    active_schema_filter: None,
                },
            )
        };
        let select_text = |items: Vec<SqlCompletionItem>| {
            items
                .into_iter()
                .find(|item| item.label == "SELECT")
                .expect("SELECT completion")
                .insert_text
        };

        assert_eq!(select_text(items_for("SEL", 3)), "SELECT ");
        assert_eq!(select_text(items_for("SEL *", 3)), "SELECT");
        assert_eq!(select_text(items_for("SEL,", 3)), "SELECT");
        assert_eq!(select_text(items_for("SEL*", 3)), "SELECT ");
    }

    #[test]
    fn keyword_completion_continues_matching_after_the_first_clause() {
        let tables = Vec::new();
        let columns = HashMap::new();
        let items_for = |query: &str| {
            let cursor = query.len();
            let context = editor::sql_completion_context(query, cursor).unwrap();
            sql_completion_items(
                query,
                cursor,
                &context,
                SqlCompletionRequest {
                    database_kind: DatabaseKind::PostgreSQL,
                    tables: &tables,
                    completion_columns: &columns,
                    selected_table: None,
                    active_columns: &[],
                    result: None,
                    active_schema_filter: None,
                },
            )
        };
        let has_keyword = |query: &str, keyword: &str| {
            items_for(query)
                .iter()
                .any(|item| item.kind == CompletionItemKind::Keyword && item.label == keyword)
        };

        let missing = [
            ("SELECT * FR", "FROM"),
            ("SELECT id FROM users WH", "WHERE"),
            ("SELECT * FROM users JO", "JOIN"),
            ("SELECT * FROM users ORDER B", "BY"),
        ]
        .into_iter()
        .filter(|(query, keyword)| !has_keyword(query, keyword))
        .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "missing keyword completions: {missing:?}"
        );
    }

    #[test]
    fn sql_completion_uses_visible_join_sources_and_projection_aliases() {
        let tables = vec![
            TableInfo::table("users", Some("public".into())),
            TableInfo::table("orders", Some("public".into())),
            TableInfo::table("accounts", Some("public".into())),
        ];
        let users = TableRef::in_schema("public", "users");
        let orders = TableRef::in_schema("public", "orders");
        let accounts = TableRef::in_schema("public", "accounts");
        let mut columns = HashMap::new();
        columns.insert(
            completion_table_key(&users),
            vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("email", 1, "TEXT"),
            ],
        );
        columns.insert(
            completion_table_key(&orders),
            vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("user_id", 1, "INTEGER"),
                ColumnInfo::result("total", 2, "DECIMAL"),
            ],
        );
        columns.insert(
            completion_table_key(&accounts),
            vec![ColumnInfo::result("account_name", 0, "TEXT")],
        );
        let sources = |query_result| SqlCompletionRequest {
            database_kind: DatabaseKind::PostgreSQL,
            tables: &tables,
            completion_columns: &columns,
            selected_table: Some(&users),
            active_columns: &[],
            result: query_result,
            active_schema_filter: Some("public"),
        };

        let qualified_query = "SELECT u. FROM users u";
        let qualified_cursor = "SELECT u.".len();
        let qualified_context =
            editor::sql_completion_context(qualified_query, qualified_cursor).unwrap();
        let qualified_items = sql_completion_items(
            qualified_query,
            qualified_cursor,
            &qualified_context,
            sources(None),
        );
        assert!(qualified_items.iter().any(|item| item.label == "email"));
        assert!(!qualified_items.iter().any(|item| item.label == "total"));

        let quoted_query = "SELECT \"public\".\"users\". FROM users u";
        let quoted_cursor = "SELECT \"public\".\"users\".".len();
        let quoted_context = editor::sql_completion_context(quoted_query, quoted_cursor).unwrap();
        let quoted_items =
            sql_completion_items(quoted_query, quoted_cursor, &quoted_context, sources(None));
        assert!(quoted_items.iter().any(|item| item.label == "email"));
        assert!(!quoted_items.iter().any(|item| item.label == "total"));

        let join_query = "SELECT * FROM users u JOIN orders o ON ";
        let join_context = editor::sql_completion_context(join_query, join_query.len()).unwrap();
        let join_items =
            sql_completion_items(join_query, join_query.len(), &join_context, sources(None));
        assert!(join_items.iter().any(|item| item.label == "email"));
        assert!(join_items.iter().any(|item| item.label == "total"));

        let comma_query = "SELECT * FROM users, orders WHERE ";
        let comma_context = editor::sql_completion_context(comma_query, comma_query.len()).unwrap();
        let comma_items = sql_completion_items(
            comma_query,
            comma_query.len(),
            &comma_context,
            sources(None),
        );
        assert!(comma_items.iter().any(|item| item.label == "email"));
        assert!(comma_items.iter().any(|item| item.label == "total"));

        let alias_query = "SELECT u.id AS user_id FROM users u ORDER BY us";
        let alias_context = editor::sql_completion_context(alias_query, alias_query.len()).unwrap();
        let alias_items = sql_completion_items(
            alias_query,
            alias_query.len(),
            &alias_context,
            sources(None),
        );
        assert!(
            alias_items
                .iter()
                .any(|item| { item.kind == CompletionItemKind::Column && item.label == "user_id" })
        );

        let nested_query = "SELECT * FROM users u WHERE EXISTS (SELECT 1 FROM orders u WHERE u.)";
        let nested_cursor = nested_query.find("u.)").unwrap() + 2;
        let nested_context = editor::sql_completion_context(nested_query, nested_cursor).unwrap();
        let nested_items =
            sql_completion_items(nested_query, nested_cursor, &nested_context, sources(None));
        assert!(nested_items.iter().any(|item| item.label == "total"));
        assert!(!nested_items.iter().any(|item| item.label == "email"));

        let sibling_query = "SELECT * FROM users u WHERE EXISTS (SELECT 1 FROM orders o WHERE o.id = 1) AND EXISTS (SELECT 1 FROM accounts a WHERE ";
        let sibling_context =
            editor::sql_completion_context(sibling_query, sibling_query.len()).unwrap();
        let sibling_items = sql_completion_items(
            sibling_query,
            sibling_query.len(),
            &sibling_context,
            sources(None),
        );
        assert!(
            sibling_items
                .iter()
                .any(|item| item.label == "account_name")
        );
        assert!(sibling_items.iter().any(|item| item.label == "email"));
        assert!(!sibling_items.iter().any(|item| item.label == "total"));

        let multi_statement_query = "SELECT o. ; SELECT * FROM orders o";
        let multi_statement_cursor = "SELECT o.".len();
        let multi_statement_context =
            editor::sql_completion_context(multi_statement_query, multi_statement_cursor).unwrap();
        let multi_statement_items = sql_completion_items(
            multi_statement_query,
            multi_statement_cursor,
            &multi_statement_context,
            sources(None),
        );
        assert!(
            !multi_statement_items
                .iter()
                .any(|item| item.label == "total")
        );
    }

    #[test]
    fn sql_completion_understands_insert_lists_ctes_and_derived_sources() {
        let tables = vec![TableInfo::table("users", Some("public".into()))];
        let users = TableRef::in_schema("public", "users");
        let user_columns = vec![
            ColumnInfo::result("id", 0, "INTEGER"),
            ColumnInfo::result("email", 1, "TEXT"),
        ];
        let mut columns = HashMap::new();
        columns.insert(completion_table_key(&users), user_columns.clone());
        let sources = || SqlCompletionRequest {
            database_kind: DatabaseKind::PostgreSQL,
            tables: &tables,
            completion_columns: &columns,
            selected_table: Some(&users),
            active_columns: &user_columns,
            result: None,
            active_schema_filter: Some("public"),
        };

        let insert_query = "INSERT INTO users (email, ";
        let insert_context =
            editor::sql_completion_context(insert_query, insert_query.len()).unwrap();
        let insert_items =
            sql_completion_items(insert_query, insert_query.len(), &insert_context, sources());
        assert!(insert_items.iter().any(|item| item.label == "id"));
        assert!(!insert_items.iter().any(|item| item.label == "email"));
        assert!(
            !insert_items
                .iter()
                .any(|item| item.kind == CompletionItemKind::Table)
        );

        let values_query = "INSERT INTO users VALUES (";
        let values_context =
            editor::sql_completion_context(values_query, values_query.len()).unwrap();
        let values_items =
            sql_completion_items(values_query, values_query.len(), &values_context, sources());
        assert!(
            !values_items
                .iter()
                .any(|item| item.kind == CompletionItemKind::Column)
        );

        let ddl_query = "ALTER TABLE users ADD COLUMN created_at TIM";
        let ddl_context = editor::sql_completion_context(ddl_query, ddl_query.len()).unwrap();
        let ddl_items = sql_completion_items(ddl_query, ddl_query.len(), &ddl_context, sources());
        assert!(
            ddl_items
                .iter()
                .any(|item| { item.kind == CompletionItemKind::Type && item.label == "TIMESTAMP" })
        );

        let cte_query =
            "WITH recent(user_id) AS (SELECT id FROM users) SELECT * FROM recent r WHERE r.";
        let cte_context = editor::sql_completion_context(cte_query, cte_query.len()).unwrap();
        let cte_items = sql_completion_items(cte_query, cte_query.len(), &cte_context, sources());
        assert!(cte_items.iter().any(|item| item.label == "user_id"));
        assert!(!cte_items.iter().any(|item| item.label == "id"));

        let derived_query = "SELECT * FROM (SELECT email FROM users) recent WHERE recent.";
        let derived_context =
            editor::sql_completion_context(derived_query, derived_query.len()).unwrap();
        let derived_items = sql_completion_items(
            derived_query,
            derived_query.len(),
            &derived_context,
            sources(),
        );
        assert!(derived_items.iter().any(|item| item.label == "email"));
    }
}
