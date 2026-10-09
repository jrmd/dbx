//! SQL language service: as-you-type diagnostics from a real SQL parser and
//! the connection's catalog.
//!
//! Diagnostics are deliberately conservative. A false error on valid SQL is
//! worse than a missed one, so syntax errors are reported only for statement
//! kinds the parser handles reliably, every engine parser must reject the
//! statement, and catalog warnings require loaded metadata and a single,
//! unambiguous table.

use std::{collections::HashSet, ops::ControlFlow, ops::Range};

use sqlparser::{
    ast::{
        AssignmentTarget, Expr, FromTable, Ident, ObjectName, ObjectNamePart, Query, SelectItem,
        SetExpr, Statement, TableFactor, TableObject, Visit, Visitor,
    },
    dialect::{
        BigQueryDialect, ClickHouseDialect, Dialect, DuckDbDialect, MsSqlDialect, MySqlDialect,
        PostgreSqlDialect, SQLiteDialect, SnowflakeDialect,
    },
    parser::Parser,
    tokenizer::Span,
};

use crate::{DatabaseKind, script::sql_statement_spans};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

/// A problem in the script, as a UTF-8 byte range of the script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlDiagnostic {
    pub range: Range<usize>,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

/// One table or view the connection exposes. `columns` is `None` until the
/// table's columns have been loaded.
#[derive(Clone, Debug, Default)]
pub struct CatalogTable {
    pub schema: Option<String>,
    pub name: String,
    pub columns: Option<Vec<String>>,
}

/// The catalog metadata diagnostics are checked against.
#[derive(Clone, Debug, Default)]
pub struct LanguageCatalog {
    pub tables: Vec<CatalogTable>,
}

impl LanguageCatalog {
    fn find(&self, name: &ObjectName) -> Lookup<'_> {
        let parts = name
            .0
            .iter()
            .map(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        let Some(parts) = parts else {
            return Lookup::Unchecked;
        };
        let (schema, table) = match parts.as_slice() {
            [table] => (None, *table),
            [schema, table] => (Some(*schema), *table),
            _ => return Lookup::Unchecked,
        };
        // Catalog tables this metadata never lists.
        let lowered = table.to_ascii_lowercase();
        if lowered.starts_with("pg_") || lowered.starts_with("sqlite_") || lowered == "dual" {
            return Lookup::Unchecked;
        }
        if let Some(schema) = schema
            && !self.tables.iter().any(|candidate| {
                candidate
                    .schema
                    .as_deref()
                    .is_some_and(|known| known.eq_ignore_ascii_case(schema))
            })
        {
            // Another database or a system schema such as information_schema.
            return Lookup::Unchecked;
        }
        self.tables
            .iter()
            .find(|candidate| {
                candidate.name.eq_ignore_ascii_case(table)
                    && schema.is_none_or(|schema| {
                        candidate
                            .schema
                            .as_deref()
                            .is_some_and(|known| known.eq_ignore_ascii_case(schema))
                    })
            })
            .map_or(Lookup::Missing, Lookup::Found)
    }
}

enum Lookup<'a> {
    Found(&'a CatalogTable),
    Missing,
    Unchecked,
}

/// The parser for an engine, or `None` where DBX has no SQL grammar.
fn dialect_for(kind: DatabaseKind) -> Option<Box<dyn Dialect>> {
    Some(match kind.dialect() {
        DatabaseKind::PostgreSQL => Box::new(PostgreSqlDialect {}),
        DatabaseKind::MySQL => Box::new(MySqlDialect {}),
        DatabaseKind::SQLite => Box::new(SQLiteDialect {}),
        DatabaseKind::DuckDB => Box::new(DuckDbDialect {}),
        DatabaseKind::SqlServer => Box::new(MsSqlDialect {}),
        DatabaseKind::Snowflake => Box::new(SnowflakeDialect {}),
        DatabaseKind::BigQuery => Box::new(BigQueryDialect {}),
        DatabaseKind::ClickHouse => Box::new(ClickHouseDialect {}),
        _ => return None,
    })
}

