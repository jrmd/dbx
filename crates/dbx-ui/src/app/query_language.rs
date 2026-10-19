//! Connects the SQL editor to the language service and the session schema:
//! live diagnostics, hover cards and go-to-definition.
use std::time::Duration;

use dbx_core::language::{CatalogTable, LanguageCatalog, SqlDiagnostic, analyze_sql};

use gpui::{Task, WeakEntity};

use super::*;
use crate::editor::{EditorDiagnostic, HoverInfo, SqlLanguageHost, SqlTokenKind};

/// Pause after the last edit before the script is analyzed.
const ANALYSIS_DELAY: Duration = Duration::from_millis(300);
/// Larger documents (pasted dumps) are not analyzed while typing.
const ANALYSIS_LIMIT: usize = 256 * 1024;

/// What a set of diagnostics was computed from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LanguageKey {
    revision: u64,
    tables: usize,
    columns: usize,
}

#[derive(Default)]
pub(super) struct LanguageState {
    key: Option<LanguageKey>,
    pending: Option<LanguageKey>,
    diagnostics: Vec<SqlDiagnostic>,
    _task: Option<Task<()>>,
}

fn catalog_for(session: &ConnectionSession) -> LanguageCatalog {
    LanguageCatalog {
        tables: session
            .tables
            .iter()
            .map(|table| CatalogTable {
                schema: table.schema.clone(),
                name: table.name.clone(),
                columns: session
                    .completion_columns
                    .get(&completion_table_key(&table_ref(table)))
                    .map(|columns| columns.iter().map(|column| column.name.clone()).collect()),
            })
            .collect(),
    }
}

impl DbxApp {
    /// Diagnostics to paint in a query editor: the last run's error position
    /// and the language service's current findings. Starts a new analysis
    /// when the text or schema changed.
    pub(super) fn query_editor_diagnostics(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Vec<EditorDiagnostic> {
        let Some(session) = self.session(session_id) else {
            return Vec::new();
        };
        let kind = session.kind;
        let catalog_size = (session.tables.len(), session.completion_columns.len());
        let catalog = (|| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            let key = LanguageKey {
                revision: query.query_revision,
                tables: catalog_size.0,
                columns: catalog_size.1,
            };
            let stale = query.language.key != Some(key) && query.language.pending != Some(key);
            Some((tab_id, key, stale.then(|| catalog_for(session))))
        })();
        let Some((tab_id, key, catalog)) = catalog else {
            return Vec::new();
        };
        if let Some(catalog) = catalog {
            self.schedule_query_analysis(session_id, tab_id, kind, key, catalog, cx);
        }
        let Some(query) = self.active_query_tab(session_id) else {
            return Vec::new();
        };
        let mut diagnostics = Vec::new();
        if query.error.is_some()
            && let Some(range) = query.error_highlight.clone()
        {
            diagnostics.push(EditorDiagnostic {
                range,
                severity: editor::DiagnosticSeverity::Error,
                message: query.error.clone().unwrap_or_default().into(),
            });
        }
        // Findings for older text would point at the wrong characters.
        if query
            .language
            .key
            .is_some_and(|analyzed| analyzed.revision == key.revision)
        {
            diagnostics.extend(query.language.diagnostics.iter().map(|diagnostic| {
                EditorDiagnostic {
                    range: diagnostic.range.clone(),
                    severity: diagnostic.severity,
                    message: diagnostic.message.clone().into(),
                }
            }));
        }
        diagnostics
    }

