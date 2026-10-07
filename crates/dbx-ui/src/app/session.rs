//! Connection and document state, independent of action handlers.
use super::*;

pub(super) struct SessionEditors {
    pub(super) filter_text: Entity<String>,
    pub(super) filter_editor: Entity<TextEditor>,
    /// Text typed into the sidebar's table search field.
    pub(super) sidebar_search: Entity<String>,
    pub(super) sidebar_search_editor: Entity<TextEditor>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl SessionEditors {
    pub(super) fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let filter_text = cx.new(|_| String::new());
        let filter_editor = cx.new(|cx| TextEditor::new(filter_text.clone(), false, window, cx));
        let sidebar_search = cx.new(|_| String::new());
        let sidebar_search_editor =
            cx.new(|cx| TextEditor::new(sidebar_search.clone(), false, window, cx));
        let subscriptions = vec![
            cx.observe(&filter_text, |_, _, cx| cx.notify()),
            cx.observe(&sidebar_search, |_, _, cx| cx.notify()),
        ];

        Self {
            filter_text,
            filter_editor,
            sidebar_search,
            sidebar_search_editor,
            _subscriptions: subscriptions,
        }
    }
}

/// Height shared by every control at the top of the sidebar.
pub(super) const SIDEBAR_CONTROL_HEIGHT: f32 = 28.0;
pub(super) const ALL_SCHEMAS_LABEL: &str = "All schemas";

pub(super) type SidebarSelect = Entity<SelectState<SearchableVec<SharedString>>>;

/// The sidebar's database and schema dropdowns plus the cached list of rows
/// they and the search field produce.
pub(super) struct SidebarState {
    pub(super) database_select: SidebarSelect,
    pub(super) schema_select: SidebarSelect,
    pub(super) synced_databases: Vec<String>,
    pub(super) synced_schemas: Vec<Option<String>>,
    pub(super) list: SidebarList,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl SidebarState {
    pub(super) fn new(
        id: SessionId,
        kind: DatabaseKind,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        let make_select = |window: &mut Window, cx: &mut Context<DbxApp>| {
            cx.new(|select_cx| {
                SelectState::new(
                    SearchableVec::new(Vec::<SharedString>::new()),
                    None,
                    window,
                    select_cx,
                )
                .searchable(true)
            })
        };
        let database_select = make_select(window, cx);
        let schema_select = make_select(window, cx);
        let database_subscription = cx.subscribe_in(
            &database_select,
            window,
            move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let database = this.session(id).and_then(|session| {
                    session
                        .databases
                        .iter()
                        .find(|database| database_label(kind, database) == value.as_ref())
                        .cloned()
                });
                if let Some(database) = database {
                    this.switch_database_for(id, database, cx);
                }
                // A refused switch (busy session) must snap the select back.
                cx.notify();
            },
        );
        let schema_subscription = cx.subscribe_in(
            &schema_select,
            window,
            move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let schema = (value.as_ref() != ALL_SCHEMAS_LABEL).then(|| value.to_string());
                this.select_schema_filter_for(id, schema, cx);
                cx.notify();
            },
        );
        Self {
            database_select,
            schema_select,
            synced_databases: Vec::new(),
            synced_schemas: Vec::new(),
            list: SidebarList::default(),
            _subscriptions: vec![database_subscription, schema_subscription],
        }
    }

    /// Push changed items into the dropdowns and keep their selection equal to
    /// the session's real state. Cheap when nothing changed.
    pub(super) fn sync_selectors(
        &mut self,
        kind: DatabaseKind,
        databases: &[String],
        current_database: Option<&str>,
        schema_filter: Option<&str>,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) {
        let databases_changed = self.synced_databases != databases;
        if databases_changed {
            let labels = databases
                .iter()
                .map(|database| SharedString::from(database_label(kind, database)))
                .collect::<Vec<_>>();
            self.database_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(labels), window, cx)
            });
            self.synced_databases = databases.to_vec();
        }
        let database_index = current_database
            .and_then(|current| databases.iter().position(|database| database == current))
            .map(IndexPath::new);
        sync_select_index(
            &self.database_select,
            database_index,
            databases_changed,
            window,
            cx,
        );

        let schema_options = self.list.schema_options.clone();
        let schemas_changed = *self.synced_schemas != *schema_options;
        if schemas_changed {
            let labels = schema_options
                .iter()
                .map(|schema| {
                    SharedString::from(schema.clone().unwrap_or_else(|| ALL_SCHEMAS_LABEL.into()))
                })
                .collect::<Vec<_>>();
            self.schema_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(labels), window, cx)
            });
            self.synced_schemas = schema_options.to_vec();
        }
        let schema_index = schema_options
            .iter()
            .position(|schema| schema.as_deref() == schema_filter)
            .map(IndexPath::new);
        sync_select_index(
            &self.schema_select,
            schema_index,
            schemas_changed,
            window,
            cx,
        );
    }
}

