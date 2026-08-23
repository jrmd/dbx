//! THESIS: DBX is a dense native database cockpit; it rejects the centered-card utility shell.
//! OWN-WORLD: Near-black layered panes, hairline borders, blue navigation, green health, 6px controls.
//! STORY: Pick or open a connection, keep its tab, browse context, then inspect or query without losing place.
//! FIRST VIEWPORT: A 46px rail, 40px primary connection tabs, explorer, data canvas, and row inspector.
//! FORM: Reference-led operator console, user-supplied DBX screen map; seed key: dbx-native-console.
//! FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md.
//! IMPLEMENTATION: `DbxApp` coordinates shared session state; focused workflows and rendering live in
//! the private `app/` module tree documented in `docs/architecture.md`.

mod connection;
mod result_table;
mod sql_completion;
mod transfer;
mod view;

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

use dbx_core::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseEngine, DatabaseExportRequest, DatabaseKind,
    DumpFormat, EntityKind, Filter, FilterOperator, ForeignKeyInfo, InsertRequest, Page,
    QueryOptions, QueryResult, ReferentialAction, RowData, TableInfo, TableRef, UpdateRequest,
    detect_file_format, export_database, export_table, import_database, import_file,
};
use gpui::{
    AnyElement, App, Context, Div, Entity, FontWeight, Image, ImageFormat, IntoElement,
    KeyDownEvent, MouseButton, PathPromptOptions, Pixels, Point, PromptButton, PromptLevel, Render,
    Rgba, SharedString, Stateful, StatefulInteractiveElement as _, Subscription, Window,
    WindowControlArea, anchored, deferred, div, img, prelude::*, px,
};
use gpui_component::{
    Disableable as _, InteractiveElementExt as _, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    select::{SearchableVec, Select, SelectEvent},
    table::{DataTable, TableEvent, TableState},
};
use uuid::Uuid;

use crate::{
    assets::LOGO_BYTES,
    connection_fields::ConnectionFields,
    editor::{self, TextEditor},
    filters::{FilterModel, FilterRowId, filter_operator_options, operator_requires_value},
    profiles::{
        ConnectionEnvironment, ConnectionProfileDraft, ProfileStore, SavedConnection, sqlite_url,
    },
    row_drafts::{FieldId, FieldRow, FieldValueState, RowDraftModel},
    theme::{
        ButtonKind, Icon, THEME, badge, button, connection_tab, database_logo, icon, panel_header,
    },
};
use result_table::{ResultTableDelegate, foreign_key_target_table};
use sql_completion::{
    CompletionItemKind, SqlCompletionItem, SqlCompletionRequest, completion_table_key,
    sql_completion_items,
};

gpui::actions!(
    dbx_ui,
    [
        RunQuery,
        FormatQuery,
        RefreshData,
        CompletionUp,
        CompletionDown,
        CompletionEnter
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pane {
    Data,
    Structure,
    Query,
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
        .border_color(THEME.border_strong)
        .bg(THEME.panel)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(THEME.danger).border_color(THEME.danger))
        .child(icon(Icon::Close, THEME.text).size(px(12.)))
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
    context: editor::SqlCompletionContext,
    items: Vec<SqlCompletionItem>,
    selected: usize,
    signature: String,
}

impl CompletionItemKind {
    fn color(self) -> Rgba {
        match self {
            Self::Keyword => gpui::rgb(0xc792ea),
            Self::Type => gpui::rgb(0x89ddff),
            Self::Table => THEME.accent,
            Self::Column => THEME.success,
            Self::Function => gpui::rgb(0xf78c6c),
        }
    }
}

struct QueryTab {
    query_text: Entity<String>,
    query_editor: Entity<TextEditor>,
    result: Option<Arc<QueryResult>>,
    result_grid: Entity<TableState<ResultTableDelegate>>,
    result_column_widths: HashMap<String, Pixels>,
    busy: bool,
    status: String,
    error: Option<String>,
    /// The query text a failed run was executed against plus the byte range
    /// its error message points at. Highlighting only paints while the editor
    /// still holds that exact text, so any edit clears it.
    error_highlight: Option<(String, Range<usize>)>,
    request_generation: u64,
    completion_signature: Option<String>,
    completion_dismissed_signature: Option<String>,
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
        let query_editor = cx.new(|cx| TextEditor::new_sql(query_text.clone(), window, cx));
        let result_grid = cx.new(|cx| {
            TableState::new(ResultTableDelegate::default(), window, cx)
                .col_resizable(true)
                .col_movable(false)
                .sortable(false)
                .row_selectable(false)
                .col_selectable(false)
                .cell_selectable(false)
        });
        let text_subscription = cx.observe(&query_text, |_, _, cx| cx.notify());
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
            result_column_widths: HashMap::new(),
            busy: false,
            status: "Ready to query".into(),
            error: None,
            error_highlight: None,
            request_generation: 0,
            completion_signature: None,
            completion_dismissed_signature: None,
            completion_index: 0,
            _subscriptions: vec![text_subscription, editor_subscription, table_subscription],
        }
    }

    fn set_result(&mut self, result: Option<QueryResult>, cx: &mut Context<DbxApp>) {
        self.result = result.map(Arc::new);
        let result = self.result.clone();
        let remembered_widths = self.result_column_widths.clone();
        self.result_grid.update(cx, move |table, cx| {
            table
                .delegate_mut()
                .set_result(result, &remembered_widths, &[], &[]);
            table.refresh(cx);
        });
    }
}