    fn schedule_query_analysis(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        kind: DatabaseKind,
        key: LanguageKey,
        catalog: LanguageCatalog,
        cx: &mut Context<Self>,
    ) {
        let Some(query) = self.query_tab_mut(session_id, tab_id) else {
            return;
        };
        let editor = query.query_editor.clone();
        query.language.pending = Some(key);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ANALYSIS_DELAY).await;
            let (text, caret) =
                editor.read_with(cx, |editor, cx| (editor.text(cx), editor.cursor_offset()));
            let diagnostics = if text.len() > ANALYSIS_LIMIT {
                Vec::new()
            } else {
                cx.background_spawn(async move { analyze_sql(kind, &text, &catalog, Some(caret)) })
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(query) = this.query_tab_mut(session_id, tab_id)
                    && query.language.pending == Some(key)
                {
                    query.language.pending = None;
                    query.language.key = Some(key);
                    query.language.diagnostics = diagnostics;
                    cx.notify();
                }
            });
        });
        if let Some(query) = self.query_tab_mut(session_id, tab_id) {
            query.language._task = Some(task);
        }
    }

    pub(super) fn query_tab_mut(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
    ) -> Option<&mut QueryTab> {
        let tab = self
            .session_mut(session_id)?
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)?;
        match &mut tab.kind {
            SecondaryTabKind::Query(query) => Some(query),
            _ => None,
        }
    }
}

/// Schema lookups for one session's query editors.
pub(super) struct QueryLanguageHost {
    pub(super) app: WeakEntity<DbxApp>,
    pub(super) session_id: SessionId,
}

/// What owns a column: a table or a CTE.
enum Owner {
    Table(TableInfo),
    Cte(String),
}

impl Owner {
    fn name(&self) -> &str {
        match self {
            Self::Table(table) => &table.name,
            Self::Cte(name) => name,
        }
    }
}

/// A table, CTE or column the pointer names.
enum Resolved {
    Table(TableInfo),
    Cte(String, Vec<ColumnInfo>),
    Column(Owner, ColumnInfo),
}

fn unquote(raw: &str) -> &str {
    raw.trim_matches(['"', '`', '[', ']'])
}

fn column_list(columns: &[ColumnInfo]) -> Option<String> {
    if columns.is_empty() {
        return None;
    }
    let shown = columns
        .iter()
        .take(8)
        .map(|column| {
            if column.data_type.is_empty() {
                column.name.clone()
            } else {
                format!("{} {}", column.name, column.data_type.to_lowercase())
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let more = columns.len().saturating_sub(8);
    Some(if more > 0 {
        format!("{shown}, … {more} more")
    } else {
        shown
    })
}

impl QueryLanguageHost {
    fn resolve(&self, text: &str, offset: usize, cx: &App) -> Option<(Range<usize>, Resolved)> {
        let app = self.app.upgrade()?;
        let session = app.read(cx).session(self.session_id)?;
        let dialect = Some(session.kind);
        let statement = editor::sql_statement_range(text, offset, dialect);
        let statement_text = &text[statement.clone()];
        let tokens = editor::lex_sql_for(statement_text, dialect);
        let local = offset - statement.start;
        let index = tokens.iter().position(|token| {
            token.range.contains(&local)
                && matches!(token.kind, SqlTokenKind::Identifier | SqlTokenKind::Keyword)
        })?;
        let token = &tokens[index];
        let name = unquote(&statement_text[token.range.clone()]);
        let range = statement.start + token.range.start..statement.start + token.range.end;
        // `qualifier.name`: the token before a dot directly in front of it.
        let qualifier = (index > 0 && statement_text[..token.range.start].ends_with('.'))
            .then(|| unquote(&statement_text[tokens[index - 1].range.clone()]));
        let find_table = |name: &str, schema: Option<&str>| {
            session.tables.iter().find(|table| {
                table.name.eq_ignore_ascii_case(name)
                    && schema.is_none_or(|schema| {
                        table
                            .schema
                            .as_deref()
                            .is_some_and(|known| known.eq_ignore_ascii_case(schema))
                    })
            })
        };
        let (ctes, sources) = statement_sources(
            text,
            offset,
            &SqlCompletionRequest {
                database_kind: session.kind,
                tables: &session.tables,
                completion_columns: &session.completion_columns,
                selected_table: None,
                active_columns: &[],
                result: None,
                active_schema_filter: session.schema_filter.as_deref(),
            },
        );
        let owner = |source: &StatementSource| {
            if source.cte {
                Some(Owner::Cte(source.relation.clone()))
            } else {
                find_table(&source.relation, source.schema.as_deref())
                    .map(|table| Owner::Table(table.clone()))
            }
        };
        let as_resolved = |owner: Owner| match owner {
            Owner::Table(table) => Resolved::Table(table),
            Owner::Cte(name) => {
                let columns = ctes
                    .iter()
                    .find(|cte| cte.relation.eq_ignore_ascii_case(&name))
                    .map(|cte| cte.columns.clone())
                    .unwrap_or_default();
                Resolved::Cte(name, columns)
            }
        };
        if qualifier.is_none() {
            if let Some(cte) = ctes
                .iter()
                .find(|cte| cte.relation.eq_ignore_ascii_case(name))
            {
                return Some((range, as_resolved(Owner::Cte(cte.relation.clone()))));
            }
            if let Some(owner) = sources
                .iter()
                .find(|source| {
                    source
                        .alias
                        .as_deref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(name))
                })
                .and_then(owner)
            {
                return Some((range, as_resolved(owner)));
            }
        }
        if let Some(table) = find_table(name, qualifier) {
            return Some((range, Resolved::Table(table.clone())));
        }
        let column = sources
            .iter()
            .filter(|source| {
                qualifier.is_none_or(|qualifier| {
                    source.relation.eq_ignore_ascii_case(qualifier)
                        || source
                            .alias
                            .as_deref()
                            .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
                })
            })
            .find_map(|source| {
                let column = source
                    .columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))?;
                Some(Resolved::Column(owner(source)?, column.clone()))
            })?;
        Some((range, column))
    }
}