/// Words that may begin a statement in some supported engine. A statement
/// starting with anything else is almost certainly a typo.
const STATEMENT_LEADERS: &[&str] = &[
    "ABORT",
    "ALTER",
    "ANALYZE",
    "ATTACH",
    "BACKUP",
    "BEGIN",
    "CACHE",
    "CALL",
    "CHECK",
    "CHECKPOINT",
    "CLOSE",
    "CLUSTER",
    "COMMENT",
    "COMMIT",
    "COPY",
    "CREATE",
    "DBCC",
    "DEALLOCATE",
    "DECLARE",
    "DELETE",
    "DELIMITER",
    "DESC",
    "DESCRIBE",
    "DETACH",
    "DISCARD",
    "DO",
    "DROP",
    "END",
    "EXEC",
    "EXECUTE",
    "EXPLAIN",
    "FETCH",
    "FLUSH",
    "GET",
    "GO",
    "GRANT",
    "HANDLER",
    "IF",
    "IMPORT",
    "INSERT",
    "INSTALL",
    "KILL",
    "LIST",
    "LISTEN",
    "LOAD",
    "LOCK",
    "MERGE",
    "MOVE",
    "NOTIFY",
    "OPTIMIZE",
    "PRAGMA",
    "PREPARE",
    "PRINT",
    "PURGE",
    "PUT",
    "RAISERROR",
    "REFRESH",
    "REINDEX",
    "RELEASE",
    "REMOVE",
    "RENAME",
    "REPAIR",
    "REPLACE",
    "RESET",
    "RESTORE",
    "REVOKE",
    "ROLLBACK",
    "SAVEPOINT",
    "SECURITY",
    "SELECT",
    "SET",
    "SHOW",
    "START",
    "SUMMARIZE",
    "SYSTEM",
    "TABLE",
    "THROW",
    "TRUNCATE",
    "TRY",
    "UNDROP",
    "UNINSTALL",
    "UNLISTEN",
    "UNLOCK",
    "UPDATE",
    "UPSERT",
    "USE",
    "VACUUM",
    "VALUES",
    "WATCH",
    "WHILE",
    "WITH",
];

/// Whether the parser's grammar for a statement starting with `words` is
/// complete enough to trust its syntax errors.
fn parser_trusted(words: &[String]) -> bool {
    let word = |index: usize| words.get(index).map(String::as_str);
    match word(0) {
        Some(
            "SELECT" | "WITH" | "INSERT" | "UPDATE" | "DELETE" | "VALUES" | "TRUNCATE" | "DROP"
            | "MERGE",
        ) => true,
        Some("ALTER") => word(1) == Some("TABLE"),
        Some("CREATE") => {
            let object = words[1..]
                .iter()
                .find(|word| !matches!(word.as_str(), "OR" | "REPLACE" | "TEMP" | "TEMPORARY"));
            object.is_some_and(|object| {
                matches!(
                    object.as_str(),
                    "TABLE" | "VIEW" | "INDEX" | "UNIQUE" | "SCHEMA" | "SEQUENCE"
                )
            })
        }
        _ => false,
    }
}

