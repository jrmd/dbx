//! THESIS: DBX is a dense native database cockpit; it rejects the centered-card utility shell.
//! OWN-WORLD: Near-black layered panes, hairline borders, blue navigation, green health, 6px controls.
//! STORY: Pick or open a connection, keep its tab, browse context, then inspect or query without losing place.
//! FIRST VIEWPORT: A 46px rail, 40px primary connection tabs, explorer, data canvas, and row inspector.
//! FORM: Reference-led operator console, user-supplied DBX screen map; seed key: dbx-native-console.
//! FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md.
//! IMPLEMENTATION: `DbxApp` coordinates shared session state; focused workflows and rendering live in
//! the private `app/` module tree documented in `docs/architecture.md`.

mod connection;
mod redis_completion;
mod result_table;
mod sql_completion;
mod transfer;
mod view;

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

use dbx_core::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseEngine, DatabaseExportRequest, DatabaseKind,
    DumpFormat, EntityKind, Filter, FilterOperator, ForeignKeyInfo, InsertRequest, Page,
    QueryOptions, QueryResult, RedisCommandCatalog, ReferentialAction, RelationalSchema, RowData,
    TableInfo, TableRef, UpdateRequest, detect_file_format, export_database, export_table,
    import_database, import_file,
};
use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable as _, FontWeight,
    Image, ImageFormat, IntoElement, KeyDownEvent, MouseButton, PathPromptOptions, Pixels, Point,
    Render, Rgba, ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _,
    Subscription, Window, WindowControlArea, WindowHandle, anchored, deferred, div, img, point,
    prelude::*, px,
};
use gpui_component::{
    Disableable as _, FocusTrapElement as _, InteractiveElementExt as _, Selectable as _,
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    resizable::ResizableState,
    select::{SearchableVec, Select, SelectEvent},
    table::{DataTable, TableEvent, TableState},
};
use secrecy::SecretString;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::{
    assets::LOGO_BYTES,
    connection_fields::ConnectionFields,
    diagram::DiagramDocument,
    editor::{self, TextEditor},
    filters::{FilterModel, FilterRowId, filter_operator_options, operator_requires_value},
    profiles::{
        ConnectionEnvironment, ConnectionProfileDraft, ProfileStore, SavedConnection, sqlite_url,
    },
    query_history::{
        QueryHistoryConnection, QueryHistoryEntry, QueryHistoryOutcome, QueryHistoryStore,
    },
    row_drafts::{FieldId, FieldRow, FieldValueKind, FieldValueState, RowDraftModel},
    settings::{Settings, SettingsStore},
    theme::{
        Appearance, ButtonKind, Icon, appearance, badge, button, connection_tab, database_logo,
        icon, panel_header, set_appearance, theme,
    },
    vault::VaultState,
};
use redis_completion::redis_completion_items;
use result_table::{ResultTableDelegate, foreign_key_target_table};
use sql_completion::{
    CompletionItemKind, SqlCompletionItem, SqlCompletionRequest, completion_table_key,
    sql_completion_items,
};

const DIAGRAM_SCENE_PADDING: f32 = 24.0;

gpui::actions!(
    dbx_ui,
    [
        RunQuery,
        RunQueryAll,
        CancelQuery,
        CopyQuerySelection,
        FormatQuery,
        RefreshData,
        CompletionUp,
        CompletionDown,
        CompletionEnter,
        DiagramPanLeft,
        DiagramPanRight,
        DiagramPanUp,
        DiagramPanDown,
        DiagramPanLeftLarge,
        DiagramPanRightLarge,
        DiagramPanUpLarge,
        DiagramPanDownLarge,
        DiagramZoomIn,
        DiagramZoomOut,
        DiagramResetView,
        DiagramFit,
        DiagramRefresh,
        VaultFocusNext,
        VaultFocusPrevious,
        SubmitVault
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pane {
    Data,
    Structure,
    Query,
    Diagram,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DraftMode {
    Insert,
    Update,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionAction {
    Up,
    Down,
    Enter,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum QueryResultExportFormat {
    Tsv,
    Csv,
    Json,
}

/// The diagram renderer owns the actual SVG/PNG encoding; app state owns the
/// native save flow so the view never has to reach into platform APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiagramExportFormat {
    Svg,
    Png,
}

impl DiagramExportFormat {
    pub(super) fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }
}

impl QueryResultExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Tsv => "tsv",
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionFormMode {
    Details,
    ConnectionString,
}

type SessionId = Uuid;

const TABLE_BROWSE_PAGE_SIZE: u32 = 1_000;
const TABLE_BROWSE_QUERY_LIMIT: u32 = TABLE_BROWSE_PAGE_SIZE + 1;

fn table_browse_page(page: u64) -> Page {
    Page {
        limit: TABLE_BROWSE_QUERY_LIMIT,
        offset: page.saturating_mul(u64::from(TABLE_BROWSE_PAGE_SIZE)),
    }
}

fn trim_table_browse_result(result: &mut QueryResult) -> bool {
    let has_next_page = result.rows.len() > TABLE_BROWSE_PAGE_SIZE as usize;
    result.rows.truncate(TABLE_BROWSE_PAGE_SIZE as usize);
    has_next_page
}

fn window_close_button() -> Stateful<Div> {
    div()
        .id("window-close")
        .size(px(28.))
        .rounded(px(5.))
        .border_1()
        .border_color(theme().border_strong)
        .bg(theme().panel)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(theme().danger).border_color(theme().danger))
        .child(icon(Icon::Close, theme().text).size(px(12.)))
}

struct ConnectionDraft {
    kind: DatabaseKind,
    mode: ConnectionFormMode,
    selected_profile: Option<Uuid>,
    environment: ConnectionEnvironment,
    connection_name: Entity<String>,
    connection_name_editor: Entity<TextEditor>,
    connection_url: Entity<String>,
    connection_editor: Entity<TextEditor>,
    host: Entity<String>,
    host_editor: Entity<TextEditor>,
    port: Entity<String>,
    port_editor: Entity<TextEditor>,
    username: Entity<String>,
    username_editor: Entity<TextEditor>,
    password: Entity<String>,
    password_editor: Entity<TextEditor>,
    database: Entity<String>,
    database_editor: Entity<TextEditor>,
}

struct VaultEditors {
    passphrase: Entity<String>,
    passphrase_editor: Entity<TextEditor>,
    confirmation: Entity<String>,
    confirmation_editor: Entity<TextEditor>,
}

impl VaultEditors {
    fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let passphrase = cx.new(|_| String::new());
        let confirmation = cx.new(|_| String::new());
        let passphrase_editor =
            cx.new(|cx| TextEditor::new(passphrase.clone(), false, window, cx).password());
        let confirmation_editor =
            cx.new(|cx| TextEditor::new(confirmation.clone(), false, window, cx).password());
        let _ = passphrase_editor.read(cx).focus_handle().tab_stop(true);
        let _ = confirmation_editor.read(cx).focus_handle().tab_stop(true);

        Self {
            passphrase,
            passphrase_editor,
            confirmation,
            confirmation_editor,
        }
    }
}

impl ConnectionDraft {
    fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let connection_name = cx.new(|_| String::new());
        let fields = ConnectionFields::from_url("sqlite://dbx.db?mode=rwc")
            .expect("default SQLite connection URL is valid");
        let connection_url = cx.new(|_| fields.connection_string.clone());
        let host = cx.new(|_| fields.host.clone());
        let port = cx.new(|_| fields.port.clone());
        let username = cx.new(|_| fields.username.clone());
        let password = cx.new(|_| fields.password.clone());
        let database = cx.new(|_| fields.database.clone());
        let connection_name_editor =
            cx.new(|cx| TextEditor::new(connection_name.clone(), false, window, cx));
        let connection_editor =
            cx.new(|cx| TextEditor::new(connection_url.clone(), false, window, cx));
        let host_editor = cx.new(|cx| TextEditor::new(host.clone(), false, window, cx));
        let port_editor = cx.new(|cx| TextEditor::new(port.clone(), false, window, cx));
        let username_editor = cx.new(|cx| TextEditor::new(username.clone(), false, window, cx));
        let password_editor =
            cx.new(|cx| TextEditor::new(password.clone(), false, window, cx).password());
        let database_editor = cx.new(|cx| TextEditor::new(database.clone(), false, window, cx));

        Self {
            kind: DatabaseKind::SQLite,
            mode: ConnectionFormMode::Details,
            selected_profile: None,
            environment: ConnectionEnvironment::default(),
            connection_name,
            connection_name_editor,
            connection_url,
            connection_editor,
            host,
            host_editor,
            port,
            port_editor,
            username,
            username_editor,
            password,
            password_editor,
            database,
            database_editor,
        }
    }
}

struct SessionEditors {
    filter_text: Entity<String>,
    filter_editor: Entity<TextEditor>,
    _subscriptions: Vec<Subscription>,
}

impl SessionEditors {
    fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let filter_text = cx.new(|_| String::new());
        let filter_editor = cx.new(|cx| TextEditor::new(filter_text.clone(), false, window, cx));
        let subscriptions = vec![cx.observe(&filter_text, |_, _, cx| cx.notify())];

        Self {
            filter_text,
            filter_editor,
            _subscriptions: subscriptions,
        }
    }
}

type SecondaryTabId = Uuid;

struct SqlCompletionMenu {
    replacement_range: Range<usize>,
    items: Vec<SqlCompletionItem>,
    selected: usize,
    signature: CompletionSignature,
}

/// A completion state identity without retaining a second copy of the query.
/// The query entity increments its revision whenever its text changes; the
/// caret offset distinguishes otherwise identical documents at different
/// insertion points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompletionSignature {
    text_revision: u64,
    cursor: usize,
}

impl CompletionItemKind {
    fn color(self) -> Rgba {
        match self {
            Self::Keyword => theme().sql_keyword,
            Self::Type => theme().sql_type,
            Self::Table => theme().accent,
            Self::Column => theme().success,
            Self::Function => theme().sql_number,
            Self::Command => theme().sql_keyword,
            Self::Key => theme().success,
        }
    }
}

#[derive(Default)]
struct AbortOnDrop(Option<tokio::task::AbortHandle>);

impl AbortOnDrop {
    fn replace(&mut self, handle: tokio::task::AbortHandle) {
        self.cancel();
        self.0 = Some(handle);
    }

    fn cancel(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }

    fn clear(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Default)]
struct BackgroundTaskSet(Vec<tokio::task::AbortHandle>);

impl BackgroundTaskSet {
    fn track<T>(&mut self, task: &tokio::task::JoinHandle<T>) {
        // Completed tasks no longer need an abort handle. Sweeping here keeps
        // this owner-scoped cancellation set bounded even when a connection
        // performs many sequential refreshes or metadata requests.
        self.0.retain(|handle| !handle.is_finished());
        self.0.push(task.abort_handle());
    }