fn qualified_name(table: &TableInfo) -> String {
    match &table.schema {
        Some(schema) => format!("{schema}.{}", table.name),
        None => table.name.clone(),
    }
}

impl SqlLanguageHost for QueryLanguageHost {
    fn hover(&self, text: &str, offset: usize, cx: &App) -> Option<HoverInfo> {
        let (range, resolved) = self.resolve(text, offset, cx)?;
        Some(match resolved {
            Resolved::Table(table) => {
                let app = self.app.upgrade()?;
                let columns = app
                    .read(cx)
                    .session(self.session_id)?
                    .completion_columns
                    .get(&completion_table_key(&table_ref(&table)))
                    .cloned()
                    .unwrap_or_default();
                let kind = match table.kind {
                    EntityKind::View => "view",
                    _ => "table",
                };
                HoverInfo {
                    range,
                    title: format!("{kind} {}", qualified_name(&table)).into(),
                    detail: column_list(&columns).map(Into::into),
                }
            }
            Resolved::Cte(name, columns) => HoverInfo {
                range,
                title: format!("CTE {name}").into(),
                detail: column_list(&columns).map(Into::into),
            },
            Resolved::Column(owner, column) => {
                let mut facts = Vec::new();
                if !column.data_type.is_empty() {
                    facts.push(column.data_type.to_lowercase());
                }
                if column.primary_key {
                    facts.push("primary key".into());
                }
                if !column.nullable && matches!(owner, Owner::Table(_)) {
                    facts.push("not null".into());
                }
                if let Some(default) = &column.default_value {
                    facts.push(format!("default {default}"));
                }
                HoverInfo {
                    range,
                    title: format!("{}.{}", owner.name(), column.name).into(),
                    detail: (!facts.is_empty()).then(|| facts.join(" · ").into()),
                }
            }
        })
    }

    fn go_to_definition(
        &self,
        text: &str,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some((_, resolved)) = self.resolve(text, offset, cx) else {
            return false;
        };
        let table = match resolved {
            Resolved::Table(table) | Resolved::Column(Owner::Table(table), _) => table,
            // CTEs are defined in the editor, which jumps there itself.
            Resolved::Cte(..) | Resolved::Column(Owner::Cte(_), _) => return false,
        };
        let session_id = self.session_id;
        let Some(app) = self.app.upgrade() else {
            return false;
        };
        // The editor is mid-update; open the table once its event finishes.
        window.defer(cx, move |window, cx| {
            app.update(cx, |app, cx| {
                app.select_table_for(session_id, table, window, cx)
            });
        });
        true
    }
}