/// Analyze `script` for `kind`. `caret` is the editing position: an
/// unfinished statement there is not reported as ending too early.
pub fn analyze_sql(
    kind: DatabaseKind,
    script: &str,
    catalog: &LanguageCatalog,
    caret: Option<usize>,
) -> Vec<SqlDiagnostic> {
    let Some(dialect) = dialect_for(kind) else {
        return Vec::new();
    };
    let mut diagnostics = Vec::new();
    let mut parsed = Vec::new();
    for span in sql_statement_spans(Some(kind), script) {
        if span.delimiter != ";" {
            continue;
        }
        let text = &script[span.range.clone()];
        let words = crate::protected::sql_words_for(Some(kind), text);
        let Some(first) = words.first() else {
            continue;
        };
        let base = span.range.start;
        if !STATEMENT_LEADERS.contains(&first.as_str()) {
            if let Some(range) = word_range(text, first) {
                diagnostics.push(SqlDiagnostic {
                    range: base + range.start..base + range.end,
                    message: format!("Unknown statement `{}`", &text[range]),
                    severity: DiagnosticSeverity::Error,
                });
            }
            continue;
        }
        match Parser::parse_sql(dialect.as_ref(), text) {
            Ok(statements) => parsed.push((base, text, statements)),
            Err(error) if parser_trusted(&words) => {
                let message = error.to_string();
                let (message, location) = split_location(&message);
                let ends_early = location.is_none();
                let caret_here =
                    caret.is_some_and(|caret| span.range.start <= caret && caret <= span.range.end);
                if ends_early && caret_here {
                    continue;
                }
                let range = location
                    .and_then(|(line, column)| offset_of(text, line, column))
                    .map(|offset| found_token_range(text, offset, &message))
                    .unwrap_or_else(|| last_token_range(text));
                diagnostics.push(SqlDiagnostic {
                    range: base + range.start..base + range.end,
                    message,
                    severity: DiagnosticSeverity::Error,
                });
            }
            Err(_) => {}
        }
    }
    if catalog.tables.is_empty() {
        return diagnostics;
    }
    // Tables created anywhere in the document are not reported as unknown.
    let mut created = HashSet::new();
    for (_, _, statements) in &parsed {
        for statement in statements {
            if let Statement::CreateTable(create) = statement
                && let Some(ObjectNamePart::Identifier(ident)) = create.name.0.last()
            {
                created.insert(ident.value.to_ascii_lowercase());
            }
            if let Statement::CreateView(create) = statement
                && let Some(ObjectNamePart::Identifier(ident)) = create.name.0.last()
            {
                created.insert(ident.value.to_ascii_lowercase());
            }
        }
    }
    for (base, text, statements) in &parsed {
        for statement in statements {
            check_catalog(statement, text, *base, catalog, &created, &mut diagnostics);
        }
    }
    diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
    diagnostics
}

/// Separate sqlparser's `... at Line: 3, Column: 7` suffix from its message.
fn split_location(message: &str) -> (String, Option<(u64, u64)>) {
    let message = message
        .strip_prefix("sql parser error: ")
        .unwrap_or(message);
    let Some((text, location)) = message.rsplit_once(" at Line: ") else {
        return (tidy_message(message), None);
    };
    let location = location
        .split_once(", Column: ")
        .and_then(|(line, column)| Some((line.trim().parse().ok()?, column.trim().parse().ok()?)));
    (tidy_message(text), location)
}

fn tidy_message(message: &str) -> String {
    let message = message.replace("Expected: ", "Expected ");
    let message = message.replace(", found: EOF", " before the statement ends");
    message.replace(", found: ", ", found ")
}

/// The byte offset of a 1-based line and character column.
fn offset_of(text: &str, line: u64, column: u64) -> Option<usize> {
    let start = if line <= 1 {
        0
    } else {
        text.match_indices('\n')
            .nth(usize::try_from(line).ok()? - 2)
            .map(|(index, _)| index + 1)?
    };
    let line_text = &text[start..];
    let column = usize::try_from(column).ok()?.saturating_sub(1);
    Some(
        start
            + line_text
                .char_indices()
                .nth(column)
                .map_or(line_text.len(), |(index, _)| index),
    )
}

/// The token an error names (`..., found FROM`). The parser sometimes
/// reports the position after it, so the nearest occurrence at or before
/// `offset` is used, falling back to the token at `offset`.
fn found_token_range(text: &str, offset: usize, message: &str) -> Range<usize> {
    let found = message
        .rsplit_once(", found ")
        .map(|(_, found)| found.trim())
        .filter(|found| !found.is_empty());
    if let Some(found) = found {
        let window_end = (offset + found.len()).min(text.len());
        let window_end = (window_end..=text.len())
            .find(|index| text.is_char_boundary(*index))
            .unwrap_or(text.len());
        if let Some(start) = text[..window_end]
            .to_ascii_lowercase()
            .rfind(&found.to_ascii_lowercase())
        {
            return start..start + found.len();
        }
    }
    token_range(text, offset)
}

/// The token starting at `offset`: a word, or a single character.
fn token_range(text: &str, offset: usize) -> Range<usize> {
    let word = text[offset..]
        .char_indices()
        .take_while(|(_, character)| character.is_alphanumeric() || *character == '_')
        .last()
        .map(|(index, character)| offset + index + character.len_utf8());
    let end =
        word.unwrap_or_else(|| offset + text[offset..].chars().next().map_or(0, char::len_utf8));
    offset..end.max(offset)
}