pub(super) fn sync_select_index(
    select: &SidebarSelect,
    index: Option<IndexPath>,
    force: bool,
    window: &mut Window,
    cx: &mut Context<DbxApp>,
) {
    if force || select.read(cx).selected_index(cx) != index {
        select.update(cx, |select, cx| {
            select.set_selected_index(index, window, cx)
        });
    }
}

pub(super) fn database_label(kind: DatabaseKind, database: &str) -> String {
    if kind == DatabaseKind::Redis {
        format!("db{database}")
    } else {
        database.to_owned()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SidebarListKey {
    pub(super) revision: u64,
    pub(super) schema_filter: Option<String>,
    pub(super) search: String,
}

/// The rows the sidebar shows, rebuilt only when the table list, the schema
/// filter or the search text changes rather than on every frame.
#[derive(Default)]
pub(super) struct SidebarList {
    pub(super) key: Option<SidebarListKey>,
    pub(super) visible: Arc<Vec<TableInfo>>,
    pub(super) schema_options: Arc<Vec<Option<String>>>,
}

impl SidebarList {
    /// Returns whether the list was rebuilt.
    pub(super) fn refresh(
        &mut self,
        kind: DatabaseKind,
        tables: &[TableInfo],
        revision: u64,
        schema_filter: Option<&str>,
        search: &str,
    ) -> bool {
        let search = search.trim();
        if self.key.as_ref().is_some_and(|key| {
            key.revision == revision
                && key.schema_filter.as_deref() == schema_filter
                && key.search == search
        }) {
            return false;
        }
        if self.key.as_ref().is_none_or(|key| key.revision != revision) {
            self.schema_options = Arc::new(schema_filter_options(kind, tables));
        }
        let needle = search.to_lowercase();
        self.visible = Arc::new(
            tables
                .iter()
                .filter(|table| {
                    table_is_visible_in(kind, schema_filter, table.schema.as_deref())
                        && (needle.is_empty() || table.name.to_lowercase().contains(&needle))
                })
                .cloned()
                .collect(),
        );
        self.key = Some(SidebarListKey {
            revision,
            schema_filter: schema_filter.map(str::to_owned),
            search: search.to_owned(),
        });
        true
    }
}

pub(super) type SecondaryTabId = Uuid;

pub(super) struct SqlCompletionMenu {
    pub(super) replacement_range: Range<usize>,
    pub(super) items: Vec<SqlCompletionItem>,
    pub(super) selected: usize,
    pub(super) signature: CompletionSignature,
}

/// A completion state identity without retaining a second copy of the query.
/// The query entity increments its revision whenever its text changes; the
/// caret offset distinguishes otherwise identical documents at different
/// insertion points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompletionSignature {
    pub(super) text_revision: u64,
    pub(super) cursor: usize,
}

#[derive(Default)]
pub(super) struct AbortOnDrop(Option<tokio::task::AbortHandle>);

impl AbortOnDrop {
    pub(super) fn replace(&mut self, handle: tokio::task::AbortHandle) {
        self.cancel();
        self.0 = Some(handle);
    }

    pub(super) fn cancel(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Default)]
pub(super) struct BackgroundTaskSet(Vec<tokio::task::AbortHandle>);

impl BackgroundTaskSet {
    pub(super) fn has_pending(&self) -> bool {
        self.0.iter().any(|handle| !handle.is_finished())
    }

    pub(super) fn track<T>(&mut self, task: &tokio::task::JoinHandle<T>) {
        // Completed tasks no longer need an abort handle. Sweeping here keeps
        // this owner-scoped cancellation set bounded even when a connection
        // performs many sequential refreshes or metadata requests.
        self.0.retain(|handle| !handle.is_finished());
        self.0.push(task.abort_handle());
    }

    pub(super) fn cancel_all(&mut self) {
        for handle in self.0.drain(..) {
            handle.abort();
        }
    }
}

impl Drop for BackgroundTaskSet {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

pub(super) struct QueryTab {
    pub(super) timeout_secs: u64,
    pub(super) plan_pending: bool,
    pub(super) plan: Option<QueryResult>,
    pub(super) plan_baseline: Option<QueryResult>,
    pub(super) name: Entity<String>,
    pub(super) name_editor: Entity<TextEditor>,
    pub(super) console: Option<Arc<QuerySession>>,
    pub(super) cancellation: Option<QueryCancellation>,
    pub(super) statement_results: Vec<StatementResult>,
    pub(super) active_result: usize,
    pub(super) in_transaction: bool,
    pub(super) execution_override: Option<String>,
    pub(super) prepared_parameters: Option<Vec<dbx_core::SqlStatement>>,
    pub(super) export_target: Entity<TextEditor>,
    pub(super) inspected_value: Option<CellValue>,
    /// Values last entered for `:name` placeholders, the open prompt, and
    /// whether the current run has confirmed them.
    pub(super) parameter_values: HashMap<String, String>,
    pub(super) parameter_prompt: Option<query_parameters::ParameterPrompt>,
    pub(super) parameters_ready: bool,
    pub(super) find: Option<find::FindBar>,
    pub(super) agent: agents::AgentQuery,
    pub(super) query_text: Entity<String>,
    pub(super) query_editor: Entity<TextEditor>,
    pub(super) result: Option<Arc<QueryResult>>,
    pub(super) result_grid: Entity<TableState<ResultTableDelegate>>,
    pub(super) split_state: Entity<ResizableState>,
    pub(super) result_selection: QueryResultSelection,
    pub(super) result_column_widths: HashMap<String, Pixels>,
    pub(super) busy: bool,
    /// The last result remains visible while a newer request is in flight or
    /// has failed, but must not be mistaken for the newest execution.
    pub(super) results_stale: bool,
    pub(super) status: String,
    pub(super) error: Option<String>,
    pub(super) executed_database: Option<String>,
    pub(super) abort_handle: AbortOnDrop,
    /// The byte range an error message points at. Query text edits increment
    /// `query_revision` and clear this range before it can be painted again.
    pub(super) error_highlight: Option<Range<usize>>,
    pub(super) request_generation: u64,
    pub(super) query_revision: u64,
    pub(super) completion_signature: Option<CompletionSignature>,
    pub(super) completion_dismissed_signature: Option<CompletionSignature>,
    pub(super) completion_index: usize,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl QueryTab {
    pub(super) fn new(
        kind: DatabaseKind,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        let name = cx.new(|_| "Untitled".to_owned());
        let name_editor = cx.new(|cx| TextEditor::new(name.clone(), false, window, cx));
        let query_text = cx.new(|_| DbxApp::default_query(kind).to_owned());
        let query_editor = cx.new(|cx| match query_editor_language(kind) {
            editor::EditorLanguage::Sql => TextEditor::new_sql(query_text.clone(), window, cx),
            editor::EditorLanguage::Redis => TextEditor::new_redis(query_text.clone(), window, cx),
            editor::EditorLanguage::Json => TextEditor::new_json(query_text.clone(), window, cx),
            editor::EditorLanguage::PlainText => {
                TextEditor::new(query_text.clone(), true, window, cx)
            }
        });
        let split_state = cx.new(|_| ResizableState::default());
        let row_actions = ResultTableDelegate::with_row_actions(
            cx.entity().downgrade(),
            session_id,
            tab_id,
            false,
        );
        let result_grid = cx.new(|cx| {
            TableState::new(row_actions, window, cx)
                .col_resizable(true)
                .col_movable(false)
                .sortable(true)
                .row_selectable(true)
                .col_selectable(true)
                .cell_selectable(true)
                .row_header(false)
        });
        let text_subscription = cx.observe(&query_text, move |this, _, cx| {
            if let Some(session) = this.session_mut(session_id)
                && let Some(tab) = session
                    .secondary_tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                && let SecondaryTabKind::Query(query) = &mut tab.kind
            {
                query.query_revision = query.query_revision.wrapping_add(1);
                query.results_stale = query.result.is_some();
                query.error_highlight = None;
                query.completion_signature = None;
                query.completion_dismissed_signature = None;
            }
            this.persist_query_workspace_for(session_id, cx);
            cx.notify();
        });
        let editor_subscription = cx.observe(&query_editor, |_, _, cx| cx.notify());
        let name_subscription = cx.observe(&name, move |this, _, cx| {
            this.persist_query_workspace_for(session_id, cx);
            cx.notify();
        });
        let table_subscription =
            cx.subscribe_in(&result_grid, window, move |this, _, event, _, cx| {
                this.on_query_grid_event(session_id, tab_id, event, cx)
            });

        Self {
            name,
            timeout_secs: 60,
            plan_pending: false,
            plan: None,
            plan_baseline: None,
            name_editor,
            console: None,
            cancellation: None,
            statement_results: Vec::new(),
            active_result: 0,
            in_transaction: false,
            execution_override: None,
            prepared_parameters: None,
            export_target: cx.new(|cx| TextEditor::empty(false, window, cx)),
            inspected_value: None,
            parameter_values: HashMap::new(),
            parameter_prompt: None,
            parameters_ready: false,
            find: None,
            agent: agents::AgentQuery::new(window, cx),
            query_text,
            query_editor,
            result: None,
            result_grid,
            split_state,
            result_selection: QueryResultSelection::None,
            result_column_widths: HashMap::new(),
            busy: false,
            results_stale: false,
            status: String::new(),
            error: None,
            executed_database: None,
            abort_handle: AbortOnDrop::default(),
            error_highlight: None,
            request_generation: 0,
            query_revision: 0,
            completion_signature: None,
            completion_dismissed_signature: None,
            completion_index: 0,
            _subscriptions: vec![
                text_subscription,
                name_subscription,
                editor_subscription,
                table_subscription,
            ],
        }
    }

    pub(super) fn set_result(&mut self, result: Option<QueryResult>, cx: &mut Context<DbxApp>) {
        self.result = result.map(Arc::new);
        self.result_selection = QueryResultSelection::None;
        let result = self.result.clone();
        let remembered_widths = self.result_column_widths.clone();
        self.result_grid.update(cx, move |table, cx| {
            table
                .delegate_mut()
                .set_result(result, &remembered_widths, &[], &[]);
            table.clear_selection(cx);
            table.refresh(cx);
        });
    }

    pub(super) fn invalidate_request(&mut self) {
        self.request_generation = self.request_generation.saturating_add(1);
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.abort_handle.cancel();
        self.console = None;
        self.in_transaction = false;
        self.busy = false;
        self.agent.cancel();
        self.agent.result = None;
    }
}

pub(super) fn query_editor_language(kind: DatabaseKind) -> editor::EditorLanguage {
    if kind.is_sql() {
        editor::EditorLanguage::Sql
    } else {
        match kind {
            DatabaseKind::Redis => editor::EditorLanguage::Redis,
            DatabaseKind::MongoDB | DatabaseKind::Kafka => editor::EditorLanguage::Json,
            _ => editor::EditorLanguage::PlainText,
        }
    }
}

impl Drop for QueryTab {
    fn drop(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.abort_handle.cancel();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum QueryResultSelection {
    #[default]
    None,
    Cell,
    Row,
    Column,
}

pub(super) fn query_result_status(result: &QueryResult) -> String {
    let outcome = match (result.rows_affected, result.rows.is_empty()) {
        (Some(affected), false) => format!(
            "{} row{} returned · {affected} row{} affected",
            result.rows.len(),
            if result.rows.len() == 1 { "" } else { "s" },
            if affected == 1 { "" } else { "s" }
        ),
        (Some(affected), true) => format!(
            "{affected} row{} affected",
            if affected == 1 { "" } else { "s" }
        ),
        (None, _) => format!(
            "{} row{} returned",
            result.rows.len(),
            if result.rows.len() == 1 { "" } else { "s" }
        ),
    };
    let truncation = if result.truncated {
        " · results limited"
    } else {
        ""
    };
    format!("{outcome} · {} ms{truncation}", result.elapsed_ms)
}

pub(super) fn query_history_connection(
    session: &ConnectionSession,
) -> Option<QueryHistoryConnection> {
    session
        .profile_id
        .map(QueryHistoryConnection::profile)
        .or_else(|| {
            QueryHistoryConnection::session(
                session.name.clone(),
                session.kind,
                session
                    .current_database
                    .clone()
                    .unwrap_or_else(|| "default".into()),
            )
            .ok()
        })
}

pub(super) struct StructureTab {
    pub(super) designer: Option<designer::Designer>,
    pub(super) table: TableRef,
    pub(super) columns: Vec<ColumnInfo>,
    pub(super) foreign_keys: Vec<ForeignKeyInfo>,
    pub(super) indexes: Vec<dbx_core::IndexInfo>,
    pub(super) checks: Vec<dbx_core::CheckConstraintInfo>,
    pub(super) definition: Option<String>,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
}

/// Per-tab state for a database-wide relationship diagram. The document is
/// deliberately independent from GPUI so SVG and PNG exports share the exact
/// same layout as the on-screen canvas.
pub(super) struct DiagramTab {
    /// The complete metadata snapshot is retained so schema filters can
    /// rebuild the scene without another database round-trip.
    pub(super) source_schema: Option<Arc<RelationalSchema>>,
    pub(super) document: Option<Arc<DiagramDocument>>,
    pub(super) available_schemas: Vec<String>,
    /// PostgreSQL-only projection. `None` means every available schema.
    pub(super) selected_schemas: Option<BTreeSet<String>>,
    pub(super) busy: bool,
    pub(super) stale: bool,
    pub(super) error: Option<String>,
    pub(super) zoom: f32,
    pub(super) selected_node: Option<String>,
    pub(super) scroll_handle: ScrollHandle,
    pub(super) focus: FocusHandle,
    pub(super) drag_anchor: Option<DiagramDragAnchor>,
    /// A card being rearranged by pointer drag.
    pub(super) node_drag: Option<DiagramNodeDrag>,
    /// Card positions the user arranged by hand, keyed by node ID. They
    /// survive refreshes and schema-filter rebuilds until the layout is reset.
    pub(super) arranged_positions: HashMap<String, (f32, f32)>,
    pub(super) request_generation: u64,
    pub(super) abort_handle: AbortOnDrop,
}

#[derive(Clone, Copy)]
pub(super) struct DiagramDragAnchor {
    pub(super) pointer: Point<Pixels>,
    pub(super) scroll_offset: Point<Pixels>,
    /// Whether the press travelled far enough to count as a pan rather than
    /// a click on empty canvas (which clears the selection).
    pub(super) moved: bool,
}

#[derive(Clone)]
pub(super) struct DiagramNodeDrag {
    pub(super) node_id: String,
    pub(super) pointer: Point<Pixels>,
    pub(super) origin: (f32, f32),
}

impl DiagramTab {
    pub(super) fn loading(
        kind: DatabaseKind,
        tables: &[TableInfo],
        explorer_schema: Option<&str>,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        Self {
            source_schema: None,
            document: None,
            available_schemas: diagram_schema_names(kind, tables),
            selected_schemas: diagram_initial_schema_selection(kind, explorer_schema),
            busy: true,
            stale: false,
            error: None,
            zoom: 1.0,
            selected_node: None,
            scroll_handle: ScrollHandle::new(),
            focus: cx.focus_handle(),
            drag_anchor: None,
            node_drag: None,
            arranged_positions: HashMap::new(),
            request_generation: 0,
            abort_handle: AbortOnDrop::default(),
        }
    }

    pub(super) fn invalidate_request(&mut self) {
        self.request_generation = self.request_generation.saturating_add(1);
        self.abort_handle.cancel();
        self.busy = false;
    }
}

impl Drop for DiagramTab {
    fn drop(&mut self) {
        self.abort_handle.cancel();
    }
}

/// Where a browsed page begins. Keyset and Redis starts are only known after
/// the previous page loads, so a data tab remembers the start of every page it
/// has visited to make Previous exact.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PageStart {
    Offset,
    /// Rows strictly after (or before, when descending) this primary key.
    After(CellValue),
    /// A Redis `SCAN` cursor.
    RedisCursor(u64),
}

/// One open table. Each tab owns its grid, filters, page, and row draft, so
/// switching between tables keeps every view exactly where the user left it.
pub(super) struct DataTab {
    pub(super) table: TableRef,
    pub(super) data_grid: Entity<TableState<ResultTableDelegate>>,
    pub(super) result_column_widths: HashMap<String, Pixels>,
    pub(super) _data_grid_subscription: Subscription,
    pub(super) filters: FilterModel,
    pub(super) filter_subscriptions: Vec<Subscription>,
    pub(super) table_columns: Vec<ColumnInfo>,
    pub(super) foreign_keys: Vec<ForeignKeyInfo>,
    pub(super) result: Option<Arc<QueryResult>>,
    /// The table that produced `result`, when it is safe to edit through the
    /// grid. Cleared while a reload is in flight.
    pub(super) result_table: Option<TableRef>,
    pub(super) table_page: u64,
    pub(super) table_has_next_page: bool,
    /// `page_starts[p]` is how page `p` began; `next_page_start` continues
    /// after the current page.
    pub(super) page_starts: Vec<PageStart>,
    pub(super) next_page_start: Option<PageStart>,
    /// Header sort applied as `ORDER BY` when the engine supports it.
    pub(super) sort: Option<Order>,
    pub(super) sortable: bool,
    /// Inline cell values staged for the next save, and the open cell editor.
    pub(super) pending_edits: cell_edits::PendingEdits,
    pub(super) cell_editor: Option<cell_edits::CellEditor>,
    pub(super) selected_row: Option<usize>,
    pub(super) selected_column: usize,
    pub(super) inspector_open: bool,
    pub(super) draft_mode: DraftMode,
    pub(super) row_draft: Option<RowDraftModel>,
    pub(super) row_draft_subscriptions: Vec<Subscription>,
    pub(super) suppress_next_grid_selection_event: bool,
    pub(super) busy: bool,
    pub(super) status: String,
    pub(super) error: Option<String>,
    pub(super) request_generation: u64,
    pub(super) abort_handle: AbortOnDrop,
}

impl DataTab {
    pub(super) fn new(
        session_id: SessionId,
        id: SecondaryTabId,
        table: TableRef,
        sortable: bool,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        let row_actions =
            ResultTableDelegate::with_row_actions(cx.entity().downgrade(), session_id, id, true);
        let data_grid = cx.new(|cx| {
            TableState::new(row_actions, window, cx)
                .col_resizable(true)
                .col_movable(false)
                .sortable(sortable)
                .row_selectable(true)
                .col_selectable(true)
                .cell_selectable(true)
        });
        let data_grid_subscription =
            cx.subscribe_in(&data_grid, window, move |this, _, event, window, cx| {
                this.on_data_grid_event(session_id, id, event, window, cx)
            });
        Self {
            table,
            data_grid,
            result_column_widths: HashMap::new(),
            _data_grid_subscription: data_grid_subscription,
            filters: FilterModel::new(),
            filter_subscriptions: Vec::new(),
            table_columns: Vec::new(),
            foreign_keys: Vec::new(),
            result: None,
            result_table: None,
            table_page: 0,
            table_has_next_page: false,
            page_starts: Vec::new(),
            next_page_start: None,
            sort: None,
            sortable,
            pending_edits: Default::default(),
            cell_editor: None,
            selected_row: None,
            selected_column: 0,
            inspector_open: false,
            draft_mode: DraftMode::Update,
            row_draft: None,
            row_draft_subscriptions: Vec::new(),
            suppress_next_grid_selection_event: false,
            busy: false,
            status: String::new(),
            error: None,
            request_generation: 0,
            abort_handle: AbortOnDrop::default(),
        }
    }

    pub(super) fn set_result(
        &mut self,
        result: Option<QueryResult>,
        tables: &[TableInfo],
        cx: &mut Context<DbxApp>,
    ) {
        self.result = result.map(Arc::new);
        self.sync_result_grid(true, tables, cx);
    }

    pub(super) fn sync_result_grid(
        &mut self,
        clear_selection: bool,
        tables: &[TableInfo],
        cx: &mut Context<DbxApp>,
    ) {
        let result = self.result.clone();
        let remembered_widths = self.result_column_widths.clone();
        let foreign_keys = self.foreign_keys.clone();
        let tables = tables.to_vec();
        let (sortable, sort) = (self.sortable, self.sort.clone());
        self.data_grid.update(cx, move |table, cx| {
            let delegate = table.delegate_mut();
            delegate.set_server_sort(sortable, sort.as_ref());
            delegate.set_result(result, &remembered_widths, &foreign_keys, &tables);
            table.refresh(cx);
            if clear_selection {
                table.clear_selection(cx);
            }
        });
    }

    pub(super) fn clear_grid_selection(&self, cx: &mut Context<DbxApp>) {
        self.data_grid
            .update(cx, |table, cx| table.clear_selection(cx));
    }

    /// Drop the row selection and any open draft ahead of a reload, whose
    /// result replaces the snapshot they point into.
    pub(super) fn reset_row_state(&mut self, cx: &mut Context<DbxApp>) {
        self.result_table = None;
        self.selected_row = None;
        self.row_draft = None;
        self.row_draft_subscriptions.clear();
        // Reloads are refused while edits are staged, so only an open editor
        // can remain here, and it points into the old page.
        self.cell_editor = None;
        self.sync_cell_edits(cx);
        self.clear_grid_selection(cx);
    }

    pub(super) fn invalidate_request(&mut self) {
        self.request_generation = self.request_generation.saturating_add(1);
        self.abort_handle.cancel();
        self.busy = false;
    }
}

impl Drop for DataTab {
    fn drop(&mut self) {
        self.abort_handle.cancel();
    }
}

pub(super) enum SecondaryTabKind {
    Data(Box<DataTab>),
    Query(Box<QueryTab>),
    Structure(Box<StructureTab>),
    Diagram(Box<DiagramTab>),
}

impl SecondaryTabKind {
    pub(super) fn pane(&self) -> Pane {
        match self {
            Self::Data(_) => Pane::Data,
            Self::Query(_) => Pane::Query,
            Self::Structure(_) => Pane::Structure,
            Self::Diagram(_) => Pane::Diagram,
        }
    }
}

pub(super) struct SecondaryTab {
    pub(super) id: SecondaryTabId,
    pub(super) kind: SecondaryTabKind,
}

/// Free functions rather than session methods so callers can hold a data tab
/// and still update the session's other fields in the same scope.
pub(super) fn find_data_tab(tabs: &[SecondaryTab], id: SecondaryTabId) -> Option<&DataTab> {
    tabs.iter().find_map(|tab| match &tab.kind {
        SecondaryTabKind::Data(data) if tab.id == id => Some(data.as_ref()),
        _ => None,
    })
}

pub(super) fn find_data_tab_mut(
    tabs: &mut [SecondaryTab],
    id: SecondaryTabId,
) -> Option<&mut DataTab> {
    tabs.iter_mut().find_map(|tab| match &mut tab.kind {
        SecondaryTabKind::Data(data) if tab.id == id => Some(data.as_mut()),
        _ => None,
    })
}

pub(super) struct ConnectionSession {
    pub(super) schema_baseline: Option<RelationalSchema>,
    pub(super) transfer_control: Option<dbx_core::TransferControl>,
    pub(super) id: SessionId,
    pub(super) profile_id: Option<Uuid>,
    pub(super) name: String,
    pub(super) kind: DatabaseKind,
    pub(super) tag: Option<ConnectionTag>,
    pub(super) engine: Option<Arc<DatabaseEngine>>,
    pub(super) editors: SessionEditors,
    pub(super) pane: Pane,
    pub(super) secondary_tabs: Vec<SecondaryTab>,
    pub(super) active_secondary_tab: Option<SecondaryTabId>,
    /// The data tab viewed most recently, even while another kind of tab is
    /// in front. Query completion and the Data/Structure rail use it.
    pub(super) recent_data_tab: Option<SecondaryTabId>,
    /// Recently closed query documents are retained for the current session
    /// only. Persisted history remains the durable source for executed work.
    pub(super) closed_queries: Vec<String>,
    pub(super) restoring_workspace: bool,
    pub(super) startup_layout: Option<crate::workspace::StartupConnection>,
    pub(super) tables: Vec<TableInfo>,
    /// Bumped whenever `tables` is replaced so derived lists know to rebuild.
    pub(super) tables_revision: u64,
    pub(super) schema_objects: Vec<dbx_core::SchemaObject>,
    pub(super) schema_objects_error: Option<String>,
    pub(super) sidebar: SidebarState,
    /// Schema metadata already fetched for completion. The navigator always
    /// supplies table names; columns are added as tables are opened or their
    /// structure is inspected, avoiding a metadata query for every keystroke.
    pub(super) completion_columns: HashMap<String, Vec<ColumnInfo>>,
    /// Authoritative command grammar discovered once from the connected
    /// Redis/Valkey server. Completion only reads this cache; it never performs
    /// network I/O while the user is typing.
    pub(super) redis_command_catalog: Option<Arc<RedisCommandCatalog>>,
    /// Databases reachable through this connection, for the sidebar switcher.
    pub(super) databases: Vec<String>,
    /// Database the engine currently uses, if the backend reports one.
    pub(super) current_database: Option<String>,
    /// PostgreSQL-only navigator filter. `None` means all schemas.
    pub(super) schema_filter: Option<String>,
    /// Connection-wide work (connecting, switching databases, table actions,
    /// transfers). Each data tab tracks its own loads and row mutations.
    pub(super) busy: bool,
    pub(super) status: String,
    pub(super) error: Option<String>,
    pub(super) request_generation: u64,
    /// Tokio work captures an `Arc<DatabaseEngine>`. Keep abort handles here
    /// so closing a connection cancels that work before dropping the session
    /// instead of leaving closed pools alive until every query completes.
    pub(super) background_tasks: BackgroundTaskSet,
}

impl ConnectionSession {
    pub(super) fn new(
        id: SessionId,
        profile_id: Option<Uuid>,
        name: String,
        kind: DatabaseKind,
        tag: Option<ConnectionTag>,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        Self {
            id,
            profile_id,
            name,
            kind,
            tag,
            engine: None,
            editors: SessionEditors::new(window, cx),
            pane: Pane::Data,
            secondary_tabs: Vec::new(),
            active_secondary_tab: None,
            recent_data_tab: None,
            closed_queries: Vec::new(),
            restoring_workspace: false,
            startup_layout: None,
            tables: Vec::new(),
            tables_revision: 0,
            schema_objects: Vec::new(),
            schema_objects_error: None,
            sidebar: SidebarState::new(id, kind, window, cx),
            completion_columns: HashMap::new(),
            redis_command_catalog: None,
            databases: Vec::new(),
            current_database: None,
            schema_filter: None,
            busy: false,
            status: "Connecting…".into(),
            error: None,
            request_generation: 0,
            background_tasks: BackgroundTaskSet::default(),
            transfer_control: None,
            schema_baseline: None,
        }
    }

    pub(super) fn set_tables(&mut self, tables: Vec<TableInfo>) {
        self.tables = tables;
        self.tables_revision += 1;
        // A dropped schema would otherwise leave an empty list behind a
        // selector that hides itself when fewer than two schemas remain.
        if let Some(schema) = self.schema_filter.as_deref()
            && !self
                .tables
                .iter()
                .any(|table| table.schema.as_deref() == Some(schema))
        {
            self.schema_filter = default_schema_filter(self.kind, &self.tables);
        }
    }

    pub(super) fn track_background_task<T>(&mut self, task: &tokio::task::JoinHandle<T>) {
        self.background_tasks.track(task);
    }

    pub(super) fn cancel_background_tasks(&mut self) {
        self.background_tasks.cancel_all();
    }

    pub(super) fn data_tab(&self, id: SecondaryTabId) -> Option<&DataTab> {
        find_data_tab(&self.secondary_tabs, id)
    }

    pub(super) fn data_tab_mut(&mut self, id: SecondaryTabId) -> Option<&mut DataTab> {
        find_data_tab_mut(&mut self.secondary_tabs, id)
    }

    /// The data tab in front, if the front tab is one.
    pub(super) fn active_data_tab_id(&self) -> Option<SecondaryTabId> {
        self.active_secondary_tab
            .filter(|id| self.data_tab(*id).is_some())
    }

    pub(super) fn active_data_tab(&self) -> Option<&DataTab> {
        self.active_data_tab_id().and_then(|id| self.data_tab(id))
    }

    /// The data tab in front, else the one viewed most recently.
    pub(super) fn recent_data(&self) -> Option<&DataTab> {
        self.active_data_tab()
            .or_else(|| self.recent_data_tab.and_then(|id| self.data_tab(id)))
    }

    pub(super) fn data_tab_for_table(&self, table: &TableRef) -> Option<SecondaryTabId> {
        self.secondary_tabs.iter().find_map(|tab| match &tab.kind {
            SecondaryTabKind::Data(data) if data.table == *table => Some(tab.id),
            _ => None,
        })
    }

    /// Close every data tab matching `predicate`, moving the front tab to a
    /// neighbour when it was one of them.
    pub(super) fn close_data_tabs_where(&mut self, predicate: impl Fn(&DataTab) -> bool) {
        let active_index = self
            .active_secondary_tab
            .and_then(|id| self.secondary_tabs.iter().position(|tab| tab.id == id));
        let mut kept_before_active = 0;
        let mut active_closed = false;
        let mut index = 0;
        self.secondary_tabs.retain(|tab| {
            let close = matches!(&tab.kind, SecondaryTabKind::Data(data) if predicate(data));
            if Some(index) == active_index {
                active_closed = close;
            } else if !close && active_index.is_some_and(|active| index < active) {
                kept_before_active += 1;
            }
            index += 1;
            !close
        });
        if self
            .recent_data_tab
            .is_some_and(|id| self.data_tab(id).is_none())
        {
            self.recent_data_tab = None;
        }
        if active_closed {
            let next = kept_before_active.min(self.secondary_tabs.len().saturating_sub(1));
            self.active_secondary_tab = self.secondary_tabs.get(next).map(|tab| tab.id);
            self.pane = self
                .secondary_tabs
                .get(next)
                .map(|tab| tab.kind.pane())
                .unwrap_or(Pane::Data);
            if self.pane == Pane::Data {
                self.recent_data_tab = self.active_secondary_tab;
            }
        }
    }

    pub(super) fn close_data_tabs(&mut self) {
        self.close_data_tabs_where(|_| true);
    }
}

impl Drop for ConnectionSession {
    fn drop(&mut self) {
        self.cancel_background_tasks();
    }
}