    fn cancel_all(&mut self) {
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

struct QueryTab {
    query_text: Entity<String>,
    query_editor: Entity<TextEditor>,
    result: Option<Arc<QueryResult>>,
    result_grid: Entity<TableState<ResultTableDelegate>>,
    split_state: Entity<ResizableState>,
    result_selection: QueryResultSelection,
    result_column_widths: HashMap<String, Pixels>,
    busy: bool,
    /// The last result remains visible while a newer request is in flight or
    /// has failed, but must not be mistaken for the newest execution.
    results_stale: bool,
    status: String,
    error: Option<String>,
    executed_database: Option<String>,
    abort_handle: AbortOnDrop,
    /// The byte range an error message points at. Query text edits increment
    /// `query_revision` and clear this range before it can be painted again.
    error_highlight: Option<Range<usize>>,
    request_generation: u64,
    query_revision: u64,
    completion_signature: Option<CompletionSignature>,
    completion_dismissed_signature: Option<CompletionSignature>,
    completion_index: usize,
    _subscriptions: Vec<Subscription>,
}

impl QueryTab {
    fn new(
        kind: DatabaseKind,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        let query_text = cx.new(|_| DbxApp::default_query(kind).to_owned());
        let query_editor = cx.new(|cx| match query_editor_language(kind) {
            editor::EditorLanguage::Sql => TextEditor::new_sql(query_text.clone(), window, cx),
            editor::EditorLanguage::Redis => TextEditor::new_redis(query_text.clone(), window, cx),
            editor::EditorLanguage::PlainText | editor::EditorLanguage::Json => {
                unreachable!("query tabs only use SQL or Redis syntax")
            }
        });
        let split_state = cx.new(|_| ResizableState::default());
        let result_grid = cx.new(|cx| {
            TableState::new(ResultTableDelegate::default(), window, cx)
                .col_resizable(true)
                .col_movable(false)
                .sortable(false)
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
            cx.notify();
        });
        let editor_subscription = cx.observe(&query_editor, |_, _, cx| cx.notify());
        let table_subscription =
            cx.subscribe_in(&result_grid, window, move |this, _, event, _, cx| {
                this.on_query_grid_event(session_id, tab_id, event, cx)
            });

        Self {
            query_text,
            query_editor,
            result: None,
            result_grid,
            split_state,
            result_selection: QueryResultSelection::None,
            result_column_widths: HashMap::new(),
            busy: false,
            results_stale: false,
            status: "Ready to query".into(),
            error: None,
            executed_database: None,
            abort_handle: AbortOnDrop::default(),
            error_highlight: None,
            request_generation: 0,
            query_revision: 0,
            completion_signature: None,
            completion_dismissed_signature: None,
            completion_index: 0,
            _subscriptions: vec![text_subscription, editor_subscription, table_subscription],
        }
    }

    fn set_result(&mut self, result: Option<QueryResult>, cx: &mut Context<DbxApp>) {
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

    fn invalidate_request(&mut self) {
        self.request_generation = self.request_generation.saturating_add(1);
        self.abort_handle.cancel();
        self.busy = false;
    }
}

fn query_editor_language(kind: DatabaseKind) -> editor::EditorLanguage {
    match kind {
        DatabaseKind::Redis => editor::EditorLanguage::Redis,
        DatabaseKind::PostgreSQL | DatabaseKind::MySQL | DatabaseKind::SQLite => {
            editor::EditorLanguage::Sql
        }
    }
}

impl Drop for QueryTab {
    fn drop(&mut self) {
        self.abort_handle.cancel();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum QueryResultSelection {
    #[default]
    None,
    Cell,
    Row,
    Column,
}

fn query_result_status(result: &QueryResult) -> String {
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

fn query_history_connection(session: &ConnectionSession) -> Option<QueryHistoryConnection> {
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

struct StructureTab {
    table: TableRef,
    columns: Vec<ColumnInfo>,
    foreign_keys: Vec<ForeignKeyInfo>,
    busy: bool,
    error: Option<String>,
}

/// Per-tab state for a database-wide relationship diagram. The document is
/// deliberately independent from GPUI so SVG and PNG exports share the exact
/// same layout as the on-screen canvas.
struct DiagramTab {
    /// The complete metadata snapshot is retained so schema filters can
    /// rebuild the scene without another database round-trip.
    source_schema: Option<Arc<RelationalSchema>>,
    document: Option<Arc<DiagramDocument>>,
    available_schemas: Vec<String>,
    /// PostgreSQL-only projection. `None` means every available schema.
    selected_schemas: Option<BTreeSet<String>>,
    busy: bool,
    stale: bool,
    error: Option<String>,
    zoom: f32,
    selected_node: Option<String>,
    scroll_handle: ScrollHandle,
    focus: FocusHandle,
    drag_anchor: Option<DiagramDragAnchor>,
    request_generation: u64,
    abort_handle: AbortOnDrop,
}

#[derive(Clone, Copy)]
struct DiagramDragAnchor {
    pointer: Point<Pixels>,
    scroll_offset: Point<Pixels>,
}

impl DiagramTab {
    fn loading(
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
            request_generation: 0,
            abort_handle: AbortOnDrop::default(),
        }
    }

    fn invalidate_request(&mut self) {
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

enum SecondaryTabKind {
    Query(Box<QueryTab>),
    Structure(StructureTab),
    Diagram(Box<DiagramTab>),
}

struct SecondaryTab {
    id: SecondaryTabId,
    kind: SecondaryTabKind,
}

struct ConnectionSession {
    id: SessionId,
    profile_id: Option<Uuid>,
    name: String,
    kind: DatabaseKind,
    environment: ConnectionEnvironment,
    engine: Option<Arc<DatabaseEngine>>,
    editors: SessionEditors,
    data_grid: Entity<TableState<ResultTableDelegate>>,
    result_column_widths: HashMap<String, Pixels>,
    _data_grid_subscription: Subscription,
    filters: FilterModel,
    filter_subscriptions: Vec<Subscription>,
    pane: Pane,
    secondary_tabs: Vec<SecondaryTab>,
    active_secondary_tab: Option<SecondaryTabId>,
    /// Recently closed query documents are retained for the current session
    /// only. Persisted history remains the durable source for executed work.
    closed_queries: Vec<String>,
    tables: Vec<TableInfo>,
    /// Schema metadata already fetched for completion. The navigator always
    /// supplies table names; columns are added as tables are opened or their
    /// structure is inspected, avoiding a metadata query for every keystroke.
    completion_columns: HashMap<String, Vec<ColumnInfo>>,
    /// Authoritative command grammar discovered once from the connected
    /// Redis/Valkey server. Completion only reads this cache; it never performs
    /// network I/O while the user is typing.
    redis_command_catalog: Option<Arc<RedisCommandCatalog>>,
    /// Databases reachable through this connection, for the sidebar switcher.
    databases: Vec<String>,
    /// Database the engine currently uses, if the backend reports one.
    current_database: Option<String>,
    /// PostgreSQL-only navigator filter. `None` means all schemas.
    schema_filter: Option<String>,
    selected_table: Option<TableRef>,
    table_columns: Vec<ColumnInfo>,
    foreign_keys: Vec<ForeignKeyInfo>,
    result: Option<Arc<QueryResult>>,
    /// The table that produced `result`, when it is safe to edit through the
    /// grid. Ad-hoc query results deliberately have no table provenance.
    result_table: Option<TableRef>,
    table_page: u64,
    table_has_next_page: bool,
    selected_row: Option<usize>,
    selected_column: usize,
    inspector_open: bool,
    draft_mode: DraftMode,
    row_draft: Option<RowDraftModel>,
    row_draft_subscriptions: Vec<Subscription>,
    suppress_next_grid_selection_event: bool,
    busy: bool,
    status: String,
    error: Option<String>,
    request_generation: u64,
    /// Tokio work captures an `Arc<DatabaseEngine>`. Keep abort handles here
    /// so closing a connection cancels that work before dropping the session
    /// instead of leaving closed pools alive until every query completes.
    background_tasks: BackgroundTaskSet,
}

impl ConnectionSession {
    fn new(
        id: SessionId,
        profile_id: Option<Uuid>,
        name: String,
        kind: DatabaseKind,
        environment: ConnectionEnvironment,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        let data_grid = cx.new(|cx| {
            TableState::new(ResultTableDelegate::default(), window, cx)
                .col_resizable(true)
                .col_movable(false)
                .sortable(false)
                .row_selectable(true)
                .col_selectable(true)
                .cell_selectable(false)
        });
        let data_grid_subscription =
            cx.subscribe_in(&data_grid, window, move |this, _, event, window, cx| {
                this.on_data_grid_event(id, event, window, cx)
            });

        Self {
            id,
            profile_id,
            name,
            kind,
            environment,
            engine: None,
            editors: SessionEditors::new(window, cx),
            data_grid,
            result_column_widths: HashMap::new(),
            _data_grid_subscription: data_grid_subscription,
            filters: FilterModel::new(),
            filter_subscriptions: Vec::new(),
            pane: Pane::Data,
            secondary_tabs: Vec::new(),
            active_secondary_tab: None,
            closed_queries: Vec::new(),
            tables: Vec::new(),
            completion_columns: HashMap::new(),
            redis_command_catalog: None,
            databases: Vec::new(),
            current_database: None,
            schema_filter: None,
            selected_table: None,
            table_columns: Vec::new(),
            foreign_keys: Vec::new(),
            result: None,
            result_table: None,
            table_page: 0,
            table_has_next_page: false,
            selected_row: None,
            selected_column: 0,
            inspector_open: true,
            draft_mode: DraftMode::Update,
            row_draft: None,
            row_draft_subscriptions: Vec::new(),
            suppress_next_grid_selection_event: false,
            busy: false,
            status: "Connecting…".into(),
            error: None,
            request_generation: 0,
            background_tasks: BackgroundTaskSet::default(),
        }
    }

    fn track_background_task<T>(&mut self, task: &tokio::task::JoinHandle<T>) {
        self.background_tasks.track(task);
    }

    fn cancel_background_tasks(&mut self) {
        self.background_tasks.cancel_all();
    }

    fn set_result(&mut self, result: Option<QueryResult>, cx: &mut Context<DbxApp>) {
        self.result = result.map(Arc::new);
        self.sync_result_grid(true, cx);
    }

    fn sync_result_grid(&mut self, clear_selection: bool, cx: &mut Context<DbxApp>) {
        let result = self.result.clone();
        let remembered_widths = self.result_column_widths.clone();
        let foreign_keys = self.foreign_keys.clone();
        let tables = self.tables.clone();
        self.data_grid.update(cx, move |table, cx| {
            table
                .delegate_mut()
                .set_result(result, &remembered_widths, &foreign_keys, &tables);
            table.refresh(cx);
            if clear_selection {
                table.clear_selection(cx);
            }
        });
    }

    fn clear_grid_selection(&self, cx: &mut Context<DbxApp>) {
        self.data_grid
            .update(cx, |table, cx| table.clear_selection(cx));
    }
}

impl Drop for ConnectionSession {
    fn drop(&mut self) {
        self.cancel_background_tasks();
    }
}

#[derive(Clone)]
struct TableContextMenu {
    session_id: SessionId,
    table: TableInfo,
    position: Point<gpui::Pixels>,
}

struct DatabaseExportDialog {
    session_id: SessionId,
    tables: Vec<TableInfo>,
    selected_tables: HashSet<String>,
    format: DumpFormat,
    schema_only: bool,
    gzipped: bool,
    output_directory: PathBuf,
    output_name: Entity<String>,
    output_name_editor: Entity<TextEditor>,
    _output_name_subscription: Subscription,
}

struct ConfirmationDialog {
    title: String,
    detail: String,
    confirm_label: &'static str,
    tone: ConfirmationTone,
    action: ConfirmationAction,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

struct MutationErrorDialog {
    session_id: SessionId,
    title: String,
    detail: String,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfirmationTone {
    Warning,
    Danger,
}

enum ConfirmationAction {
    RunQuery {
        session_id: SessionId,
        run_all: bool,
    },
    CloseQuery {
        session_id: SessionId,
        tab_id: SecondaryTabId,
    },
    ClearQueryHistory {
        session_id: SessionId,
    },
    Table {
        action: TableAction,
        session_id: SessionId,
        table: TableInfo,
    },
    DeleteRow {
        session_id: SessionId,
        table: TableRef,
        filters: Vec<Filter>,
    },
    DatabaseImport {
        session_id: SessionId,
        path: PathBuf,
    },
    TableImport {
        session_id: SessionId,
        table: TableInfo,
        path: PathBuf,
    },
}

impl ConfirmationAction {
    fn session_id(&self) -> SessionId {
        match self {
            Self::RunQuery { session_id, .. }
            | Self::CloseQuery { session_id, .. }
            | Self::ClearQueryHistory { session_id }
            | Self::Table { session_id, .. }
            | Self::DeleteRow { session_id, .. }
            | Self::DatabaseImport { session_id, .. }
            | Self::TableImport { session_id, .. } => *session_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableAction {
    Truncate,
    Drop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableClickAction {
    Select,
    OpenContextMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SavedConnectionClickAction {
    Select,
    Open,
}

fn saved_connection_click_action(click_count: usize) -> SavedConnectionClickAction {
    if click_count > 1 {
        SavedConnectionClickAction::Open
    } else {
        SavedConnectionClickAction::Select
    }
}

fn compact_connection_picker_visible(
    compact_layout: bool,
    compact_connection_form_open: bool,
    saved_connection_count: usize,
) -> bool {
    compact_layout && !compact_connection_form_open && saved_connection_count > 0
}

pub struct DbxApp {
    runtime: Arc<tokio::runtime::Runtime>,
    logo: Arc<Image>,
    draft: ConnectionDraft,
    vault_editors: VaultEditors,
    vault_state: Option<VaultState>,
    vault_busy: bool,
    saving_connection: bool,
    vault_generation: u64,
    credential_hydrating: bool,
    credential_hydration_generation: u64,
    credential_connect_window: Option<WindowHandle<DbxApp>>,
    profile_store: Option<ProfileStore>,
    saved_connections: Vec<SavedConnection>,
    query_history_store: Option<QueryHistoryStore>,
    /// Newest-first cache for the history UI. Disk access is never performed
    /// from render or query completion on the GPUI thread.
    recent_query_history: Vec<QueryHistoryEntry>,
    sessions: Vec<ConnectionSession>,
    active_session_id: Option<SessionId>,
    connection_picker_open: bool,
    compact_connection_form_open: bool,
    table_context_menu: Option<TableContextMenu>,
    database_export_dialog: Option<DatabaseExportDialog>,
    confirmation_dialog: Option<ConfirmationDialog>,
    mutation_error_dialog: Option<MutationErrorDialog>,
    appearance: Appearance,
    settings_store: Option<SettingsStore>,
    compact_layout: bool,
    narrow_workspace: bool,
    window_drag_armed: bool,
    test_generation: u64,
    testing_connection: bool,
    _subscriptions: Vec<Subscription>,
    status: String,
    error: Option<String>,
}

impl DbxApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = ConnectionDraft::new(window, cx);
        let vault_editors = VaultEditors::new(window, cx);

        let subscriptions = vec![
            cx.observe(&draft.connection_name, |_, _, cx| cx.notify()),
            cx.observe(&draft.connection_url, |_, _, cx| cx.notify()),
            cx.observe(&draft.host, |_, _, cx| cx.notify()),
            cx.observe(&draft.port, |_, _, cx| cx.notify()),
            cx.observe(&draft.username, |_, _, cx| cx.notify()),
            cx.observe(&draft.password, |_, _, cx| cx.notify()),
            cx.observe(&draft.database, |_, _, cx| cx.notify()),
        ];

        let (profile_store, saved_connections, profile_error) = match ProfileStore::new() {
            Ok(store) => match store.list() {
                Ok(profiles) => (Some(store), profiles, None),
                Err(error) => (Some(store), Vec::new(), Some(error.to_string())),
            },
            Err(error) => (None, Vec::new(), Some(error.to_string())),
        };
        let compact_connection_form_open = saved_connections.is_empty();
        let vault_state = profile_store
            .as_ref()
            .and_then(ProfileStore::vault)
            .map(|vault| vault.state());
        if vault_state != Some(VaultState::Unlocked) {
            vault_editors
                .passphrase_editor
                .read(cx)
                .focus_handle()
                .focus(window, cx);
        }
        let (query_history_store, recent_query_history) = match QueryHistoryStore::new() {
            Ok(store) => {
                let entries = store
                    .load()
                    .map(|entries| entries.into_iter().rev().take(500).collect())
                    .unwrap_or_default();
                (Some(store), entries)
            }
            Err(error) => {
                eprintln!("DBX could not initialize query history: {error}");
                (None, Vec::new())
            }
        };

        Self {
            runtime: Arc::new(tokio::runtime::Runtime::new().expect("create DBX Tokio runtime")),
            logo: Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_BYTES.to_vec())),
            draft,
            vault_editors,
            vault_state,
            vault_busy: false,
            saving_connection: false,
            vault_generation: 0,
            credential_hydrating: false,
            credential_hydration_generation: 0,
            credential_connect_window: None,
            profile_store,
            saved_connections,
            query_history_store,
            recent_query_history,
            sessions: Vec::new(),
            active_session_id: None,
            connection_picker_open: false,
            compact_connection_form_open,
            table_context_menu: None,
            database_export_dialog: None,
            confirmation_dialog: None,
            mutation_error_dialog: None,
            appearance: appearance(),
            settings_store: SettingsStore::new().ok(),
            compact_layout: false,
            narrow_workspace: false,
            window_drag_armed: false,
            test_generation: 0,
            testing_connection: false,
            _subscriptions: subscriptions,
            status: "Choose an engine and connect".into(),
            error: profile_error,
        }
    }

    fn record_query_history(
        &mut self,
        connection: Option<QueryHistoryConnection>,
        query: String,
        outcome: QueryHistoryOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = connection else {
            return;
        };
        let Some(store) = self.query_history_store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let entries = runtime
                .spawn_blocking(move || {
                    store.record(connection, query, outcome)?;
                    store.load()
                })
                .await;
            if let Ok(Ok(entries)) = entries {
                this.update(cx, |this, _| {
                    this.recent_query_history = entries.into_iter().rev().take(500).collect();
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Recent entries for the current connection, newest first. This only
    /// reads the in-memory cache and is therefore safe to call while rendering.
    pub(super) fn recent_query_history_for(&self, session_id: SessionId) -> Vec<QueryHistoryEntry> {
        let Some(connection) = self.session(session_id).and_then(query_history_connection) else {
            return Vec::new();
        };
        self.recent_query_history
            .iter()
            .filter(|entry| entry.connection == connection)
            .cloned()
            .collect()
    }

    pub(super) fn request_clear_query_history_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recent_query_history_for(session_id).is_empty() {
            return;
        }
        let return_focus = self
            .active_query_editor_for(session_id)
            .map(|editor| editor.read(cx).focus_handle());
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: "Clear query history?".into(),
            detail: "This removes the saved query history for the current connection. Open query tabs are not affected.".into(),
            confirm_label: "Clear history",
            tone: ConfirmationTone::Warning,
            action: ConfirmationAction::ClearQueryHistory { session_id },
            focus: focus.clone(),
            return_focus,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn clear_query_history_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(connection) = self.session(session_id).and_then(query_history_connection) else {
            return;
        };
        let Some(store) = self.query_history_store.clone() else {
            return;
        };
        let retained_connection = connection.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let cleared = runtime
                .spawn_blocking(move || store.clear(&connection))
                .await;
            this.update(cx, |this, cx| {
                match cleared {
                    Ok(Ok(count)) => {
                        this.recent_query_history
                            .retain(|entry| entry.connection != retained_connection);
                        if let Some(session) = this.session_mut(session_id) {
                            session.error = None;
                            session.status = format!(
                                "Cleared {count} history entr{}",
                                if count == 1 { "y" } else { "ies" }
                            );
                        }
                    }
                    Ok(Err(error)) => {
                        if let Some(session) = this.session_mut(session_id) {
                            session.error = Some(format!("Could not clear query history: {error}"));
                            session.status = "Query history was not cleared".into();
                        }
                    }
                    Err(error) => {
                        if let Some(session) = this.session_mut(session_id) {
                            session.error =
                                Some(format!("Query history task stopped unexpectedly: {error}"));
                            session.status = "Query history was not cleared".into();
                        }
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn toggle_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.appearance {
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::Light,
        };
        self.appearance = next;
        set_appearance(next);
        gpui_component::Theme::change(
            match next {
                Appearance::Light => gpui_component::ThemeMode::Light,
                Appearance::Dark => gpui_component::ThemeMode::Dark,
            },
            Some(window),
            cx,
        );
        if let Some(store) = &self.settings_store {
            if let Err(error) = store.save(Settings::new(next)) {
                self.error = Some(format!("Could not save appearance preference: {error}"));
            } else {
                self.status = format!("Using {} appearance", next.label().to_ascii_lowercase());
            }
        } else {
            self.error = Some("Appearance preference storage is unavailable".into());
        }
        cx.notify();
    }

    fn dismiss_overlay_on_escape(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() || event.keystroke.key.as_str() != "escape" {
            return;
        }
        let dismissed = if self.dismiss_mutation_error_dialog(window, cx) {
            true
        } else if self.confirmation_dialog.is_some() {
            self.cancel_confirmation(window, cx);
            true
        } else {
            self.database_export_dialog.take().is_some() || self.table_context_menu.take().is_some()
        };
        if dismissed {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn default_query(kind: DatabaseKind) -> &'static str {
        match kind {
            DatabaseKind::PostgreSQL => "SELECT current_database(), current_user;",
            DatabaseKind::MySQL => "SELECT DATABASE(), CURRENT_USER();",
            DatabaseKind::SQLite => "SELECT sqlite_version();",
            DatabaseKind::Redis => "SCAN 0 COUNT 100",
        }
    }

    fn close_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(index) = self
            .sessions
            .iter()
            .position(|session| session.id == session_id)
        else {
            return;
        };
        if self
            .database_export_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.session_id == session_id)
        {
            self.database_export_dialog = None;
        }
        if self
            .confirmation_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.action.session_id() == session_id)
        {
            self.confirmation_dialog = None;
        }
        if self
            .mutation_error_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.session_id == session_id)
        {
            self.mutation_error_dialog = None;
        }
        self.sessions[index].request_generation += 1;
        self.sessions[index].cancel_background_tasks();
        for tab in &mut self.sessions[index].secondary_tabs {
            match &mut tab.kind {
                SecondaryTabKind::Query(query) => query.invalidate_request(),
                SecondaryTabKind::Diagram(diagram) => diagram.invalidate_request(),
                SecondaryTabKind::Structure(_) => {}
            }
        }
        self.sessions.remove(index);
        if self.active_session_id == Some(session_id) {
            self.active_session_id = self
                .sessions
                .get(index.min(self.sessions.len().saturating_sub(1)))
                .map(|session| session.id)
                .or_else(|| self.sessions.last().map(|session| session.id));
        }
        if self.sessions.is_empty() {
            self.active_session_id = None;
            self.connection_picker_open = false;
            self.compact_connection_form_open = self.saved_connections.is_empty();
            self.error = None;
            self.status = "Disconnected".into();
        }
        cx.notify();
    }

    fn activate_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if self.sessions.iter().any(|session| session.id == session_id) {
            if self
                .database_export_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.session_id != session_id)
            {
                self.database_export_dialog = None;
            }
            if self
                .confirmation_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.action.session_id() != session_id)
            {
                self.confirmation_dialog = None;
            }
            if self
                .mutation_error_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.session_id != session_id)
            {
                self.mutation_error_dialog = None;
            }
            self.active_session_id = Some(session_id);
            self.connection_picker_open = false;
            cx.notify();
        }
    }

    fn session(&self, session_id: SessionId) -> Option<&ConnectionSession> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
    }

    fn session_mut(&mut self, session_id: SessionId) -> Option<&mut ConnectionSession> {
        self.sessions
            .iter_mut()
            .find(|session| session.id == session_id)
    }

    fn active_session_id(&self) -> Option<SessionId> {
        self.active_session_id
            .filter(|session_id| self.session(*session_id).is_some())
    }

    fn active_session(&self) -> Option<&ConnectionSession> {
        self.active_session_id().and_then(|id| self.session(id))
    }

    fn active_query_editor_for(&self, session_id: SessionId) -> Option<Entity<TextEditor>> {
        self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some(query.query_editor.clone())
        })
    }

    fn focus_active_query_editor_for(
        &self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_query_editor_for(session_id) {
            let focus = editor.read(cx).focus_handle();
            focus.focus(window, cx);
        }
    }

    fn row_draft_focus_for(
        &self,
        session_id: SessionId,
        field_id: Option<FieldId>,
        cx: &App,
    ) -> Option<FocusHandle> {
        let draft = self.session(session_id)?.row_draft.as_ref()?;
        let field = field_id
            .and_then(|field_id| {
                draft
                    .fields()
                    .iter()
                    .find(|field| field.id == field_id && field.editable)
            })
            .or_else(|| draft.fields().iter().find(|field| field.editable))?;

        match field.state {
            FieldValueState::Sql => Some(field.sql_editor.read(cx).focus_handle()),
            FieldValueState::Value => field
                .boolean_selector
                .as_ref()
                .or(field.enum_selector.as_ref())
                .map(|selector| selector.focus_handle(cx))
                .or_else(|| Some(field.editor.read(cx).focus_handle())),
            FieldValueState::Null | FieldValueState::Default => field
                .state_selector
                .as_ref()
                .map(|selector| selector.focus_handle(cx))
                .or_else(|| Some(field.editor.read(cx).focus_handle())),
        }
    }

    fn add_query_tab_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.session(session_id).map(|session| session.kind) else {
            return;
        };

        let id = Uuid::new_v4();
        let query_tab = QueryTab::new(kind, session_id, id, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.secondary_tabs.push(SecondaryTab {
            id,
            kind: SecondaryTabKind::Query(Box::new(query_tab)),
        });
        session.active_secondary_tab = Some(id);
        session.pane = Pane::Query;
        let focus = session
            .secondary_tabs
            .last()
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(query.query_editor.read(cx).focus_handle()),
                SecondaryTabKind::Structure(_) => None,
                SecondaryTabKind::Diagram(_) => None,
            });
        if let Some(focus) = focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Load a persisted history item into the active query document, creating
    /// one when the current document is Data or Structure. History never
    /// executes implicitly; the user still chooses Run.
    pub(super) fn load_query_history_entry_for(
        &mut self,
        session_id: SessionId,
        entry: &QueryHistoryEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query_is_active = self.session(session_id).is_some_and(|session| {
            session.active_secondary_tab.is_some_and(|tab_id| {
                session
                    .secondary_tabs
                    .iter()
                    .any(|tab| tab.id == tab_id && matches!(&tab.kind, SecondaryTabKind::Query(_)))
            })
        });
        if !query_is_active {
            self.add_query_tab_for(session_id, window, cx);
        }
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };
        let focus = query_tab.query_editor.read(cx).focus_handle();
        query_tab.query_editor.update(cx, |editor, cx| {
            editor.set_text(entry.sql.clone(), cx);
        });
        query_tab.error = None;
        query_tab.error_highlight = None;
        query_tab.status = "History query loaded".into();
        focus.focus(window, cx);
        cx.notify();
    }

    fn open_structure_tab_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, table_ref)) = self.session(session_id).and_then(|session| {
            session
                .engine
                .clone()
                .map(|engine| (engine, table_ref(&table)))
        }) else {
            return;
        };
        let id = Uuid::new_v4();
        if let Some(session) = self.session_mut(session_id) {
            session.secondary_tabs.push(SecondaryTab {
                id,
                kind: SecondaryTabKind::Structure(StructureTab {
                    table: table_ref.clone(),
                    columns: Vec::new(),
                    foreign_keys: Vec::new(),
                    busy: true,
                    error: None,
                }),
            });
            session.active_secondary_tab = Some(id);
            session.pane = Pane::Structure;
        }
        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move { engine.table_structure(&table_ref).await });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                let Some(tab) = session.secondary_tabs.iter_mut().find(|tab| tab.id == id) else {
                    return;
                };
                let SecondaryTabKind::Structure(structure) = &mut tab.kind else {
                    return;
                };
                structure.busy = false;
                match result {
                    Ok(table_structure) => {
                        session.completion_columns.insert(
                            completion_table_key(&structure.table),
                            table_structure.columns.clone(),
                        );
                        structure.columns = table_structure.columns;
                        structure.foreign_keys = table_structure.foreign_keys;
                        structure.error = None;
                    }
                    Err(error) => structure.error = Some(error.to_string()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Open the one relationship diagram for this connection, or return to it
    /// when it is already open. Redis deliberately has no relational surface.
    pub(super) fn open_diagram_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, existing, tables, explorer_schema)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.secondary_tabs.iter().find_map(|tab| {
                        matches!(&tab.kind, SecondaryTabKind::Diagram(_)).then_some(tab.id)
                    }),
                    session.tables.clone(),
                    session.schema_filter.clone(),
                )
            })
        else {
            return;
        };
        if !kind.is_sql() {
            if let Some(session) = self.session_mut(session_id) {
                session.error =
                    Some("Database diagrams are available for relational connections".into());
                session.status = "Diagram unavailable".into();
            }
            cx.notify();
            return;
        }
        if let Some(tab_id) = existing {
            self.activate_secondary_tab_for(session_id, tab_id, window, cx);
            return;
        }

        let id = Uuid::new_v4();
        let diagram = DiagramTab::loading(kind, &tables, explorer_schema.as_deref(), cx);
        let focus = diagram.focus.clone();
        if let Some(session) = self.session_mut(session_id) {
            session.secondary_tabs.push(SecondaryTab {
                id,
                kind: SecondaryTabKind::Diagram(Box::new(diagram)),
            });
            session.active_secondary_tab = Some(id);
            session.pane = Pane::Diagram;
        }
        focus.focus(window, cx);
        self.load_diagram_for(session_id, id, false, cx);
    }