fn last_token_range(text: &str) -> Range<usize> {
    let trimmed = text.trim_end().trim_end_matches(';').trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, character)| !character.is_whitespace())
        .last()
        .map_or(trimmed.len(), |(index, _)| index);
    start..trimmed.len()
}

/// The range of the first occurrence of `word` (case-insensitively) outside
/// comments: the statement's first keyword.
fn word_range(text: &str, word: &str) -> Option<Range<usize>> {
    let upper = text.to_ascii_uppercase();
    let mut from = 0;
    while let Some(found) = upper[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let boundary = |index: usize| {
            text[..index]
                .chars()
                .next_back()
                .is_none_or(|character| !character.is_alphanumeric() && character != '_')
        };
        let line = &text[text[..start].rfind('\n').map_or(0, |index| index + 1)..start];
        if boundary(start)
            && text[end..]
                .chars()
                .next()
                .is_none_or(|character| !character.is_alphanumeric() && character != '_')
            && !line.contains("--")
        {
            return Some(start..end);
        }
        from = end;
    }
    None
}

fn span_range(text: &str, base: usize, span: Span) -> Option<Range<usize>> {
    let start = offset_of(text, span.start.line, span.start.column)?;
    let end = offset_of(text, span.end.line, span.end.column)?;
    (end > start).then_some(base + start..base + end)
}

/// Collects the relations, CTE names and identifiers a statement uses.
#[derive(Default)]
struct References {
    tables: Vec<(ObjectName, Option<Ident>)>,
    table_functions: usize,
    ctes: HashSet<String>,
    queries: usize,
    identifiers: Vec<Vec<Ident>>,
    aliases: HashSet<String>,
}