struct StructureTab {
    table: TableRef,
    columns: Vec<ColumnInfo>,
    foreign_keys: Vec<ForeignKeyInfo>,
    busy: bool,
    error: Option<String>,
}

enum SecondaryTabKind {
    Query(Box<QueryTab>),
    Structure(StructureTab),
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
    tables: Vec<TableInfo>,
    /// Schema metadata already fetched for completion. The navigator always
    /// supplies table names; columns are added as tables are opened or their
    /// structure is inspected, avoiding a metadata query for every keystroke.
    completion_columns: HashMap<String, Vec<ColumnInfo>>,
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
    busy: bool,
    status: String,
    error: Option<String>,
    request_generation: u64,
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
            tables: Vec::new(),
            completion_columns: HashMap::new(),
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
            busy: false,
            status: "Connecting…".into(),
            error: None,
            request_generation: 0,
        }
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

pub struct DbxApp {
    runtime: Arc<tokio::runtime::Runtime>,
    logo: Arc<Image>,
    draft: ConnectionDraft,
    profile_store: Option<ProfileStore>,
    saved_connections: Vec<SavedConnection>,
    sessions: Vec<ConnectionSession>,
    active_session_id: Option<SessionId>,
    connection_picker_open: bool,
    table_context_menu: Option<TableContextMenu>,
    database_export_dialog: Option<DatabaseExportDialog>,
    compact_layout: bool,
    narrow_workspace: bool,
    test_generation: u64,
    testing_connection: bool,
    _subscriptions: Vec<Subscription>,
    status: String,
    error: Option<String>,
}

