//! THESIS: DBX is a dense native database cockpit; it rejects the centered-card utility shell.
//! OWN-WORLD: Near-black layered panes, hairline borders, blue navigation, green health, 6px controls.
//! STORY: Pick or open a connection, keep its tab, browse context, then inspect or query without losing place.
//! FIRST VIEWPORT: A 46px rail, 40px primary connection tabs, explorer, data canvas, and row inspector.
//! FORM: Reference-led operator console, user-supplied DBX screen map; seed key: dbx-native-console.
//! FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use dbx_core::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseEngine, DatabaseKind, EntityKind, Filter,
    FilterOperator, ForeignKeyInfo, InsertRequest, Page, QueryOptions, QueryResult,
    ReferentialAction, RowData, TableInfo, TableRef, UpdateRequest,
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
    table::{Column as DataColumn, DataTable, TableDelegate, TableEvent, TableState},
};
use uuid::Uuid;

use crate::{
    assets::LOGO_BYTES,
    connection_fields::ConnectionFields,
    editor::{self, SqlCompletionTarget, TextEditor},
    filters::{
        FilterModel, FilterRowId, filter_operator_options, operator_label, operator_requires_value,
    },
    profiles::{
        ConnectionEnvironment, ConnectionProfileDraft, ProfileStore, SavedConnection, sqlite_url,
    },
    row_drafts::{FieldId, FieldRow, FieldValueState, RowDraftModel},
    theme::{
        ButtonKind, Icon, THEME, badge, button, connection_tab, database_logo, icon, panel_header,
    },
};

gpui::actions!(dbx_ui, [RunQuery, RefreshData]);

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
enum ConnectionFormMode {
    Details,
    ConnectionString,
}

type SessionId = Uuid;

const ROW_NUMBER_COLUMN_KEY: &str = "__dbx_row_number";
const AUTO_WIDTH_SAMPLE_ROWS: usize = 200;

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

/// Shared, virtualized backing model for both table browsing and ad-hoc query results.
///
/// `QueryResult` stays owned by the session/tab through an `Arc`, while DataTable only
/// asks this delegate to render cells that are currently visible.
struct ResultTableDelegate {
    result: Option<Arc<QueryResult>>,
    columns: Vec<DataColumn>,
    foreign_keys: Vec<ForeignKeyInfo>,
}

impl Default for ResultTableDelegate {
    fn default() -> Self {
        Self {
            result: None,
            columns: vec![Self::row_number_column()],
            foreign_keys: Vec::new(),
        }
    }
}

impl ResultTableDelegate {
    fn row_number_column() -> DataColumn {
        DataColumn::new(ROW_NUMBER_COLUMN_KEY, "#")
            .width(44.)
            .fixed_left()
            .resizable(false)
            .movable(false)
            .selectable(false)
            .min_width(44.)
            .max_width(44.)
            .p_0()
    }

    fn data_column_key(index: usize, column: &ColumnInfo) -> String {
        // Query results may legally contain duplicate column names, so the ordinal is
        // part of the key. Humanity has already made SQL aliases difficult enough.
        format!("column:{index}:{}", column.name)
    }

    fn auto_width(result: &QueryResult, column_index: usize, column: &ColumnInfo) -> Pixels {
        let header_chars = format!("{}  {}", column.name, column.data_type)
            .chars()
            .count();
        let value_chars = result
            .rows
            .iter()
            .take(AUTO_WIDTH_SAMPLE_ROWS)
            .filter_map(|row| row.values.get(column_index))
            .map(|value| value.to_string().chars().count())
            .max()
            .unwrap_or_default();

        // This is an initial width, not a prison sentence. The user can resize it.
        px(((header_chars.max(value_chars) as f32 * 7.0) + 20.0).clamp(80.0, 420.0))
    }

    fn set_result(
        &mut self,
        result: Option<Arc<QueryResult>>,
        remembered_widths: &HashMap<String, Pixels>,
        foreign_keys: &[ForeignKeyInfo],
        tables: &[TableInfo],
    ) {
        let mut columns = vec![Self::row_number_column()];

        if let Some(result) = result.as_deref() {
            columns.extend(result.columns.iter().enumerate().map(|(index, column)| {
                let key = Self::data_column_key(index, column);
                let width = remembered_widths
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| Self::auto_width(result, index, column));

                DataColumn::new(key, format!("{}  {}", column.name, column.data_type))
                    .width(width)
                    .resizable(true)
                    .movable(false)
                    .min_width(80.)
                    .max_width(600.)
                    .p_0()
            }));
        }

        self.result = result;
        self.columns = columns;
        self.foreign_keys = foreign_keys
            .iter()
            .filter(|foreign_key| foreign_key_target_table(tables, foreign_key).is_some())
            .cloned()
            .collect();
    }

    fn widths_by_key(result: Option<&QueryResult>, widths: &[Pixels]) -> HashMap<String, Pixels> {
        let mut remembered = HashMap::new();

        if let Some(width) = widths.first().copied() {
            remembered.insert(ROW_NUMBER_COLUMN_KEY.to_owned(), width);
        }

        if let Some(result) = result {
            for (index, column) in result.columns.iter().enumerate() {
                if let Some(width) = widths.get(index + 1).copied() {
                    remembered.insert(Self::data_column_key(index, column), width);
                }
            }
        }

        remembered
    }

    fn foreign_key_for_cell(&self, row_ix: usize, col_ix: usize) -> Option<ForeignKeyInfo> {
        if col_ix == 0 {
            return None;
        }
        let result = self.result.as_ref()?;
        let row = result.rows.get(row_ix)?;
        let column = result.columns.get(col_ix - 1)?;

        self.foreign_keys
            .iter()
            .find(|foreign_key| {
                foreign_key.columns.first() == Some(&column.name)
                    && foreign_key.columns.iter().all(|local_column| {
                        let Some(index) = result
                            .columns
                            .iter()
                            .position(|result_column| result_column.name == *local_column)
                        else {
                            return false;
                        };
                        row.values
                            .get(index)
                            .is_some_and(|value| !matches!(value, CellValue::Null))
                    })
            })
            .cloned()
    }
}

impl TableDelegate for ResultTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.result
            .as_ref()
            .map(|result| result.rows.len())
            .unwrap_or_default()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> DataColumn {
        self.columns[col_ix].clone()
    }

    fn render_header(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id("dbx-result-header")
            .bg(THEME.panel_raised)
            .border_color(THEME.border_strong)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px(px(8.))
            .text_size(px(10.))
            .text_color(THEME.text_muted)
            .truncate()
            .child(self.columns[col_ix].name.clone())
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id(("dbx-result-row", row_ix))
            .border_color(THEME.border)
            .bg(if row_ix.is_multiple_of(2) {
                THEME.canvas
            } else {
                THEME.grid_alternate
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (text, text_color) = if col_ix == 0 {
            ((row_ix + 1).to_string(), THEME.text_muted)
        } else {
            self.result
                .as_ref()
                .and_then(|result| result.rows.get(row_ix))
                .and_then(|row| row.values.get(col_ix - 1))
                .map(|value| {
                    if matches!(value, CellValue::Null) {
                        ("NULL".to_owned(), THEME.text_muted)
                    } else {
                        (value.to_string(), THEME.text)
                    }
                })
                .unwrap_or_else(|| ("—".to_owned(), THEME.text_muted))
        };
        let foreign_key = self.foreign_key_for_cell(row_ix, col_ix);

        let mut cell = div()
            .size_full()
            .flex()
            .items_center()
            .px(px(8.))
            .whitespace_nowrap()
            .truncate()
            .text_size(px(11.))
            .text_color(text_color);
        if foreign_key.is_some() {
            cell = cell
                .child(div().flex_1().min_w_0().truncate().child(text))
                .child(
                    Button::new(SharedString::from(format!(
                        "foreign-key-link-{row_ix}-{col_ix}"
                    )))
                    .with_size(Size::XSmall)
                    .compact()
                    .ghost()
                    .tooltip("Open referenced row")
                    .text_color(THEME.accent)
                    .child(icon(Icon::ArrowRight, THEME.accent))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(TableEvent::DoubleClickedCell(row_ix, col_ix));
                    })),
                );
        } else {
            cell = cell.child(text);
        }
        cell
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(THEME.text_muted)
            .child("No rows returned")
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        if col_ix == 0 {
            return (row_ix + 1).to_string();
        }

        self.result
            .as_ref()
            .and_then(|result| result.rows.get(row_ix))
            .and_then(|row| row.values.get(col_ix - 1))
            .map(ToString::to_string)
            .unwrap_or_default()
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionItemKind {
    Keyword,
    Type,
    Table,
    Column,
}