    /// Reload the active diagram while retaining its last successful scene as
    /// a visible, explicitly stale snapshot.
    pub(super) fn refresh_diagram_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(tab_id) = self.session(session_id).and_then(|session| {
            session
                .secondary_tabs
                .iter()
                .find(|tab| matches!(&tab.kind, SecondaryTabKind::Diagram(_)))
                .map(|tab| tab.id)
        }) else {
            return;
        };
        self.load_diagram_for(session_id, tab_id, true, cx);
    }

    fn load_diagram_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        retain_document: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self
            .session(session_id)
            .and_then(|session| session.engine.clone())
        else {
            return;
        };
        let runtime = self.runtime.clone();
        let Some(tab) = self.session_mut(session_id).and_then(|session| {
            session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
        }) else {
            return;
        };
        let SecondaryTabKind::Diagram(diagram) = &mut tab.kind else {
            return;
        };
        diagram.invalidate_request();
        diagram.busy = true;
        diagram.stale = retain_document && diagram.document.is_some();
        diagram.error = None;
        diagram.request_generation = diagram.request_generation.saturating_add(1);
        let generation = diagram.request_generation;
        let task = runtime.spawn(async move { engine.relational_schema().await });
        diagram.abort_handle.replace(task.abort_handle());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let Some(tab) = this.session_mut(session_id).and_then(|session| {
                    session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| tab.id == tab_id)
                }) else {
                    return;
                };
                let SecondaryTabKind::Diagram(diagram) = &mut tab.kind else {
                    return;
                };
                if generation != diagram.request_generation {
                    return;
                }
                diagram.busy = false;
                diagram.abort_handle.clear();
                match result {
                    Ok(Ok(schema)) => {
                        let source_schema = Arc::new(schema);
                        diagram.available_schemas = relational_schema_names(&source_schema);
                        normalize_diagram_schema_selection(
                            &mut diagram.selected_schemas,
                            &diagram.available_schemas,
                        );
                        diagram.document = Some(Arc::new(diagram_document_for_selection(
                            &source_schema,
                            diagram.selected_schemas.as_ref(),
                        )));
                        diagram.source_schema = Some(source_schema);
                        diagram.stale = false;
                        diagram.error = None;
                    }
                    Ok(Err(error)) => {
                        diagram.stale = diagram.document.is_some();
                        diagram.error = Some(error.to_string());
                    }
                    Err(error) => {
                        diagram.stale = diagram.document.is_some();
                        diagram.error =
                            Some(format!("Diagram request stopped unexpectedly: {error}"));
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn set_diagram_zoom_for(
        &mut self,
        session_id: SessionId,
        zoom: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let next_zoom = zoom.clamp(0.35, 2.0);
        if let Some(document) = diagram.document.as_ref() {
            let old_scene = point(
                px(document.width * diagram.zoom + DIAGRAM_SCENE_PADDING * 2.0),
                px(document.height * diagram.zoom + DIAGRAM_SCENE_PADDING * 2.0),
            );
            let next_scene = point(
                px(document.width * next_zoom + DIAGRAM_SCENE_PADDING * 2.0),
                px(document.height * next_zoom + DIAGRAM_SCENE_PADDING * 2.0),
            );
            let offset = diagram.scroll_handle.offset();
            let max_offset = diagram.scroll_handle.max_offset();
            diagram.scroll_handle.set_offset(point(
                remap_diagram_scroll_axis(offset.x, max_offset.x, old_scene.x, next_scene.x),
                remap_diagram_scroll_axis(offset.y, max_offset.y, old_scene.y, next_scene.y),
            ));
        }
        diagram.zoom = next_zoom;
        cx.notify();
    }

    pub(super) fn reset_diagram_view_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.zoom = 1.0;
        diagram.scroll_handle.set_offset(point(px(0.), px(0.)));
        diagram.drag_anchor = None;
        cx.notify();
    }

    pub(super) fn fit_diagram_for(
        &mut self,
        session_id: SessionId,
        zoom: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.zoom = zoom.clamp(0.35, 2.0);
        diagram.scroll_handle.set_offset(point(px(0.), px(0.)));
        diagram.drag_anchor = None;
        cx.notify();
    }

    pub(super) fn begin_diagram_pan_for(
        &mut self,
        session_id: SessionId,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.drag_anchor = Some(DiagramDragAnchor {
            pointer,
            scroll_offset: diagram.scroll_handle.offset(),
        });
        cx.notify();
    }

    pub(super) fn pan_diagram_to_for(
        &mut self,
        session_id: SessionId,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let Some(anchor) = diagram.drag_anchor else {
            return;
        };
        let offset = point(
            anchor.scroll_offset.x + (pointer.x - anchor.pointer.x),
            anchor.scroll_offset.y + (pointer.y - anchor.pointer.y),
        );
        diagram
            .scroll_handle
            .set_offset(clamp_diagram_scroll_offset(
                offset,
                diagram.scroll_handle.max_offset(),
            ));
        cx.notify();
    }

    pub(super) fn pan_diagram_by_for(
        &mut self,
        session_id: SessionId,
        horizontal: f32,
        vertical: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        let offset = diagram.scroll_handle.offset();
        let requested = point(offset.x - px(horizontal), offset.y - px(vertical));
        diagram
            .scroll_handle
            .set_offset(clamp_diagram_scroll_offset(
                requested,
                diagram.scroll_handle.max_offset(),
            ));
        cx.notify();
    }

    pub(super) fn end_diagram_pan_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(diagram) = self.active_diagram_tab_mut(session_id) {
            diagram.drag_anchor = None;
            cx.notify();
        }
    }

    pub(super) fn set_all_diagram_schemas_for(
        &mut self,
        session_id: SessionId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.selected_schemas = if enabled { None } else { Some(BTreeSet::new()) };
        rebuild_diagram_document(diagram);
        cx.notify();
    }

    pub(super) fn set_diagram_schema_enabled_for(
        &mut self,
        session_id: SessionId,
        schema: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        if diagram.available_schemas.binary_search(&schema).is_err() {
            return;
        }

        let mut selected = diagram
            .selected_schemas
            .clone()
            .unwrap_or_else(|| diagram.available_schemas.iter().cloned().collect());
        if enabled {
            selected.insert(schema);
        } else {
            selected.remove(&schema);
        }
        diagram.selected_schemas = Some(selected);
        normalize_diagram_schema_selection(
            &mut diagram.selected_schemas,
            &diagram.available_schemas,
        );
        rebuild_diagram_document(diagram);
        cx.notify();
    }

    pub(super) fn select_diagram_node_for(
        &mut self,
        session_id: SessionId,
        node_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(diagram) = self.active_diagram_tab_mut(session_id) else {
            return;
        };
        diagram.selected_node = node_id;
        cx.notify();
    }

    /// Drill into a table from the diagram using the same data-loading path as
    /// the explorer, keeping filters, paging, and mutation safety consistent.
    pub(super) fn open_diagram_table_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_table_for(session_id, table, window, cx);
    }

    /// Save pre-rendered diagram bytes through the native file picker. The
    /// renderer supplies bytes so app state remains presentation agnostic.
    pub(super) fn export_diagram_for(
        &mut self,
        session_id: SessionId,
        format: DiagramExportFormat,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let Some(database) = self.session(session_id).map(|session| {
            session
                .current_database
                .clone()
                .unwrap_or_else(|| session.name.clone())
        }) else {
            return;
        };
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let stem = database
            .chars()
            .map(|character| match character {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
                _ => '-',
            })
            .collect::<String>();
        let suggested = format!("{}-diagram.{}", stem.trim_matches('-'), format.extension());
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(path))) => {
                    let destination = path.display().to_string();
                    let result = runtime
                        .spawn_blocking(move || std::fs::write(path, bytes))
                        .await;
                    this.update(cx, |this, cx| {
                        if let Some(session) = this.session_mut(session_id) {
                            match result {
                                Ok(Ok(())) => {
                                    session.status = format!("Exported diagram to {destination}");
                                    session.error = None;
                                }
                                Ok(Err(error)) => {
                                    session.status = "Diagram export failed".into();
                                    session.error =
                                        Some(format!("Could not export diagram: {error}"));
                                }
                                Err(error) => {
                                    session.status = "Diagram export failed".into();
                                    session.error =
                                        Some(format!("Diagram export task stopped: {error}"));
                                }
                            }
                        }
                        cx.notify();
                    })?;
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open the save dialog: {error}"));
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Save dialog closed unexpectedly: {error}"));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn active_diagram_tab_mut(&mut self, session_id: SessionId) -> Option<&mut DiagramTab> {
        let session = self.session_mut(session_id)?;
        let tab_id = session.active_secondary_tab?;
        let tab = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)?;
        let SecondaryTabKind::Diagram(diagram) = &mut tab.kind else {
            return None;
        };
        Some(diagram)
    }

    fn activate_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session.secondary_tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        session.active_secondary_tab = Some(tab_id);
        let tab_focus = match &tab.kind {
            SecondaryTabKind::Query(query) => {
                session.pane = Pane::Query;
                Some(query.query_editor.read(cx).focus_handle())
            }
            SecondaryTabKind::Structure(_) => {
                session.pane = Pane::Structure;
                None
            }
            SecondaryTabKind::Diagram(diagram) => {
                session.pane = Pane::Diagram;
                Some(diagram.focus.clone())
            }
        };
        if let Some(focus) = tab_focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Ask before discarding an edited query document. The confirmation keeps
    /// accidental tab closes from silently losing work.
    pub(super) fn request_close_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, text, return_focus)) = self.session(session_id).and_then(|session| {
            session
                .secondary_tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| {
                    let SecondaryTabKind::Query(query) = &tab.kind else {
                        return None;
                    };
                    let editor = query.query_editor.read(cx);
                    Some((session.kind, editor.text(cx), editor.focus_handle()))
                })
        }) else {
            self.close_secondary_tab_for(session_id, tab_id, cx);
            return;
        };
        if text.trim().is_empty() || text == Self::default_query(kind) {
            self.close_secondary_tab_for(session_id, tab_id, cx);
            return;
        }
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: "Close query?".into(),
            detail: "Closing discards this tab. You can reopen its text from Query options during this connection; eligible executed queries also remain in local history.".into(),
            confirm_label: "Close query",
            tone: ConfirmationTone::Warning,
            action: ConfirmationAction::CloseQuery { session_id, tab_id },
            focus: focus.clone(),
            return_focus: Some(return_focus),
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn close_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(index) = session
            .secondary_tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let tab = session.secondary_tabs.remove(index);
        match tab.kind {
            SecondaryTabKind::Query(mut query) => {
                query.invalidate_request();
                let text = query.query_editor.read(cx).text(cx);
                if !text.trim().is_empty() && text != Self::default_query(session.kind) {
                    session.closed_queries.push(text);
                    const MAX_CLOSED_QUERIES: usize = 10;
                    let overflow = session
                        .closed_queries
                        .len()
                        .saturating_sub(MAX_CLOSED_QUERIES);
                    if overflow > 0 {
                        session.closed_queries.drain(..overflow);
                    }
                }
            }
            SecondaryTabKind::Diagram(mut diagram) => diagram.invalidate_request(),
            SecondaryTabKind::Structure(_) => {}
        }
        if session.active_secondary_tab == Some(tab_id) {
            let next = index.min(session.secondary_tabs.len().saturating_sub(1));
            session.active_secondary_tab = session.secondary_tabs.get(next).map(|tab| tab.id);
            session.pane = session
                .secondary_tabs
                .get(next)
                .map(|tab| match &tab.kind {
                    SecondaryTabKind::Query(_) => Pane::Query,
                    SecondaryTabKind::Structure(_) => Pane::Structure,
                    SecondaryTabKind::Diagram(_) => Pane::Diagram,
                })
                .unwrap_or(Pane::Data);
        }
        cx.notify();
    }

    /// Reopen the most recently closed query document in this connection.
    pub(super) fn reopen_last_closed_query_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self
            .session_mut(session_id)
            .and_then(|session| session.closed_queries.pop());
        let Some(text) = text else { return };
        self.add_query_tab_for(session_id, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(SecondaryTab {
            kind: SecondaryTabKind::Query(query),
            ..
        }) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        query
            .query_editor
            .update(cx, |editor, cx| editor.set_text(text, cx));
        query.status = "Reopened closed query".into();
        cx.notify();
    }

    fn select_schema_filter_for(
        &mut self,
        session_id: SessionId,
        schema: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        if session.kind != DatabaseKind::PostgreSQL || session.schema_filter == schema {
            return;
        }

        session.schema_filter = schema;
        let selected_table_is_visible = session.selected_table.as_ref().is_none_or(|table| {
            table_is_visible(session.kind, session.schema_filter.as_deref(), table)
        });
        if !selected_table_is_visible {
            // Changing the navigator filter is local and must not kick off a
            // query for an implicitly selected table. Clear the stale snapshot
            // instead and let the user choose a table in the new schema.
            session.selected_table = None;
            session.table_columns.clear();
            session.set_result(None, cx);
            session.result_table = None;
            session.table_page = 0;
            session.table_has_next_page = false;
            session.selected_row = None;
            session.row_draft = None;
            session.row_draft_subscriptions.clear();
            session.foreign_keys.clear();
            session.selected_column = 0;
            session.status = "Select a table".into();
        } else {
            session.status = "Schema filter updated".into();
        }
        session.error = None;
        cx.notify();
    }

    fn select_table_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_table_with_filters_for(session_id, table, Vec::new(), window, cx);
    }

    fn select_table_with_filters_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        filters: Vec<Filter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, kind)) = self
            .session(session_id)
            .and_then(|session| session.engine.clone().map(|engine| (engine, session.kind)))
        else {
            return;
        };
        let table_ref = table_ref(&table);
        let runtime = self.runtime.clone();
        let filter_columns = self
            .session(session_id)
            .map(|session| session.table_columns.clone())
            .unwrap_or_default();
        let mut filter_model = FilterModel::new();
        for filter in &filters {
            if let Some(value) = filter.value.as_ref() {
                filter_model.add_row_with_value_and_columns(
                    filter.column.clone(),
                    filter.operator,
                    value.to_string(),
                    &filter_columns,
                    window,
                    cx,
                );
            }
        }
        let filter_row_ids = filter_model
            .rows()
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.active_secondary_tab = None;
        session.pane = Pane::Data;
        session.selected_table = Some(table_ref.clone());
        session.table_page = 0;
        session.table_has_next_page = false;
        // Until this request completes, the visible snapshot belongs to the
        // previous table and must not be used for a mutation.
        session.result_table = None;
        session.selected_row = None;
        session.row_draft = None;
        session.row_draft_subscriptions.clear();
        session.foreign_keys.clear();
        session.clear_grid_selection(cx);
        session.filters = filter_model;
        session.filter_subscriptions.clear();
        session.busy = true;
        session.error = None;
        session.status = format!("Loading {}…", table.name);
        session.request_generation += 1;
        let generation = session.request_generation;
        let result_table = table_ref.clone();
        let row_navigation = !filters.is_empty();
        for row_id in filter_row_ids {
            self.watch_filter_row_for(session_id, row_id, window, cx);
        }
        let task = runtime.spawn(async move {
            let structure = engine.table_structure(&table_ref).await?;
            let (result, has_next_page) = if kind.is_sql() {
                let mut result = engine
                    .query_table(
                        &table_ref,
                        &[],
                        &filters,
                        &[],
                        Some(table_browse_page(0)),
                        QueryOptions::default(),
                    )
                    .await?;
                let has_next_page = trim_table_browse_result(&mut result);
                (result, has_next_page)
            } else {
                (
                    engine
                        .query("SCAN 0 COUNT 100", QueryOptions::default())
                        .await?,
                    false,
                )
            };
            Ok::<_, dbx_core::DbxError>((structure, result, has_next_page))
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok((structure, result, has_next_page)) => {
                        let has_rows = !result.rows.is_empty();
                        session.table_columns = structure.columns;
                        session.completion_columns.insert(
                            completion_table_key(&result_table),
                            session.table_columns.clone(),
                        );
                        session.foreign_keys = structure.foreign_keys;
                        session.table_page = 0;
                        session.table_has_next_page = has_next_page;
                        session.set_result(Some(result), cx);
                        session.result_table = Some(result_table.clone());
                        session.status = if row_navigation && has_rows {
                            "Opened referenced row".into()
                        } else if row_navigation {
                            "Referenced row not found".into()
                        } else {
                            "Ready".into()
                        };
                        session.error = None;
                        session.pane = Pane::Data;
                        if row_navigation && has_rows {
                            session
                                .data_grid
                                .update(cx, |table, cx| table.set_selected_row(0, cx));
                        }
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Operation failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn navigate_to_foreign_key_for(
        &mut self,
        session_id: SessionId,
        foreign_key: ForeignKeyInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_table) = self
            .session(session_id)
            .and_then(|session| foreign_key_target_table(&session.tables, &foreign_key))
        else {
            if let Some(session) = self.session_mut(session_id) {
                session.status = format!(
                    "Referenced table {} is not available in this database",
                    foreign_key.referenced_table
                );
                session.error = None;
            }
            cx.notify();
            return;
        };

        // A PostgreSQL foreign key may cross schemas. Keep the navigator and
        // the selected table in the same visible context before loading data.
        if self
            .session(session_id)
            .is_some_and(|session| session.kind == DatabaseKind::PostgreSQL)
        {
            self.select_schema_filter_for(session_id, target_table.schema.clone(), cx);
        }
        self.select_table_for(session_id, target_table, window, cx);
    }

    fn navigate_to_foreign_key_row_for(
        &mut self,
        session_id: SessionId,
        row_index: usize,
        column_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((foreign_key, target_table, filters)) =
            self.session(session_id).and_then(|session| {
                let result = session.result.as_ref()?;
                let row = result.rows.get(row_index)?;
                let local_column = result.columns.get(column_index.checked_sub(1)?)?;
                let foreign_key = session
                    .foreign_keys
                    .iter()
                    .find(|foreign_key| foreign_key.columns.first() == Some(&local_column.name))?;
                let target_table = foreign_key_target_table(&session.tables, foreign_key)?.clone();
                if foreign_key.columns.len() != foreign_key.referenced_columns.len() {
                    return None;
                }

                let mut filters = Vec::with_capacity(foreign_key.columns.len());
                for (local_column, referenced_column) in foreign_key
                    .columns
                    .iter()
                    .zip(&foreign_key.referenced_columns)
                {
                    let result_column_index = result
                        .columns
                        .iter()
                        .position(|result_column| result_column.name == *local_column)?;
                    let value = row.values.get(result_column_index)?.clone();
                    if matches!(value, CellValue::Null) {
                        return None;
                    }
                    filters.push(Filter::new(
                        referenced_column.clone(),
                        FilterOperator::Equals,
                        Some(value),
                    ));
                }
                Some((foreign_key.clone(), target_table, filters))
            })
        else {
            return;
        };

        if self
            .session(session_id)
            .is_some_and(|session| session.kind == DatabaseKind::PostgreSQL)
        {
            self.select_schema_filter_for(session_id, target_table.schema.clone(), cx);
        }
        self.select_table_with_filters_for(session_id, target_table, filters, window, cx);
        if let Some(session) = self.session_mut(session_id) {
            session.status = format!(
                "Opening referenced row via {}",
                foreign_key
                    .constraint_name
                    .as_deref()
                    .unwrap_or("foreign key")
            );
        }
        cx.notify();
    }

    fn refresh_table(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.active_session_id() else {
            return;
        };
        self.refresh_table_for(session_id, cx);
    }

    fn add_filter_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((column, columns)) = self.session(session_id).map(|session| {
            (
                session
                    .table_columns
                    .first()
                    .map(|column| column.name.clone())
                    .unwrap_or_default(),
                session.table_columns.clone(),
            )
        }) else {
            return;
        };
        let row_id = {
            let Some(session) = self.session_mut(session_id) else {
                return;
            };
            if session.table_columns.is_empty() {
                return;
            }
            session.filters.add_row_with_columns(
                column,
                FilterOperator::Equals,
                &columns,
                window,
                cx,
            )
        };
        self.watch_filter_row_for(session_id, row_id, window, cx);
        cx.notify();
    }

    fn watch_filter_row_for(
        &mut self,
        session_id: SessionId,
        row_id: FilterRowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((column_selector, operator_selector)) =
            self.session(session_id).and_then(|session| {
                session
                    .filters
                    .rows()
                    .iter()
                    .find(|row| row.id == row_id)
                    .map(|row| (row.column_selector.clone(), row.operator_selector.clone()))
            })
        else {
            return;
        };
        let column_subscription = cx.subscribe_in(
            &column_selector,
            window,
            move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.set_filter_column_for(session_id, row_id, value.to_string(), cx);
                }
            },
        );
        let operator_subscription = cx.subscribe_in(
            &operator_selector,
            window,
            move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let Some(operator) = filter_operator_options()
                    .iter()
                    .find(|option| option.label == value.as_ref())
                    .map(|option| option.operator)
                else {
                    return;
                };
                this.set_filter_operator_for(session_id, row_id, operator, cx);
            },
        );
        if let Some(session) = self.session_mut(session_id) {
            session
                .filter_subscriptions
                .extend([column_subscription, operator_subscription]);
        }
    }

    fn remove_filter_for(
        &mut self,
        session_id: SessionId,
        row_id: FilterRowId,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            session.filters.remove(row_id);
            cx.notify();
        }
    }

    fn clear_filters_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(session_id) {
            session.filters = FilterModel::new();
            session.filter_subscriptions.clear();
            cx.notify();
        }
    }

    fn set_filter_column_for(
        &mut self,
        session_id: SessionId,
        row_id: FilterRowId,
        column: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            if let Some(row) = session
                .filters
                .rows_mut()
                .iter_mut()
                .find(|row| row.id == row_id)
            {
                row.set_selected_column(column);
            }
            cx.notify();
        }
    }

    fn set_filter_operator_for(
        &mut self,
        session_id: SessionId,
        row_id: FilterRowId,
        operator: FilterOperator,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            if let Some(row) = session
                .filters
                .rows_mut()
                .iter_mut()
                .find(|row| row.id == row_id)
            {
                row.set_operator(operator);
            }
            cx.notify();
        }
    }

    fn refresh_table_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        self.load_table_page_for(session_id, 0, cx);
    }

    fn set_table_page(&mut self, session_id: SessionId, page: u64, cx: &mut Context<Self>) {
        let Some((current_page, has_next_page, busy)) = self.session(session_id).map(|session| {
            (
                session.table_page,
                session.table_has_next_page,
                session.busy,
            )
        }) else {
            return;
        };
        if busy || page == current_page || (page > current_page && !has_next_page) {
            return;
        }
        self.load_table_page_for(session_id, page, cx);
    }

    fn load_table_page_for(&mut self, session_id: SessionId, page: u64, cx: &mut Context<Self>) {
        let Some((engine, table, kind, busy)) = self.session(session_id).map(|session| {
            (
                session.engine.clone(),
                session.selected_table.clone(),
                session.kind,
                session.busy,
            )
        }) else {
            return;
        };
        let (Some(engine), Some(table)) = (engine, table) else {
            return;
        };
        // Refresh is explicitly a table reload.  It must also work after an
        // ad-hoc query, whose result_table provenance is intentionally None.
        if busy {
            return;
        }
        let filters = match self.active_filters_for(session_id, cx) {
            Ok(filters) => filters,
            Err(error) => {
                if let Some(session) = self.session_mut(session_id) {
                    session.error = Some(error);
                    session.status = "Filter needs attention".into();
                }
                cx.notify();
                return;
            }
        };
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = format!("Loading page {}…", page + 1);
        session.result_table = None;
        session.selected_row = None;
        session.row_draft = None;
        session.row_draft_subscriptions.clear();
        session.clear_grid_selection(cx);
        session.request_generation += 1;
        let generation = session.request_generation;
        let result_table = table.clone();
        let task = runtime.spawn(async move {
            if kind.is_sql() {
                let mut result = engine
                    .query_table(
                        &table,
                        &[],
                        &filters,
                        &[],
                        Some(table_browse_page(page)),
                        QueryOptions::default(),
                    )
                    .await?;
                let has_next_page = trim_table_browse_result(&mut result);
                Ok::<_, dbx_core::DbxError>((result, has_next_page))
            } else {
                let pattern = filters
                    .first()
                    .and_then(|filter| filter.value.as_ref())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "*".into());
                let pattern = redis_command_word(&pattern);
                let result = engine
                    .query(
                        &format!("SCAN 0 MATCH {pattern} COUNT 100"),
                        QueryOptions::default(),
                    )
                    .await?;
                Ok((result, false))
            }
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok((result, has_next_page)) => {
                        session.table_page = page;
                        session.table_has_next_page = has_next_page;
                        session.set_result(Some(result), cx);
                        session.result_table = Some(result_table.clone());
                        session.selected_row = None;
                        session.row_draft = None;
                        session.status = "Ready".into();
                        session.error = None;
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Operation failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn active_filters_for(&self, session_id: SessionId, cx: &App) -> Result<Vec<Filter>, String> {
        let Some(session) = self.session(session_id) else {
            return Ok(Vec::new());
        };
        if session.kind.is_sql() {
            return session
                .filters
                .validate(cx, &session.table_columns)
                .map_err(|error| error.to_string());
        }
        let value = session.editors.filter_text.read(cx).trim();
        let column = selected_filter_column(
            session.selected_column,
            &session.table_columns,
            session.result.as_deref(),
        )
        .map(|column| column.name.clone());
        Ok(match (value.is_empty(), column) {
            (false, Some(column)) => vec![Filter::new(
                column,
                FilterOperator::Contains,
                Some(CellValue::Text(value.to_owned())),
            )],
            _ => Vec::new(),
        })
    }

    fn editable_table_for(&self, session_id: SessionId) -> Option<&TableRef> {
        let session = self.session(session_id)?;
        let selected_table = session.selected_table.as_ref()?;
        let is_real_table = session
            .tables
            .iter()
            .any(|table| table.kind == EntityKind::Table && table_ref(table) == *selected_table);
        can_mutate_result(
            session.kind,
            session.busy,
            session.selected_table.as_ref(),
            session.result_table.as_ref(),
        )
        .then_some(selected_table)
        .filter(|_| is_real_table)
    }

    fn query_completion_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Option<SqlCompletionMenu> {
        let (tab_id, replacement_range, items, signature) = {
            let session = self.session(session_id)?;
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query_tab) = &tab.kind else {
                return None;
            };
            let text_revision = query_tab.query_revision;
            let query_text = query_tab.query_text.read(cx).clone();
            let cursor = query_tab.query_editor.read(cx).cursor_offset();
            if session.kind.is_sql() {
                let context = editor::sql_completion_context(&query_text, cursor)?;
                let items = sql_completion_items(
                    &query_text,
                    cursor,
                    &context,
                    SqlCompletionRequest {
                        database_kind: session.kind,
                        tables: &session.tables,
                        completion_columns: &session.completion_columns,
                        selected_table: session.selected_table.as_ref(),
                        active_columns: &session.table_columns,
                        result: session.result.as_deref(),
                        active_schema_filter: session.schema_filter.as_deref(),
                    },
                );
                (
                    tab_id,
                    context.replacement_range,
                    items,
                    CompletionSignature {
                        text_revision,
                        cursor,
                    },
                )
            } else if session.kind == DatabaseKind::Redis {
                let (replacement_range, items) = redis_completion_items(
                    &query_text,
                    cursor,
                    session.redis_command_catalog.as_deref(),
                    query_tab.result.as_deref(),
                    session.result.as_deref(),
                )?;
                (
                    tab_id,
                    replacement_range,
                    items,
                    CompletionSignature {
                        text_revision,
                        cursor,
                    },
                )
            } else {
                return None;
            }
        };

        if items.is_empty() {
            return None;
        }

        let session = self.session_mut(session_id)?;
        let tab = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)?;
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return None;
        };
        if query_tab.completion_dismissed_signature == Some(signature) {
            return None;
        }
        if query_tab.completion_signature != Some(signature) {
            query_tab.completion_signature = Some(signature);
            query_tab.completion_index = 0;
        }
        query_tab.completion_dismissed_signature = None;
        let selected = query_tab
            .completion_index
            .min(items.len().saturating_sub(1));
        query_tab.completion_index = selected;

        Some(SqlCompletionMenu {
            replacement_range,
            items,
            selected,
            signature,
        })
    }

    fn handle_completion_key(
        &mut self,
        session_id: SessionId,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor_focused = self
            .session(session_id)
            .and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                session.secondary_tabs.iter().find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(
                    query
                        .query_editor
                        .read(cx)
                        .focus_handle()
                        .is_focused(window),
                ),
                SecondaryTabKind::Structure(_) => None,
                SecondaryTabKind::Diagram(_) => None,
            })
            .unwrap_or(false);
        if !editor_focused {
            return;
        }
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let Some(menu) = self.query_completion_for(session_id, cx) else {
            return;
        };
        let Some(tab_id) = self
            .session(session_id)
            .and_then(|session| session.active_secondary_tab)
        else {
            return;
        };

        match key {
            "up" | "down" => {
                let Some(session) = self.session_mut(session_id) else {
                    return;
                };
                let Some(tab) = session
                    .secondary_tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                else {
                    return;
                };
                let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
                    return;
                };
                let count = menu.items.len();
                query_tab.completion_index = if key == "up" {
                    if query_tab.completion_index == 0 {
                        count - 1
                    } else {
                        query_tab.completion_index - 1
                    }
                } else {
                    (query_tab.completion_index + 1) % count
                };
                cx.stop_propagation();
                cx.notify();
            }
            "enter" | "tab" => {
                let Some(item) = menu.items.get(menu.selected).cloned() else {
                    return;
                };
                self.accept_completion_for(
                    session_id,
                    tab_id,
                    menu.replacement_range,
                    item,
                    window,
                    cx,
                );
                cx.stop_propagation();
            }
            "escape" => {
                if let Some(session) = self.session_mut(session_id)
                    && let Some(tab) = session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| tab.id == tab_id)
                    && let SecondaryTabKind::Query(query_tab) = &mut tab.kind
                {
                    query_tab.completion_dismissed_signature = Some(menu.signature);
                }
                cx.stop_propagation();
                cx.notify();
            }
            _ => {}
        }
    }

    fn handle_completion_action(
        &mut self,
        session_id: SessionId,
        action: CompletionAction,
        query_editor: Entity<TextEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.query_completion_for(session_id, cx) else {
            match action {
                CompletionAction::Up => {
                    query_editor.update(cx, |editor, cx| editor.move_vertical(-1, cx));
                }
                CompletionAction::Down => {
                    query_editor.update(cx, |editor, cx| editor.move_vertical(1, cx));
                }
                CompletionAction::Enter => {
                    query_editor.update(cx, |editor, cx| editor.insert_newline(cx));
                }
            }
            return;
        };
        let Some(tab_id) = self
            .session(session_id)
            .and_then(|session| session.active_secondary_tab)
        else {
            return;
        };

        match action {
            CompletionAction::Up | CompletionAction::Down => {
                let Some(session) = self.session_mut(session_id) else {
                    return;
                };
                let Some(tab) = session
                    .secondary_tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                else {
                    return;
                };
                let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
                    return;
                };
                let count = menu.items.len();
                query_tab.completion_index = if action == CompletionAction::Up {
                    if query_tab.completion_index == 0 {
                        count - 1
                    } else {
                        query_tab.completion_index - 1
                    }
                } else {
                    (query_tab.completion_index + 1) % count
                };
                cx.notify();
            }
            CompletionAction::Enter => {
                let Some(item) = menu.items.get(menu.selected).cloned() else {
                    return;
                };
                self.accept_completion_for(
                    session_id,
                    tab_id,
                    menu.replacement_range,
                    item,
                    window,
                    cx,
                );
            }
        }
    }

    fn accept_completion_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        replacement_range: Range<usize>,
        item: SqlCompletionItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(query_editor) = self
            .session(session_id)
            .and_then(|session| session.secondary_tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(query.query_editor.clone()),
                SecondaryTabKind::Structure(_) => None,
                SecondaryTabKind::Diagram(_) => None,
            })
        else {
            return;
        };
        let focus = query_editor.read(cx).focus_handle();
        query_editor.update(cx, |editor, cx| {
            editor.replace_range(replacement_range, item.insert_text, cx);
        });

        // Accepting a candidate produces a new text/cursor signature. Dismiss
        // that exact state so the item we just committed does not immediately
        // reopen under the caret; the next edit or caret move changes the
        // signature and makes completion available again.
        let accepted_signature = self
            .query_completion_for(session_id, cx)
            .map(|menu| menu.signature);
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            && let SecondaryTabKind::Query(query_tab) = &mut tab.kind
        {
            query_tab.completion_signature = None;
            query_tab.completion_dismissed_signature = accepted_signature;
            query_tab.completion_index = 0;
        }
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn request_run_query_for(
        &mut self,
        session_id: SessionId,
        run_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, query_editor)) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some((session.kind, query.query_editor.clone()))
        }) else {
            return;
        };
        if !kind.is_sql() {
            // Redis is intentionally line-oriented: a multiline document is
            // not a Redis pipeline, and flattening it into one command would
            // silently change its meaning. The only execution unit is the
            // selection or current line.
            self.run_query_for_execution(session_id, false, cx);
            return;
        }
        let text = query_editor.read(cx).text(cx);
        let scope = if run_all {
            editor::QueryExecutionScope::Document
        } else {
            editor::QueryExecutionScope::SelectionOrStatement
        };
        let range = if run_all {
            0..text.len()
        } else {
            query_editor.read(cx).execution_range(scope, cx)
        };
        let query = text[range].trim();
        if query.is_empty() {
            return;
        }
        let (title, detail, confirm_label, tone) = match editor::sql_execution_kind(query) {
            editor::SqlExecutionKind::Destructive => (
                "Run destructive query?",
                "This statement can permanently change or delete data. Review it before continuing.",
                "Run query",
                ConfirmationTone::Danger,
            ),
            _ if editor::sql_statement_count(query) > 1 => (
                "Run multiple statements?",
                "Statements run in order and are not automatically transactional. Earlier changes may remain if a later statement fails.",
                "Run statements",
                ConfirmationTone::Warning,
            ),
            _ => {
                self.run_query_for_execution(session_id, run_all, cx);
                return;
            }
        };
        let return_focus = Some(query_editor.read(cx).focus_handle());
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: title.into(),
            detail: detail.into(),
            confirm_label,
            tone,
            action: ConfirmationAction::RunQuery {
                session_id,
                run_all,
            },
            focus: focus.clone(),
            return_focus,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    /// Execute the selected text or current statement. `run_all` is reserved
    /// for the explicit whole-document action; it intentionally ignores an
    /// editor selection.
    fn run_query_for_execution(
        &mut self,
        session_id: SessionId,
        run_all: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, tab_id, kind, database, history_connection, query_editor, busy)) =
            self.session(session_id).and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Query(query_tab) = &tab.kind else {
                    return None;
                };
                Some((
                    session.engine.clone(),
                    tab_id,
                    session.kind,
                    session.current_database.clone(),
                    query_history_connection(session),
                    query_tab.query_editor.clone(),
                    query_tab.busy,
                ))
            })
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if busy {
            return;
        }
        let full_query = query_editor.read(cx).text(cx);
        let scope = if run_all {
            editor::QueryExecutionScope::Document
        } else if kind.is_sql() {
            editor::QueryExecutionScope::SelectionOrStatement
        } else {
            editor::QueryExecutionScope::SelectionOrCurrentLine
        };
        let range = if run_all {
            0..full_query.len()
        } else {
            query_editor.read(cx).execution_range(scope, cx)
        };
        let selected_query = &full_query[range.clone()];
        let executed_leading_whitespace = selected_query.len() - selected_query.trim_start().len();
        let query = selected_query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        let may_change_schema = kind.is_sql() && editor::sql_may_change_schema(&query);
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };
        query_tab.busy = true;
        query_tab.results_stale = query_tab.result.is_some();
        query_tab.error = None;
        query_tab.status = "Running query…".into();
        query_tab.request_generation = query_tab.request_generation.saturating_add(1);
        let generation = query_tab.request_generation;
        let query_revision = query_tab.query_revision;
        query_tab.executed_database = database.clone();
        cx.notify();
        // The executed statement moves into the blocking task; the original
        // stays behind so failures can locate the offending token in it.
        let executed_query = query.clone();
        let task = runtime
            .spawn(async move { engine.query(&executed_query, QueryOptions::default()).await });
        query_tab.abort_handle.replace(task.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let (history_outcome, refresh_schema) = {
                    let Some(session) = this.session_mut(session_id) else {
                        return;
                    };
                    let Some(tab) = session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| tab.id == tab_id)
                    else {
                        return;
                    };
                    let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
                        return;
                    };
                    if generation != query_tab.request_generation {
                        return;
                    }
                    query_tab.busy = false;
                    query_tab.abort_handle.clear();
                    match result {
                        Ok(Ok(result)) => {
                            query_tab.status = query_result_status(&result);
                            let outcome = QueryHistoryOutcome::success(query_tab.status.clone());
                            query_tab.set_result(Some(result), cx);
                            query_tab.results_stale = false;
                            query_tab.error = None;
                            query_tab.error_highlight = None;
                            (outcome, may_change_schema)
                        }
                        Ok(Err(error)) => {
                            let message = error.to_string();
                            // Positions reported against the trimmed statement
                            // shift by the trimmed leading whitespace.
                            let lead = range.start + executed_leading_whitespace;
                            query_tab.error_highlight =
                                if query_tab.query_revision == query_revision {
                                    editor::sql_error_range(&message, &query)
                                        .map(|range| range.start + lead..range.end + lead)
                                } else {
                                    None
                                };
                            query_tab.error = Some(message.clone());
                            query_tab.results_stale = query_tab.result.is_some();
                            query_tab.status = "Operation failed".into();
                            (QueryHistoryOutcome::failure(message), false)
                        }
                        Err(error) => {
                            let message = format!("Query task stopped unexpectedly: {error}");
                            query_tab.error = Some(message.clone());
                            query_tab.results_stale = query_tab.result.is_some();
                            query_tab.status = "Operation failed".into();
                            (QueryHistoryOutcome::failure(message), false)
                        }
                    }
                };
                this.record_query_history(
                    history_connection.clone(),
                    query.clone(),
                    history_outcome,
                    cx,
                );
                if refresh_schema {
                    if let Some(session) = this.session_mut(session_id) {
                        for tab in &mut session.secondary_tabs {
                            if let SecondaryTabKind::Diagram(diagram) = &mut tab.kind {
                                diagram.stale = diagram.document.is_some();
                            }
                        }
                    }
                    // Refresh Explorer first; its guarded completion then
                    // rebuilds any open diagram from the same catalogue.
                    this.refresh_tables_for(session_id, cx);
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn copy_query_selection_action(
        &mut self,
        _: &CopyQuerySelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session_id) = self.active_session_id() {
            self.copy_query_selection_for(session_id, cx);
        }
    }

    fn cancel_query_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };
        if !query_tab.busy {
            return;
        }
        query_tab.invalidate_request();
        query_tab.results_stale = query_tab.result.is_some();
        query_tab.error = None;
        query_tab.error_highlight = None;
        query_tab.status = "Query cancelled".into();
        cx.notify();
    }

    fn query_result_text_for(
        &self,
        session_id: SessionId,
        format: QueryResultExportFormat,
        cx: &App,
    ) -> Option<String> {
        let query = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some(query)
        })?;
        let delegate = query.result_grid.read(cx).delegate();
        match format {
            QueryResultExportFormat::Tsv => delegate.result_as_tsv(),
            QueryResultExportFormat::Csv => delegate.result_as_csv(),
            QueryResultExportFormat::Json => delegate.result_as_json(),
        }
    }

    /// Copy the most specific active selection: cell, then row, then column.
    /// Column zero is DBX's synthetic row-number column and has no database value.
    pub(super) fn copy_query_selection_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some((text, label)) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            let grid = query.result_grid.read(cx);
            let delegate = grid.delegate();
            match query.result_selection {
                QueryResultSelection::Cell => {
                    let (row, column) = grid.selected_cell()?;
                    let text = if column == 0 {
                        Some((row + 1).to_string())
                    } else {
                        delegate.cell_as_plain_text(row, column - 1)
                    }?;
                    Some((text, "cell"))
                }
                QueryResultSelection::Row => grid
                    .selected_row()
                    .and_then(|row| delegate.row_as_tsv(row))
                    .map(|text| (text, "row")),
                QueryResultSelection::Column => grid.selected_col().and_then(|column| {
                    (column > 0)
                        .then(|| delegate.column_as_tsv(column - 1))
                        .flatten()
                        .map(|text| (text, "column"))
                }),
                QueryResultSelection::None => None,
            }
        }) else {
            if let Some(session) = self.session_mut(session_id) {
                session.status = "Select a result cell, row, or column to copy".into();
            }
            cx.notify();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if let Some(session) = self.session_mut(session_id) {
            session.status = format!("Copied {label}");
        }
        cx.notify();
    }

    pub(super) fn copy_query_result_for(
        &mut self,
        session_id: SessionId,
        format: QueryResultExportFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.query_result_text_for(session_id, format, cx) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if let Some(session) = self.session_mut(session_id) {
            session.status = format!("Copied {} result", format.extension().to_ascii_uppercase());
        }
        cx.notify();
    }

    pub(super) fn export_query_result_for(
        &mut self,
        session_id: SessionId,
        format: QueryResultExportFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.query_result_text_for(session_id, format, cx) else {
            return;
        };
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested = format!("query-result.{}", format.extension());
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(path))) => {
                    let destination = path.display().to_string();
                    let result = runtime
                        .spawn_blocking(move || std::fs::write(path, text))
                        .await;
                    this.update(cx, |this, cx| {
                        if let Some(session) = this.session_mut(session_id) {
                            match result {
                                Ok(Ok(())) => {
                                    session.error = None;
                                    session.status = format!("Exported result to {destination}");
                                }
                                Ok(Err(error)) => {
                                    session.error =
                                        Some(format!("Could not export result: {error}"));
                                    session.status = "Result export failed".into();
                                }
                                Err(error) => {
                                    session.error =
                                        Some(format!("Result export task stopped: {error}"));
                                    session.status = "Result export failed".into();
                                }
                            }
                        }
                        cx.notify();
                    })?;
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open the save dialog: {error}"));
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Save dialog closed unexpectedly: {error}"));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn copy_query_error_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let error = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            query.error.clone()
        });
        if let Some(error) = error {
            cx.write_to_clipboard(ClipboardItem::new_string(error));
            if let Some(session) = self.session_mut(session_id) {
                session.status = "Copied query error".into();
            }
            cx.notify();
        }
    }

    pub(super) fn focus_query_error_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            query
                .error_highlight
                .as_ref()
                .map(|range| (query.query_editor.clone(), range.start))
        });
        if let Some((editor, offset)) = editor {
            let focus = editor.read(cx).focus_handle();
            editor.update(cx, |editor, cx| editor.move_cursor_to(offset, cx));
            focus.focus(window, cx);
        }
    }

    /// Pretty-print the active query tab's SQL in place, keeping the caret
    /// anchored to the token it sat on. Redis tabs have nothing to format.
    fn format_query_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(query_editor) = self.session(session_id).and_then(|session| {
            if !session.kind.is_sql() {
                return None;
            }
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some(query.query_editor.clone())
        }) else {
            return;
        };

        let (text, cursor, focus_handle) = query_editor.update(cx, |editor, cx| {
            (
                editor.text(cx),
                editor.cursor_offset(),
                editor.focus_handle(),
            )
        });
        let (formatted, mapped_cursor) = editor::format_sql_at_cursor(&text, cursor);
        if formatted != text {
            let length = text.len();
            query_editor.update(cx, |editor, cx| {
                editor.replace_range(0..length, formatted.as_str(), cx);
                editor.move_cursor_to(mapped_cursor, cx);
            });
        }
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab_id) = session.active_secondary_tab
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            && let SecondaryTabKind::Query(query_tab) = &mut tab.kind
        {
            query_tab.completion_signature = None;
            query_tab.completion_dismissed_signature = None;
            query_tab.completion_index = 0;
        }
        focus_handle.focus(window, cx);
        cx.notify();
    }

    fn refresh_tables_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else {
            return;
        };

        let Some(engine) = session.engine.clone() else {
            return;
        };
        let expected_engine = engine.clone();
        let expected_database = session.current_database.clone();
        let expected_kind = session.kind;

        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move { engine.list_tables().await });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }

        cx.spawn(async move |this, cx| {
            let tables = task.await??;

            this.update(cx, |this, cx| {
                let request_is_current = this.session(session_id).is_some_and(|session| {
                    session.kind == expected_kind
                        && session.current_database == expected_database
                        && session
                            .engine
                            .as_ref()
                            .is_some_and(|engine| Arc::ptr_eq(engine, &expected_engine))
                });
                if !request_is_current {
                    return;
                }
                let diagram_open = if let Some(session) = this.session_mut(session_id) {
                    session.tables = tables;
                    for tab in &mut session.secondary_tabs {
                        if let SecondaryTabKind::Diagram(diagram) = &mut tab.kind {
                            // Table discovery is the authoritative signal that
                            // this cached scene may no longer describe the DB.
                            diagram.stale = diagram.document.is_some();
                        }
                    }
                    session
                        .secondary_tabs
                        .iter()
                        .any(|tab| matches!(&tab.kind, SecondaryTabKind::Diagram(_)))
                } else {
                    false
                };

                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
                if diagram_open {
                    this.refresh_diagram_for(session_id, cx);
                }
            })?;

            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Warm a bounded schema cache in the background so query completion can
    /// resolve columns for tables the user has not opened yet. Failures are
    /// intentionally ignored here: table names remain useful completions and
    /// opening a table still retries its authoritative structure request.
    fn prefetch_completion_columns_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        const MAX_COMPLETION_METADATA_TABLES: usize = 128;

        let Some((engine, kind, database, tables)) = self.session(session_id).map(|session| {
            (
                session.engine.clone(),
                session.kind,
                session.current_database.clone(),
                session.tables.clone(),
            )
        }) else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if !kind.is_sql() {
            return;
        }

        let request_engine = engine.clone();
        let expected_engine = engine;
        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move {
            let mut metadata = HashMap::new();
            for table in tables
                .into_iter()
                .filter(|table| matches!(table.kind, EntityKind::Table | EntityKind::View))
                .take(MAX_COMPLETION_METADATA_TABLES)
            {
                let table_ref = table_ref(&table);
                if let Ok(columns) = request_engine.describe_table(&table_ref).await {
                    metadata.insert(completion_table_key(&table_ref), columns);
                }
            }
            metadata
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }

        cx.spawn(async move |this, cx| {
            let metadata = task.await?;

            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                let same_engine = session
                    .engine
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &expected_engine));
                if session.kind != kind || session.current_database != database || !same_engine {
                    return;
                }
                session.completion_columns.extend(metadata);
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Discover the exact command grammar exposed by this Redis/Valkey server
    /// once per connection. The editor keeps using its local fallback until
    /// this background request finishes, and discovery failures never make an
    /// otherwise healthy database connection unusable.
    fn prefetch_redis_command_catalog_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, kind)) = self
            .session(session_id)
            .map(|session| (session.engine.clone(), session.kind))
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if kind != DatabaseKind::Redis {
            return;
        }

        let expected_engine = engine.clone();
        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                engine.redis_command_catalog(),
            )
            .await
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }

        cx.spawn(async move |this, cx| {
            let catalog = task.await?;
            if let Ok(Ok(catalog)) = catalog {
                this.update(cx, |this, cx| {
                    let Some(session) = this.session_mut(session_id) else {
                        return;
                    };
                    let same_engine = session
                        .engine
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &expected_engine));
                    if session.kind != DatabaseKind::Redis || !same_engine {
                        return;
                    }
                    session.redis_command_catalog = Some(Arc::new(catalog));
                    cx.notify();
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn refresh_action(&mut self, _: &RefreshData, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_table(cx);
    }

    /// Switch the session's active database on the existing engine. The
    /// engine keeps its connection; only the selected database changes, so
    /// tables and data are reloaded for the new context.
    fn switch_database_for(
        &mut self,
        session_id: SessionId,
        database: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(engine) = session.engine.clone() else {
            return;
        };
        if session.current_database.as_deref() == Some(database.as_str()) || session.busy {
            return;
        }
        for tab in &mut session.secondary_tabs {
            match &mut tab.kind {
                SecondaryTabKind::Query(query) => {
                    query.invalidate_request();
                    query.results_stale = query.result.is_some();
                    query.status = "Results are from the previous database".into();
                }
                SecondaryTabKind::Diagram(diagram) => {
                    diagram.invalidate_request();
                    diagram.busy = true;
                    diagram.stale = diagram.document.is_some();
                    diagram.error = None;
                }
                SecondaryTabKind::Structure(_) => {}
            }
        }
        session.busy = true;
        session.status = format!("Switching to {database}…");
        session.error = None;
        let runtime = self.runtime.clone();
        let task = runtime.spawn({
            let target = database.clone();
            async move {
                engine.use_database(&target).await?;
                let tables = engine.list_tables().await?;
                Ok::<_, dbx_core::DbxError>(tables)
            }
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;

            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                session.busy = false;
                let diagram_open = session
                    .secondary_tabs
                    .iter()
                    .any(|tab| matches!(&tab.kind, SecondaryTabKind::Diagram(_)));
                let mut diagram_needs_reload = false;
                match result {
                    Ok(Ok(tables)) => {
                        session.tables = tables;
                        session.current_database = Some(database.clone());
                        session.selected_table = None;
                        session.table_columns.clear();
                        session.completion_columns.clear();
                        session.set_result(None, cx);
                        session.result_table = None;
                        session.schema_filter = None;
                        let diagram_schemas = diagram_schema_names(session.kind, &session.tables);
                        let diagram_selection = diagram_initial_schema_selection(
                            session.kind,
                            session.schema_filter.as_deref(),
                        );
                        for tab in &mut session.secondary_tabs {
                            if let SecondaryTabKind::Diagram(diagram) = &mut tab.kind {
                                diagram.source_schema = None;
                                diagram.document = None;
                                diagram.available_schemas = diagram_schemas.clone();
                                diagram.selected_schemas = diagram_selection.clone();
                                diagram.selected_node = None;
                                diagram.scroll_handle.set_offset(point(px(0.), px(0.)));
                                diagram.drag_anchor = None;
                            }
                        }
                        session.foreign_keys.clear();
                        session.row_draft = None;
                        session.row_draft_subscriptions.clear();
                        session.status = format!("Switched to {database}");
                        session.error = None;
                        diagram_needs_reload = diagram_open;
                    }
                    Ok(Err(error)) => {
                        for tab in &mut session.secondary_tabs {
                            if let SecondaryTabKind::Diagram(diagram) = &mut tab.kind {
                                diagram.busy = false;
                                diagram.stale = diagram.document.is_some();
                            }
                        }
                        session.error = Some(error.to_string());
                        session.status = "Database switch failed".into();
                    }
                    Err(error) => {
                        for tab in &mut session.secondary_tabs {
                            if let SecondaryTabKind::Diagram(diagram) = &mut tab.kind {
                                diagram.busy = false;
                                diagram.stale = diagram.document.is_some();
                            }
                        }
                        session.error = Some(format!(
                            "Database switch task stopped unexpectedly: {error}"
                        ));
                        session.status = "Database switch failed".into();
                    }
                }
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
                if diagram_needs_reload {
                    this.refresh_diagram_for(session_id, cx);
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn begin_insert_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id).is_none() {
            return;
        }
        let Some(columns) = self
            .session(session_id)
            .map(|session| session.table_columns.clone())
        else {
            return;
        };
        let mut row_draft = RowDraftModel::new();
        for column in columns {
            row_draft.push(FieldRow::new_insert(column, None, window, cx));
        }
        self.watch_draft_fields_for(session_id, &row_draft, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.draft_mode = DraftMode::Insert;
        session.selected_row = None;
        session.inspector_open = true;
        session.clear_grid_selection(cx);
        session.row_draft = Some(row_draft);
        session.error = None;
        session.status = "Preparing a new row".into();
        cx.notify();
    }

    fn watch_draft_fields_for(
        &mut self,
        session_id: SessionId,
        row_draft: &RowDraftModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut subscriptions = Vec::new();
        for field in row_draft.fields() {
            if let Some(selector) = field.enum_selector.clone() {
                let field_id = field.id;
                subscriptions.push(cx.subscribe_in(
                    &selector,
                    window,
                    move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                        let SelectEvent::Confirm(value) = event;
                        let value = value.as_ref().map(ToString::to_string);
                        this.set_row_value_text_for(session_id, field_id, value, cx);
                    },
                ));
            }
            if let Some(selector) = field.boolean_selector.clone() {
                let field_id = field.id;
                subscriptions.push(cx.subscribe_in(
                    &selector,
                    window,
                    move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                        let SelectEvent::Confirm(value) = event;
                        let value = value.as_ref().map(ToString::to_string);
                        this.set_row_value_text_for(session_id, field_id, value, cx);
                    },
                ));
            }
            if let Some(selector) = field.state_selector.clone() {
                let field_id = field.id;
                subscriptions.push(cx.subscribe_in(
                    &selector,
                    window,
                    move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                        let SelectEvent::Confirm(value) = event;
                        let state = value
                            .as_ref()
                            .and_then(|value| FieldValueState::from_label(value.as_ref()));
                        if let Some(state) = state {
                            this.set_row_field_state_for(session_id, field_id, state, cx);
                        }
                    },
                ));
            }
        }
        if let Some(session) = self.session_mut(session_id) {
            session.row_draft_subscriptions = subscriptions;
        }
    }

    fn set_row_value_text_for(
        &mut self,
        session_id: SessionId,
        field_id: FieldId,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(value) = value else {
            return;
        };
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(field) = session.row_draft.as_mut().and_then(|draft| {
            draft
                .fields_mut()
                .iter_mut()
                .find(|field| field.id == field_id)
        }) else {
            return;
        };
        field.set_value();
        field.value.update(cx, |current, cx| {
            *current = value;
            cx.notify();
        });
        session.error = None;
        cx.notify();
    }

    fn on_data_grid_event(
        &mut self,
        session_id: SessionId,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, TableEvent::SelectRow(_) | TableEvent::ClearSelection)
            && self.session_mut(session_id).is_some_and(|session| {
                if session.suppress_next_grid_selection_event {
                    session.suppress_next_grid_selection_event = false;
                    true
                } else {
                    false
                }
            })
        {
            return;
        }
        match event {
            TableEvent::ColumnWidthsChanged(widths) => {
                if let Some(session) = self.session_mut(session_id) {
                    session.result_column_widths =
                        ResultTableDelegate::widths_by_key(session.result.as_deref(), widths);
                }
            }
            TableEvent::SelectRow(row_index) => {
                self.select_row_for(session_id, *row_index, window, cx);
            }
            // The inline foreign-key action uses the table component's
            // existing cell event channel so the virtualized grid remains
            // responsible for rendering and hit testing its cells.
            TableEvent::DoubleClickedCell(row_index, column_index) => {
                self.navigate_to_foreign_key_row_for(
                    session_id,
                    *row_index,
                    *column_index,
                    window,
                    cx,
                );
            }
            TableEvent::SelectColumn(column_index) if *column_index > 0 => {
                self.select_column_for(session_id, *column_index - 1, cx);
            }
            TableEvent::ClearSelection => {
                if let Some(session) = self.session_mut(session_id)
                    && session.draft_mode == DraftMode::Update
                {
                    session.selected_row = None;
                    session.row_draft = None;
                    session.row_draft_subscriptions.clear();
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn on_query_grid_event(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        event: &TableEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };

        match event {
            TableEvent::ColumnWidthsChanged(widths) => {
                query_tab.result_column_widths =
                    ResultTableDelegate::widths_by_key(query_tab.result.as_deref(), widths);
            }
            TableEvent::SelectCell(..) => {
                query_tab.result_selection = QueryResultSelection::Cell;
            }
            TableEvent::SelectRow(..) => {
                query_tab.result_selection = QueryResultSelection::Row;
            }
            TableEvent::SelectColumn(..) => {
                query_tab.result_selection = QueryResultSelection::Column;
            }
            TableEvent::ClearSelection => {
                query_tab.result_selection = QueryResultSelection::None;
            }
            _ => return,
        }
        cx.notify();
    }

    fn select_row_for(
        &mut self,
        session_id: SessionId,
        row: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending_draft = self.session(session_id).and_then(|session| {
            session.row_draft.as_ref()?;
            Some((session.selected_row, session.data_grid.clone()))
        });
        if let Some((selected_row, data_grid)) = pending_draft {
            if let Some(session) = self.session_mut(session_id) {
                session.suppress_next_grid_selection_event = true;
            }
            data_grid.update(cx, |table, cx| {
                if let Some(selected_row) = selected_row {
                    table.set_selected_row(selected_row, cx);
                } else {
                    table.clear_selection(cx);
                }
            });
            if let Some(session) = self.session_mut(session_id) {
                session.status = "Save or cancel the current row before selecting another".into();
            }
            cx.notify();
            return;
        }
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(result) = session.result.as_ref() else {
            return;
        };
        if result.rows.get(row).is_none() {
            return;
        }
        session.selected_row = Some(row);
        session.draft_mode = DraftMode::Update;
        session.inspector_open = true;
        session.row_draft = None;
        session.row_draft_subscriptions.clear();
        session.error = None;
        session.status = "Inspecting selected row".into();
        cx.notify();
    }

    fn begin_edit_selected_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id).is_none() {
            return;
        }
        let draft_data = self.session(session_id).and_then(|session| {
            let row = session.selected_row?;
            let result = session.result.as_ref()?;
            let values = result.rows.get(row)?.values.clone();
            Some((
                session.table_columns.clone(),
                result.columns.clone(),
                values,
            ))
        });
        let Some((table_columns, result_columns, values)) = draft_data else {
            return;
        };
        let mut draft = RowDraftModel::new();
        for column in table_columns {
            let Some(index) = result_columns
                .iter()
                .position(|result_column| result_column.name == column.name)
            else {
                if let Some(session) = self.session_mut(session_id) {
                    session.error = Some(format!(
                        "Column {} is missing from the loaded table result",
                        column.name
                    ));
                    session.status = "Row cannot be edited".into();
                }
                cx.notify();
                return;
            };
            let Some(original) = values.get(index).cloned() else {
                return;
            };
            draft.push(FieldRow::new_update(column, original, window, cx));
        }
        self.watch_draft_fields_for(session_id, &draft, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.draft_mode = DraftMode::Update;
        session.inspector_open = true;
        session.row_draft = Some(draft);
        session.error = None;
        session.status = "Editing selected row".into();
        cx.notify();
    }

    fn close_inspector_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(session_id) {
            session.inspector_open = false;
            cx.notify();
        }
    }

    fn select_column_for(&mut self, session_id: SessionId, column: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.selected_column = column;
        cx.notify();
    }

    fn set_row_field_state_for(
        &mut self,
        session_id: SessionId,
        field_id: FieldId,
        state: FieldValueState,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(field) = session.row_draft.as_mut().and_then(|draft| {
            draft
                .fields_mut()
                .iter_mut()
                .find(|field| field.id == field_id)
        }) else {
            return;
        };
        if state == FieldValueState::Null && !field.column.nullable {
            return;
        }
        if state == FieldValueState::Default && session.draft_mode != DraftMode::Insert {
            return;
        }
        field.set_state(state);
        session.error = None;
        cx.notify();
    }

    fn cancel_row_draft_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(session_id) {
            let was_insert = session.draft_mode == DraftMode::Insert;
            session.row_draft = None;
            session.row_draft_subscriptions.clear();
            if was_insert {
                session.selected_row = None;
                session.clear_grid_selection(cx);
            }
            session.draft_mode = DraftMode::Update;
            session.error = None;
            session.status = if was_insert {
                "New row cancelled".into()
            } else {
                "Row edit cancelled".into()
            };
            cx.notify();
        }
    }

    fn save_draft_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy {
            return;
        }
        let draft_mode = session.draft_mode;
        if !session.kind.is_sql() {
            let return_focus = self
                .row_draft_focus_for(session_id, None, cx)
                .or_else(|| window.focused(cx));
            self.show_mutation_error_for(
                session_id,
                draft_mode,
                "Use the Redis command console to mutate keys in this MVP.".into(),
                return_focus,
                window,
                cx,
            );
            return;
        }
        let (Some(engine), Some(table), Some(row_draft)) = (
            session.engine.clone(),
            self.editable_table_for(session_id).cloned(),
            session.row_draft.as_ref(),
        ) else {
            return;
        };
        let runtime = self.runtime.clone();
        let request = match draft_mode {
            DraftMode::Insert => row_draft
                .insert_values(cx)
                .map(|values| {
                    Some(Mutation::Insert(InsertRequest::from_mutation_row(
                        table.clone(),
                        values,
                    )))
                })
                .map_err(|error| (error.to_string(), Some(error.field_id()))),
            DraftMode::Update => {
                let Some(row) = session
                    .selected_row
                    .and_then(|row| session.result.as_ref()?.rows.get(row))
                    .cloned()
                else {
                    return;
                };
                row_draft
                    .changed_fields(cx)
                    .map_err(|error| (error.to_string(), Some(error.field_id())))
                    .and_then(|assignments| {
                        if assignments.is_empty() {
                            return Ok(None);
                        }
                        self.identity_filters_for(session_id, &row)
                            .map_err(|error| (error, None))
                            .map(|filters| {
                                Some(Mutation::Update(UpdateRequest::new_with_mutation_values(
                                    table.clone(),
                                    assignments,
                                    filters,
                                )))
                            })
                    })
            }
        };
        let request = match request {
            Ok(Some(request)) => request,
            Ok(None) => {
                if let Some(session) = self.session_mut(session_id) {
                    session.error = None;
                    session.status = "No row fields changed".into();
                }
                cx.notify();
                return;
            }
            Err((error, field_id)) => {
                let return_focus = self
                    .row_draft_focus_for(session_id, field_id, cx)
                    .or_else(|| window.focused(cx));
                self.show_mutation_error_for(
                    session_id,
                    draft_mode,
                    error,
                    return_focus,
                    window,
                    cx,
                );
                return;
            }
        };
        let error_return_focus = self
            .row_draft_focus_for(session_id, None, cx)
            .or_else(|| window.focused(cx));
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = "Applying row change…".into();
        session.request_generation += 1;
        let generation = session.request_generation;
        let task = runtime.spawn(async move {
            match request {
                Mutation::Insert(request) => engine.insert(&request).await,
                Mutation::Update(request) => engine.update(&request).await,
            }
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let outcome = task
                .await
                .map_err(|error| format!("Row mutation task failed: {error}"))
                .and_then(|outcome| outcome.map_err(|error| error.to_string()));
            this.update_in(cx, |this, window, cx| {
                let Some(session) = this.session(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                match outcome {
                    Ok(result) => {
                        if let Some(session) = this.session_mut(session_id) {
                            session.busy = false;
                            session.status =
                                format!("Saved · {} row(s) changed", result.rows_affected);
                            session.error = None;
                        }
                        this.refresh_table_for(session_id, cx);
                    }
                    Err(error) => {
                        this.show_mutation_error_for(
                            session_id,
                            draft_mode,
                            error,
                            error_return_focus,
                            window,
                            cx,
                        );
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn request_delete_selected_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((table, selected_row, row)) = self.session(session_id).and_then(|session| {
            let selected_row = session.selected_row?;
            let row = session.result.as_ref()?.rows.get(selected_row)?.clone();
            Some((
                self.editable_table_for(session_id)?.clone(),
                selected_row,
                row,
            ))
        }) else {
            return;
        };
        let filters = match self.identity_filters_for(session_id, &row) {
            Ok(filters) => filters,
            Err(error) => {
                if let Some(session) = self.session_mut(session_id) {
                    session.error = Some(error);
                    session.status = "Operation failed".into();
                }
                cx.notify();
                return;
            }
        };
        let return_focus = window.focused(cx);
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: format!("Delete row {}?", selected_row + 1),
            detail: format!(
                "This permanently deletes the selected row from {}. This action cannot be undone.",
                table_ref_label(&table)
            ),
            confirm_label: "Delete row",
            tone: ConfirmationTone::Danger,
            action: ConfirmationAction::DeleteRow {
                session_id,
                table,
                filters,
            },
            focus: focus.clone(),
            return_focus,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn delete_row_for(
        &mut self,
        session_id: SessionId,
        table: TableRef,
        filters: Vec<Filter>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy {
            return;
        }
        if !session.kind.is_sql() {
            if let Some(session) = self.session_mut(session_id) {
                session.error =
                    Some("Use the Redis command console to mutate keys in this MVP.".into());
                session.status = "Operation failed".into();
            }
            cx.notify();
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = "Deleting row…".into();
        session.request_generation += 1;
        let generation = session.request_generation;
        let task = runtime.spawn(async move { engine.delete(&table, &filters).await });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                match outcome {
                    Ok(result) => {
                        if let Some(session) = this.session_mut(session_id) {
                            session.busy = false;
                            session.status =
                                format!("Deleted · {} row(s) changed", result.rows_affected);
                            session.error = None;
                        }
                        this.refresh_table_for(session_id, cx);
                    }
                    Err(error) => {
                        if let Some(session) = this.session_mut(session_id) {
                            session.busy = false;
                            session.error = Some(error.to_string());
                            session.status = "Operation failed".into();
                        }
                        cx.notify();
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn open_table_context_menu(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        position: Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.table_context_menu = Some(TableContextMenu {
            session_id,
            table,
            position,
        });
        cx.notify();
    }

    fn close_table_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.table_context_menu.take().is_some() {
            cx.notify();
        }
    }

    fn confirm_table_action(
        &mut self,
        action: TableAction,
        session_id: SessionId,
        table: TableInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = window.focused(cx);
        self.table_context_menu = None;
        let qualified_name = table_sidebar_label(&table, None);
        let (title, detail, confirm_label) = match action {
            TableAction::Truncate => (
                format!("Truncate {qualified_name}?"),
                "Every row in this table will be permanently deleted. The table structure remains."
                    .to_owned(),
                "Truncate table",
            ),
            TableAction::Drop => (
                format!("Delete table {qualified_name}?"),
                "The table, its rows, indexes, and constraints will be permanently removed."
                    .to_owned(),
                "Delete table",
            ),
        };
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title,
            detail,
            confirm_label,
            tone: ConfirmationTone::Danger,
            action: ConfirmationAction::Table {
                action,
                session_id,
                table,
            },
            focus: focus.clone(),
            return_focus,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn cancel_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.confirmation_dialog.take() else {
            return;
        };
        if let Some(return_focus) = dialog.return_focus {
            return_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn show_mutation_error_for(
        &mut self,
        session_id: SessionId,
        draft_mode: DraftMode,
        detail: String,
        return_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, status) = match draft_mode {
            DraftMode::Insert => ("Couldn’t insert row", "Row insert failed"),
            DraftMode::Update => ("Couldn’t update row", "Row update failed"),
        };
        if let Some(session) = self.session_mut(session_id) {
            session.busy = false;
            session.error = Some(detail.clone());
            session.status = status.into();
        }
        let focus = cx.focus_handle();
        self.mutation_error_dialog = Some(MutationErrorDialog {
            session_id,
            title: title.into(),
            detail,
            focus: focus.clone(),
            return_focus,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn dismiss_mutation_error_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(dialog) = self.mutation_error_dialog.take() else {
            return false;
        };
        if let Some(session) = self.session_mut(dialog.session_id) {
            session.error = None;
            session.status = "Row changes are still open".into();
        }
        if let Some(return_focus) = dialog.return_focus {
            return_focus.focus(window, cx);
        }
        true
    }

    fn dismiss_mutation_error(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismiss_mutation_error_dialog(window, cx) {
            cx.notify();
        }
    }

    fn confirm_pending_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.confirmation_dialog.take() else {
            return;
        };
        let return_focus = dialog.return_focus.clone();
        let closes_query = matches!(dialog.action, ConfirmationAction::CloseQuery { .. });
        match dialog.action {
            ConfirmationAction::RunQuery {
                session_id,
                run_all,
            } => {
                self.run_query_for_execution(session_id, run_all, cx);
            }
            ConfirmationAction::CloseQuery { session_id, tab_id } => {
                self.close_secondary_tab_for(session_id, tab_id, cx);
                self.focus_active_query_editor_for(session_id, window, cx);
            }
            ConfirmationAction::ClearQueryHistory { session_id } => {
                self.clear_query_history_for(session_id, cx)
            }
            ConfirmationAction::Table {
                action,
                session_id,
                table,
            } => self.execute_table_action(action, session_id, table, cx),
            ConfirmationAction::DeleteRow {
                session_id,
                table,
                filters,
            } => self.delete_row_for(session_id, table, filters, cx),
            ConfirmationAction::DatabaseImport { session_id, path } => {
                self.execute_database_import(session_id, path, cx)
            }
            ConfirmationAction::TableImport {
                session_id,
                table,
                path,
            } => self.execute_table_import(session_id, table, path, cx),
        }
        if !closes_query && let Some(return_focus) = return_focus {
            return_focus.focus(window, cx);
        }
    }

    fn execute_table_action(
        &mut self,
        action: TableAction,
        session_id: SessionId,
        table: TableInfo,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, busy, kind)) = self
            .session(session_id)
            .map(|session| (session.engine.clone(), session.busy, session.kind))
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if busy || !kind.is_sql() || table.kind != EntityKind::Table {
            return;
        }
        let target_table = table_ref(&table);
        let action_target = target_table.clone();
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = match action {
            TableAction::Truncate => format!("Truncating {}…", table.name),
            TableAction::Drop => format!("Deleting table {}…", table.name),
        };
        session.request_generation += 1;
        let generation = session.request_generation;
        let task = runtime.spawn(async move {
            let outcome = match action {
                TableAction::Truncate => engine.truncate_table(&action_target).await,
                TableAction::Drop => engine.drop_table(&action_target).await,
            }?;
            let tables = engine.list_tables().await?;
            Ok::<_, dbx_core::DbxError>((outcome, tables))
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if session.request_generation != generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok((outcome, tables)) => {
                        session.tables = tables;
                        session.error = None;
                        match action {
                            TableAction::Truncate => {
                                if let Some(result) = session.result.as_mut() {
                                    let result = Arc::make_mut(result);
                                    result.rows.clear();
                                    result.rows_affected = Some(outcome.rows_affected);
                                    result.elapsed_ms = outcome.elapsed_ms;
                                    session.sync_result_grid(true, cx);
                                } else {
                                    session.set_result(
                                        Some(QueryResult::empty(
                                            Some(outcome.rows_affected),
                                            outcome.elapsed_ms,
                                        )),
                                        cx,
                                    );
                                }
                                session.result_table = Some(target_table.clone());
                                session.selected_row = None;
                                session.row_draft = None;
                                session.status = format!(
                                    "Truncated {} · {} row(s) changed",
                                    table.name, outcome.rows_affected
                                );
                            }
                            TableAction::Drop => {
                                if session.selected_table.as_ref() == Some(&target_table) {
                                    session.selected_table = None;
                                    session.table_columns.clear();
                                    session
                                        .completion_columns
                                        .remove(&completion_table_key(&target_table));
                                    session.set_result(None, cx);
                                    session.result_table = None;
                                    session.selected_row = None;
                                    session.row_draft = None;
                                }
                                session.status = format!("Deleted table {}", table.name);
                            }
                        }
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Table action failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn identity_filters_for(
        &self,
        session_id: SessionId,
        row: &RowData,
    ) -> Result<Vec<Filter>, String> {
        let session = self
            .session(session_id)
            .ok_or("No connection session is loaded")?;
        let result = session.result.as_ref().ok_or("No row result is loaded")?;
        let primary_keys: Vec<_> = session
            .table_columns
            .iter()
            .filter(|column| column.primary_key)
            .collect();
        if primary_keys.is_empty() {
            return Err("Editing and deletion require a primary key for safe row identity.".into());
        }
        primary_keys
            .into_iter()
            .map(|primary_key| {
                let index = result
                    .columns
                    .iter()
                    .position(|column| column.name == primary_key.name)
                    .ok_or_else(|| {
                        format!("Primary key {} is not in the result", primary_key.name)
                    })?;
                Ok(Filter::new(
                    primary_key.name.clone(),
                    FilterOperator::Equals,
                    row.values.get(index).cloned(),
                ))
            })
            .collect()
    }

    fn create_table_template_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.session(session_id).map(|session| session.kind) else {
            return;
        };
        let sql = match kind {
            DatabaseKind::PostgreSQL => {
                "CREATE TABLE public.new_table (\n    id BIGSERIAL PRIMARY KEY,\n    name TEXT NOT NULL\n);"
            }
            DatabaseKind::MySQL => {
                "CREATE TABLE new_table (\n    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,\n    name VARCHAR(255) NOT NULL\n);"
            }
            DatabaseKind::SQLite => {
                "CREATE TABLE new_table (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL\n);"
            }
            DatabaseKind::Redis => "SET new_key value",
        };
        let is_query_active = self.session(session_id).is_some_and(|session| {
            session.active_secondary_tab.is_some_and(|tab_id| {
                session
                    .secondary_tabs
                    .iter()
                    .any(|tab| tab.id == tab_id && matches!(&tab.kind, SecondaryTabKind::Query(_)))
            })
        });
        if !is_query_active {
            self.add_query_tab_for(session_id, window, cx);
        }
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab_id) = session.active_secondary_tab
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            && let SecondaryTabKind::Query(query_tab) = &mut tab.kind
        {
            query_tab.query_text.update(cx, |query, cx| {
                *query = sql.into();
                cx.notify();
            });
            session.pane = Pane::Query;
        }
        cx.notify();
    }

    fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.status = "Operation failed".into();
    }
}

enum Mutation {
    Insert(InsertRequest),
    Update(UpdateRequest),
}

fn table_ref(table: &TableInfo) -> TableRef {
    match &table.schema {
        Some(schema) => TableRef::in_schema(schema, &table.name),
        None => TableRef::new(&table.name),
    }
}

fn table_ref_label(table: &TableRef) -> String {
    match &table.schema {
        Some(schema) => format!("{schema}.{}", table.name),
        None => table.name.clone(),
    }
}

/// A filesystem-friendly base name for exported files, for example
/// `public_orders` or `events`.
fn export_file_stem(table: &TableInfo) -> String {
    let raw = match &table.schema {
        Some(schema) => format!("{schema}_{}", table.name),
        None => table.name.clone(),
    };
    let sanitized: String = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "table".to_owned()
    } else {
        sanitized
    }
}

fn table_selection_key(table: &TableInfo) -> String {
    completion_table_key(&table_ref(table))
}

fn transfer_name_stem(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "database".into()
    } else {
        sanitized
    }
}

/// Semantic color per deployment environment: production is risky (red),
/// staging warns, develop stays neutral-accent, local is healthy.
fn environment_color(environment: ConnectionEnvironment) -> Rgba {
    match environment {
        ConnectionEnvironment::Production => theme().danger,
        ConnectionEnvironment::Staging => theme().warning,
        ConnectionEnvironment::Develop => theme().accent,
        ConnectionEnvironment::Local => theme().success,
    }
}

fn environment_badge(environment: ConnectionEnvironment) -> Div {
    div()
        .px(px(6.))
        .py(px(2.))
        .rounded_full()
        .border_1()
        .border_color(environment_color(environment))
        .text_size(px(9.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(environment_color(environment))
        .child(environment.to_string())
}

fn display_url(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return "<redacted>".into();
    };
    if parsed.password().is_some() {
        let _ = parsed.set_password(None);
    }
    parsed.to_string()
}

fn default_schema_filter(kind: DatabaseKind, tables: &[TableInfo]) -> Option<String> {
    (kind == DatabaseKind::PostgreSQL
        && tables
            .iter()
            .any(|table| table.schema.as_deref() == Some("public")))
    .then(|| "public".to_owned())
}

fn schema_filter_options(kind: DatabaseKind, tables: &[TableInfo]) -> Vec<Option<String>> {
    if kind != DatabaseKind::PostgreSQL {
        return Vec::new();
    }

    let mut schemas: Vec<_> = tables
        .iter()
        .filter_map(|table| table.schema.clone())
        .collect();
    schemas.sort_unstable();
    schemas.dedup();

    let mut options = Vec::with_capacity(schemas.len() + 1);
    options.push(None);
    options.extend(schemas.into_iter().map(Some));
    options
}

fn diagram_schema_names(kind: DatabaseKind, tables: &[TableInfo]) -> Vec<String> {
    if kind != DatabaseKind::PostgreSQL {
        return Vec::new();
    }

    let mut schemas = tables
        .iter()
        .filter_map(|table| table.schema.clone())
        .collect::<Vec<_>>();
    schemas.sort_unstable();
    schemas.dedup();
    schemas
}

fn relational_schema_names(schema: &RelationalSchema) -> Vec<String> {
    let mut schemas = schema
        .tables
        .iter()
        .filter_map(|table| table.table.schema.clone())
        .collect::<Vec<_>>();
    schemas.sort_unstable();
    schemas.dedup();
    schemas
}

fn diagram_initial_schema_selection(
    kind: DatabaseKind,
    explorer_schema: Option<&str>,
) -> Option<BTreeSet<String>> {
    (kind == DatabaseKind::PostgreSQL)
        .then(|| explorer_schema.map(|schema| BTreeSet::from([schema.to_owned()])))
        .flatten()
}

fn normalize_diagram_schema_selection(
    selection: &mut Option<BTreeSet<String>>,
    available_schemas: &[String],
) {
    let available = available_schemas.iter().cloned().collect::<BTreeSet<_>>();
    let selects_every_schema = selection.as_mut().is_some_and(|selected| {
        selected.retain(|schema| available.contains(schema));
        *selected == available
    });
    if selects_every_schema {
        *selection = None;
    }
}

fn rebuild_diagram_document(diagram: &mut DiagramTab) {
    let Some(source_schema) = diagram.source_schema.as_ref() else {
        return;
    };
    let document = Arc::new(diagram_document_for_selection(
        source_schema,
        diagram.selected_schemas.as_ref(),
    ));
    if diagram
        .selected_node
        .as_deref()
        .is_some_and(|selected| document.nodes.iter().all(|node| node.id != selected))
    {
        diagram.selected_node = None;
    }
    diagram.document = Some(document);
    diagram.scroll_handle.set_offset(point(px(0.), px(0.)));
    diagram.drag_anchor = None;
}

fn diagram_document_for_selection(
    source_schema: &RelationalSchema,
    selected_schemas: Option<&BTreeSet<String>>,
) -> DiagramDocument {
    selected_schemas.map_or_else(
        || DiagramDocument::from_schema(source_schema),
        |selected| DiagramDocument::from_schema_selection(source_schema, Some(selected)),
    )
}

fn schema_filtered_tables(
    kind: DatabaseKind,
    tables: &[TableInfo],
    schema_filter: Option<&str>,
) -> Vec<TableInfo> {
    tables
        .iter()
        .filter(|table| {
            kind != DatabaseKind::PostgreSQL
                || schema_filter.is_none()
                || table.schema.as_deref() == schema_filter
        })
        .cloned()
        .collect()
}

fn table_is_visible(kind: DatabaseKind, schema_filter: Option<&str>, table: &TableRef) -> bool {
    kind != DatabaseKind::PostgreSQL
        || schema_filter.is_none()
        || table.schema.as_deref() == schema_filter
}

fn schema_filter_id(schema: Option<&str>) -> String {
    format!(
        "schema-filter-{}",
        schema.unwrap_or("all").replace([' ', '/'], "-")
    )
}

fn can_mutate_result(
    kind: DatabaseKind,
    busy: bool,
    selected_table: Option<&TableRef>,
    result_table: Option<&TableRef>,
) -> bool {
    !busy
        && kind.is_sql()
        && matches!((selected_table, result_table), (Some(selected), Some(result)) if selected == result)
}

fn selected_filter_column<'a>(
    selected_column: usize,
    table_columns: &'a [ColumnInfo],
    result: Option<&'a QueryResult>,
) -> Option<&'a ColumnInfo> {
    result
        .and_then(|result| result.columns.get(selected_column))
        .or_else(|| table_columns.get(selected_column))
        .or_else(|| result.and_then(|result| result.columns.first()))
        .or_else(|| table_columns.first())
}

fn foreign_key_actions(foreign_key: &ForeignKeyInfo) -> String {
    let mut actions = Vec::with_capacity(2);
    if let Some(action) = foreign_key.on_update {
        actions.push(format!("ON UPDATE {}", referential_action_label(action)));
    }
    if let Some(action) = foreign_key.on_delete {
        actions.push(format!("ON DELETE {}", referential_action_label(action)));
    }
    actions.join(" · ")
}

fn referential_action_label(action: ReferentialAction) -> &'static str {
    match action {
        ReferentialAction::NoAction => "NO ACTION",
        ReferentialAction::Restrict => "RESTRICT",
        ReferentialAction::Cascade => "CASCADE",
        ReferentialAction::SetNull => "SET NULL",
        ReferentialAction::SetDefault => "SET DEFAULT",
    }
}

fn table_sidebar_label(table: &TableInfo, active_schema_filter: Option<&str>) -> String {
    match &table.schema {
        Some(schema) if Some(schema.as_str()) != active_schema_filter => {
            format!("{schema}.{}", table.name)
        }
        _ => table.name.clone(),
    }
}

fn table_sidebar_id(table: &TableInfo) -> String {
    format!(
        "table-{}-{}",
        table.schema.as_deref().unwrap_or("<default>"),
        table.name
    )
}

fn table_click_action(event: &gpui::ClickEvent) -> TableClickAction {
    if event.is_right_click() {
        TableClickAction::OpenContextMenu
    } else {
        TableClickAction::Select
    }
}

fn redis_command_word(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn clamp_diagram_scroll_offset(offset: Point<Pixels>, max_offset: Point<Pixels>) -> Point<Pixels> {
    point(
        offset.x.clamp(-max_offset.x, px(0.)),
        offset.y.clamp(-max_offset.y, px(0.)),
    )
}

fn remap_diagram_scroll_axis(
    offset: Pixels,
    old_max_offset: Pixels,
    old_scene_size: Pixels,
    next_scene_size: Pixels,
) -> Pixels {
    let old_max = f32::from(old_max_offset).max(0.0);
    if old_max <= f32::EPSILON {
        return px(0.0);
    }

    let viewport_size = (f32::from(old_scene_size) - old_max).max(0.0);
    let next_max = (f32::from(next_scene_size) - viewport_size).max(0.0);
    let progress = (-f32::from(offset) / old_max).clamp(0.0, 1.0);
    px(-next_max * progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    struct VaultEnterHarness {
        editor: Entity<TextEditor>,
        submitted: bool,
    }

    impl Render for VaultEnterHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let focus = self.editor.read(cx).focus_handle();
            div()
                .key_context("VaultGate")
                .on_action(cx.listener(|this, _: &SubmitVault, _, cx| {
                    this.submitted = true;
                    cx.notify();
                }))
                .child(editor::input_with_key_context(
                    self.editor.clone(),
                    focus,
                    false,
                    "DbxTextEditor VaultGate",
                ))
        }
    }

    #[gpui::test]
    fn enter_in_focused_vault_password_field_submits(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(|cx| {
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([gpui::KeyBinding::new(
                "enter",
                SubmitVault,
                Some("VaultGate"),
            )]);
        });
        let (harness, cx) = cx.add_window_view(|window, cx| {
            let value = cx.new(|_| String::new());
            let editor = cx.new(|cx| TextEditor::new(value, false, window, cx).password());
            VaultEnterHarness {
                editor,
                submitted: false,
            }
        });
        cx.update(|window, cx| {
            harness
                .read(cx)
                .editor
                .read(cx)
                .focus_handle()
                .focus(window, cx);
        });

        cx.simulate_keystrokes("enter");

        assert!(cx.update(|_, cx| harness.read(cx).submitted));
    }

    #[gpui::test]
    fn compact_picker_single_click_selects_without_replacing_the_list(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().expect("create profile directory");
        let store = ProfileStore::at(directory.path().join("connections.json"));
        let profile = store
            .save(ConnectionProfileDraft::new(
                "Local database",
                DatabaseKind::PostgreSQL,
                "postgres://developer@localhost:5432/app",
            ))
            .expect("save profile fixture");
        let (app, cx) = cx.add_window_view(DbxApp::new);

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.compact_layout = true;
                app.compact_connection_form_open = false;
                app.select_saved_connection_in_compact_picker(profile.clone(), cx);
                assert_eq!(app.draft.selected_profile, Some(profile.id));
                assert!(!app.compact_connection_form_open);
            });
        });
    }

    #[gpui::test]
    fn double_click_queues_open_while_saved_password_is_hydrating(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().expect("create profile directory");
        let store = ProfileStore::at(directory.path().join("connections.json"));
        store
            .vault()
            .expect("test store has a vault")
            .create("test vault passphrase")
            .expect("create test vault");
        let profile = store
            .save(
                ConnectionProfileDraft::new(
                    "Local database",
                    DatabaseKind::PostgreSQL,
                    "postgres://developer@localhost:5432/app",
                )
                .with_secret("secret"),
            )
            .expect("save profile fixture");
        let (app, cx) = cx.add_window_view(DbxApp::new);

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.saved_connections = vec![profile.clone()];
                app.draft.selected_profile = Some(profile.id);
                app.credential_hydrating = true;
                app.open_saved_connection(profile.clone(), window, cx);
                assert!(app.credential_connect_window.is_some());
                assert!(app.sessions.is_empty());
            });
        });
    }

    #[test]
    fn redis_query_tabs_must_use_a_syntax_aware_editor_language() {
        assert_eq!(
            query_editor_language(DatabaseKind::Redis),
            editor::EditorLanguage::Redis
        );
    }

    #[gpui::test]
    fn focused_redis_query_renders_runtime_catalog_completion_menu(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            let tab_id = Uuid::new_v4();
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Redis test".into(),
                DatabaseKind::Redis,
                ConnectionEnvironment::Local,
                window,
                cx,
            );
            let query = QueryTab::new(DatabaseKind::Redis, session_id, tab_id, window, cx);
            query
                .query_editor
                .update(cx, |editor, cx| editor.set_text("", cx));
            session.redis_command_catalog = Some(Arc::new(RedisCommandCatalog {
                commands: vec![dbx_core::RedisCommand {
                    name: "JSON.GET".into(),
                    summary: Some("Get a value from a JSON document".into()),
                    group: Some("json".into()),
                    since: Some("1.0.0".into()),
                    arguments: Vec::new(),
                }],
            }));
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Query(Box::new(query)),
            });
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app.connection_picker_open = false;
            app.activate_secondary_tab_for(session_id, tab_id, window, cx);
            app
        });

        cx.simulate_input("JSON.G");
        cx.update(|window, cx| {
            let focus = app
                .read(cx)
                .active_query_editor_for(session_id)
                .expect("Redis query editor should be active")
                .read(cx)
                .focus_handle();
            assert!(focus.is_focused(window), "Redis query editor lost focus");
            let menu = app.update(cx, |app, cx| app.query_completion_for(session_id, cx));
            assert!(
                menu.as_ref()
                    .is_some_and(|menu| menu.items.iter().any(|item| item.label == "JSON.GET")),
                "focused Redis query should resolve a module command before painting"
            );
            window.draw(cx).clear(cx)
        });

        assert!(
            cx.debug_bounds("sql-completion-menu").is_some(),
            "a focused Redis query with a partial module command must paint its completion popup"
        );

        cx.simulate_keystrokes("tab");
        let query_text = cx.update(|_, cx| {
            app.read(cx)
                .session(session_id)
                .and_then(|session| {
                    let tab_id = session.active_secondary_tab?;
                    session.secondary_tabs.iter().find(|tab| tab.id == tab_id)
                })
                .and_then(|tab| match &tab.kind {
                    SecondaryTabKind::Query(query) => Some(query.query_text.read(cx).clone()),
                    SecondaryTabKind::Structure(_) | SecondaryTabKind::Diagram(_) => None,
                })
        });
        assert_eq!(query_text.as_deref(), Some("JSON.GET "));
    }

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn tab_abort_guard_cancels_inflight_work_when_its_owner_drops() {
        let runtime = tokio::runtime::Runtime::new().expect("create test runtime");
        let dropped = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let probe = DropProbe(dropped.clone());
        let task = runtime.spawn(async move {
            let _probe = probe;
            started_tx.send(()).expect("signal task start");
            std::future::pending::<()>().await;
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("task should start");

        let mut guard = AbortOnDrop::default();
        guard.replace(task.abort_handle());
        drop(guard);

        let error = runtime
            .block_on(task)
            .expect_err("dropping the tab owner should cancel its task");
        assert!(error.is_cancelled());
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn connection_task_set_cancels_inflight_work_when_its_owner_drops() {
        let runtime = tokio::runtime::Runtime::new().expect("create test runtime");
        let dropped = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let probe = DropProbe(dropped.clone());
        let task = runtime.spawn(async move {
            let _probe = probe;
            started_tx.send(()).expect("signal task start");
            std::future::pending::<()>().await;
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("task should start");

        let mut tasks = BackgroundTaskSet::default();
        tasks.track(&task);
        drop(tasks);

        let error = runtime
            .block_on(task)
            .expect_err("dropping the connection owner should cancel its tasks");
        assert!(error.is_cancelled());
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn diagram_scroll_offset_clamps_to_the_canvas_extent() {
        let max = point(px(240.), px(120.));
        assert_eq!(
            clamp_diagram_scroll_offset(point(px(-400.), px(30.)), max),
            point(px(-240.), px(0.))
        );
    }

    #[test]
    fn diagram_keyboard_pan_moves_the_viewport_in_the_requested_direction() {
        let offset = point(px(-80.), px(-40.));
        let right_and_down = point(offset.x - px(48.), offset.y - px(48.));
        assert_eq!(right_and_down, point(px(-128.), px(-88.)));
    }

    #[test]
    fn diagram_zoom_remaps_scroll_progress_to_the_new_canvas_extent() {
        assert_eq!(
            remap_diagram_scroll_axis(px(-600.), px(600.), px(1_000.), px(700.)),
            px(-300.)
        );
        assert_eq!(
            remap_diagram_scroll_axis(px(-300.), px(600.), px(1_000.), px(700.)),
            px(-150.)
        );
        assert_eq!(
            remap_diagram_scroll_axis(px(0.), px(0.), px(300.), px(700.)),
            px(0.)
        );
    }

    #[test]
    fn diagram_schema_selection_starts_from_the_postgres_explorer_filter() {
        assert_eq!(
            diagram_initial_schema_selection(DatabaseKind::PostgreSQL, Some("analytics")),
            Some(BTreeSet::from(["analytics".to_owned()]))
        );
        assert_eq!(
            diagram_initial_schema_selection(DatabaseKind::PostgreSQL, None),
            None
        );
        assert_eq!(
            diagram_initial_schema_selection(DatabaseKind::MySQL, Some("ignored")),
            None
        );
    }

    #[test]
    fn diagram_schema_selection_drops_missing_names_and_normalizes_all() {
        let available = vec!["analytics".to_owned(), "public".to_owned()];
        let mut every_schema = Some(BTreeSet::from([
            "analytics".to_owned(),
            "public".to_owned(),
        ]));
        normalize_diagram_schema_selection(&mut every_schema, &available);
        assert_eq!(every_schema, None);

        let mut one_schema = Some(BTreeSet::from(["missing".to_owned(), "public".to_owned()]));
        normalize_diagram_schema_selection(&mut one_schema, &available);
        assert_eq!(one_schema, Some(BTreeSet::from(["public".to_owned()])));
    }

    #[test]
    fn table_browser_pages_are_bounded_and_offset_by_page() {
        let page = table_browse_page(3);

        assert_eq!(page.limit, TABLE_BROWSE_QUERY_LIMIT);
        assert_eq!(page.offset, 3 * u64::from(TABLE_BROWSE_PAGE_SIZE));
    }

    #[test]
    fn table_browser_keeps_the_probe_row_out_of_the_grid() {
        let mut result = QueryResult::empty(None, 0);
        result.rows = (0..TABLE_BROWSE_QUERY_LIMIT)
            .map(|_| RowData::default())
            .collect();

        assert!(trim_table_browse_result(&mut result));
        assert_eq!(result.rows.len(), TABLE_BROWSE_PAGE_SIZE as usize);
    }

    #[test]
    fn query_status_distinguishes_rows_writes_limits_and_database() {
        let returned = QueryResult {
            columns: Vec::new(),
            rows: vec![RowData::default(), RowData::default()],
            rows_affected: None,
            truncated: true,
            elapsed_ms: 18,
        };
        assert_eq!(
            query_result_status(&returned),
            "2 rows returned · 18 ms · results limited"
        );

        let written = QueryResult::empty(Some(1), 4);
        assert_eq!(query_result_status(&written), "1 row affected · 4 ms");
    }

    #[test]
    fn redis_filter_stays_one_command_argument() {
        assert_eq!(redis_command_word("user:* archive"), "\"user:* archive\"");
        assert_eq!(redis_command_word("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn display_url_redacts_embedded_password() {
        let displayed = display_url("postgres://user:secret@example.test:5432/app");
        assert!(!displayed.contains("secret"));
        assert!(displayed.contains("user@example.test"));
    }

    #[test]
    fn display_url_handles_invalid_input_without_leaking_it() {
        assert_eq!(display_url("not a URL"), "<redacted>");
    }

    #[test]
    fn ad_hoc_results_cannot_mutate_selected_table() {
        let table = TableRef::in_schema("public", "users");

        assert!(can_mutate_result(
            DatabaseKind::PostgreSQL,
            false,
            Some(&table),
            Some(&table),
        ));
        assert!(!can_mutate_result(
            DatabaseKind::PostgreSQL,
            false,
            Some(&table),
            None,
        ));
        assert!(!can_mutate_result(
            DatabaseKind::PostgreSQL,
            true,
            Some(&table),
            Some(&table),
        ));
        assert!(!can_mutate_result(
            DatabaseKind::Redis,
            false,
            Some(&table),
            Some(&table),
        ));
    }

    #[test]
    fn filter_uses_the_selected_grid_column() {
        let table_columns = vec![
            ColumnInfo::result("id", 0, "INTEGER"),
            ColumnInfo::result("name", 1, "TEXT"),
        ];
        let result = QueryResult {
            columns: table_columns.clone(),
            rows: Vec::new(),
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };

        assert_eq!(
            selected_filter_column(1, &table_columns, Some(&result))
                .map(|column| column.name.as_str()),
            Some("name")
        );
    }

    #[test]
    fn sidebar_identity_includes_schema() {
        let table = TableInfo::table("users", Some("analytics".into()));
        assert_eq!(table_sidebar_label(&table, None), "analytics.users");
        assert_eq!(table_sidebar_label(&table, Some("analytics")), "users");
        assert_eq!(
            table_sidebar_label(&table, Some("public")),
            "analytics.users"
        );
        assert_eq!(table_sidebar_id(&table), "table-analytics-users");
    }

    #[test]
    fn right_click_routes_to_the_table_context_menu_action() {
        let event = gpui::ClickEvent::Mouse(gpui::MouseClickEvent {
            down: gpui::MouseDownEvent {
                button: gpui::MouseButton::Right,
                ..Default::default()
            },
            up: gpui::MouseUpEvent {
                button: gpui::MouseButton::Right,
                ..Default::default()
            },
        });

        assert_eq!(
            table_click_action(&event),
            TableClickAction::OpenContextMenu
        );
    }

    #[test]
    fn foreign_key_actions_are_presented_in_database_order() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("orders_customer_id_fkey".into()),
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: Some(ReferentialAction::Cascade),
            on_delete: Some(ReferentialAction::SetNull),
        };

        assert_eq!(
            foreign_key_actions(&foreign_key),
            "ON UPDATE CASCADE · ON DELETE SET NULL"
        );
    }

    #[test]
    fn postgres_schema_filter_defaults_to_public_and_lists_unique_options() {
        let tables = vec![
            TableInfo::table("events", Some("analytics".into())),
            TableInfo::table("users", Some("public".into())),
            TableInfo::table("accounts", Some("public".into())),
        ];

        assert_eq!(
            default_schema_filter(DatabaseKind::PostgreSQL, &tables),
            Some("public".into())
        );
        assert_eq!(
            schema_filter_options(DatabaseKind::PostgreSQL, &tables),
            vec![None, Some("analytics".into()), Some("public".into())]
        );
    }

    #[test]
    fn schema_filter_is_postgres_only_and_does_not_requery() {
        let tables = vec![
            TableInfo::table("events", Some("analytics".into())),
            TableInfo::table("users", Some("public".into())),
        ];

        assert_eq!(
            schema_filtered_tables(DatabaseKind::PostgreSQL, &tables, Some("public"))
                .iter()
                .map(|table| table.name.as_str())
                .collect::<Vec<_>>(),
            vec!["users"]
        );
        assert_eq!(
            schema_filtered_tables(DatabaseKind::MySQL, &tables, Some("public")).len(),
            tables.len()
        );
        assert!(table_is_visible(
            DatabaseKind::PostgreSQL,
            Some("public"),
            &TableRef::in_schema("public", "users")
        ));
        assert!(!table_is_visible(
            DatabaseKind::PostgreSQL,
            Some("public"),
            &TableRef::in_schema("analytics", "events")
        ));
    }
}