impl DbxApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = ConnectionDraft::new(window, cx);

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

        Self {
            runtime: Arc::new(tokio::runtime::Runtime::new().expect("create DBX Tokio runtime")),
            logo: Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_BYTES.to_vec())),
            draft,
            profile_store,
            saved_connections,
            sessions: Vec::new(),
            active_session_id: None,
            connection_picker_open: false,
            table_context_menu: None,
            database_export_dialog: None,
            compact_layout: false,
            narrow_workspace: false,
            test_generation: 0,
            testing_connection: false,
            _subscriptions: subscriptions,
            status: "Choose an engine and connect".into(),
            error: profile_error,
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
        self.sessions[index].request_generation += 1;
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
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { engine.table_structure(&table_ref).await })
                .await?;
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

    fn activate_secondary_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session.secondary_tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        session.active_secondary_tab = Some(tab_id);
        match &tab.kind {
            SecondaryTabKind::Query(_) => {
                session.pane = Pane::Query;
            }
            SecondaryTabKind::Structure(_) => session.pane = Pane::Structure,
        }
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
        session.secondary_tabs.remove(index);
        if session.active_secondary_tab == Some(tab_id) {
            session.active_secondary_tab = None;
            session.pane = Pane::Data;
        }
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
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
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
                })
                .await?;
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
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
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
                })
                .await?;
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
        let (tab_id, context, items, signature) = {
            let session = self.session(session_id)?;
            if !session.kind.is_sql() {
                return None;
            }
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query_tab) = &tab.kind else {
                return None;
            };
            let query_text = query_tab.query_text.read(cx).clone();
            let cursor = query_tab.query_editor.read(cx).cursor_offset();
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
            let signature = format!("{query_text}\u{0}{cursor}\u{0}{context:?}");
            (tab_id, context, items, signature)
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
        if query_tab.completion_dismissed_signature.as_deref() == Some(signature.as_str()) {
            return None;
        }
        if query_tab.completion_signature.as_deref() != Some(signature.as_str()) {
            query_tab.completion_signature = Some(signature.clone());
            query_tab.completion_index = 0;
        }
        query_tab.completion_dismissed_signature = None;
        let selected = query_tab
            .completion_index
            .min(items.len().saturating_sub(1));
        query_tab.completion_index = selected;

        Some(SqlCompletionMenu {
            context,
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
                self.accept_completion_for(session_id, tab_id, menu.context, item, window, cx);
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
                self.accept_completion_for(session_id, tab_id, menu.context, item, window, cx);
            }
        }
    }

    fn accept_completion_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        context: editor::SqlCompletionContext,
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
            })
        else {
            return;
        };
        let focus = query_editor.read(cx).focus_handle();
        query_editor.update(cx, |editor, cx| {
            editor.replace_range(context.replacement_range, item.insert_text, cx);
        });
        if let Some(session) = self.session_mut(session_id)
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
        focus.focus(window, cx);
        cx.notify();
    }

    fn run_query(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.active_session_id() else {
            return;
        };
        self.run_query_for(session_id, cx);
    }

    fn run_query_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some((engine, tab_id, query, full_query, busy)) =
            self.session(session_id).and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Query(query_tab) = &tab.kind else {
                    return None;
                };
                let full = query_tab.query_text.read(cx).clone();
                Some((
                    session.engine.clone(),
                    tab_id,
                    full.trim().to_owned(),
                    full,
                    query_tab.busy,
                ))
            })
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if query.is_empty() || busy {
            return;
        }
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
        query_tab.error = None;
        query_tab.status = "Running query…".into();
        query_tab.request_generation += 1;
        let generation = query_tab.request_generation;
        cx.notify();
        // The executed statement moves into the blocking task; the original
        // stays behind so failures can locate the offending token in it.
        let executed_query = query.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { engine.query(&executed_query, QueryOptions::default()).await })
                .await?;
            this.update(cx, |this, cx| {
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
                match result {
                    Ok(result) => {
                        query_tab.status = format!(
                            "{} row{} · {} ms · read-only",
                            result.rows.len(),
                            if result.rows.len() == 1 { "" } else { "s" },
                            result.elapsed_ms
                        );
                        query_tab.set_result(Some(result), cx);
                        query_tab.error = None;
                        query_tab.error_highlight = None;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        // Positions reported against the trimmed statement
                        // shift by the trimmed leading whitespace.
                        let lead = full_query.len() - full_query.trim_start().len();
                        query_tab.error_highlight =
                            editor::sql_error_range(&message, &query).map(|range| {
                                (full_query.clone(), range.start + lead..range.end + lead)
                            });
                        query_tab.error = Some(message);
                        query_tab.status = "Operation failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn run_query_action(&mut self, _: &RunQuery, _: &mut Window, cx: &mut Context<Self>) {
        self.run_query(cx);
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

        let runtime = self.runtime.clone();

        cx.spawn(async move |this, cx| {
            let tables = runtime
                .spawn(async move { engine.list_tables().await })
                .await??;

            this.update(cx, |this, cx| {
                if let Some(session) = this.session_mut(session_id) {
                    session.tables = tables;
                }

                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
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
        cx.spawn(async move |this, cx| {
            let metadata = runtime
                .spawn(async move {
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
                })
                .await?;

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
        session.busy = true;
        session.status = format!("Switching to {database}…");
        session.error = None;
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let target = database.clone();
            let result = runtime
                .spawn(async move {
                    engine.use_database(&target).await?;
                    let tables = engine.list_tables().await?;
                    Ok::<_, dbx_core::DbxError>(tables)
                })
                .await?;

            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                session.busy = false;
                match result {
                    Ok(tables) => {
                        session.tables = tables;
                        session.current_database = Some(database.clone());
                        session.selected_table = None;
                        session.table_columns.clear();
                        session.completion_columns.clear();
                        session.set_result(None, cx);
                        session.result_table = None;
                        session.schema_filter = None;
                        session.foreign_keys.clear();
                        session.row_draft = None;
                        session.row_draft_subscriptions.clear();
                        session.status = format!("Switched to {database}");
                        session.error = None;
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Database switch failed".into();
                    }
                }
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
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
        self.watch_enum_fields_for(session_id, &row_draft, window, cx);
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

    fn watch_enum_fields_for(
        &mut self,
        session_id: SessionId,
        row_draft: &RowDraftModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscriptions = row_draft
            .fields()
            .iter()
            .filter_map(|field| {
                let selector = field.enum_selector.clone()?;
                let field_id = field.id;
                Some(cx.subscribe_in(
                    &selector,
                    window,
                    move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                        let SelectEvent::Confirm(value) = event;
                        let value = value.as_ref().map(ToString::to_string);
                        this.set_row_enum_value_for(session_id, field_id, value, cx);
                    },
                ))
            })
            .collect();
        if let Some(session) = self.session_mut(session_id) {
            session.row_draft_subscriptions = subscriptions;
        }
    }

    fn set_row_enum_value_for(
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
        let TableEvent::ColumnWidthsChanged(widths) = event else {
            return;
        };
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

        query_tab.result_column_widths =
            ResultTableDelegate::widths_by_key(query_tab.result.as_deref(), widths);
        cx.notify();
    }

    fn select_row_for(
        &mut self,
        session_id: SessionId,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            session.row_draft_subscriptions.clear();
        }
        let editable = self.editable_table_for(session_id).is_some();
        let draft_data = self.session(session_id).and_then(|session| {
            let result = session.result.as_ref()?;
            let values = result.rows.get(row)?.values.clone();
            Some((
                session.table_columns.clone(),
                result.columns.clone(),
                values,
            ))
        });
        let row_draft = if editable {
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
            self.watch_enum_fields_for(session_id, &draft, window, cx);
            Some(draft)
        } else {
            None
        };
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.selected_row = Some(row);
        session.draft_mode = DraftMode::Update;
        session.inspector_open = true;
        session.row_draft = row_draft;
        session.error = None;
        session.status = if editable {
            "Editing selected row".into()
        } else {
            "Inspecting read-only query row".into()
        };
        cx.notify();
    }

    fn toggle_inspector_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(session_id) {
            session.inspector_open = !session.inspector_open;
            cx.notify();
        }
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
            session.row_draft = None;
            session.row_draft_subscriptions.clear();
            session.selected_row = None;
            session.clear_grid_selection(cx);
            session.draft_mode = DraftMode::Update;
            session.error = None;
            session.status = "Row edit cancelled".into();
            cx.notify();
        }
    }

    fn save_draft_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
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
        let (Some(engine), Some(table), Some(row_draft)) = (
            session.engine.clone(),
            self.editable_table_for(session_id).cloned(),
            session.row_draft.as_ref(),
        ) else {
            return;
        };
        let runtime = self.runtime.clone();
        let request = match session.draft_mode {
            DraftMode::Insert => row_draft
                .insert_values(cx)
                .map(|values| {
                    Some(Mutation::Insert(InsertRequest::from_row(
                        table.clone(),
                        values,
                    )))
                })
                .map_err(|error| error.to_string()),
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
                    .map_err(|error| error.to_string())
                    .and_then(|assignments| {
                        if assignments.is_empty() {
                            return Ok(None);
                        }
                        self.identity_filters_for(session_id, &row).map(|filters| {
                            Some(Mutation::Update(UpdateRequest::new(
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
            Err(error) => {
                if let Some(session) = self.session_mut(session_id) {
                    session.error = Some(error);
                    session.status = "Row fields need attention".into();
                }
                cx.notify();
                return;
            }
        };
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = "Applying row change…".into();
        session.request_generation += 1;
        let generation = session.request_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = runtime
                .spawn(async move {
                    match request {
                        Mutation::Insert(request) => engine.insert(&request).await,
                        Mutation::Update(request) => engine.update(&request).await,
                    }
                })
                .await?;
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
                                format!("Saved · {} row(s) changed", result.rows_affected);
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

    fn delete_selected_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
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
        let (Some(engine), Some(table), Some(row)) = (
            session.engine.clone(),
            self.editable_table_for(session_id).cloned(),
            session
                .selected_row
                .and_then(|index| session.result.as_ref()?.rows.get(index))
                .cloned(),
        ) else {
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
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = "Deleting row…".into();
        session.request_generation += 1;
        let generation = session.request_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = runtime
                .spawn(async move { engine.delete(&table, &filters).await })
                .await?;
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
        self.table_context_menu = None;
        let qualified_name = table_sidebar_label(&table, None);
        let (message, detail, confirmation) = match action {
            TableAction::Truncate => (
                format!("Truncate {qualified_name}?"),
                "Every row in this table will be deleted. The table structure remains.",
                "Truncate table",
            ),
            TableAction::Drop => (
                format!("Delete table {qualified_name}?"),
                "The table, its rows, indexes, and constraints will be permanently removed.",
                "Delete table",
            ),
        };
        let receiver = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(detail),
            &[
                PromptButton::cancel("Cancel"),
                PromptButton::ok(confirmation),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if matches!(receiver.await, Ok(1)) {
                this.update(cx, |this, cx| {
                    this.execute_table_action(action, session_id, table, cx)
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
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
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let outcome = match action {
                        TableAction::Truncate => engine.truncate_table(&action_target).await,
                        TableAction::Drop => engine.drop_table(&action_target).await,
                    }?;
                    let tables = engine.list_tables().await?;
                    Ok::<_, dbx_core::DbxError>((outcome, tables))
                })
                .await?;
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
        ConnectionEnvironment::Production => THEME.danger,
        ConnectionEnvironment::Staging => THEME.warning,
        ConnectionEnvironment::Develop => THEME.accent,
        ConnectionEnvironment::Local => THEME.success,
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

#[cfg(test)]
mod tests {
    use super::*;

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