impl CompletionItemKind {
    fn label(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Table => "table",
            Self::Column => "column",
        }
    }

    fn color(self) -> Rgba {
        match self {
            Self::Keyword => gpui::rgb(0xc792ea),
            Self::Type => gpui::rgb(0x89ddff),
            Self::Table => THEME.accent,
            Self::Column => THEME.success,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlCompletionItem {
    label: String,
    insert_text: String,
    detail: String,
    search_text: String,
    kind: CompletionItemKind,
}

struct SqlCompletionSources<'a> {
    database_kind: DatabaseKind,
    tables: &'a [TableInfo],
    completion_columns: &'a HashMap<String, Vec<ColumnInfo>>,
    selected_table: Option<&'a TableRef>,
    active_columns: &'a [ColumnInfo],
    result: Option<&'a QueryResult>,
    active_schema_filter: Option<&'a str>,
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

#[derive(Clone, Debug)]
struct SqlCompletionMenu {
    context: editor::SqlCompletionContext,
    items: Vec<SqlCompletionItem>,
    selected: usize,
    signature: String,
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
    Query(QueryTab),
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
    filter_picker: Option<FilterPicker>,
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
    selected_row: Option<usize>,
    selected_column: usize,
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
            filter_picker: None,
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
            selected_row: None,
            selected_column: 0,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableAction {
    Truncate,
    Drop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FilterPickerKind {
    Column,
    Operator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FilterPicker {
    row_id: FilterRowId,
    kind: FilterPickerKind,
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
            compact_layout: false,
            narrow_workspace: false,
            test_generation: 0,
            testing_connection: false,
            _subscriptions: subscriptions,
            status: "Choose an engine and connect".into(),
            error: profile_error,
        }
    }

    fn default_url(kind: DatabaseKind) -> &'static str {
        match kind {
            DatabaseKind::PostgreSQL => "postgres://postgres@localhost:5432/postgres",
            DatabaseKind::MySQL => "mysql://root@localhost:3306/mysql",
            DatabaseKind::SQLite => "sqlite://dbx.db?mode=rwc",
            DatabaseKind::Redis => "redis://127.0.0.1:6379/0",
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

    fn hydrate_connection_fields(
        &mut self,
        kind: DatabaseKind,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let fields =
            ConnectionFields::from_url(url.clone()).unwrap_or_else(|_| ConnectionFields::new(kind));
        self.draft.kind = kind;
        self.draft.mode = ConnectionFormMode::Details;
        self.draft.connection_url.update(cx, |value, cx| {
            *value = url;
            cx.notify();
        });
        self.draft.host.update(cx, |value, cx| {
            *value = fields.host;
            cx.notify();
        });
        self.draft.port.update(cx, |value, cx| {
            *value = fields.port;
            cx.notify();
        });
        self.draft.username.update(cx, |value, cx| {
            *value = fields.username;
            cx.notify();
        });
        self.draft.password.update(cx, |value, cx| {
            *value = fields.password;
            cx.notify();
        });
        self.draft.database.update(cx, |value, cx| {
            *value = fields.database;
            cx.notify();
        });
    }

    fn connection_fields(&self, cx: &App) -> ConnectionFields {
        let mut fields = ConnectionFields::new(self.draft.kind);
        fields.host = self.draft.host.read(cx).clone();
        fields.port = self.draft.port.read(cx).clone();
        fields.username = self.draft.username.read(cx).clone();
        fields.password = self.draft.password.read(cx).clone();
        fields.database = self.draft.database.read(cx).clone();
        if self.draft.mode == ConnectionFormMode::ConnectionString
            || self.draft.kind == DatabaseKind::SQLite
        {
            fields.connection_string = self.draft.connection_url.read(cx).clone();
        } else {
            fields.use_structured_fields();
        }
        fields
    }

    fn draft_connection(&self, cx: &App) -> Result<(DatabaseKind, String), String> {
        let fields = self.connection_fields(cx);
        let url = fields.url().map_err(|error| error.to_string())?;
        Ok((fields.kind, url))
    }

    /// Resolve the visible form through the profile store when it still names
    /// the selected profile. This is the only path that reads keyring secrets.
    fn resolve_draft(&self, cx: &App) -> Result<(DatabaseKind, String, ConnectionConfig), String> {
        let (mut kind, visible_url) = self.draft_connection(cx)?;
        let config = if let (Some(store), Some(profile_id)) =
            (&self.profile_store, self.draft.selected_profile)
        {
            match store.get(profile_id).map_err(|error| error.to_string())? {
                Some(profile) if profile.kind == kind && profile.url == visible_url => {
                    let loaded = store.load(profile_id).map_err(|error| error.to_string())?;
                    kind = loaded.config.kind;
                    loaded.config
                }
                _ => ConnectionConfig::new(kind, visible_url.clone()),
            }
        } else {
            ConnectionConfig::new(kind, visible_url.clone())
        };
        Ok((kind, visible_url, config))
    }

    fn draft_test_fingerprint(
        &self,
        cx: &App,
    ) -> Result<(DatabaseKind, ConnectionFormMode, String, String), String> {
        let fields = self.connection_fields(cx);
        Ok((
            fields.kind,
            self.draft.mode,
            fields.url().map_err(|error| error.to_string())?,
            fields.redacted_url().map_err(|error| error.to_string())?,
        ))
    }

    fn set_connection_form_mode(&mut self, mode: ConnectionFormMode, cx: &mut Context<Self>) {
        if mode == self.draft.mode {
            return;
        }

        match mode {
            ConnectionFormMode::Details if self.draft.kind != DatabaseKind::SQLite => {
                let connection_string = self.draft.connection_url.read(cx).trim().to_owned();
                let fields = match ConnectionFields::from_url(connection_string) {
                    Ok(fields) if fields.kind == self.draft.kind => fields,
                    Ok(_) => {
                        self.set_error(format!(
                            "Connection string must be for {}",
                            self.draft.kind
                        ));
                        cx.notify();
                        return;
                    }
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                self.draft.host.update(cx, |value, cx| {
                    *value = fields.host;
                    cx.notify();
                });
                self.draft.port.update(cx, |value, cx| {
                    *value = fields.port;
                    cx.notify();
                });
                self.draft.username.update(cx, |value, cx| {
                    *value = fields.username;
                    cx.notify();
                });
                self.draft.password.update(cx, |value, cx| {
                    *value = fields.password;
                    cx.notify();
                });
                self.draft.database.update(cx, |value, cx| {
                    *value = fields.database;
                    cx.notify();
                });
                self.draft.mode = ConnectionFormMode::Details;
            }
            ConnectionFormMode::ConnectionString => {
                let url = match self.connection_fields(cx).url() {
                    Ok(url) => url,
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                self.draft.connection_url.update(cx, |value, cx| {
                    *value = url;
                    cx.notify();
                });
                self.draft.mode = ConnectionFormMode::ConnectionString;
            }
            ConnectionFormMode::Details => return,
        }
        self.error = None;
        cx.notify();
    }

    fn select_kind(&mut self, kind: DatabaseKind, cx: &mut Context<Self>) {
        self.draft.selected_profile = None;
        self.hydrate_connection_fields(kind, Self::default_url(kind).to_owned(), cx);
        self.error = None;
        cx.notify();
    }

    fn select_environment(&mut self, environment: ConnectionEnvironment, cx: &mut Context<Self>) {
        self.draft.environment = environment;
        cx.notify();
    }

    fn select_saved_connection(&mut self, profile: SavedConnection, cx: &mut Context<Self>) {
        self.draft.selected_profile = Some(profile.id);
        self.draft.environment = profile.environment;
        self.draft.connection_name.update(cx, |name, cx| {
            *name = profile.name;
            cx.notify();
        });
        // Selecting a profile only changes the draft. Resolve the password
        // lazily when Test Connection or Connect actually needs it; opening
        // the picker must not trigger a Keychain prompt or block the UI.
        self.hydrate_connection_fields(profile.kind, profile.url.clone(), cx);
        self.error = None;
        self.status = "Saved connection selected".into();
        cx.notify();
    }

    fn save_connection(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.profile_store.clone() else {
            self.set_error("Connection profile storage is unavailable".into());
            cx.notify();
            return;
        };
        let name = self.draft.connection_name.read(cx).trim().to_owned();
        let (kind, url) = match self.draft_connection(cx) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        let mut draft = ConnectionProfileDraft::new(name, kind, url.clone())
            .with_environment(self.draft.environment);
        if let Some(id) = self.draft.selected_profile {
            draft = draft.with_id(id);
        }
        match store.save(draft) {
            Ok(profile) => {
                self.draft.selected_profile = Some(profile.id);
                // Rehydrate from the entered URL rather than the scrubbed
                // profile so a newly entered password remains in the draft.
                self.hydrate_connection_fields(profile.kind, url, cx);
                match store.list() {
                    Ok(profiles) => self.saved_connections = profiles,
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                }
                self.error = None;
                self.status = format!("Saved connection ‘{}’", profile.name);
            }
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
    }

    fn delete_saved_connection(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.profile_store.clone() else {
            return;
        };
        match store.delete(id) {
            Ok(true) => {
                self.saved_connections.retain(|profile| profile.id != id);
                if self.draft.selected_profile == Some(id) {
                    self.draft.selected_profile = None;
                    self.draft.connection_name.update(cx, |name, cx| {
                        name.clear();
                        cx.notify();
                    });
                }
                self.error = None;
                self.status = "Saved connection deleted".into();
            }
            Ok(false) => self.error = Some("Saved connection no longer exists".into()),
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
    }

    fn choose_sqlite_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Choose database")),
        });
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.update(cx, |this, cx| {
                            if this.draft.kind != DatabaseKind::SQLite {
                                return;
                            }
                            this.draft.selected_profile = None;
                            this.hydrate_connection_fields(
                                DatabaseKind::SQLite,
                                sqlite_url(&path),
                                cx,
                            );
                            this.error = None;
                            this.status = format!("Selected {}", path.display());
                            cx.notify();
                        })?;
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open file picker: {error}"));
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("File picker closed unexpectedly: {error}"));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (kind, _, config) = match self.resolve_draft(cx) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        let profile_id = self.draft.selected_profile;
        let session_id = Uuid::new_v4();
        let name = self.draft.connection_name.read(cx).trim().to_owned();
        let environment = profile_id
            .and_then(|id| {
                self.saved_connections
                    .iter()
                    .find(|profile| profile.id == id)
            })
            .map(|profile| profile.environment)
            .unwrap_or(self.draft.environment);
        let mut session =
            ConnectionSession::new(session_id, profile_id, name, kind, environment, window, cx);
        session.busy = true;
        session.request_generation = 1;
        let generation = session.request_generation;
        self.sessions.push(session);
        self.active_session_id = Some(session_id);
        self.connection_picker_open = false;
        self.error = None;
        self.status = format!("Connecting to {kind}…");
        let runtime = self.runtime.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let engine = Arc::new(DatabaseEngine::connect(config).await?);
                    let tables = engine.list_tables().await?;
                    let databases = engine.list_databases().await.unwrap_or_default();
                    let current_database = engine.current_database().await.ok();
                    let schema_filter = default_schema_filter(kind, &tables);
                    let initial_table =
                        schema_filtered_tables(kind, &tables, schema_filter.as_deref())
                            .into_iter()
                            .next();
                    let initial = if let Some(table) = initial_table {
                        let table_ref = table_ref(&table);
                        let columns = engine.describe_table(&table_ref).await?;
                        let result = if kind.is_sql() {
                            Some(
                                engine
                                    .query_table(
                                        &table_ref,
                                        &[],
                                        &[],
                                        &[],
                                        Some(Page::default()),
                                        QueryOptions::default(),
                                    )
                                    .await?,
                            )
                        } else {
                            Some(
                                engine
                                    .query("SCAN 0 COUNT 100", QueryOptions::default())
                                    .await?,
                            )
                        };
                        Some((table_ref, columns, result))
                    } else {
                        None
                    };
                    Ok::<_, dbx_core::DbxError>((
                        engine,
                        tables,
                        databases,
                        current_database,
                        schema_filter,
                        initial,
                    ))
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
                    Ok((engine, tables, databases, current_database, schema_filter, initial)) => {
                        session.engine = Some(engine);
                        session.tables = tables;
                        session.databases = databases;
                        session.current_database = current_database;
                        session.schema_filter = schema_filter;
                        if let Some((table, columns, result)) = initial {
                            session.selected_table = Some(table.clone());
                            session.table_columns = columns;
                            session.completion_columns.insert(
                                completion_table_key(&table),
                                session.table_columns.clone(),
                            );
                            session.set_result(result, cx);
                            session.result_table = Some(table);
                        } else {
                            session.selected_table = None;
                            session.table_columns.clear();
                            session.completion_columns.clear();
                            session.set_result(None, cx);
                            session.result_table = None;
                        }
                        session.status = format!("Connected to {kind}");
                        session.error = None;
                        session.pane = Pane::Data;
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Connection failed".into();
                    }
                }
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn test_connection(&mut self, cx: &mut Context<Self>) {
        let (kind, _, config) = match self.resolve_draft(cx) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        self.test_generation += 1;
        let generation = self.test_generation;
        let fingerprint = match self.draft_test_fingerprint(cx) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        self.testing_connection = true;
        self.error = None;
        self.status = format!("Testing {kind} connection…");
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let engine = DatabaseEngine::connect(config).await?;
                    let tables = engine.list_tables().await?;
                    Ok::<usize, dbx_core::DbxError>(tables.len())
                })
                .await?;
            this.update(cx, |this, cx| {
                if this.test_generation != generation {
                    return;
                }
                if this.draft_test_fingerprint(cx).ok().as_ref() != Some(&fingerprint) {
                    this.testing_connection = false;
                    this.status = "Connection details changed · test again".into();
                    this.error = None;
                    cx.notify();
                    return;
                }
                this.testing_connection = false;
                match result {
                    Ok(table_count) => {
                        this.status =
                            format!("Connection succeeded · {table_count} table(s) found");
                        this.error = None;
                    }
                    Err(error) => {
                        this.status = "Connection test failed".into();
                        this.error = Some(error.to_string());
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn begin_new_connection(&mut self, cx: &mut Context<Self>) {
        self.draft.selected_profile = None;
        self.draft.environment = ConnectionEnvironment::default();
        self.draft.connection_name.update(cx, |name, cx| {
            name.clear();
            cx.notify();
        });
        self.hydrate_connection_fields(
            DatabaseKind::SQLite,
            Self::default_url(DatabaseKind::SQLite).to_owned(),
            cx,
        );
        self.connection_picker_open = true;
        self.error = None;
        cx.notify();
    }

    fn close_connection_picker(&mut self, cx: &mut Context<Self>) {
        self.connection_picker_open = false;
        self.error = None;
        cx.notify();
    }

    fn close_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(index) = self
            .sessions
            .iter()
            .position(|session| session.id == session_id)
        else {
            return;
        };
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
            kind: SecondaryTabKind::Query(query_tab),
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
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.active_secondary_tab = None;
        session.pane = Pane::Data;
        session.selected_table = Some(table_ref.clone());
        // Until this request completes, the visible snapshot belongs to the
        // previous table and must not be used for a mutation.
        session.result_table = None;
        session.selected_row = None;
        session.row_draft = None;
        session.row_draft_subscriptions.clear();
        session.foreign_keys.clear();
        session.clear_grid_selection(cx);
        let mut filter_model = FilterModel::new();
        for filter in &filters {
            if let Some(value) = filter.value.as_ref() {
                filter_model.add_row_with_value(
                    filter.column.clone(),
                    filter.operator,
                    value.to_string(),
                    window,
                    cx,
                );
            }
        }
        session.filters = filter_model;
        session.filter_picker = None;
        session.busy = true;
        session.error = None;
        session.status = format!("Loading {}…", table.name);
        session.request_generation += 1;
        let generation = session.request_generation;
        let result_table = table_ref.clone();
        let row_navigation = !filters.is_empty();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let structure = engine.table_structure(&table_ref).await?;
                    let result = if kind.is_sql() {
                        engine
                            .query_table(
                                &table_ref,
                                &[],
                                &filters,
                                &[],
                                Some(Page::default()),
                                QueryOptions::default(),
                            )
                            .await?
                    } else {
                        engine
                            .query("SCAN 0 COUNT 100", QueryOptions::default())
                            .await?
                    };
                    Ok::<_, dbx_core::DbxError>((structure, result))
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
                    Ok((structure, result)) => {
                        let has_rows = !result.rows.is_empty();
                        session.table_columns = structure.columns;
                        session.completion_columns.insert(
                            completion_table_key(&result_table),
                            session.table_columns.clone(),
                        );
                        session.foreign_keys = structure.foreign_keys;
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
        let Some(column) = self
            .session(session_id)
            .and_then(|session| session.table_columns.first())
            .map(|column| column.name.clone())
        else {
            return;
        };
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let row_id = session
            .filters
            .add_row(column, FilterOperator::Equals, window, cx);
        session.filter_picker = Some(FilterPicker {
            row_id,
            kind: FilterPickerKind::Column,
        });
        cx.notify();
    }

    fn remove_filter_for(
        &mut self,
        session_id: SessionId,
        row_id: FilterRowId,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            session.filters.remove(row_id);
            if session
                .filter_picker
                .is_some_and(|picker| picker.row_id == row_id)
            {
                session.filter_picker = None;
            }
            cx.notify();
        }
    }

    fn clear_filters_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(session_id) {
            session.filters = FilterModel::new();
            session.filter_picker = None;
            cx.notify();
        }
    }

    fn toggle_filter_picker_for(
        &mut self,
        session_id: SessionId,
        row_id: FilterRowId,
        kind: FilterPickerKind,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            let picker = FilterPicker { row_id, kind };
            session.filter_picker = (session.filter_picker != Some(picker)).then_some(picker);
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
            session.filter_picker = None;
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
            session.filter_picker = None;
            cx.notify();
        }
    }

    fn refresh_table_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
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
        session.status = "Applying filters…".into();
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
                        engine
                            .query_table(
                                &table,
                                &[],
                                &filters,
                                &[],
                                Some(Page::default()),
                                QueryOptions::default(),
                            )
                            .await
                    } else {
                        let pattern = filters
                            .first()
                            .and_then(|filter| filter.value.as_ref())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "*".into());
                        let pattern = redis_command_word(&pattern);
                        engine
                            .query(
                                &format!("SCAN 0 MATCH {pattern} COUNT 100"),
                                QueryOptions::default(),
                            )
                            .await
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
                    Ok(result) => {
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
                SqlCompletionSources {
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
        let Some((engine, tab_id, query, busy)) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query_tab) = &tab.kind else {
                return None;
            };
            Some((
                session.engine.clone(),
                tab_id,
                query_tab.query_text.read(cx).trim().to_owned(),
                query_tab.busy,
            ))
        }) else {
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
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { engine.query(&query, QueryOptions::default()).await })
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
                    }
                    Err(error) => {
                        query_tab.error = Some(error.to_string());
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
        session.row_draft = row_draft;
        session.error = None;
        session.status = if editable {
            "Editing selected row".into()
        } else {
            "Inspecting read-only query row".into()
        };
        cx.notify();
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

    #[allow(dead_code)]
    fn render_connection_legacy(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let connection_name_focus = self.draft.connection_name_editor.read(cx).focus_handle();
        let connection_focus = self.draft.connection_editor.read(cx).focus_handle();
        let saved_connections = self.saved_connections.clone();
        let selected_profile = self.draft.selected_profile;
        let draft_kind = self.draft.kind;
        let has_sessions = !self.sessions.is_empty();
        let kinds = [
            DatabaseKind::PostgreSQL,
            DatabaseKind::MySQL,
            DatabaseKind::SQLite,
            DatabaseKind::Redis,
        ];
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .bg(THEME.canvas)
            .child(
                div()
                    .w(px(720.))
                    .p(px(32.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(THEME.border)
                    .bg(THEME.panel)
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(28.))
                                            .text_color(THEME.text)
                                            .child("DBX"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(THEME.text_muted)
                                            .child("A fast, native database workbench"),
                                    ),
                            )
                            .child(
                                div()
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(99.))
                                    .bg(THEME.accent_soft)
                                    .text_color(THEME.accent)
                                    .text_size(px(11.))
                                    .child("GPUI · RUST"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(7.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(THEME.text_muted)
                                    .child("SAVED CONNECTIONS"),
                            )
                            .child(
                                div()
                                    .id("saved-connections")
                                    .max_h(px(126.))
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .when(saved_connections.is_empty(), |view| {
                                        view.child(
                                            div()
                                                .p(px(10.))
                                                .rounded(px(6.))
                                                .bg(THEME.panel_raised)
                                                .text_size(px(12.))
                                                .text_color(THEME.text_muted)
                                                .child("No saved connections yet"),
                                        )
                                    })
                                    .children(saved_connections.into_iter().map(|profile| {
                                        let id = profile.id;
                                        let selected = selected_profile == Some(id);
                                        let select_profile = profile.clone();
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "saved-connection-{id}"
                                                    )))
                                                    .flex_1()
                                                    .px(px(10.))
                                                    .py(px(8.))
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(if selected {
                                                        THEME.accent
                                                    } else {
                                                        THEME.border
                                                    })
                                                    .bg(if selected {
                                                        THEME.accent_soft
                                                    } else {
                                                        THEME.panel_raised
                                                    })
                                                    .cursor_pointer()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .text_color(THEME.text)
                                                            .child(profile.name),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(10.))
                                                            .text_color(THEME.text_muted)
                                                            .child(profile.kind.to_string()),
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.select_saved_connection(
                                                                select_profile.clone(),
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "delete-connection-{id}"
                                                    )))
                                                    .px(px(9.))
                                                    .py(px(8.))
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(THEME.border)
                                                    .text_size(px(11.))
                                                    .text_color(THEME.text_muted)
                                                    .cursor_pointer()
                                                    .hover(|style| {
                                                        style
                                                            .border_color(THEME.danger)
                                                            .text_color(THEME.danger)
                                                    })
                                                    .child("Delete")
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.delete_saved_connection(id, cx)
                                                        },
                                                    )),
                                            )
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .children(kinds.into_iter().map(|kind| {
                                let selected = kind == draft_kind;
                                div()
                                    .id(SharedString::from(format!("engine-{kind}")))
                                    .flex_1()
                                    .p(px(12.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(if selected {
                                        THEME.accent
                                    } else {
                                        THEME.border
                                    })
                                    .bg(if selected {
                                        THEME.accent_soft
                                    } else {
                                        THEME.panel_raised
                                    })
                                    .text_color(if selected { THEME.accent } else { THEME.text })
                                    .cursor_pointer()
                                    .hover(|style| style.border_color(THEME.border_strong))
                                    .child(kind.to_string())
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.select_kind(kind, cx)
                                        }),
                                    )
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(7.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(THEME.text_muted)
                                    .child("CONNECTION NAME"),
                            )
                            .child(editor::input(
                                self.draft.connection_name_editor.clone(),
                                connection_name_focus,
                                false,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(7.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(THEME.text_muted)
                                    .child("CONNECTION URL"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div().flex_1().child(editor::input(
                                            self.draft.connection_editor.clone(),
                                            connection_focus,
                                            false,
                                        )),
                                    )
                                    .when(draft_kind == DatabaseKind::SQLite, |view| {
                                        view.child(
                                            div()
                                                .id("choose-sqlite-file")
                                                .px(px(12.))
                                                .py(px(9.))
                                                .rounded(px(6.))
                                                .border_1()
                                                .border_color(THEME.border)
                                                .bg(THEME.panel_raised)
                                                .text_size(px(11.))
                                                .text_color(THEME.text)
                                                .cursor_pointer()
                                                .hover(|style| {
                                                    style.border_color(THEME.accent)
                                                })
                                                .child("Choose file…")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.choose_sqlite_file(cx)
                                                })),
                                        )
                                    }),
                            ),
                    )
                    .when_some(self.error.clone(), |view, error| {
                        view.child(
                            div()
                                .p(px(10.))
                                .rounded(px(6.))
                                .bg(THEME.panel_raised)
                                .border_1()
                                .border_color(THEME.danger)
                                .text_color(THEME.danger)
                                .text_size(px(12.))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(THEME.text_muted)
                                            .child("Profiles persist on disk; passwords use the OS keyring."),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(THEME.text_muted)
                                            .child("A password is never written to connections.json."),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .id("save-connection")
                                            .px(px(14.))
                                            .py(px(9.))
                                            .rounded(px(7.))
                                            .border_1()
                                            .border_color(THEME.border)
                                            .bg(THEME.panel_raised)
                                            .text_color(THEME.text)
                                            .cursor_pointer()
                                            .hover(|style| style.border_color(THEME.accent))
                                            .child("Save")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.save_connection(cx)
                                            })),
                                    )
                                    .when(has_sessions, |view| {
                                        view.child(
                                            div()
                                                .id("cancel-connection")
                                                .px(px(12.))
                                                .py(px(9.))
                                                .rounded(px(7.))
                                                .border_1()
                                                .border_color(THEME.border)
                                                .bg(THEME.panel_raised)
                                                .text_color(THEME.text)
                                                .cursor_pointer()
                                                .child("Cancel")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.close_connection_picker(cx)
                                                })),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id("connect")
                                            .px(px(18.))
                                            .py(px(10.))
                                            .rounded(px(7.))
                                            .bg(THEME.accent)
                                            .text_color(THEME.canvas)
                                            .cursor_pointer()
                                            .child("Connect")
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    this.connect(window, cx)
                                                }),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    #[allow(dead_code)]
    fn render_connection_previous(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let connection_name_focus = self.draft.connection_name_editor.read(cx).focus_handle();
        let connection_focus = self.draft.connection_editor.read(cx).focus_handle();
        let saved_connections = self.saved_connections.clone();
        let selected_profile = self.draft.selected_profile;
        let draft_kind = self.draft.kind;
        let has_sessions = !self.sessions.is_empty();
        let kinds = [
            DatabaseKind::PostgreSQL,
            DatabaseKind::MySQL,
            DatabaseKind::SQLite,
            DatabaseKind::Redis,
        ];

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .bg(THEME.canvas)
            .child(
                div()
                    .w(if self.compact_layout {
                        px(0.)
                    } else {
                        px(320.)
                    })
                    .flex_none()
                    .when(self.compact_layout, |view| view.overflow_hidden())
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(THEME.border)
                    .bg(THEME.panel)
                    .child(
                        div()
                            .h(px(58.))
                            .px(px(16.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(THEME.border)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Connections"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(THEME.text_muted)
                                            .child(format!("{} saved", saved_connections.len())),
                                    ),
                            )
                            .child(
                                div()
                                    .id("new-connection-from-list")
                                    .size(px(28.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(THEME.accent)
                                    .cursor_pointer()
                                    .child(icon(Icon::Add, THEME.text))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.begin_new_connection(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("saved-connections")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .when(saved_connections.is_empty(), |view| {
                                view.child(
                                    div()
                                        .p(px(14.))
                                        .text_size(px(11.))
                                        .text_color(THEME.text_muted)
                                        .child("No saved connections yet. Create one to keep it here."),
                                )
                            })
                            .children(saved_connections.into_iter().map(|profile| {
                                let id = profile.id;
                                let selected = selected_profile == Some(id);
                                let select_profile = profile.clone();
                                let label = profile.name.clone();
                                let detail = truncate(&profile.url, 35);
                                let kind = profile.kind.to_string();
                                div()
                                    .id(SharedString::from(format!("saved-connection-{id}")))
                                    .h(px(54.))
                                    .px(px(10.))
                                    .rounded(px(6.))
                                    .bg(if selected {
                                        THEME.accent_soft
                                    } else {
                                        THEME.panel
                                    })
                                    .border_1()
                                    .border_color(if selected {
                                        THEME.accent
                                    } else {
                                        THEME.panel
                                    })
                                    .flex()
                                    .items_center()
                                    .gap(px(9.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(THEME.panel_raised))
                                    .child(icon(
                                        Icon::Database,
                                        if selected {
                                            THEME.accent
                                        } else {
                                            THEME.text_muted
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap(px(3.))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(7.))
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .truncate()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child(label),
                                                    )
                                                    .child(badge(kind, THEME.text_muted)),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(9.))
                                                    .text_color(THEME.text_muted)
                                                    .child(detail),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "delete-connection-{id}"
                                            )))
                                            .size(px(26.))
                                            .rounded(px(5.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .hover(|style| style.bg(THEME.panel_raised))
                                            .child(icon(Icon::Close, THEME.text_muted))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.delete_saved_connection(id, cx)
                                            })),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_saved_connection(select_profile.clone(), cx)
                                    }))
                            })),
                    ),
            )
            .child(
                div()
                    .id("connection-form-scroll")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .p(if self.compact_layout {
                        px(12.)
                    } else {
                        px(24.)
                    })
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(900.))
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .child(
                                div()
                                    .flex()
                                    .items_end()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(4.))
                                            .child(
                                                div()
                                                    .text_size(px(18.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(if selected_profile.is_some() {
                                                        "Edit connection"
                                                    } else {
                                                        "New connection"
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(THEME.text_muted)
                                                    .child("Name it, choose an engine, then open it in its own tab."),
                                            ),
                                    )
                                    .child(badge("NATIVE · GPUI", THEME.success)),
                            )
                            .child(
                                div()
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(THEME.border)
                                    .bg(THEME.panel)
                                    .flex()
                                    .min_h(px(360.))
                                    .child(
                                        div()
                                            .w(if self.compact_layout {
                                                px(124.)
                                            } else {
                                                px(170.)
                                            })
                                            .flex_none()
                                            .p(px(8.))
                                            .border_r_1()
                                            .border_color(THEME.border)
                                            .flex()
                                            .flex_col()
                                            .gap(px(3.))
                                            .child(
                                                div()
                                                    .px(px(8.))
                                                    .pt(px(4.))
                                                    .pb(px(7.))
                                                    .text_size(px(9.))
                                                    .text_color(THEME.text_muted)
                                                    .child("DATABASE ENGINE"),
                                            )
                                            .children(kinds.into_iter().map(|kind| {
                                                let selected = kind == draft_kind;
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "engine-{kind}"
                                                    )))
                                                    .h(px(36.))
                                                    .px(px(9.))
                                                    .rounded(px(6.))
                                                    .bg(if selected {
                                                        THEME.accent_soft
                                                    } else {
                                                        THEME.panel
                                                    })
                                                    .text_color(if selected {
                                                        THEME.accent
                                                    } else {
                                                        THEME.text_muted
                                                    })
                                                    .text_size(px(11.))
                                                    .font_weight(if selected {
                                                        FontWeight::MEDIUM
                                                    } else {
                                                        FontWeight::NORMAL
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(9.))
                                                    .cursor_pointer()
                                                    .hover(|style| style.bg(THEME.panel_raised))
                                                    .child(icon(
                                                        Icon::Database,
                                                        if selected {
                                                            THEME.accent
                                                        } else {
                                                            THEME.text_muted
                                                        },
                                                    ))
                                                    .child(kind.to_string())
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.select_kind(kind, cx)
                                                        },
                                                    ))
                                            })),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .p(px(16.))
                                            .flex()
                                            .flex_col()
                                            .gap(px(14.))
                                            .child(panel_header(
                                                draft_kind.to_string(),
                                                "Connection details",
                                            ))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(6.))
                                                    .child(
                                                        div()
                                                            .text_size(px(10.))
                                                            .text_color(THEME.text_muted)
                                                            .child("Connection name"),
                                                    )
                                                    .child(editor::input(
                                                        self.draft.connection_name_editor.clone(),
                                                        connection_name_focus,
                                                        false,
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(6.))
                                                    .child(
                                                        div()
                                                            .text_size(px(10.))
                                                            .text_color(THEME.text_muted)
                                                            .child(if draft_kind
                                                                == DatabaseKind::SQLite
                                                            {
                                                                "Database file"
                                                            } else {
                                                                "Connection URL"
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap(px(8.))
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .child(editor::input(
                                                                        self.draft
                                                                            .connection_editor
                                                                            .clone(),
                                                                        connection_focus,
                                                                        false,
                                                                    )),
                                                            )
                                                            .when(
                                                                draft_kind
                                                                    == DatabaseKind::SQLite,
                                                                |view| {
                                                                    view.child(
                                                                        button(
                                                                            "legacy-choose-sqlite-file",
                                                                            "Choose file…",
                                                                            ButtonKind::Quiet,
                                                                        )
                                                                        .cursor_pointer()
                                                                        .hover(|style| {
                                                                            style.border_color(
                                                                                THEME.accent,
                                                                            )
                                                                        })
                                                                        .on_click(cx.listener(
                                                                            |this, _, _, cx| {
                                                                                this.choose_sqlite_file(cx)
                                                                            },
                                                                        )),
                                                                    )
                                                                },
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .mt(px(4.))
                                                    .p(px(10.))
                                                    .rounded(px(6.))
                                                    .bg(THEME.canvas)
                                                    .border_1()
                                                    .border_color(THEME.border)
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(8.))
                                                    .child(
                                                        div()
                                                            .size(px(6.))
                                                            .rounded_full()
                                                            .bg(THEME.success),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(10.))
                                                            .text_color(THEME.text_muted)
                                                            .child("Profiles persist on disk. Secrets stay in the OS keyring."),
                                                    ),
                                            )
                                            .when_some(self.error.clone(), |view, error| {
                                                view.child(
                                                    div()
                                                        .p(px(10.))
                                                        .rounded(px(6.))
                                                        .bg(THEME.panel_raised)
                                                        .border_1()
                                                        .border_color(THEME.danger)
                                                        .text_color(THEME.danger)
                                                        .text_size(px(11.))
                                                        .child(error),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .mt_auto()
                                                    .flex()
                                                    .items_center()
                                                    .justify_end()
                                                    .gap(px(8.))
                                                    .when(has_sessions, |view| {
                                                        view.child(
                                                            button(
                                                                "legacy-cancel-new-connection",
                                                                "Cancel",
                                                                ButtonKind::Quiet,
                                                            )
                                                            .cursor_pointer()
                                                            .on_click(cx.listener(
                                                                |this, _, _, cx| {
                                                                    this.close_connection_picker(
                                                                        cx,
                                                                    )
                                                                },
                                                            )),
                                                        )
                                                    })
                                                    .child(
                                                        button("legacy-save-connection", "Save", ButtonKind::Quiet)
                                                            .cursor_pointer()
                                                            .on_click(cx.listener(
                                                                |this, _, _, cx| {
                                                                    this.save_connection(cx)
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        button(
                                                            "legacy-connect",
                                                            "Connect",
                                                            ButtonKind::Primary,
                                                        )
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.connect(window, cx)
                                                            },
                                                        )),
                                                    ),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    fn render_connection(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let name_focus = self.draft.connection_name_editor.read(cx).focus_handle();
        let url_focus = self.draft.connection_editor.read(cx).focus_handle();
        let host_focus = self.draft.host_editor.read(cx).focus_handle();
        let port_focus = self.draft.port_editor.read(cx).focus_handle();
        let username_focus = self.draft.username_editor.read(cx).focus_handle();
        let password_focus = self.draft.password_editor.read(cx).focus_handle();
        let database_focus = self.draft.database_editor.read(cx).focus_handle();
        let kind = self.draft.kind;
        let details =
            self.draft.mode == ConnectionFormMode::Details && kind != DatabaseKind::SQLite;
        let environment = self.draft.environment;
        let saved_connections = self.saved_connections.clone();
        let selected_profile = self.draft.selected_profile;

        div().flex_1().min_h_0().flex().bg(THEME.canvas)
            .child(
                div().w(if self.compact_layout { px(0.) } else { px(252.) }).flex_none()
                    .when(self.compact_layout, |view| view.overflow_hidden())
                    .flex().flex_col().border_r_1().border_color(THEME.border).bg(THEME.panel)
                    .child(div().h(px(54.)).px(px(14.)).flex().items_center().justify_between().border_b_1().border_color(THEME.border)
                        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Connections"))
                        .child(div().id("new-connection-from-list").size(px(26.)).rounded(px(6.)).flex().items_center().justify_center().bg(THEME.accent).cursor_pointer().child(icon(Icon::Add, THEME.text)).on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx)))))
                    .child(div().id("saved-connections").flex_1().min_h_0().overflow_y_scroll().p(px(8.)).flex().flex_col().gap(px(3.))
                        .when(saved_connections.is_empty(), |view| view.child(div().p(px(10.)).text_size(px(11.)).text_color(THEME.text_muted).child("No saved connections yet.")))
                        .children(saved_connections.into_iter().map(|profile| {
                            let id = profile.id; let selected = selected_profile == Some(id); let choose = profile.clone();
                            div().id(SharedString::from(format!("saved-connection-{id}"))).h(px(48.)).px(px(9.)).rounded(px(6.)).bg(if selected { THEME.accent_soft } else { THEME.panel }).cursor_pointer().flex().items_center().gap(px(8.))
                                .child(database_logo(profile.kind, if selected { THEME.accent } else { THEME.text_muted }))
                                .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().text_size(px(11.)).child(profile.name)).child(div().truncate().text_size(px(9.)).text_color(THEME.text_muted).child(display_url(&profile.url))))
                                .child(environment_badge(profile.environment))
                                .on_click(cx.listener(move |this, _, _, cx| this.select_saved_connection(choose.clone(), cx)))
                        }))),
            )
            .child(div().flex_1().min_w_0().flex().flex_col()
                .child(div().id("connection-form-scroll").flex_1().min_h_0().overflow_y_scroll().p(if self.compact_layout { px(14.) } else { px(24.) }).flex().justify_center()
                    .child(div().w_full().max_w(px(720.)).flex().flex_col().gap(px(14.))
                        .child(div().flex().items_end().justify_between().child(div().flex().flex_col().gap(px(3.)).child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(if self.draft.selected_profile.is_some() { "Edit connection" } else { "New connection" })).child(div().text_size(px(11.)).text_color(THEME.text_muted).child("Configure a saved profile or connect once."))).child(div().flex().items_center().gap(px(6.)).px(px(8.)).py(px(4.)).rounded_full().bg(THEME.panel_raised).child(database_logo(kind, THEME.accent)).child(div().text_size(px(10.)).font_weight(FontWeight::MEDIUM).text_color(THEME.accent).child(kind.to_string()))))
                        .child(div().rounded(px(9.)).border_1().border_color(THEME.border).bg(THEME.panel).p(px(16.)).flex().flex_col().gap(px(12.))
                            .child(div().flex().gap(px(6.)).children([DatabaseKind::PostgreSQL, DatabaseKind::MySQL, DatabaseKind::SQLite, DatabaseKind::Redis].into_iter().map(|option| { let selected = option == kind; div().id(SharedString::from(format!("engine-{option}"))).flex().items_center().gap(px(5.)).px(px(9.)).py(px(7.)).rounded(px(6.)).bg(if selected { THEME.accent_soft } else { THEME.panel_raised }).text_color(if selected { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child(database_logo(option, if selected { THEME.accent } else { THEME.text_muted })).child(option.to_string()).on_click(cx.listener(move |this, _, _, cx| this.select_kind(option, cx))) })))
                            .child(div().flex().items_center().gap(px(6.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Environment")).children(ConnectionEnvironment::ALL.into_iter().map(|option| { let selected = option == environment; div().id(SharedString::from(format!("environment-{option}"))).flex().items_center().gap(px(5.)).px(px(9.)).py(px(7.)).rounded(px(6.)).bg(if selected { THEME.accent_soft } else { THEME.panel_raised }).text_color(if selected { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child(div().size(px(6.)).rounded_full().bg(environment_color(option))).child(option.to_string()).on_click(cx.listener(move |this, _, _, cx| this.select_environment(option, cx))) })))
                            .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Connection name")).child(editor::input(self.draft.connection_name_editor.clone(), name_focus, false)))
                            .when(kind != DatabaseKind::SQLite, |view| view.child(div().flex().gap(px(5.))
                                .child(div().id("connection-details-mode").flex().items_center().gap(px(5.)).px(px(9.)).py(px(6.)).rounded(px(5.)).bg(if details { THEME.accent_soft } else { THEME.panel_raised }).text_color(if details { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child("Details").on_click(cx.listener(|this, _, _, cx| this.set_connection_form_mode(ConnectionFormMode::Details, cx))))
                                .child(div().id("connection-string-mode").flex().items_center().gap(px(5.)).px(px(9.)).py(px(6.)).rounded(px(5.)).bg(if !details { THEME.accent_soft } else { THEME.panel_raised }).text_color(if !details { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child("Connection string").on_click(cx.listener(|this, _, _, cx| this.set_connection_form_mode(ConnectionFormMode::ConnectionString, cx))))))
                            .when(details, |view| view
                                .child(div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Host")).child(editor::input(self.draft.host_editor.clone(), host_focus, false))).child(div().w(px(110.)).flex_none().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Port")).child(editor::input(self.draft.port_editor.clone(), port_focus, false))))
                                .child(div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Username")).child(editor::input(self.draft.username_editor.clone(), username_focus, false))).child(div().flex_1().min_w_0().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Password")).child(editor::input(self.draft.password_editor.clone(), password_focus, false))))
                                .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child(if kind == DatabaseKind::Redis { "Database index (optional)" } else { "Database" })).child(editor::input(self.draft.database_editor.clone(), database_focus, false))))
                            .when(!details, |view| view.child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child(if kind == DatabaseKind::SQLite { "Database file or connection string" } else { "Connection string" })).child(div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(editor::input(self.draft.connection_editor.clone(), url_focus, false))).when(kind == DatabaseKind::SQLite, |view| view.child(button("choose-sqlite-file", "Choose file…", ButtonKind::Quiet).flex_none().cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.choose_sqlite_file(cx))))))))
                            .child(div().p(px(9.)).rounded(px(6.)).bg(THEME.canvas).text_size(px(10.)).text_color(THEME.text_muted).child("Profiles stay on disk; passwords stay in the OS keyring.")))))
                .child(div().flex_none().border_t_1().border_color(THEME.border).bg(THEME.panel).px(if self.compact_layout { px(14.) } else { px(24.) }).py(px(12.)).flex().items_center().justify_between().gap(px(12.))
                    .child(div().min_w_0().flex().flex_col().gap(px(4.))
                        .child(div().truncate().text_size(px(10.)).text_color(if self.error.is_some() { THEME.text_muted } else { THEME.success }).child(self.status.clone()))
                        .when_some(self.error.clone(), |view, error| view.child(div().truncate().text_size(px(10.)).text_color(THEME.danger).child(error))))
                    .child(div().flex_none().flex().items_center().gap(px(8.))
                        .child(button("test-connection", if self.testing_connection { "Testing…" } else { "Test connection" }, ButtonKind::Quiet).when(!self.testing_connection, |button| button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.test_connection(cx)))))
                        .child(button("save-connection", "Save", ButtonKind::Quiet).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.save_connection(cx))))
                        .child(button("connect", "Connect", ButtonKind::Primary).cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))))))
    }

    fn render_workspace(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(THEME.canvas)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_main(window, cx)),
            )
            .child(self.render_status())
            .child(self.render_table_context_menu(cx))
    }

    fn render_app_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_pane = self.active_session().map(|session| session.pane);
        div()
            .w(px(46.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .border_r_1()
            .border_color(THEME.border)
            .bg(THEME.rail)
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(THEME.border)
                    .child(img(self.logo.clone()).id("rail-logo").size(px(24.))),
            )
            .child(
                div()
                    .flex_1()
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(self.rail_button(
                        "rail-data",
                        Icon::Table,
                        active_pane == Some(Pane::Data),
                        cx.listener(|this, _, _, cx| this.set_active_pane(Pane::Data, cx)),
                    ))
                    .child(self.rail_button(
                        "rail-structure",
                        Icon::Structure,
                        active_pane == Some(Pane::Structure),
                        cx.listener(|this, _, _, cx| this.set_active_pane(Pane::Structure, cx)),
                    ))
                    .child(self.rail_button(
                        "rail-query",
                        Icon::Query,
                        active_pane == Some(Pane::Query),
                        cx.listener(|this, _, window, cx| {
                            if let Some(session_id) = this.active_session_id() {
                                this.add_query_tab_for(session_id, window, cx);
                            }
                        }),
                    )),
            )
            .child(
                div()
                    .mb(px(11.))
                    .size(px(7.))
                    .rounded_full()
                    .bg(THEME.success),
            )
    }

    fn set_active_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.connection_picker_open = false;
        let Some(session_id) = self.active_session_id else {
            return;
        };
        match pane {
            Pane::Data => {
                if let Some(session) = self.session_mut(session_id) {
                    session.active_secondary_tab = None;
                    session.pane = Pane::Data;
                }
            }
            Pane::Query => return,
            Pane::Structure => {
                let table = self.session(session_id).and_then(|session| {
                    session.selected_table.as_ref().and_then(|selected| {
                        session
                            .tables
                            .iter()
                            .find(|table| table_ref(table) == *selected)
                            .cloned()
                    })
                });
                if let Some(table) = table {
                    self.open_structure_tab_for(session_id, table, cx);
                    return;
                }
                if let Some(session) = self.session_mut(session_id) {
                    session.status = "Select a table before opening its structure".into();
                }
            }
        }
        cx.notify();
    }

    fn render_table_context_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = self.table_context_menu.clone() else {
            return div().into_any_element();
        };
        let destructive_enabled = self.session(menu.session_id).is_some_and(|session| {
            session.kind.is_sql()
                && !session.busy
                && session.engine.is_some()
                && menu.table.kind == EntityKind::Table
        });
        let open_table = menu.table.clone();
        let open_structure = menu.table.clone();
        let refresh_table = menu.table.clone();
        let truncate_table = menu.table.clone();
        let drop_table = menu.table.clone();
        let session_id = menu.session_id;

        deferred(
            anchored()
                .position(menu.position)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("table-context-menu")
                        .w(px(220.))
                        .p(px(6.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(THEME.border_strong)
                        .bg(THEME.panel_raised)
                        .text_size(px(12.))
                        .on_mouse_down_out(
                            cx.listener(|this, _, _, cx| this.close_table_context_menu(cx)),
                        )
                        .child(
                            div()
                                .id("context-open-structure")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.accent_soft))
                                .child("Open structure")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.table_context_menu = None;
                                    this.open_structure_tab_for(
                                        session_id,
                                        open_structure.clone(),
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .text_size(px(10.))
                                .text_color(THEME.text_muted)
                                .child(table_sidebar_label(&menu.table, None)),
                        )
                        .child(
                            div()
                                .id("context-open-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.accent_soft))
                                .child("Open")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.table_context_menu = None;
                                    this.select_table_for(
                                        session_id,
                                        open_table.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            div()
                                .id("context-refresh-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.accent_soft))
                                .child("Refresh table")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.table_context_menu = None;
                                    this.select_table_for(
                                        session_id,
                                        refresh_table.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(div().my(px(4.)).border_t_1().border_color(THEME.border))
                        .child(
                            div()
                                .id("context-truncate-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if destructive_enabled {
                                    THEME.warning
                                } else {
                                    THEME.text_muted
                                })
                                .when(destructive_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(THEME.accent_soft))
                                })
                                .child("Truncate table…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if destructive_enabled {
                                        this.confirm_table_action(
                                            TableAction::Truncate,
                                            session_id,
                                            truncate_table.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        )
                        .child(
                            div()
                                .id("context-delete-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if destructive_enabled {
                                    THEME.danger
                                } else {
                                    THEME.text_muted
                                })
                                .when(destructive_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(THEME.accent_soft))
                                })
                                .child("Delete table…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if destructive_enabled {
                                        this.confirm_table_action(
                                            TableAction::Drop,
                                            session_id,
                                            drop_table.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        ),
                ),
        )
        .with_priority(10)
        .into_any_element()
    }

    fn render_topbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_active_session = self.active_session().is_some();
        let connected = self
            .active_session()
            .is_some_and(|session| session.engine.is_some());
        div()
            .h(px(42.))
            .flex_none()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(THEME.border)
            .bg(THEME.rail)
            .child(
                div()
                    .w(if self.compact_layout {
                        px(96.)
                    } else {
                        px(122.)
                    })
                    .flex_none()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(img(self.logo.clone()).id("topbar-logo").size(px(18.)))
                    .child(
                        div()
                            .id("window-title-drag")
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .window_control_area(WindowControlArea::Drag)
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.start_window_move();
                            })
                            .on_double_click(|_, window, _| window.zoom_window())
                            .text_size(px(15.))
                            .font_weight(FontWeight::BOLD)
                            .child("DBX"),
                    ),
            )
            .child(self.render_connection_tabs(cx))
            .child(
                div()
                    .id("window-title-drag-spacer")
                    .w(if self.compact_layout {
                        px(24.)
                    } else {
                        px(48.)
                    })
                    .h_full()
                    .flex_none()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .on_double_click(|_, window, _| window.zoom_window()),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .when(connected && !self.compact_layout, |view| {
                        view.child(self.rail_button(
                            "refresh",
                            Icon::Refresh,
                            false,
                            cx.listener(|this, _, _, cx| this.refresh_table(cx)),
                        ))
                    })
                    .child(
                        button("connections", "New connection", ButtonKind::Primary)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx))),
                    )
                    .when(!has_active_session, |view| {
                        view.child(
                            div()
                                .ml(px(2.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(THEME.text_muted),
                        )
                    })
                    .child(window_close_button().on_click(|_, window, cx| {
                        cx.stop_propagation();
                        window.remove_window();
                    })),
            )
    }

    fn render_connection_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_session_id = self.active_session_id();
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                (
                    session.id,
                    if session.name.trim().is_empty() {
                        format!("{} {}", session.kind, index + 1)
                    } else {
                        session.name.clone()
                    },
                    session.busy,
                    session.kind,
                    session.profile_id.is_some(),
                    session.environment,
                )
            })
            .collect();
        div()
            .id("connection-tabs-scroll")
            .flex_1()
            .min_w_0()
            .px(px(6.))
            .h_full()
            .flex()
            .items_end()
            .gap(px(3.))
            .overflow_scroll()
            .children(sessions.into_iter().map(
                |(session_id, label, busy, kind, saved, environment)| {
                    let selected = active_session_id == Some(session_id);
                    connection_tab(kind, label, selected)
                        .id(SharedString::from(format!("connection-tab-{session_id}")))
                        .flex_none()
                        .cursor_pointer()
                        .child(div().size(px(5.)).rounded_full().bg(if busy {
                            THEME.warning
                        } else {
                            THEME.success
                        }))
                        .when(saved, |tab| tab.child(environment_badge(environment)))
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "close-connection-tab-{session_id}"
                                )))
                                .size(px(18.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.panel_raised))
                                .child(icon(Icon::Close, THEME.text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.close_session(session_id, cx)
                                })),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.activate_session(session_id, cx)
                            }),
                        )
                },
            ))
            .child(
                div()
                    .id("add-connection-tab")
                    .flex_none()
                    .mb(px(5.))
                    .size(px(26.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(THEME.panel_raised))
                    .child(icon(Icon::Add, THEME.accent))
                    .on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx))),
            )
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((kind, tables, databases, current_database, selected_schema, selected_table)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.tables.clone(),
                    session.databases.clone(),
                    session.current_database.clone(),
                    session.schema_filter.clone(),
                    session.selected_table.clone(),
                )
            })
        else {
            return div().into_any_element();
        };
        let schema_options = schema_filter_options(kind, &tables);
        let visible_tables = schema_filtered_tables(kind, &tables, selected_schema.as_deref());
        div()
            .w(if self.compact_layout {
                px(180.)
            } else {
                px(224.)
            })
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .child(
                div()
                    .px(px(10.))
                    .py(px(7.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .border_b_1()
                    .border_color(THEME.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .child(icon(Icon::Search, THEME.text_muted))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(THEME.text_muted)
                                            .child(if kind == DatabaseKind::Redis {
                                                "KEYSPACE"
                                            } else {
                                                "EXPLORER"
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .id("refresh-tables")
                                            .size(px(24.))
                                            .rounded(px(5.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|style| style.bg(THEME.panel_raised))
                                            .child(icon(Icon::Refresh, THEME.accent))
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                this.refresh_tables_for(session_id, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("create-table")
                                            .size(px(24.))
                                            .rounded(px(5.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|style| style.bg(THEME.panel_raised))
                                            .child(icon(Icon::Add, THEME.accent))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.create_table_template_for(
                                                    session_id, window, cx,
                                                )
                                            })),
                                    ),
                            ),
                    )
                    .when(databases.len() > 1, |view| {
                        view.child(
                            div()
                                .id("database-switcher-scroll")
                                .flex()
                                .gap(px(4.))
                                .overflow_x_scroll()
                                .children(databases.into_iter().map(|database| {
                                    let selected =
                                        current_database.as_deref() == Some(database.as_str());
                                    let label = if kind == DatabaseKind::Redis {
                                        format!("db{database}")
                                    } else {
                                        database.clone()
                                    };
                                    div()
                                        .id(SharedString::from(format!("db-{database}")))
                                        .px(px(7.))
                                        .py(px(3.))
                                        .rounded(px(4.))
                                        .bg(if selected {
                                            THEME.accent_soft
                                        } else {
                                            THEME.panel_raised
                                        })
                                        .text_color(if selected {
                                            THEME.accent
                                        } else {
                                            THEME.text_muted
                                        })
                                        .text_size(px(9.))
                                        .cursor_pointer()
                                        .hover(|style| {
                                            style.bg(THEME.panel_raised).text_color(THEME.text)
                                        })
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.switch_database_for(
                                                session_id,
                                                database.clone(),
                                                cx,
                                            )
                                        }))
                                })),
                        )
                    })
                    .when(kind == DatabaseKind::PostgreSQL, |view| {
                        view.child(
                            div()
                                .id("schema-filter-scroll")
                                .flex()
                                .gap(px(4.))
                                .overflow_x_scroll()
                                .children(schema_options.into_iter().map(|schema| {
                                    let selected = selected_schema.as_deref() == schema.as_deref();
                                    let label = schema.as_deref().unwrap_or("All").to_owned();
                                    let schema_id = schema_filter_id(schema.as_deref());
                                    div()
                                        .id(SharedString::from(schema_id))
                                        .px(px(7.))
                                        .py(px(3.))
                                        .rounded(px(4.))
                                        .bg(if selected {
                                            THEME.accent_soft
                                        } else {
                                            THEME.panel_raised
                                        })
                                        .text_color(if selected {
                                            THEME.accent
                                        } else {
                                            THEME.text_muted
                                        })
                                        .text_size(px(9.))
                                        .cursor_pointer()
                                        .hover(|style| {
                                            style.bg(THEME.panel_raised).text_color(THEME.text)
                                        })
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_schema_filter_for(
                                                session_id,
                                                schema.clone(),
                                                cx,
                                            )
                                        }))
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .py(px(5.))
                    .children(visible_tables.into_iter().map(|table| {
                        let selected = selected_table.as_ref().is_some_and(|current| {
                            current.name == table.name && current.schema == table.schema
                        });
                        let label = table_sidebar_label(&table, selected_schema.as_deref());
                        let menu_table = table.clone();
                        div()
                            .id(SharedString::from(table_sidebar_id(&table)))
                            .mx(px(5.))
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .bg(if selected {
                                THEME.accent_soft
                            } else {
                                THEME.panel
                            })
                            .text_color(if selected {
                                THEME.accent
                            } else {
                                THEME.text_muted
                            })
                            .text_size(px(11.))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .cursor_pointer()
                            .hover(|style| style.bg(THEME.panel_raised).text_color(THEME.text))
                            .child(icon(
                                if table.kind == EntityKind::Table {
                                    Icon::Table
                                } else {
                                    Icon::Search
                                },
                                if selected {
                                    THEME.accent
                                } else {
                                    THEME.text_muted
                                },
                            ))
                            .child(div().truncate().child(label))
                            .on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, window, cx| {
                                    if event.is_right_click() {
                                        this.open_table_context_menu(
                                            session_id,
                                            menu_table.clone(),
                                            event.position(),
                                            cx,
                                        );
                                    } else {
                                        this.select_table_for(
                                            session_id,
                                            table.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ))
                    })),
            )
            .into_any_element()
    }

    fn render_main(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = self
            .active_session()
            .map(|session| session.pane)
            .unwrap_or(Pane::Data);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx))
            .child(match pane {
                Pane::Data => self.render_data(cx).into_any_element(),
                Pane::Structure => self.render_structure(cx).into_any_element(),
                Pane::Query => self.render_query(window, cx).into_any_element(),
            })
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let session_id = self.active_session_id();
        let (active_secondary_tab, tabs) = self
            .active_session()
            .map(|session| {
                let mut query_number = 0;
                let tabs = session
                    .secondary_tabs
                    .iter()
                    .map(|tab| {
                        let label = match &tab.kind {
                            SecondaryTabKind::Query(_) => {
                                query_number += 1;
                                format!("Query {query_number}")
                            }
                            SecondaryTabKind::Structure(structure) => {
                                format!("{} structure", structure.table.name)
                            }
                        };
                        (
                            tab.id,
                            label,
                            matches!(&tab.kind, SecondaryTabKind::Query(_)),
                        )
                    })
                    .collect::<Vec<_>>();
                (session.active_secondary_tab, tabs)
            })
            .unwrap_or_default();
        div()
            .id("document-tabs")
            .h(px(36.))
            .px(px(9.))
            .flex()
            .min_w_0()
            .items_end()
            .gap(px(3.))
            .overflow_x_scroll()
            .border_b_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .child(
                div()
                    .id("document-data")
                    .h(px(31.))
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded_t(px(5.))
                    .border_1()
                    .border_color(if active_secondary_tab.is_none() {
                        THEME.border_strong
                    } else {
                        THEME.panel
                    })
                    .bg(if active_secondary_tab.is_none() {
                        THEME.canvas
                    } else {
                        THEME.panel
                    })
                    .text_color(if active_secondary_tab.is_none() {
                        THEME.text
                    } else {
                        THEME.text_muted
                    })
                    .text_size(px(11.))
                    .cursor_pointer()
                    .child(icon(
                        Icon::Table,
                        if active_secondary_tab.is_none() {
                            THEME.accent
                        } else {
                            THEME.text_muted
                        },
                    ))
                    .child("Data")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_active_pane(Pane::Data, cx)),
                    ),
            )
            .children(tabs.into_iter().map(|(tab_id, label, is_query)| {
                let selected = active_secondary_tab == Some(tab_id);
                div()
                    .id(SharedString::from(format!("document-{tab_id}")))
                    .h(px(31.))
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded_t(px(5.))
                    .border_1()
                    .border_color(if selected {
                        THEME.border_strong
                    } else {
                        THEME.panel
                    })
                    .bg(if selected { THEME.canvas } else { THEME.panel })
                    .text_color(if selected {
                        THEME.text
                    } else {
                        THEME.text_muted
                    })
                    .text_size(px(11.))
                    .cursor_pointer()
                    .child(icon(
                        if is_query {
                            Icon::Query
                        } else {
                            Icon::Structure
                        },
                        if selected {
                            THEME.accent
                        } else {
                            THEME.text_muted
                        },
                    ))
                    .child(label)
                    .child(
                        div()
                            .id(SharedString::from(format!("close-document-{tab_id}")))
                            .ml(px(2.))
                            .size(px(18.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(THEME.text_muted)
                            .hover(|style| style.bg(THEME.panel_raised).text_color(THEME.danger))
                            .child(icon(Icon::Close, THEME.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(session_id) = session_id {
                                    this.close_secondary_tab_for(session_id, tab_id, cx);
                                }
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(session_id) = session_id {
                            this.activate_secondary_tab_for(session_id, tab_id, cx);
                        }
                    }))
            }))
            .child(
                div()
                    .id("add-query-document")
                    .h(px(26.))
                    .w(px(26.))
                    .mb(px(2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_color(THEME.text_muted)
                    .cursor_pointer()
                    .hover(|style| style.bg(THEME.accent_soft).text_color(THEME.accent))
                    .child(icon(Icon::Add, THEME.text_muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(session_id) = session_id {
                            this.add_query_tab_for(session_id, window, cx);
                        }
                    })),
            )
    }

    fn render_data(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((kind, redis_filter_editor, can_mutate, can_delete, filter_rows, picker, columns)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.editors.filter_editor.clone(),
                    self.editable_table_for(session_id).is_some(),
                    self.editable_table_for(session_id).is_some() && session.selected_row.is_some(),
                    session
                        .filters
                        .rows()
                        .iter()
                        .map(|row| {
                            (
                                row.id,
                                row.selected_column.clone(),
                                row.operator,
                                row.editor.clone(),
                            )
                        })
                        .collect::<Vec<_>>(),
                    session.filter_picker,
                    session.table_columns.clone(),
                )
            })
        else {
            return div().into_any_element();
        };
        let has_filter_rows = !filter_rows.is_empty();
        let redis_filter_focus = redis_filter_editor.read(cx).focus_handle();
        let picker_panel = match picker {
            Some(FilterPicker {
                row_id,
                kind: FilterPickerKind::Column,
            }) => div()
                .id("filter-column-options")
                .max_h(px(150.))
                .overflow_y_scroll()
                .p(px(5.))
                .rounded(px(6.))
                .border_1()
                .border_color(THEME.border)
                .bg(THEME.panel_raised)
                .children(columns.iter().map(|column| {
                    let column_name = column.name.clone();
                    div()
                        .id(SharedString::from(format!(
                            "filter-column-{row_id}-{}",
                            column.name
                        )))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(THEME.accent_soft))
                        .flex()
                        .justify_between()
                        .child(column.name.clone())
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(THEME.text_muted)
                                .child(column.data_type.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_filter_column_for(session_id, row_id, column_name.clone(), cx)
                        }))
                }))
                .into_any_element(),
            Some(FilterPicker {
                row_id,
                kind: FilterPickerKind::Operator,
            }) => div()
                .id("filter-operator-options")
                .max_h(px(180.))
                .overflow_y_scroll()
                .p(px(5.))
                .rounded(px(6.))
                .border_1()
                .border_color(THEME.border)
                .bg(THEME.panel_raised)
                .children(filter_operator_options().iter().copied().map(|option| {
                    div()
                        .id(SharedString::from(format!(
                            "filter-operator-{row_id}-{:?}",
                            option.operator
                        )))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(THEME.accent_soft))
                        .child(option.label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_filter_operator_for(session_id, row_id, option.operator, cx)
                        }))
                }))
                .into_any_element(),
            None => div().into_any_element(),
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .border_b_1()
                    .border_color(THEME.border)
                    .bg(THEME.panel)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_size(px(9.)).text_color(THEME.text_muted).child(
                                if kind.is_sql() {
                                    "FILTERS · ALL RULES MUST MATCH"
                                } else {
                                    "KEY PATTERN"
                                },
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .when(kind.is_sql(), |view| {
                                        view.child(self.small_button(
                                            "add-filter",
                                            "+ Filter",
                                            cx.listener(move |this, _, window, cx| {
                                                this.add_filter_for(session_id, window, cx)
                                            }),
                                        ))
                                        .child(
                                            self.small_button(
                                                "clear-filters",
                                                "Clear",
                                                cx.listener(move |this, _, _, cx| {
                                                    this.clear_filters_for(session_id, cx)
                                                }),
                                            ),
                                        )
                                    })
                                    .child(self.small_button(
                                        "apply-filter",
                                        "Apply",
                                        cx.listener(move |this, _, _, cx| {
                                            this.refresh_table_for(session_id, cx)
                                        }),
                                    ))
                                    .child(self.small_button_state(
                                        "add-row",
                                        "+ Row",
                                        can_mutate,
                                        cx.listener(move |this, _, window, cx| {
                                            this.begin_insert_for(session_id, window, cx)
                                        }),
                                    ))
                                    .child(self.small_button_state(
                                        "delete-row",
                                        "Delete",
                                        can_delete,
                                        cx.listener(move |this, _, _, cx| {
                                            this.delete_selected_for(session_id, cx)
                                        }),
                                    )),
                            ),
                    )
                    .when(!kind.is_sql(), |view| {
                        view.child(div().min_w_0().child(editor::input(
                            redis_filter_editor,
                            redis_filter_focus,
                            false,
                        )))
                    })
                    .when(kind.is_sql() && !has_filter_rows, |view| {
                        view.child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .text_size(px(11.))
                                .text_color(THEME.text_muted)
                                .child("No filters — all rows are shown."),
                        )
                    })
                    .when(kind.is_sql() && has_filter_rows, |view| {
                        view.child(
                            div()
                                .id("filter-rows-scroll")
                                .max_h(px(132.))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .children(filter_rows.into_iter().map(
                                    |(row_id, column, operator, value_editor)| {
                                        let value_focus = value_editor.read(cx).focus_handle();
                                        div()
                                            .id(SharedString::from(format!("filter-row-{row_id}")))
                                            .flex()
                                            .items_center()
                                            .gap(px(7.))
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "filter-column-button-{row_id}"
                                                    )))
                                                    .w(px(150.))
                                                    .h(px(32.))
                                                    .px(px(9.))
                                                    .flex()
                                                    .items_center()
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(THEME.border)
                                                    .bg(THEME.panel_raised)
                                                    .cursor_pointer()
                                                    .child(column)
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.toggle_filter_picker_for(
                                                                session_id,
                                                                row_id,
                                                                FilterPickerKind::Column,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "filter-operator-button-{row_id}"
                                                    )))
                                                    .w(px(170.))
                                                    .h(px(32.))
                                                    .px(px(9.))
                                                    .flex()
                                                    .items_center()
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(THEME.border)
                                                    .bg(THEME.panel_raised)
                                                    .cursor_pointer()
                                                    .child(operator_label(operator))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.toggle_filter_picker_for(
                                                                session_id,
                                                                row_id,
                                                                FilterPickerKind::Operator,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .when(
                                                        operator_requires_value(operator),
                                                        |view| {
                                                            view.child(editor::input(
                                                                value_editor,
                                                                value_focus,
                                                                false,
                                                            ))
                                                        },
                                                    )
                                                    .when(
                                                        !operator_requires_value(operator),
                                                        |view| {
                                                            view.h(px(36.))
                                                                .px(px(9.))
                                                                .flex()
                                                                .items_center()
                                                                .rounded(px(6.))
                                                                .bg(THEME.panel_raised)
                                                                .text_color(THEME.text_muted)
                                                                .child("No value")
                                                        },
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "remove-filter-{row_id}"
                                                    )))
                                                    .px(px(9.))
                                                    .py(px(8.))
                                                    .rounded(px(5.))
                                                    .text_color(THEME.text_muted)
                                                    .cursor_pointer()
                                                    .hover(|style| style.text_color(THEME.danger))
                                                    .child("Remove")
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.remove_filter_for(
                                                                session_id, row_id, cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                    },
                                )),
                        )
                    })
                    .child(picker_panel),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_grid(cx))
                    .when(!self.narrow_workspace, |view| {
                        view.child(self.render_inspector(cx))
                    }),
            )
            .into_any_element()
    }

    fn render_grid(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((result_grid, has_result, busy)) = self.session(session_id).map(|session| {
            (
                session.data_grid.clone(),
                session.result.is_some(),
                session.busy,
            )
        }) else {
            return div().into_any_element();
        };

        if !has_result {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(THEME.text_muted)
                .child(if busy {
                    "Loading rows…"
                } else {
                    "Select a table to browse rows"
                })
                .into_any_element();
        }

        div()
            .id("grid")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                DataTable::new(&result_grid)
                    .with_size(px(30.))
                    .stripe(false)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .into_any_element()
    }

    fn render_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((read_only_result, can_save, draft_mode, draft_fields, static_fields)) =
            self.session(session_id).map(|session| {
                let can_mutate = self.editable_table_for(session_id).is_some();
                let draft_fields = session
                    .row_draft
                    .as_ref()
                    .map(|draft| {
                        draft
                            .fields()
                            .iter()
                            .map(|field| {
                                (
                                    field.id,
                                    field.column.name.clone(),
                                    field.column.data_type.clone(),
                                    field.column.nullable,
                                    field.column.primary_key,
                                    field.state,
                                    field.editor.clone(),
                                    field.enum_selector.clone(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let static_fields = session
                    .selected_row
                    .and_then(|row_index| session.result.as_ref()?.rows.get(row_index))
                    .and_then(|row| {
                        session.result.as_ref().map(|result| {
                            result
                                .columns
                                .iter()
                                .enumerate()
                                .map(|(index, column)| {
                                    let value = row.values.get(index);
                                    (
                                        column.name.clone(),
                                        column.data_type.clone(),
                                        value
                                            .map(ToString::to_string)
                                            .unwrap_or_else(|| "—".into()),
                                        value.is_some_and(|value| matches!(value, CellValue::Null)),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .unwrap_or_default();
                (
                    session.result.is_some() && session.result_table.is_none(),
                    can_mutate && session.row_draft.is_some(),
                    session.draft_mode,
                    draft_fields,
                    static_fields,
                )
            })
        else {
            return div().into_any_element();
        };
        let has_draft = !draft_fields.is_empty();
        div()
            .w(px(330.))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .border_l_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .child(
                div()
                    .px(px(14.))
                    .pt(px(14.))
                    .pb(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.text)
                            .child(match draft_mode {
                                DraftMode::Insert => "New row",
                                DraftMode::Update if has_draft => "Edit row",
                                DraftMode::Update => "Row details",
                            }),
                    )
                    .child(badge(
                        match draft_mode {
                            DraftMode::Insert => "INSERT",
                            DraftMode::Update if has_draft => "EDITING",
                            DraftMode::Update => "READ ONLY",
                        },
                        if draft_mode == DraftMode::Update && !has_draft {
                            THEME.text_muted
                        } else {
                            THEME.accent
                        },
                    )),
            )
            .child(
                div()
                    .id("row-fields-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(
                        !has_draft && static_fields.is_empty() && draft_mode == DraftMode::Update,
                        |view| {
                            view.child(
                                div()
                                    .px(px(5.))
                                    .py(px(12.))
                                    .text_size(px(12.))
                                    .text_color(THEME.text_muted)
                                    .child("Select a row to inspect all of its fields."),
                            )
                        },
                    )
                    .children(draft_fields.into_iter().map(
                        |(
                            field_id,
                            name,
                            data_type,
                            nullable,
                            primary_key,
                            state,
                            field_editor,
                            enum_selector,
                        )| {
                            let field_focus = field_editor.read(cx).focus_handle();
                            let is_enum = enum_selector.is_some();
                            let value_control = enum_selector
                                .map(|selector| {
                                    Select::new(&selector)
                                        .with_size(Size::Small)
                                        .w_full()
                                        .menu_max_h(px(220.))
                                        .text_size(px(11.))
                                        .bg(THEME.canvas)
                                        .border_color(THEME.border_strong)
                                        .text_color(THEME.text)
                                        .into_any_element()
                                })
                                .unwrap_or_else(|| {
                                    editor::input(field_editor, field_focus, false).into_any_element()
                                });
                            div()
                                .id(SharedString::from(format!("row-field-{field_id}")))
                                .px(px(9.))
                                .py(px(10.))
                                .border_b_1()
                                .border_color(THEME.border)
                                .flex()
                                .flex_col()
                                .gap(px(7.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(THEME.text)
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(9.))
                                                .text_color(THEME.text_muted)
                                                .child(format!(
                                                    "{} · {}{}",
                                                    if is_enum {
                                                        format!("enum · {data_type}")
                                                    } else {
                                                        data_type
                                                    },
                                                    if nullable { "nullable" } else { "required" },
                                                    if primary_key { " · primary key" } else { "" }
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .child(
                                            Button::new(("row-field-value", field_id))
                                                .label("Value")
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .outline()
                                                .selected(state == FieldValueState::Value)
                                                .bg(if state == FieldValueState::Value {
                                                    THEME.accent_soft
                                                } else {
                                                    THEME.panel_raised
                                                })
                                                .border_color(if state == FieldValueState::Value {
                                                    THEME.accent
                                                } else {
                                                    THEME.border
                                                })
                                                .text_color(if state == FieldValueState::Value {
                                                    THEME.accent
                                                } else {
                                                    THEME.text_muted
                                                })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.set_row_field_state_for(
                                                        session_id,
                                                        field_id,
                                                        FieldValueState::Value,
                                                        cx,
                                                    )
                                                })),
                                        )
                                        .when(draft_mode == DraftMode::Insert, |view| {
                                            view.child(
                                                Button::new(("row-field-default", field_id))
                                                    .label("Default")
                                                    .with_size(Size::XSmall)
                                                    .compact()
                                                    .outline()
                                                    .selected(state == FieldValueState::Default)
                                                    .bg(if state == FieldValueState::Default {
                                                        THEME.accent_soft
                                                    } else {
                                                        THEME.panel_raised
                                                    })
                                                    .border_color(if state == FieldValueState::Default {
                                                        THEME.accent
                                                    } else {
                                                        THEME.border
                                                    })
                                                    .text_color(if state == FieldValueState::Default {
                                                        THEME.accent
                                                    } else {
                                                        THEME.text_muted
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.set_row_field_state_for(
                                                                session_id,
                                                                field_id,
                                                                FieldValueState::Default,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        })
                                        .when(nullable, |view| {
                                            view.child(
                                                Button::new(("row-field-null", field_id))
                                                    .label("NULL")
                                                    .with_size(Size::XSmall)
                                                    .compact()
                                                    .outline()
                                                    .selected(state == FieldValueState::Null)
                                                    .bg(if state == FieldValueState::Null {
                                                        THEME.accent_soft
                                                    } else {
                                                        THEME.panel_raised
                                                    })
                                                    .border_color(if state == FieldValueState::Null {
                                                        THEME.accent
                                                    } else {
                                                        THEME.border
                                                    })
                                                    .text_color(if state == FieldValueState::Null {
                                                        THEME.accent
                                                    } else {
                                                        THEME.text_muted
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.set_row_field_state_for(
                                                                session_id,
                                                                field_id,
                                                                FieldValueState::Null,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                                .when(state == FieldValueState::Value, |view| {
                                    view.child(value_control)
                                })
                                .when(state != FieldValueState::Value, |view| {
                                    view.child(
                                        div()
                                            .h(px(36.))
                                            .px(px(9.))
                                            .flex()
                                            .items_center()
                                            .rounded(px(6.))
                                            .bg(THEME.panel_raised)
                                            .text_size(px(11.))
                                            .text_color(THEME.text_muted)
                                            .child(if state == FieldValueState::Null {
                                                "This field will be saved as NULL."
                                            } else {
                                                "This column is omitted; the database supplies it."
                                            }),
                                    )
                                })
                        },
                    ))
                    .when(!has_draft, |view| {
                        view.children(static_fields.into_iter().map(
                            |(name, data_type, value, is_null)| {
                                div()
                                    .px(px(9.))
                                    .py(px(9.))
                                    .border_b_1()
                                    .border_color(THEME.border)
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(div().text_size(px(11.)).child(name))
                                            .child(
                                                div()
                                                    .text_size(px(9.))
                                                    .text_color(THEME.text_muted)
                                                    .child(data_type),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(if is_null {
                                                THEME.text_muted
                                            } else {
                                                THEME.text
                                            })
                                            .child(value),
                                    )
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .p(px(12.))
                    .border_t_1()
                    .border_color(THEME.border)
                    .flex()
                    .flex_col()
                    .gap(px(9.))
                    .child(div().text_size(px(10.)).text_color(THEME.text_muted).child(
                        if read_only_result {
                            "Query results are read-only."
                        } else {
                            match draft_mode {
                                DraftMode::Insert => {
                                    "Choose Value or NULL for fields to send; Default omits the column."
                                }
                                DraftMode::Update => {
                                    "All changed fields save together using the original primary key."
                                }
                            }
                        },
                    ))
                    .when(has_draft, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(self.small_button(
                                    "cancel-row-draft",
                                    "Cancel",
                                    cx.listener(move |this, _, _, cx| {
                                        this.cancel_row_draft_for(session_id, cx)
                                    }),
                                ))
                                .child(self.small_button_state(
                                    "save-row",
                                    if draft_mode == DraftMode::Insert {
                                        "Insert row"
                                    } else {
                                        "Save row"
                                    },
                                    can_save,
                                    cx.listener(move |this, _, _, cx| {
                                        this.save_draft_for(session_id, cx)
                                    }),
                                )),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_structure(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (session_id, table_name, table_columns, foreign_keys, tables, busy, error) = self
            .active_session()
            .and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Structure(structure) = &tab.kind else {
                    return None;
                };
                Some((
                    session.id,
                    table_ref_label(&structure.table),
                    structure.columns.clone(),
                    structure.foreign_keys.clone(),
                    session.tables.clone(),
                    structure.busy,
                    structure.error.clone(),
                ))
            })
            .unwrap_or_else(|| {
                (
                    Uuid::nil(),
                    "Structure".into(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                    None,
                )
            });
        div()
            .id("structure-scroll")
            .flex_1()
            .overflow_y_scroll()
            .p(px(12.))
            .flex()
            .flex_col()
            .child(panel_header(
                table_name,
                if busy {
                    "Loading metadata…".into()
                } else {
                    format!(
                        "{} columns · {} foreign keys",
                        table_columns.len(),
                        foreign_keys.len()
                    )
                },
            ))
            .when(error.is_some(), |view| {
                view.child(
                    div()
                        .mt(px(8.))
                        .text_color(THEME.danger)
                        .child(error.clone().unwrap_or_default()),
                )
            })
            .child(
                div()
                    .h(px(34.))
                    .mt(px(10.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(THEME.border_strong)
                    .bg(THEME.panel_raised)
                    .text_size(px(9.))
                    .text_color(THEME.text_muted)
                    .child("COLUMN")
                    .child("TYPE / CONSTRAINTS"),
            )
            .children(table_columns.iter().map(|column| {
                div()
                    .h(px(34.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(THEME.border)
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(THEME.text_muted)
                            .child(format!(
                                "{}{}{}",
                                column.data_type,
                                if column.nullable {
                                    " · nullable"
                                } else {
                                    " · required"
                                },
                                if column.primary_key {
                                    " · primary key"
                                } else {
                                    ""
                                }
                            )),
                    )
            }))
            .child(div().mt(px(18.)).child(panel_header(
                "Foreign keys",
                format!("{} constraints", foreign_keys.len()),
            )))
            .when(
                foreign_keys.is_empty() && !busy && error.is_none(),
                |view| {
                    view.child(
                        div()
                            .mt(px(8.))
                            .px(px(10.))
                            .py(px(12.))
                            .border_1()
                            .border_color(THEME.border)
                            .rounded(px(6.))
                            .text_size(px(11.))
                            .text_color(THEME.text_muted)
                            .child("No foreign-key constraints on this table."),
                    )
                },
            )
            .children(foreign_keys.iter().enumerate().map(|(index, foreign_key)| {
                let source = foreign_key.columns.join(", ");
                let target_table = match &foreign_key.referenced_schema {
                    Some(schema) => format!("{schema}.{}", foreign_key.referenced_table),
                    None => foreign_key.referenced_table.clone(),
                };
                let target = format!(
                    "{} ({})",
                    target_table,
                    foreign_key.referenced_columns.join(", ")
                );
                let actions = foreign_key_actions(foreign_key);
                let can_navigate = foreign_key_target_table(&tables, foreign_key).is_some();
                let foreign_key = foreign_key.clone();
                div()
                    .min_h(px(44.))
                    .px(px(10.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(THEME.border)
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        foreign_key
                                            .constraint_name
                                            .clone()
                                            .unwrap_or_else(|| "Unnamed constraint".into()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(THEME.text_muted)
                                    .child(source),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "foreign-key-target-{session_id}-{index}"
                            )))
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(3.))
                            .when(can_navigate, |view| {
                                view.cursor_pointer()
                                    .hover(|style| style.text_color(THEME.text))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigate_to_foreign_key_for(
                                            session_id,
                                            foreign_key.clone(),
                                            window,
                                            cx,
                                        )
                                    }))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(if can_navigate {
                                                THEME.accent
                                            } else {
                                                THEME.text_muted
                                            })
                                            .child(format!("REFERENCES {target}")),
                                    )
                                    .when(can_navigate, |view| {
                                        view.child(icon(Icon::ArrowRight, THEME.accent))
                                    }),
                            )
                            .when(!actions.is_empty(), |view| {
                                view.child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(THEME.text_muted)
                                        .child(actions),
                                )
                            }),
                    )
            }))
    }

    fn render_query_grid(
        result_grid: Entity<TableState<ResultTableDelegate>>,
        has_result: bool,
        busy: bool,
    ) -> AnyElement {
        if !has_result {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(THEME.text_muted)
                .child(if busy {
                    "Running query…"
                } else {
                    "Run a query to see rows"
                })
                .into_any_element();
        }

        div()
            .id("query-grid")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                DataTable::new(&result_grid)
                    .with_size(px(30.))
                    .stripe(false)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .into_any_element()
    }

    fn render_sql_completion(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        menu: SqlCompletionMenu,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = menu.selected;
        let rows = menu.items.iter().enumerate().map(|(index, item)| {
            let item = item.clone();
            let context = menu.context.clone();
            let item_kind = item.kind;
            div()
                .id(SharedString::from(format!(
                    "sql-completion-{session_id}-{tab_id}-{index}"
                )))
                .h(px(28.))
                .px(px(8.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .bg(if index == selected {
                    THEME.accent_soft
                } else {
                    THEME.panel_raised
                })
                .hover(|style| style.bg(THEME.accent_soft))
                .child(
                    div()
                        .w(px(52.))
                        .flex_none()
                        .text_size(px(9.))
                        .text_color(item_kind.color())
                        .child(item_kind.label()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(THEME.text)
                        .child(item.label.clone()),
                )
                .child(
                    div()
                        .max_w(px(190.))
                        .truncate()
                        .text_size(px(10.))
                        .text_color(THEME.text_muted)
                        .child(item.detail.clone()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.accept_completion_for(
                        session_id,
                        tab_id,
                        context.clone(),
                        item.clone(),
                        window,
                        cx,
                    );
                }))
        });

        deferred(
            div()
                .id("sql-completion-menu")
                .absolute()
                .left(px(10.))
                .right(px(10.))
                .top(px(214.))
                .max_h(px(270.))
                .p(px(5.))
                .rounded(px(7.))
                .border_1()
                .border_color(THEME.border_strong)
                .bg(THEME.panel_raised)
                .text_size(px(12.))
                .children(rows)
                .child(
                    div()
                        .mt(px(4.))
                        .pt(px(5.))
                        .px(px(8.))
                        .border_t_1()
                        .border_color(THEME.border)
                        .text_size(px(9.))
                        .text_color(THEME.text_muted)
                        .child("↑↓ navigate · Tab/Enter insert · Esc dismiss"),
                ),
        )
        .with_priority(20)
        .into_any_element()
    }

    fn render_query(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((tab_id, query_editor, busy, has_result, result_grid)) =
            self.session(session_id).and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Query(query) = &tab.kind else {
                    return None;
                };
                Some((
                    tab_id,
                    query.query_editor.clone(),
                    query.busy,
                    query.result.is_some(),
                    query.result_grid.clone(),
                ))
            })
        else {
            return div().into_any_element();
        };
        let query_focus = query_editor.read(cx).focus_handle();
        let completion = query_focus
            .is_focused(window)
            .then(|| self.query_completion_for(session_id, cx))
            .flatten();
        let completion_element =
            completion.map(|menu| self.render_sql_completion(session_id, tab_id, menu, cx));
        let completion_key_listener = cx.listener(move |this, event, window, cx| {
            this.handle_completion_key(session_id, event, window, cx)
        });
        let mut editor_panel = div()
            .relative()
            .h(px(224.))
            .p(px(10.))
            .border_b_1()
            .border_color(THEME.border)
            .capture_key_down(completion_key_listener)
            .child(editor::input(query_editor, query_focus, true));
        if let Some(completion_element) = completion_element {
            editor_panel = editor_panel.child(completion_element);
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(38.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(THEME.border)
                    .bg(THEME.panel)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(icon(Icon::Query, THEME.accent))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("SQL editor"),
                            ),
                    )
                    .child(
                        button(
                            "run-query",
                            if busy { "Running…" } else { "Run  ⌘↵" },
                            ButtonKind::Primary,
                        )
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.run_query_for(session_id, cx)),
                        ),
                    ),
            )
            .child(editor_panel)
            .child(Self::render_query_grid(result_grid, has_result, busy))
            .into_any_element()
    }

    fn render_status(&self) -> impl IntoElement {
        let (error, status, result) = self
            .active_session()
            .map(|session| {
                if let Some((error, status, result)) =
                    session.active_secondary_tab.and_then(|tab_id| {
                        session
                            .secondary_tabs
                            .iter()
                            .find(|tab| tab.id == tab_id)
                            .and_then(|tab| {
                                let SecondaryTabKind::Query(query) = &tab.kind else {
                                    return None;
                                };
                                Some((
                                    query.error.clone(),
                                    query.status.clone(),
                                    query.result.clone(),
                                ))
                            })
                    })
                {
                    return (error, status, result);
                }
                (
                    session.error.clone(),
                    session.status.clone(),
                    session.result.clone(),
                )
            })
            .unwrap_or_else(|| (self.error.clone(), self.status.clone(), None));
        div()
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .text_size(px(10.))
            .text_color(THEME.text_muted)
            .child(error.unwrap_or(status))
            .child(
                result
                    .as_ref()
                    .map(|result| format!("{} rows · limit 10,000", result.rows.len()))
                    .unwrap_or_default(),
            )
    }

    fn small_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        self.small_button_state(id, label, true, listener)
    }

    fn small_button_state(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        enabled: bool,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        Button::new(id)
            .label(label)
            .with_size(Size::Small)
            .compact()
            .outline()
            .disabled(!enabled)
            .border_color(THEME.border)
            .bg(if enabled {
                THEME.panel_raised
            } else {
                THEME.panel
            })
            .text_color(if enabled {
                THEME.text
            } else {
                THEME.text_muted
            })
            .when(enabled, |view| view.cursor_pointer())
            .on_click(listener)
    }

    fn rail_button(
        &self,
        id: &'static str,
        kind: Icon,
        selected: bool,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(32.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .bg(if selected {
                THEME.accent_soft
            } else {
                THEME.rail
            })
            .cursor_pointer()
            .hover(|style| style.bg(THEME.panel_raised))
            .child(icon(
                kind,
                if selected {
                    THEME.accent
                } else {
                    THEME.text_muted
                },
            ))
            .on_click(listener)
    }
}

impl Render for DbxApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.compact_layout = window.bounds().size.width < px(900.);
        self.narrow_workspace = window.bounds().size.width < px(1180.);
        let content = if self.connection_picker_open || self.active_session().is_none() {
            self.render_connection(cx).into_any_element()
        } else {
            self.render_workspace(window, cx).into_any_element()
        };
        // The pane rail only makes sense once a connection is live.
        let connected = self
            .active_session()
            .is_some_and(|session| session.engine.is_some());
        div()
            .size_full()
            .flex()
            .bg(THEME.canvas)
            .text_color(THEME.text)
            .on_action(cx.listener(Self::run_query_action))
            .on_action(cx.listener(Self::refresh_action))
            .when(connected, |view| view.child(self.render_app_rail(cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_topbar(cx))
                    .child(content),
            )
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

const MAX_SQL_COMPLETIONS: usize = 10;

fn sql_completion_items(
    query_text: &str,
    cursor: usize,
    context: &editor::SqlCompletionContext,
    sources: SqlCompletionSources<'_>,
) -> Vec<SqlCompletionItem> {
    let index = infer_sql_query_index(query_text, cursor, &sources);
    let area = sql_completion_area(query_text, cursor, context, &index);
    let SqlCompletionSources {
        database_kind,
        tables,
        completion_columns,
        selected_table,
        active_columns,
        result,
        active_schema_filter,
    } = sources;
    let prefix = context.prefix.trim_matches(['"', '`']).to_ascii_lowercase();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let mut push = |item: SqlCompletionItem| {
        let matches_prefix = prefix.is_empty()
            || item
                .search_text
                .to_ascii_lowercase()
                .split_whitespace()
                .any(|candidate| candidate.starts_with(prefix.as_str()));
        let key = item.insert_text.to_ascii_lowercase();
        if matches_prefix && seen.insert(key) {
            items.push(item);
        }
    };

    match area {
        SqlCompletionArea::General => {
            push_sql_keywords(&mut push);
            if sql_is_create_table_columns(query_text, cursor) {
                push_sql_types(&mut push);
            }
        }
        SqlCompletionArea::Type => push_sql_types(&mut push),
        SqlCompletionArea::Table => {
            push_table_candidates(
                &mut push,
                &index,
                tables,
                database_kind,
                context,
                active_schema_filter,
            );
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
        }
    }

    items.sort_by(|left, right| {
        let left_exact = left.label.eq_ignore_ascii_case(&prefix);
        let right_exact = right.label.eq_ignore_ascii_case(&prefix);
        right_exact.cmp(&left_exact).then_with(|| {
            left.label
                .to_ascii_lowercase()
                .cmp(&right.label.to_ascii_lowercase())
        })
    });
    items.truncate(MAX_SQL_COMPLETIONS);
    items
}

fn infer_sql_query_index(
    query_text: &str,
    cursor: usize,
    sources: &SqlCompletionSources<'_>,
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
    sources: &SqlCompletionSources<'_>,
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
    sources: &SqlCompletionSources<'_>,
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
    sources: &SqlCompletionSources<'_>,
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
    sources: &SqlCompletionSources<'_>,
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
    sources: &SqlCompletionSources<'_>,
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
    sources: &SqlCompletionSources<'_>,
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

fn push_sql_keywords(push: &mut impl FnMut(SqlCompletionItem)) {
    for keyword in editor::sql_completion_keywords() {
        push(SqlCompletionItem {
            label: (*keyword).into(),
            insert_text: (*keyword).into(),
            detail: "SQL keyword".into(),
            search_text: (*keyword).into(),
            kind: CompletionItemKind::Keyword,
        });
    }
}

fn push_sql_types(push: &mut impl FnMut(SqlCompletionItem)) {
    for sql_type in editor::sql_completion_types() {
        push(SqlCompletionItem {
            label: (*sql_type).into(),
            insert_text: (*sql_type).into(),
            detail: "SQL type".into(),
            search_text: (*sql_type).into(),
            kind: CompletionItemKind::Type,
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

fn completion_table_key(table: &TableRef) -> String {
    format!(
        "{}\u{0}{}",
        table.schema.as_deref().unwrap_or_default(),
        table.name
    )
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

fn foreign_key_target_table(
    tables: &[TableInfo],
    foreign_key: &ForeignKeyInfo,
) -> Option<TableInfo> {
    tables
        .iter()
        .find(|table| {
            table.name == foreign_key.referenced_table
                && match foreign_key.referenced_schema.as_deref() {
                    Some(schema) => table.schema.as_deref() == Some(schema),
                    None => true,
                }
        })
        .cloned()
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

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn redis_command_word(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_character_safe() {
        assert_eq!(truncate("éclair", 2), "éc…");
        assert_eq!(truncate("short", 10), "short");
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
    fn foreign_key_target_resolves_the_referenced_schema() {
        let tables = vec![
            TableInfo::table("users", Some("analytics".into())),
            TableInfo::table("users", Some("public".into())),
        ];
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("events_user_id_fkey".into()),
            columns: vec!["user_id".into()],
            referenced_schema: Some("analytics".into()),
            referenced_table: "users".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };

        assert_eq!(
            foreign_key_target_table(&tables, &foreign_key),
            Some(TableInfo::table("users", Some("analytics".into())))
        );
    }

    #[test]
    fn foreign_key_target_is_unavailable_when_the_table_is_not_listed() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: None,
            columns: vec!["owner_id".into()],
            referenced_schema: None,
            referenced_table: "owners".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };

        assert_eq!(foreign_key_target_table(&[], &foreign_key), None);
    }

    #[test]
    fn result_grid_marks_populated_foreign_key_cells_as_navigable() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("orders_customer_id_fkey".into()),
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };
        let tables = vec![TableInfo::table("customers", Some("public".into()))];
        let result = QueryResult {
            columns: vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("customer_id", 1, "INTEGER"),
            ],
            rows: vec![RowData::new(vec![
                CellValue::Integer(1),
                CellValue::Integer(42),
            ])],
            rows_affected: None,
            elapsed_ms: 0,
        };
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            &[foreign_key],
            &tables,
        );

        assert!(delegate.foreign_key_for_cell(0, 2).is_some());
    }

    #[test]
    fn result_grid_hides_foreign_key_action_for_null_values() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: None,
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };
        let tables = vec![TableInfo::table("customers", Some("public".into()))];
        let result = QueryResult {
            columns: vec![ColumnInfo::result("customer_id", 0, "INTEGER")],
            rows: vec![RowData::new(vec![CellValue::Null])],
            rows_affected: None,
            elapsed_ms: 0,
        };
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            &[foreign_key],
            &tables,
        );

        assert!(delegate.foreign_key_for_cell(0, 1).is_none());
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
            SqlCompletionSources {
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
            SqlCompletionSources {
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
            SqlCompletionSources {
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
        let sources = |query_result| SqlCompletionSources {
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
        let sources = || SqlCompletionSources {
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