impl Visitor for References {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        self.queries += 1;
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.ctes.insert(cte.alias.name.value.to_ascii_lowercase());
            }
        }
        if let SetExpr::Select(select) = query.body.as_ref() {
            for item in &select.projection {
                if let SelectItem::ExprWithAlias { alias, .. } = item {
                    self.aliases.insert(alias.value.to_ascii_lowercase());
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<()> {
        match factor {
            TableFactor::Table {
                name,
                alias,
                args: None,
                ..
            } => self
                .tables
                .push((name.clone(), alias.as_ref().map(|alias| alias.name.clone()))),
            _ => self.table_functions += 1,
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        match expr {
            Expr::Identifier(ident) => self.identifiers.push(vec![ident.clone()]),
            Expr::CompoundIdentifier(parts) => self.identifiers.push(parts.clone()),
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

/// Built-in values that parse as bare identifiers in some dialects.
const NILADIC: &[&str] = &[
    "current_date",
    "current_time",
    "current_timestamp",
    "localtime",
    "localtimestamp",
    "current_user",
    "session_user",
    "current_role",
    "current_schema",
    "current_catalog",
    "user",
    "sysdate",
    "true",
    "false",
    "null",
    "default",
];

fn check_catalog(
    statement: &Statement,
    text: &str,
    base: usize,
    catalog: &LanguageCatalog,
    created: &HashSet<String>,
    diagnostics: &mut Vec<SqlDiagnostic>,
) {
    let (insert_target, insert_columns, assignments) = match statement {
        Statement::Query(_) | Statement::Delete(_) => (None, &[][..], Vec::new()),
        Statement::Insert(insert) => match &insert.table {
            TableObject::TableName(name) => (Some(name), insert.columns.as_slice(), Vec::new()),
            _ => return,
        },
        Statement::Update(update) => (
            None,
            &[][..],
            update
                .assignments
                .iter()
                .filter_map(|assignment| match &assignment.target {
                    AssignmentTarget::ColumnName(name) => Some(name.clone()),
                    _ => None,
                })
                .collect(),
        ),
        _ => return,
    };
    let mut references = References::default();
    let _ = statement.visit(&mut references);
    if let Statement::Delete(delete) = statement {
        let from = match &delete.from {
            FromTable::WithFromKeyword(from) | FromTable::WithoutKeyword(from) => from,
        };
        if from.is_empty() {
            return;
        }
    }
    if let Some(target) = insert_target {
        references.tables.push((target.clone(), None));
    }
    let known = |name: &ObjectName| {
        let last = name
            .0
            .last()
            .and_then(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident.value.to_ascii_lowercase()),
                _ => None,
            })
            .unwrap_or_default();
        name.0.len() == 1 && (references.ctes.contains(&last) || created.contains(&last))
    };
    let mut resolved = Vec::new();
    for (name, alias) in &references.tables {
        if known(name) {
            continue;
        }
        match catalog.find(name) {
            Lookup::Found(table) => resolved.push((table, alias.clone(), name)),
            Lookup::Missing => {
                if let Some(range) = name
                    .0
                    .last()
                    .and_then(|part| match part {
                        ObjectNamePart::Identifier(ident) => Some(ident.span),
                        _ => None,
                    })
                    .and_then(|span| span_range(text, base, span))
                {
                    diagnostics.push(SqlDiagnostic {
                        range,
                        message: format!("Unknown table `{name}`"),
                        severity: DiagnosticSeverity::Warning,
                    });
                }
            }
            Lookup::Unchecked => {}
        }
    }
    // INSERT columns and UPDATE targets name columns of the target table.
    let target = match statement {
        Statement::Insert(_) => insert_target.map(|target| catalog.find(target)),
        Statement::Update(_) => references
            .tables
            .first()
            .map(|(name, _)| catalog.find(name)),
        _ => None,
    };
    if let Some(Lookup::Found(table)) = target
        && let Some(columns) = &table.columns
    {
        report_unknown_columns(insert_columns, columns, table, text, base, diagnostics);
        report_unknown_columns(&assignments, columns, table, text, base, diagnostics);
    }
    // Expressions are only checked against one unambiguous table.
    let [(table, alias, name)] = resolved.as_slice() else {
        return;
    };
    if references.tables.len() != 1 || references.table_functions > 0 || references.queries > 1 {
        return;
    }
    let Some(columns) = &table.columns else {
        return;
    };
    let has_column = |ident: &Ident| {
        columns
            .iter()
            .any(|column| column.eq_ignore_ascii_case(&ident.value))
    };
    let table_name = name.0.last().map(ToString::to_string).unwrap_or_default();
    let qualifier = |ident: &Ident| {
        ident.value.eq_ignore_ascii_case(&table.name)
            || ident
                .value
                .eq_ignore_ascii_case(table_name.trim_matches(['"', '`']))
            || alias
                .as_ref()
                .is_some_and(|alias| alias.value.eq_ignore_ascii_case(&ident.value))
    };
    for parts in &references.identifiers {
        let column = match parts.as_slice() {
            [column] => {
                let lowered = column.value.to_ascii_lowercase();
                if column.quote_style.is_none()
                    && (NILADIC.contains(&lowered.as_str())
                        || column.value.starts_with(['@', '$', ':']))
                    || references.aliases.contains(&lowered)
                    || qualifier(column)
                {
                    continue;
                }
                column
            }
            [prefix, column] if qualifier(prefix) => column,
            _ => continue,
        };
        if !has_column(column)
            && let Some(range) = span_range(text, base, column.span)
        {
            diagnostics.push(SqlDiagnostic {
                range,
                message: format!("Unknown column `{}` in `{}`", column.value, table.name),
                severity: DiagnosticSeverity::Warning,
            });
        }
    }
}

fn report_unknown_columns(
    names: &[ObjectName],
    columns: &[String],
    table: &CatalogTable,
    text: &str,
    base: usize,
    diagnostics: &mut Vec<SqlDiagnostic>,
) {
    for name in names {
        let Some(ObjectNamePart::Identifier(ident)) = name.0.last() else {
            continue;
        };
        if !columns
            .iter()
            .any(|column| column.eq_ignore_ascii_case(&ident.value))
            && let Some(range) = span_range(text, base, ident.span)
        {
            diagnostics.push(SqlDiagnostic {
                range,
                message: format!("Unknown column `{}` in `{}`", ident.value, table.name),
                severity: DiagnosticSeverity::Warning,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> LanguageCatalog {
        LanguageCatalog {
            tables: vec![
                CatalogTable {
                    schema: Some("public".into()),
                    name: "users".into(),
                    columns: Some(vec!["id".into(), "name".into(), "email".into()]),
                },
                CatalogTable {
                    schema: Some("public".into()),
                    name: "orders".into(),
                    columns: None,
                },
            ],
        }
    }

    fn found(script: &str) -> Vec<(String, String)> {
        analyze_sql(DatabaseKind::PostgreSQL, script, &catalog(), None)
            .into_iter()
            .map(|diagnostic| (script[diagnostic.range].to_owned(), diagnostic.message))
            .collect()
    }

    #[test]
    fn syntax_errors_point_at_the_offending_token() {
        assert_eq!(
            found("SELECT 1;\nSELECT a, FROM users"),
            [(
                "FROM".to_owned(),
                "Expected an expression, found FROM".to_owned()
            )]
        );
        assert_eq!(
            found("SELEC 1"),
            [("SELEC".to_owned(), "Unknown statement `SELEC`".to_owned())]
        );
        let unfinished = "SELECT * FROM users WHERE";
        assert_eq!(found(unfinished).len(), 1);
        assert!(
            analyze_sql(
                DatabaseKind::PostgreSQL,
                unfinished,
                &catalog(),
                Some(unfinished.len())
            )
            .is_empty(),
            "the statement being typed is not reported as unfinished"
        );
    }

    #[test]
    fn valid_engine_specific_statements_stay_clean() {
        for script in [
            "SELECT name #>> '{a}', id::int FROM users WHERE name ILIKE $1",
            "DO $$ BEGIN RAISE NOTICE 'x'; END $$",
            "VACUUM ANALYZE users",
            "REFRESH MATERIALIZED VIEW mv",
            "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END",
            "WITH recent AS (SELECT id FROM users) SELECT id FROM recent",
            "SELECT * FROM generate_series(1, 3)",
            "SELECT * FROM information_schema.tables",
            "SELECT * FROM pg_class",
            "CREATE TEMP TABLE scratch (id int); SELECT * FROM scratch",
            "SELECT current_date, name AS label FROM users ORDER BY label",
        ] {
            assert_eq!(found(script), [], "{script}");
        }
        for (kind, script) in [
            (DatabaseKind::MySQL, "SET @x := 1"),
            (
                DatabaseKind::MySQL,
                "CREATE PROCEDURE p() BEGIN SELECT 1; END",
            ),
            (DatabaseKind::SQLite, "PRAGMA table_info(users)"),
            (
                DatabaseKind::MySQL,
                "SELECT * FROM users # comment; with a semicolon",
            ),
        ] {
            assert_eq!(analyze_sql(kind, script, &catalog(), None), [], "{script}");
        }
    }

    #[test]
    fn catalog_warnings_cover_unknown_tables_and_columns() {
        assert_eq!(
            found("SELECT * FROM userz"),
            [("userz".to_owned(), "Unknown table `userz`".to_owned())]
        );
        assert_eq!(
            found("SELECT nmae FROM users u WHERE u.emial = 'x'"),
            [
                (
                    "nmae".to_owned(),
                    "Unknown column `nmae` in `users`".to_owned()
                ),
                (
                    "emial".to_owned(),
                    "Unknown column `emial` in `users`".to_owned()
                )
            ]
        );
        assert_eq!(
            found("UPDATE users SET nme = 'x' WHERE id = 1"),
            [(
                "nme".to_owned(),
                "Unknown column `nme` in `users`".to_owned()
            )]
        );
        assert_eq!(
            found("INSERT INTO users (id, mail) SELECT id, total FROM orders"),
            [(
                "mail".to_owned(),
                "Unknown column `mail` in `users`".to_owned()
            )]
        );
        // Joins and unloaded columns are not guessed at.
        assert_eq!(found("SELECT anything FROM users JOIN orders ON true"), []);
        assert_eq!(found("SELECT anything FROM orders"), []);
        assert!(
            analyze_sql(
                DatabaseKind::PostgreSQL,
                "SELECT * FROM userz",
                &LanguageCatalog::default(),
                None
            )
            .is_empty()
        );
    }
}
