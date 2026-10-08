//! THESIS: DBX is a dense native database cockpit; it rejects the centered-card utility shell.
//! OWN-WORLD: Near-black layered panes, hairline borders, blue navigation, green health, 6px controls.
//! STORY: Pick or open a connection, keep its tab, browse context, then inspect or query without losing place.
//! FIRST VIEWPORT: A 46px rail, 40px primary connection tabs, explorer, data canvas, and row inspector.
//! FORM: Reference-led operator console, user-supplied DBX screen map; seed key: dbx-native-console.
//! FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md.
//! IMPLEMENTATION: `DbxApp` coordinates shared session state; focused workflows and rendering live in
//! the private `app/` module tree documented in `docs/architecture.md`.

mod agents;
mod cell_edits;
mod designer;
mod query_actions;
mod schema_objects;
mod session;
mod tabs;
use session::*;
mod backups;
mod connection;
mod data_clipboard;
mod data_import;
mod diagnostics;
mod find;
mod mcp;
mod profile_transfer;
mod query_parameters;
mod quick_open;
mod redis_completion;
mod result_table;
mod row_count;
mod sql_completion;
mod table_layout;
mod transfer;
mod value_view;
mod view;
mod workspace;

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

use dbx_core::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseEngine, DatabaseExportRequest, DatabaseKind,
    DumpFormat, EntityKind, Filter, FilterOperator, ForeignKeyInfo, InsertRequest, MutationValue,
    Order, OrderDirection, Page, QueryCancellation, QueryOptions, QueryResult, QuerySession,
    RedisCommandCatalog, ReferentialAction, RelationalSchema, RowData, StatementResult, TableInfo,
    TableRef, UpdateRequest, detect_file_format, export_database, export_table, import_database,
    import_file,
};
use gpui::{
    AnyElement, App, ClipboardItem, Context, CursorStyle, Decorations, Div, ElementId, Entity,
    FocusHandle, Focusable as _, FontWeight, Image, ImageFormat, IntoElement, KeyDownEvent,
    MouseButton, PathPromptOptions, Pixels, Point, Render, ResizeEdge, Rgba, ScrollHandle,
    SharedString, Stateful, StatefulInteractiveElement, Subscription, Window, WindowControlArea,
    WindowHandle, anchored, deferred, div, img, point, prelude::*, px, uniform_list,
};
use gpui_component::{
    Disableable as _, FocusTrapElement as _, IndexPath, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    resizable::ResizableState,
    select::{SearchableVec, Select, SelectEvent, SelectState},
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
        ConnectionProfileDraft, ConnectionTag, ProfileStore, SavedConnection, default_tags,
        sqlite_url,
    },
    query_history::{
        QueryHistoryConnection, QueryHistoryEntry, QueryHistoryOutcome, QueryHistoryStore,
    },
    row_drafts::{
        FieldId, FieldRow, FieldValueKind, FieldValueState, RowDraftModel, field_editor_text,
    },
    settings::{Settings, SettingsStore},
    theme::{
        Appearance, ButtonKind, FollowCorners, GLASS_INSET, Icon, RADIUS_CONTROL, RADIUS_GLASS,
        RADIUS_PANEL, appearance, badge, button, connection_tab, database_logo, glass,
        glass_icon_button, glass_raised, glass_shadow, icon, panel_header, reduce_transparency,
        segment, segmented_track, set_appearance, set_reduce_transparency, set_system_appearance,
        settings_group, settings_row, shortcut, sync_component_theme, theme, tip,
        window_background,
    },
    vault::{VaultError, VaultState},
};
use redis_completion::redis_completion_items;
use result_table::{ResultTableDelegate, foreign_key_target_table};
use sql_completion::{
    SqlCompletionItem, SqlCompletionRequest, completion_table_key, sql_completion_items,
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
        NewConnection,
        NewQuery,
        CloseTab,
        NextConnection,
        PreviousConnection,
        ToggleSidebar,
        CheckForUpdates,
        SubmitVault,
        ApplyFilters,
        CommitCellEdit,
        CommitCellEditNext,
        CommitCellEditPrevious,
        CancelCellEdit,
        SetCellNull,
        CommitChanges,
        DeleteRows,
        CopyDataSelection,
        PasteRows,
        OpenQuickOpen,
        QuickOpenNext,
        QuickOpenPrevious,
        QuickOpenConfirm,
        SubmitQueryParameters,
        CancelQueryParameters,
        OpenFind,
        OpenReplace,
        FindNext,
        FindPrevious,
        CloseFind,
        ToggleQueryAgent,
        SubmitQueryAgent,
        DismissQueryAgent
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
    Insert,
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
            Self::Insert => "sql",
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

/// "1 row", "3 rows": proper plurals instead of "row(s)".
pub(crate) fn counted(count: impl TryInto<u64>, singular: &str, plural: &str) -> String {
    let count = count.try_into().unwrap_or(u64::MAX);
    format!("{count} {}", if count == 1 { singular } else { plural })
}

/// The primary key to page by, when keyset paging can replace OFFSET: a
/// single-column key on a native SQL engine, with no sort or a sort on that
/// key. Composite keys and other sorts keep exact OFFSET paging.
fn keyset_column(
    kind: DatabaseKind,
    columns: &[ColumnInfo],
    sort: Option<&Order>,
) -> Option<Order> {
    if !supports_keyset_paging(kind) {
        return None;
    }
    let mut keys = columns.iter().filter(|column| column.primary_key);
    let key = keys.next()?;
    if keys.next().is_some() {
        return None;
    }
    match sort {
        None => Some(Order {
            column: key.name.clone(),
            direction: OrderDirection::Ascending,
        }),
        Some(order) if order.column == key.name => Some(order.clone()),
        Some(_) => None,
    }
}

fn supports_keyset_paging(kind: DatabaseKind) -> bool {
    matches!(
        kind,
        DatabaseKind::PostgreSQL
            | DatabaseKind::MySQL
            | DatabaseKind::SQLite
            | DatabaseKind::CockroachDB
            | DatabaseKind::DuckDB
            | DatabaseKind::Turso
            | DatabaseKind::CloudflareD1
            | DatabaseKind::SqlServer
    )
}

/// The seek position after a keyset-ordered page: its last primary key.
fn keyset_start(result: &QueryResult, order_by: &Order) -> Option<PageStart> {
    let index = result
        .columns
        .iter()
        .position(|column| column.name == order_by.column)?;
    let value = result.rows.last()?.values.get(index)?.clone();
    (value != CellValue::Null).then_some(PageStart::After(value))
}

fn trim_table_browse_result(result: &mut QueryResult) -> bool {
    let has_next_page = result.rows.len() > TABLE_BROWSE_PAGE_SIZE as usize;
    result.rows.truncate(TABLE_BROWSE_PAGE_SIZE as usize);
    has_next_page
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Error,
}

/// Transient, self-dismissing feedback for outcomes that have no other
/// visible trace (a passing connection test, a save). Persistent state and
/// form errors stay inline instead.
pub(crate) struct Toast {
    id: u64,
    kind: ToastKind,
    message: SharedString,
}

const MAX_TOASTS: usize = 3;

struct ConnectionDraft {
    read_only: bool,
    cloud_auth: Option<dbx_core::CloudAuthentication>,
    kind: DatabaseKind,
    mode: ConnectionFormMode,
    selected_profile: Option<Uuid>,
    /// New connections start on the type grid; the form follows a choice.
    choosing_kind: bool,
    import_url: Entity<String>,
    import_url_editor: Entity<TextEditor>,
    tag: Option<ConnectionTag>,
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
    transport: ConnectionTransportDraft,
}

struct ConnectionTransportDraft {
    socket_enabled: bool,
    ssh_enabled: bool,
    socket: Entity<String>,
    socket_editor: Entity<TextEditor>,
    ssh_host: Entity<String>,
    ssh_host_editor: Entity<TextEditor>,
    ssh_port: Entity<String>,
    ssh_port_editor: Entity<TextEditor>,
    ssh_user: Entity<String>,
    ssh_user_editor: Entity<TextEditor>,
    ssh_key: Entity<String>,
    ssh_key_editor: Entity<TextEditor>,
    ssh_jump: Entity<String>,
    ssh_jump_editor: Entity<TextEditor>,
    ssh_password: Entity<String>,
    ssh_password_editor: Entity<TextEditor>,
}

impl ConnectionTransportDraft {
    fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let mut field = |initial: &str| {
            let value = cx.new(|_| initial.to_owned());
            let editor = cx.new(|cx| TextEditor::new(value.clone(), false, window, cx));
            (value, editor)
        };
        let (socket, socket_editor) = field("");
        let (ssh_host, ssh_host_editor) = field("");
        let (ssh_port, ssh_port_editor) = field("22");
        let (ssh_user, ssh_user_editor) = field("");
        let (ssh_key, ssh_key_editor) = field("");
        let (ssh_jump, ssh_jump_editor) = field("");
        let ssh_password = cx.new(|_| String::new());
        let ssh_password_editor =
            cx.new(|cx| TextEditor::new(ssh_password.clone(), false, window, cx).password());
        Self {
            socket_enabled: false,
            ssh_enabled: false,
            socket,
            socket_editor,
            ssh_host,
            ssh_host_editor,
            ssh_port,
            ssh_port_editor,
            ssh_user,
            ssh_user_editor,
            ssh_key,
            ssh_key_editor,
            ssh_jump,
            ssh_jump_editor,
            ssh_password,
            ssh_password_editor,
        }
    }
}

/// The create/edit form for shared connection tags in Settings.
struct TagEditor {
    editing: Option<Uuid>,
    /// The new-tag row is open.
    creating: bool,
    /// The tag whose delete confirmation is showing.
    deleting: Option<Uuid>,
    name: Entity<String>,
    name_editor: Entity<TextEditor>,
    color: Entity<String>,
    color_editor: Entity<TextEditor>,
}

impl TagEditor {
    fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let name = cx.new(|_| String::new());
        let color = cx.new(|_| "82AAFF".to_owned());
        let name_editor = cx.new(|cx| TextEditor::new(name.clone(), false, window, cx));
        let color_editor = cx.new(|cx| TextEditor::new(color.clone(), false, window, cx));
        Self {
            editing: None,
            creating: false,
            deleting: None,
            name,
            name_editor,
            color,
            color_editor,
        }
    }
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
        let import_url = cx.new(|_| String::new());
        let import_url_editor = cx.new(|cx| TextEditor::new(import_url.clone(), false, window, cx));
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
            read_only: false,
            cloud_auth: None,
            mode: ConnectionFormMode::Details,
            selected_profile: None,
            choosing_kind: true,
            import_url,
            import_url_editor,
            tag: Some(default_tags().remove(3)),
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
            transport: ConnectionTransportDraft::new(window, cx),
        }
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
    /// SQL shown for review in a scrollable block below the detail.
    sql: Option<String>,
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
    LockVault,
    DeleteProfile {
        id: uuid::Uuid,
    },
    RunQuery {
        session_id: SessionId,
        tab_id: SecondaryTabId,
        run_all: bool,
        query: String,
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
    CommitChanges {
        session_id: SessionId,
        tab_id: SecondaryTabId,
    },
    Quit,
    DiscardDataTab {
        session_id: SessionId,
        tab_id: SecondaryTabId,
    },
    NativeRestore {
        session_id: SessionId,
        path: PathBuf,
        database: Option<String>,
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
    fn session_id(&self) -> Option<SessionId> {
        match self {
            Self::LockVault | Self::Quit | Self::DeleteProfile { .. } => None,
            Self::RunQuery { session_id, .. }
            | Self::CloseQuery { session_id, .. }
            | Self::ClearQueryHistory { session_id }
            | Self::Table { session_id, .. }
            | Self::CommitChanges { session_id, .. }
            | Self::DiscardDataTab { session_id, .. }
            | Self::NativeRestore { session_id, .. }
            | Self::DatabaseImport { session_id, .. }
            | Self::TableImport { session_id, .. } => Some(*session_id),
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SettingsSection {
    #[default]
    Appearance,
    QueryAgent,
    Tags,
    Updates,
}

pub struct DbxApp {
    runtime: Arc<tokio::runtime::Runtime>,
    update_state: crate::updater::UpdateState,
    logo: Arc<Image>,
    draft: ConnectionDraft,
    vault_editors: VaultEditors,
    vault_state: Option<VaultState>,
    vault_busy: bool,
    /// Whether a passphrase unlock also trusts this device, keeping the vault
    /// key in the system keychain so later launches skip the passphrase.
    remember_device: bool,
    saving_connection: bool,
    vault_generation: u64,
    credential_hydrating: bool,
    credential_hydration_generation: u64,
    credential_connect_window: Option<WindowHandle<DbxApp>>,
    profile_store: Option<ProfileStore>,
    saved_connections: Vec<SavedConnection>,
    connection_tags: Vec<ConnectionTag>,
    query_history_store: Option<QueryHistoryStore>,
    /// Newest-first cache for the history UI. Disk access is never performed
    /// from render or query completion on the GPUI thread.
    recent_query_history: Vec<QueryHistoryEntry>,
    workspace_store: Option<Arc<crate::workspace::WorkspaceStore>>,
    workspace_documents: HashMap<String, crate::workspace::WorkspaceDocument>,
    startup_recovery_started: bool,
    sessions: Vec<ConnectionSession>,
    active_session_id: Option<SessionId>,
    connection_picker_open: bool,
    compact_connection_form_open: bool,
    table_context_menu: Option<TableContextMenu>,
    database_export_dialog: Option<DatabaseExportDialog>,
    confirmation_dialog: Option<ConfirmationDialog>,
    quick_open: Option<quick_open::QuickOpen>,
    copied_table_data: Option<Arc<dbx_core::data_import::ImportData>>,
    data_import_dialog: Option<data_import::DataImportDialog>,
    profile_transfer_dialog: Option<profile_transfer::ProfileTransferDialog>,
    mutation_error_dialog: Option<MutationErrorDialog>,
    settings_open: bool,
    settings_section: SettingsSection,
    tag_editor: TagEditor,
    appearance: Appearance,
    reduce_transparency: bool,
    settings_store: Option<SettingsStore>,
    agent_setup: agents::AgentSetup,
    compact_layout: bool,
    narrow_workspace: bool,
    sidebar_hidden: bool,
    /// Owns keyboard focus whenever no control does, so app shortcuts keep
    /// working after the focused element (e.g. a closed tab's editor) is gone.
    focus_handle: FocusHandle,
    toasts: Vec<Toast>,
    next_toast_id: u64,
    window_drag_armed: bool,
    test_generation: u64,
    testing_connection: bool,
    connection_test_abort: AbortOnDrop,
    _subscriptions: Vec<Subscription>,
    error: Option<String>,
}

impl DbxApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = ConnectionDraft::new(window, cx);
        let tag_editor = TagEditor::new(window, cx);
        let vault_editors = VaultEditors::new(window, cx);

        let subscriptions = vec![
            cx.on_app_quit(|this, cx| {
                let _ = this.flush_query_workspaces(cx);
                async {}
            }),
            cx.observe(&draft.connection_name, |_, _, cx| cx.notify()),
            cx.observe(&draft.connection_url, |_, _, cx| cx.notify()),
            cx.observe(&draft.host, |_, _, cx| cx.notify()),
            cx.observe(&draft.port, |_, _, cx| cx.notify()),
            cx.observe(&draft.username, |_, _, cx| cx.notify()),
            cx.observe(&draft.password, |_, _, cx| cx.notify()),
            cx.observe(&draft.database, |_, _, cx| cx.notify()),
            cx.observe(&draft.transport.socket, |_, _, cx| cx.notify()),
            cx.observe(&draft.transport.ssh_host, |_, _, cx| cx.notify()),
            cx.observe(&draft.transport.ssh_port, |_, _, cx| cx.notify()),
            cx.observe(&draft.transport.ssh_user, |_, _, cx| cx.notify()),
            cx.observe(&draft.transport.ssh_key, |_, _, cx| cx.notify()),
            cx.observe(&draft.transport.ssh_jump, |_, _, cx| cx.notify()),
            // Follow live OS light/dark changes when the preference is System.
            cx.observe_window_appearance(window, |this, window, cx| {
                this.apply_material(window, cx);
                cx.notify();
            }),
        ];

        let (profile_store, saved_connections, profile_error) = match ProfileStore::new() {
            Ok(store) => match store.list() {
                Ok(profiles) => (Some(store), profiles, None),
                Err(error) => (Some(store), Vec::new(), Some(error.to_string())),
            },
            Err(error) => (None, Vec::new(), Some(error.to_string())),
        };
        let connection_tags = profile_store
            .as_ref()
            .and_then(|store| store.tags().ok())
            .unwrap_or_else(default_tags);
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
                    .map(|entries| entries.into_iter().rev().collect())
                    .unwrap_or_default();
                (Some(store), entries)
            }
            Err(error) => {
                eprintln!("DBX could not initialize query history: {error}");
                (None, Vec::new())
            }
        };

        let mut this = Self {
            runtime: Arc::new(tokio::runtime::Runtime::new().expect("create DBX Tokio runtime")),
            update_state: crate::updater::UpdateState::Idle,
            logo: Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_BYTES.to_vec())),
            draft,
            vault_editors,
            vault_state,
            vault_busy: false,
            remember_device: SettingsStore::new()
                .and_then(|store| store.load())
                .map_or(true, |settings| settings.remember_device),
            saving_connection: false,
            vault_generation: 0,
            credential_hydrating: false,
            credential_hydration_generation: 0,
            credential_connect_window: None,
            workspace_store: profile_store
                .as_ref()
                .and_then(ProfileStore::vault)
                .map(|vault| Arc::new(crate::workspace::WorkspaceStore::new(vault))),
            workspace_documents: HashMap::new(),
            startup_recovery_started: cfg!(test),
            profile_store,
            saved_connections,
            connection_tags,
            query_history_store,
            recent_query_history,
            sessions: Vec::new(),
            active_session_id: None,
            connection_picker_open: false,
            compact_connection_form_open,
            table_context_menu: None,
            database_export_dialog: None,
            confirmation_dialog: None,
            quick_open: None,
            profile_transfer_dialog: None,
            data_import_dialog: None,
            copied_table_data: None,
            mutation_error_dialog: None,
            settings_open: false,
            settings_section: SettingsSection::Appearance,
            tag_editor,
            appearance: appearance(),
            reduce_transparency: reduce_transparency(),
            settings_store: SettingsStore::new().ok(),
            agent_setup: agents::AgentSetup::new(window, cx),
            compact_layout: false,
            narrow_workspace: false,
            sidebar_hidden: false,
            focus_handle: cx.focus_handle(),
            toasts: Vec::new(),
            next_toast_id: 0,
            window_drag_armed: false,
            test_generation: 0,
            testing_connection: false,
            connection_test_abort: AbortOnDrop::default(),
            _subscriptions: subscriptions,
            error: profile_error,
        };
        this.try_device_unlock(cx);
        if !cfg!(test) && std::env::var_os("DBX_DISABLE_UPDATES").is_none() {
            this.check_for_updates(cx);
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(6 * 60 * 60))
                        .await;
                    this.update(cx, |this, cx| {
                        if matches!(
                            this.update_state,
                            crate::updater::UpdateState::Idle
                                | crate::updater::UpdateState::Current
                                | crate::updater::UpdateState::Failed(_)
                        ) {
                            this.check_for_updates(cx);
                        }
                    })?;
                }
                #[allow(unreachable_code)]
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        }
        this
    }

    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        use crate::updater::UpdateState;
        if matches!(
            self.update_state,
            UpdateState::Checking | UpdateState::Installing(_) | UpdateState::Installed(_)
        ) {
            return;
        }
        self.update_state = UpdateState::Checking;
        cx.notify();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime.spawn_blocking(crate::updater::check).await;
            this.update(cx, |this, cx| {
                this.update_state = match result {
                    Ok(Ok(Some(update))) => {
                        if !this.settings_open {
                            this.show_toast(
                                ToastKind::Info,
                                format!("DBX {} is available in Settings", update.version),
                                cx,
                            );
                        }
                        UpdateState::Available(update)
                    }
                    Ok(Ok(None)) => UpdateState::Current,
                    Ok(Err(error)) => UpdateState::Failed(format!("{error:#}")),
                    Err(error) => UpdateState::Failed(error.to_string()),
                };
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        self.settings_open = true;
        self.check_agent_cli(cx);
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.settings_open) {
            self.edit_tag(None, cx);
            cx.notify();
        }
    }

    pub(super) fn activate_update(&mut self, cx: &mut Context<Self>) {
        use crate::updater::UpdateState;
        match self.update_state.clone() {
            UpdateState::Available(update) => {
                self.update_state =
                    UpdateState::Installing(crate::updater::UpdateProgress::Checksum);
                cx.notify();
                let runtime = self.runtime.clone();
                let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
                cx.spawn(async move |this, cx| {
                    let mut task = runtime.spawn_blocking(move || {
                        crate::updater::install(&update, |progress| {
                            let _ = progress_tx.send(progress);
                        })
                    });
                    let result = loop {
                        tokio::select! {
                            result = &mut task => break result,
                            Some(progress) = progress_rx.recv() => {
                                this.update(cx, |this, cx| {
                                    this.update_state = UpdateState::Installing(progress);
                                    cx.notify();
                                })?;
                            }
                        }
                    };
                    this.update(cx, |this, cx| {
                        this.update_state = match result {
                            Ok(Ok(destination)) => UpdateState::Installed(destination),
                            Ok(Err(error)) => UpdateState::Failed(format!("{error:#}")),
                            Err(error) => UpdateState::Failed(error.to_string()),
                        };
                        match &this.update_state {
                            UpdateState::Installed(_) => this.show_toast(
                                ToastKind::Success,
                                "Update installed. Restart DBX from Settings to finish.",
                                cx,
                            ),
                            UpdateState::Failed(_) => this.show_toast(
                                ToastKind::Error,
                                "Update failed. Open Settings for the error and retry.",
                                cx,
                            ),
                            _ => {}
                        }
                        cx.notify();
                    })?;
                    Ok::<(), anyhow::Error>(())
                })
                .detach();
            }
            UpdateState::Installed(destination) => match crate::updater::restart(&destination) {
                Ok(()) => cx.quit(),
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                }
            },
            UpdateState::Checking | UpdateState::Installing(_) => {}
            _ => self.check_for_updates(cx),
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
        let policy = self
            .workspace_documents
            .get(&crate::workspace::connection_key(&connection));
        if policy.is_some_and(|document| document.history_disabled) {
            return;
        }
        let retention = policy
            .map(|document| document.history_retention)
            .filter(|limit| *limit > 0)
            .unwrap_or(100);
        let Some(store) = self.query_history_store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let entries = runtime
                .spawn_blocking(move || {
                    store.record(connection.clone(), query, outcome)?;
                    store.retain(&connection, retention)?;
                    store.load()
                })
                .await;
            if let Ok(Ok(entries)) = entries {
                this.update(cx, |this, _| {
                    this.recent_query_history = entries.into_iter().rev().collect();
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Recent entries for the current connection, newest first. This only
    /// reads the in-memory cache and is therefore safe to call while rendering.
    pub(super) fn recent_query_history_for(&self, session_id: SessionId) -> Vec<QueryHistoryEntry> {
        self.recent_query_history_limited(session_id, usize::MAX)
    }

    /// The newest `limit` entries for the session's connection. Render paths use
    /// this so a long history is not cloned every frame.
    pub(super) fn recent_query_history_limited(
        &self,
        session_id: SessionId,
        limit: usize,
    ) -> Vec<QueryHistoryEntry> {
        let Some(connection) = self.session(session_id).and_then(query_history_connection) else {
            return Vec::new();
        };
        self.recent_query_history
            .iter()
            .filter(|entry| entry.connection == connection)
            .take(limit)
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
            detail: "Saved history for this connection will be removed. Open tabs stay.".into(),
            confirm_label: "Clear history",
            tone: ConfirmationTone::Warning,
            action: ConfirmationAction::ClearQueryHistory { session_id },
            focus: focus.clone(),
            return_focus,
            sql: None,
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
            this.update(cx, |this, cx| match cleared {
                Ok(Ok(count)) => {
                    this.recent_query_history
                        .retain(|entry| entry.connection != retained_connection);
                    this.show_toast(
                        ToastKind::Success,
                        format!(
                            "Cleared {}",
                            counted(count, "history entry", "history entries")
                        ),
                        cx,
                    );
                }
                Ok(Err(error)) => this.show_toast(
                    ToastKind::Error,
                    format!("Could not clear query history: {error}"),
                    cx,
                ),
                Err(error) => this.show_toast(
                    ToastKind::Error,
                    format!("Query history task stopped unexpectedly: {error}"),
                    cx,
                ),
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(crate) fn set_appearance_preference(
        &mut self,
        next: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = next;
        set_appearance(next);
        self.apply_material(window, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn toggle_reduce_transparency(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reduce_transparency = !self.reduce_transparency;
        set_reduce_transparency(self.reduce_transparency);
        self.apply_material(window, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Re-resolve the palette against the OS appearance and push it into the
    /// window backdrop and gpui-component.
    pub(crate) fn apply_material(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_system_appearance(window.appearance());
        window.set_background_appearance(window_background());
        sync_component_theme(Some(window), cx);
    }

    fn persist_settings(&mut self, cx: &mut Context<Self>) {
        let mut settings = Settings::new(self.appearance)
            .with_reduce_transparency(self.reduce_transparency)
            .with_remember_device(self.remember_device);
        settings.agents = self.agent_setup.preferences.clone();
        let failure = match &self.settings_store {
            Some(store) => store
                .save(settings)
                .err()
                .map(|error| format!("Couldn’t save preferences: {error}")),
            None => Some("Preference storage is unavailable".into()),
        };
        if let Some(message) = failure {
            self.agent_setup.save_error = Some(message.clone());
            self.show_toast(ToastKind::Error, message, cx);
        } else {
            self.agent_setup.save_error = None;
        }
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
        let dismissed = if self
            .data_import_dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.busy)
        {
            self.data_import_dialog = None;
            self.focus_handle.focus(window, cx);
            true
        } else if self
            .profile_transfer_dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.busy)
        {
            self.profile_transfer_dialog = None;
            self.focus_handle.focus(window, cx);
            true
        } else if self.quick_open.is_some() {
            self.close_quick_open(window, cx);
            true
        } else if self.dismiss_mutation_error_dialog(window, cx) {
            true
        } else if self.confirmation_dialog.is_some() {
            self.cancel_confirmation(window, cx);
            true
        } else {
            self.database_export_dialog.take().is_some()
                || std::mem::take(&mut self.settings_open)
                || self.table_context_menu.take().is_some()
                || self.dismiss_connection_picker()
        };
        if dismissed {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn default_query(kind: DatabaseKind) -> &'static str {
        kind.default_query()
    }

    /// Allow closing the window or quitting, or ask first when a table tab
    /// holds uncommitted changes.
    pub(crate) fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let changes = self
            .sessions
            .iter()
            .flat_map(|session| &session.secondary_tabs)
            .filter_map(|tab| match &tab.kind {
                SecondaryTabKind::Data(data) => Some(data.change_counts().total()),
                _ => None,
            })
            .sum::<usize>();
        if changes == 0 {
            return true;
        }
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: "Quit without committing?".into(),
            detail: format!(
                "{} remain in encrypted recovery. They will require review next time.",
                counted(changes, "staged change", "staged changes")
            ),
            confirm_label: "Quit",
            tone: ConfirmationTone::Danger,
            action: ConfirmationAction::Quit,
            focus: focus.clone(),
            return_focus: window.focused(cx),
            sql: None,
        });
        focus.focus(window, cx);
        cx.notify();
        false
    }

    fn close_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if self.vault_state == Some(VaultState::Unlocked)
            && self.session(session_id).is_some_and(|session| {
                session.secondary_tabs.iter().any(|tab| {
                matches!(&tab.kind, SecondaryTabKind::Data(data) if data.has_unsaved_cell_work())
            })
            })
        {
            self.show_toast(
                ToastKind::Info,
                "Save or discard cell edits before closing this connection",
                cx,
            );
            return;
        }
        self.persist_query_workspace_for(session_id, cx);
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
            .is_some_and(|dialog| dialog.action.session_id() == Some(session_id))
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
                SecondaryTabKind::Data(data) => data.invalidate_request(),
                SecondaryTabKind::Query(query) => query.invalidate_request(),
                SecondaryTabKind::Diagram(diagram) => diagram.invalidate_request(),
                SecondaryTabKind::Structure(_) => {}
            }
        }
        self.sessions.remove(index);
        if self.vault_state == Some(VaultState::Unlocked) {
            self.persist_startup_workspace(cx);
        }
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
            if self.confirmation_dialog.as_ref().is_some_and(|dialog| {
                dialog
                    .action
                    .session_id()
                    .is_some_and(|id| id != session_id)
            }) {
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
            self.settings_open = false;
            self.persist_startup_workspace(cx);
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
        tab_id: SecondaryTabId,
        field_id: Option<FieldId>,
        cx: &App,
    ) -> Option<FocusHandle> {
        let draft = self.data_tab(session_id, tab_id)?.row_draft.as_ref()?;
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

    fn select_schema_filter_for(
        &mut self,
        session_id: SessionId,
        schema: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        if session.kind.dialect() != DatabaseKind::PostgreSQL || session.schema_filter == schema {
            return;
        }

        // Open tabs outside the new schema stay open; the filter only narrows
        // the navigator.
        session.schema_filter = schema;
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

    /// Bring a table's data tab to the front, opening one when the table is
    /// not open yet. Filters (from foreign-key row navigation) replace the
    /// tab's current filters and reload it.
    fn select_table_with_filters_for(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        filters: Vec<Filter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let table_ref = table_ref(&table);
        let Some(session) = self
            .session(session_id)
            .filter(|session| session.engine.is_some())
        else {
            return;
        };
        if let Some(tab_id) = session.data_tab_for_table(&table_ref) {
            self.activate_secondary_tab_for(session_id, tab_id, window, cx);
            let loaded = self
                .session(session_id)
                .and_then(|session| session.data_tab(tab_id))
                .is_some_and(|data| data.result.is_some() || data.busy);
            if !filters.is_empty() || !loaded {
                self.load_data_tab_for(session_id, tab_id, filters, window, cx);
            }
            return;
        }

        let tab_id = Uuid::new_v4();
        let sortable = session.kind.is_sql();
        let layout = self.table_layout_for(session_id, &table_ref);
        let mut data = DataTab::new(session_id, tab_id, table_ref, sortable, window, cx);
        data.layout = layout;
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.secondary_tabs.push(SecondaryTab {
            id: tab_id,
            kind: SecondaryTabKind::Data(Box::new(data)),
        });
        self.activate_secondary_tab_for(session_id, tab_id, window, cx);
        self.load_data_tab_for(session_id, tab_id, filters, window, cx);
        self.persist_query_workspace_for(session_id, cx);
    }

    /// Load a data tab's structure and first page together, starting from
    /// `filters`.
    fn load_data_tab_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        filters: Vec<Filter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_edits_block(session_id, tab_id, cx) {
            return;
        }
        let Some((engine, kind, table_ref, filter_columns)) =
            self.session(session_id).and_then(|session| {
                let data = session.data_tab(tab_id)?;
                // Seed the filter editors with whatever columns are known
                // before the structure request returns.
                let columns = if data.table_columns.is_empty() {
                    session
                        .completion_columns
                        .get(&completion_table_key(&data.table))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    data.table_columns.clone()
                };
                Some((
                    session.engine.clone()?,
                    session.kind,
                    data.table.clone(),
                    columns,
                ))
            })
        else {
            return;
        };
        let runtime = self.runtime.clone();
        let mut filter_model = FilterModel::new();
        for filter in &filters {
            // IS NULL and IS NOT NULL carry no value but are still filters.
            filter_model.add_row_with_value_and_columns(
                filter.column.clone(),
                filter.operator,
                filter
                    .value
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                &filter_columns,
                window,
                cx,
            );
        }
        let filter_row_ids = filter_model
            .rows()
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.error = None;
        let Some(data) = session.data_tab_mut(tab_id) else {
            return;
        };
        data.table_page = 0;
        data.table_has_next_page = false;
        // Until this request completes, the visible snapshot must not be used
        // for a mutation.
        data.reset_row_state(cx);
        data.filters = filter_model;
        data.filter_subscriptions.clear();
        let sort = data.sort.clone();
        let keyset = keyset_column(kind, &filter_columns, sort.as_ref());
        let order = sort
            .clone()
            .or_else(|| keyset.clone())
            .into_iter()
            .collect::<Vec<_>>();
        data.busy = true;
        data.error = None;
        data.status = format!("Loading {}…", table_ref.name);
        data.request_generation += 1;
        let generation = data.request_generation;
        let result_table = table_ref.clone();
        let row_navigation = !filters.is_empty();
        for row_id in filter_row_ids {
            self.watch_filter_row_for(session_id, tab_id, row_id, window, cx);
        }
        let keyset_capable = filter_columns.is_empty() && supports_keyset_paging(kind);
        let loaded_filters = filters.clone();
        let task = runtime.spawn(async move {
            let first_page = |order: Vec<Order>| {
                let (engine, table_ref, filters) = (&engine, &table_ref, &filters);
                async move {
                    if kind != DatabaseKind::Redis {
                        let mut result = engine
                            .query_table(
                                table_ref,
                                &[],
                                filters,
                                &order,
                                Some(table_browse_page(0)),
                                QueryOptions::default(),
                            )
                            .await?;
                        let has_next_page = trim_table_browse_result(&mut result);
                        Ok::<_, dbx_core::DbxError>((
                            result,
                            has_next_page.then_some(PageStart::Offset),
                        ))
                    } else {
                        let (result, next) = engine
                            .redis_scan_page("*", 0, TABLE_BROWSE_PAGE_SIZE as usize)
                            .await?;
                        Ok((result, (next != 0).then_some(PageStart::RedisCursor(next))))
                    }
                }
            };
            let (structure, (result, next), keyset) = if keyset.is_none() && keyset_capable {
                // The key is unknown until the structure arrives; reading it
                // first lets even the first visit page by key.
                let structure = engine.table_structure(&table_ref).await?;
                let keyset = keyset_column(kind, &structure.columns, sort.as_ref());
                let order = sort
                    .clone()
                    .or_else(|| keyset.clone())
                    .into_iter()
                    .collect();
                (structure, first_page(order).await?, keyset)
            } else {
                // Structure and the first page are independent, so overlap
                // them instead of paying two sequential round trips.
                let (structure, page) =
                    tokio::try_join!(engine.table_structure(&table_ref), first_page(order))?;
                let confirmed = keyset_column(kind, &structure.columns, sort.as_ref());
                let keyset = keyset.filter(|keyset| confirmed.as_ref() == Some(keyset));
                (structure, page, keyset)
            };
            let next = match (&keyset, next) {
                (Some(order_by), Some(_)) => keyset_start(&result, order_by),
                (_, next) => next,
            };
            Ok::<_, dbx_core::DbxError>((structure, result, next))
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
            if let Some(data) = session.data_tab_mut(tab_id) {
                data.abort_handle.replace(task.abort_handle());
            }
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let Ok(result) = task.await else {
                return Ok(());
            };
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                let Some(data) = find_data_tab_mut(&mut session.secondary_tabs, tab_id) else {
                    return;
                };
                if generation != data.request_generation {
                    return;
                }
                data.busy = false;
                data.abort_handle.clear();
                let mut referenced_row_missing = false;
                let loaded = result.is_ok();
                match result {
                    Ok((structure, result, next)) => {
                        let has_rows = !result.rows.is_empty();
                        data.table_columns = structure.columns;
                        session.completion_columns.insert(
                            completion_table_key(&result_table),
                            data.table_columns.clone(),
                        );
                        data.foreign_keys = structure.foreign_keys;
                        data.table_page = 0;
                        data.table_has_next_page = next.is_some();
                        data.next_page_start = next;
                        data.page_starts = vec![match kind {
                            DatabaseKind::Redis => PageStart::RedisCursor(0),
                            _ => PageStart::Offset,
                        }];
                        data.set_result(Some(result), &session.tables, cx);
                        data.result_table = Some(result_table.clone());
                        referenced_row_missing = row_navigation && !has_rows;
                        data.error = None;
                        if row_navigation && has_rows {
                            data.data_grid
                                .update(cx, |table, cx| table.set_selected_row(0, cx));
                        }
                    }
                    Err(error) => {
                        data.error = Some(error.to_string());
                    }
                }
                if loaded {
                    this.page_loaded_for(session_id, tab_id, loaded_filters, cx);
                }
                if referenced_row_missing {
                    this.show_toast(ToastKind::Info, "Referenced row not found", cx);
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
            self.show_toast(
                ToastKind::Info,
                format!(
                    "Referenced table {} is not available in this database",
                    foreign_key.referenced_table
                ),
                cx,
            );
            return;
        };

        // A PostgreSQL foreign key may cross schemas. Keep the navigator and
        // the selected table in the same visible context before loading data.
        if self
            .session(session_id)
            .is_some_and(|session| session.kind.dialect() == DatabaseKind::PostgreSQL)
        {
            self.select_schema_filter_for(session_id, target_table.schema.clone(), cx);
        }
        self.select_table_for(session_id, target_table, window, cx);
    }

    fn navigate_to_foreign_key_row_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row_index: usize,
        column_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((foreign_key, target_table, filters)) =
            self.session(session_id).and_then(|session| {
                let data = session.data_tab(tab_id)?;
                let result = data.result.as_ref()?;
                let row = result.rows.get(row_index)?;
                let local_column = result.columns.get(
                    data.data_grid
                        .read(cx)
                        .delegate()
                        .result_column(column_index)?,
                )?;
                let foreign_key = data
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
            .is_some_and(|session| session.kind.dialect() == DatabaseKind::PostgreSQL)
        {
            self.select_schema_filter_for(session_id, target_table.schema.clone(), cx);
        }
        let target_ref = table_ref(&target_table);
        self.select_table_with_filters_for(session_id, target_table, filters, window, cx);
        if let Some(data) = self.session_mut(session_id).and_then(|session| {
            let tab_id = session.data_tab_for_table(&target_ref)?;
            session.data_tab_mut(tab_id)
        }) {
            data.status = format!(
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
        let Some((session_id, tab_id)) = self
            .active_session()
            .and_then(|session| Some((session.id, session.active_data_tab_id()?)))
        else {
            return;
        };
        self.refresh_table_for(session_id, tab_id, cx);
    }

    fn add_filter_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let Some(column) = data.table_columns.first().map(|column| column.name.clone()) else {
            return;
        };
        let columns = data.table_columns.clone();
        let row_id =
            data.filters
                .add_row_with_columns(column, FilterOperator::Equals, &columns, window, cx);
        self.watch_filter_row_for(session_id, tab_id, row_id, window, cx);
        cx.notify();
    }

    fn watch_filter_row_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row_id: FilterRowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((column_selector, operator_selector)) =
            self.data_tab(session_id, tab_id).and_then(|data| {
                data.filters
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
                    this.set_filter_column_for(session_id, tab_id, row_id, value.to_string(), cx);
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
                this.set_filter_operator_for(session_id, tab_id, row_id, operator, cx);
            },
        );
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.filter_subscriptions
                .extend([column_subscription, operator_subscription]);
        }
    }

    fn remove_filter_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row_id: FilterRowId,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.filters.remove(row_id);
            cx.notify();
        }
    }

    fn clear_filters_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.filters = FilterModel::new();
            data.filter_subscriptions.clear();
            cx.notify();
        }
    }

    fn set_filter_column_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row_id: FilterRowId,
        column: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            if let Some(row) = data
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
        tab_id: SecondaryTabId,
        row_id: FilterRowId,
        operator: FilterOperator,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            if let Some(row) = data
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

    fn refresh_table_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        self.load_table_page_for(session_id, tab_id, 0, cx);
    }

    /// Apply a header sort to a data tab and reload its first page.
    pub(super) fn set_table_sort_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        order: Option<Order>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let tables = session.tables.clone();
        let Some(data) = session.data_tab_mut(tab_id) else {
            return;
        };
        if data.busy || data.row_draft.is_some() || data.has_unsaved_cell_work() {
            // Restore the indicator the grid already advanced.
            data.sync_result_grid(false, &tables, cx);
            cx.notify();
            return;
        }
        data.sort = order;
        self.load_table_page_for(session_id, tab_id, 0, cx);
    }

    fn set_table_page(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        page: u64,
        cx: &mut Context<Self>,
    ) {
        let Some((current_page, has_next_page, busy)) = self
            .data_tab(session_id, tab_id)
            .map(|data| (data.table_page, data.table_has_next_page, data.busy))
        else {
            return;
        };
        if busy || page == current_page || (page > current_page && !has_next_page) {
            return;
        }
        self.load_table_page_for(session_id, tab_id, page, cx);
    }

    fn load_table_page_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        page: u64,
        cx: &mut Context<Self>,
    ) {
        if self.pending_edits_block(session_id, tab_id, cx) {
            return;
        }
        let Some((engine, table, kind, busy, known_columns, start)) =
            self.session(session_id).and_then(|session| {
                let data = session.data_tab(tab_id)?;
                let start = if page == 0 {
                    None
                } else if page == data.table_page + 1 {
                    data.next_page_start.clone()
                } else {
                    data.page_starts.get(page as usize).cloned()
                };
                Some((
                    session.engine.clone(),
                    data.table.clone(),
                    session.kind,
                    data.busy,
                    data.table_columns.clone(),
                    start,
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
        let filters = match self.active_filters_for(session_id, tab_id, cx) {
            Ok(filters) => filters,
            Err(error) => {
                if let Some(data) = self.data_tab_mut(session_id, tab_id) {
                    data.error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        let runtime = self.runtime.clone();
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let keyset = keyset_column(kind, &known_columns, data.sort.as_ref());
        let start = start.unwrap_or(match (kind, &keyset) {
            (DatabaseKind::Redis, _) => PageStart::RedisCursor(0),
            _ => PageStart::Offset,
        });
        let mut order = data.sort.clone().into_iter().collect::<Vec<_>>();
        // A page reached by OFFSET continues by OFFSET; keyset paging starts
        // from a fresh first page.
        let keyset = keyset.filter(|_| page == 0 || matches!(start, PageStart::After(_)));
        let loaded_filters = filters.clone();
        let mut filters = filters;
        let mut offset_page = page;
        if let Some(order_by) = &keyset {
            // Seeking by primary key keeps every page as fast as the first,
            // where OFFSET rescans all earlier rows.
            if order.is_empty() {
                order.push(order_by.clone());
            }
            if let PageStart::After(value) = &start {
                let operator = match order_by.direction {
                    OrderDirection::Ascending => FilterOperator::GreaterThan,
                    OrderDirection::Descending => FilterOperator::LessThan,
                };
                filters.push(Filter::new(
                    order_by.column.clone(),
                    operator,
                    Some(value.clone()),
                ));
            }
            offset_page = 0;
        }
        let pattern = filters
            .first()
            .and_then(|filter| filter.value.as_ref())
            .map(ToString::to_string)
            .unwrap_or_else(|| "*".into());
        let page_start = start.clone();
        data.busy = true;
        data.error = None;
        data.status = format!("Loading page {}…", page + 1);
        data.reset_row_state(cx);
        data.request_generation += 1;
        let generation = data.request_generation;
        let result_table = table.clone();
        let task = runtime.spawn(async move {
            if kind != DatabaseKind::Redis {
                // The open table's columns are already cached; passing them
                // saves the PostgreSQL filter-cast lookup on every page.
                let mut result = engine
                    .query_table_with_columns(
                        &table,
                        &[],
                        &filters,
                        &order,
                        Some(table_browse_page(offset_page)),
                        QueryOptions::default(),
                        Some(&known_columns),
                    )
                    .await?;
                let has_next_page = trim_table_browse_result(&mut result);
                let next = match &keyset {
                    Some(order_by) if has_next_page => keyset_start(&result, order_by),
                    None if has_next_page => Some(PageStart::Offset),
                    _ => None,
                };
                Ok::<_, dbx_core::DbxError>((result, next))
            } else {
                let cursor = match start {
                    PageStart::RedisCursor(cursor) => cursor,
                    _ => 0,
                };
                let (result, next) = engine
                    .redis_scan_page(&pattern, cursor, TABLE_BROWSE_PAGE_SIZE as usize)
                    .await?;
                Ok((result, (next != 0).then_some(PageStart::RedisCursor(next))))
            }
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
            if let Some(data) = session.data_tab_mut(tab_id) {
                data.abort_handle.replace(task.abort_handle());
            }
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let Ok(result) = task.await else {
                return Ok(());
            };
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                let Some(data) = find_data_tab_mut(&mut session.secondary_tabs, tab_id) else {
                    return;
                };
                if generation != data.request_generation {
                    return;
                }
                data.busy = false;
                data.abort_handle.clear();
                match result {
                    Ok((result, next)) => {
                        data.table_page = page;
                        data.table_has_next_page = next.is_some();
                        data.next_page_start = next;
                        data.page_starts.truncate(page as usize);
                        data.page_starts.push(page_start);
                        data.set_result(Some(result), &session.tables, cx);
                        data.result_table = Some(result_table.clone());
                        data.error = None;
                        this.page_loaded_for(session_id, tab_id, loaded_filters, cx);
                    }
                    Err(error) => {
                        data.error = Some(error.to_string());
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn active_filters_for(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &App,
    ) -> Result<Vec<Filter>, String> {
        let Some(session) = self.session(session_id) else {
            return Ok(Vec::new());
        };
        let Some(data) = session.data_tab(tab_id) else {
            return Ok(Vec::new());
        };
        if session.kind.is_sql() {
            return data
                .filters
                .validate(cx, &data.table_columns)
                .map_err(|error| error.to_string());
        }
        if session.kind != DatabaseKind::Redis {
            return Ok(Vec::new());
        }
        let value = session.editors.filter_text.read(cx).trim();
        let column = selected_filter_column(
            data.selected_column,
            &data.table_columns,
            data.result.as_deref(),
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

    fn data_tab(&self, session_id: SessionId, tab_id: SecondaryTabId) -> Option<&DataTab> {
        self.session(session_id)?.data_tab(tab_id)
    }

    fn data_tab_mut(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
    ) -> Option<&mut DataTab> {
        self.session_mut(session_id)?.data_tab_mut(tab_id)
    }

    fn editable_table_for(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
    ) -> Option<&TableRef> {
        let session = self.session(session_id)?;
        if session
            .engine
            .as_ref()
            .is_some_and(|engine| engine.is_read_only())
        {
            return None;
        }
        let data = session.data_tab(tab_id)?;
        let is_real_table = session
            .tables
            .iter()
            .any(|table| table.kind == EntityKind::Table && table_ref(table) == data.table);
        can_mutate_result(
            session.kind,
            session.busy || data.busy,
            Some(&data.table),
            data.result_table.as_ref(),
        )
        .then_some(&data.table)
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
            let cursor = query_tab.query_editor.read(cx).cursor_offset();
            let recent_data = session.recent_data();
            let signature = CompletionSignature {
                text_revision,
                cursor,
            };
            let key = CompletionCacheKey {
                signature,
                tables: session.tables.len(),
                columns: session.completion_columns.len(),
                recent_result: recent_data
                    .and_then(|data| data.result.as_ref())
                    .map_or(0, |result| Arc::as_ptr(result) as usize),
            };
            if let Some((cached, computed)) = &query_tab.completion_cache
                && *cached == key
            {
                let (replacement_range, items) = computed.clone()?;
                (tab_id, replacement_range, items, signature)
            } else {
                let query_text = query_tab.query_text.read(cx).clone();
                let computed = if session.kind.is_sql() {
                    editor::sql_completion_context(&query_text, cursor).map(|context| {
                        let items = sql_completion_items(
                            &query_text,
                            cursor,
                            &context,
                            SqlCompletionRequest {
                                database_kind: session.kind,
                                tables: &session.tables,
                                completion_columns: &session.completion_columns,
                                selected_table: recent_data.map(|data| &data.table),
                                active_columns: recent_data
                                    .map(|data| data.table_columns.as_slice())
                                    .unwrap_or_default(),
                                result: recent_data.and_then(|data| data.result.as_deref()),
                                active_schema_filter: session.schema_filter.as_deref(),
                            },
                        );
                        (context.replacement_range, items)
                    })
                } else if session.kind == DatabaseKind::Redis {
                    redis_completion_items(
                        &query_text,
                        cursor,
                        session.redis_command_catalog.as_deref(),
                        query_tab.result.as_deref(),
                        recent_data.and_then(|data| data.result.as_deref()),
                    )
                } else {
                    None
                };
                let session = self.session_mut(session_id)?;
                if let Some(SecondaryTab {
                    kind: SecondaryTabKind::Query(query_tab),
                    ..
                }) = session
                    .secondary_tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                {
                    query_tab.completion_cache = Some((key, computed.clone()));
                }
                let (replacement_range, items) = computed?;
                (tab_id, replacement_range, items, signature)
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
                SecondaryTabKind::Data(_)
                | SecondaryTabKind::Structure(_)
                | SecondaryTabKind::Diagram(_) => None,
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
                SecondaryTabKind::Data(_)
                | SecondaryTabKind::Structure(_)
                | SecondaryTabKind::Diagram(_) => None,
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
                    session.set_tables(tables);
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
                this.load_schema_objects_for(session_id, cx);
                this.prefetch_completion_columns_for(session_id, cx);
                this.refresh_open_structures_for(session_id, cx);
                let data_tabs = this
                    .session(session_id)
                    .map(|session| {
                        session
                            .secondary_tabs
                            .iter()
                            .filter_map(|tab| match &tab.kind {
                                SecondaryTabKind::Data(data)
                                    if !data.has_unsaved_cell_work() && !data.busy =>
                                {
                                    Some(tab.id)
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for tab_id in data_tabs {
                    this.refresh_table_for(session_id, tab_id, cx);
                }
                if diagram_open {
                    this.refresh_diagram_for(session_id, cx);
                }
            })?;

            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Warm the schema cache in the background so query completion can
    /// resolve columns for tables the user has not opened yet. Failures are
    /// intentionally ignored here: table names remain useful completions and
    /// opening a table still retries its authoritative structure request.
    fn prefetch_completion_columns_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some((engine, kind, database)) = self.session(session_id).map(|session| {
            (
                session.engine.clone(),
                session.kind,
                session.current_database.clone(),
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

        let expected_engine = engine.clone();
        let runtime = self.runtime.clone();
        // One bulk catalog snapshot replaces a describe call per table.
        let task = runtime.spawn(async move {
            let schema = engine.relational_schema().await;
            schema
                .map(|schema| {
                    schema
                        .tables
                        .into_iter()
                        .filter(|entry| {
                            matches!(entry.table.kind, EntityKind::Table | EntityKind::View)
                        })
                        .map(|entry| {
                            (
                                completion_table_key(&table_ref(&entry.table)),
                                entry.structure.columns,
                            )
                        })
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default()
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

    fn new_connection_action(&mut self, _: &NewConnection, _: &mut Window, cx: &mut Context<Self>) {
        if self.vault_state == Some(VaultState::Unlocked) {
            self.begin_new_connection(cx);
        }
    }

    fn new_query_action(&mut self, _: &NewQuery, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_picker_open {
            return;
        }
        if let Some(session_id) = self.active_session_id() {
            self.add_query_tab_for(session_id, window, cx);
        }
    }

    /// ⌘W closes the active document tab; on the persistent Data document it
    /// closes the connection itself, mirroring browser/editor tab semantics.
    fn close_tab_action(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_picker_open {
            return;
        }
        let Some(session) = self.active_session() else {
            return;
        };
        let session_id = session.id;
        match session.active_secondary_tab {
            Some(tab_id) => self.request_close_secondary_tab_for(session_id, tab_id, window, cx),
            None => self.close_session(session_id, cx),
        }
    }

    fn cycle_connection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.sessions.len();
        if count == 0 {
            return;
        }
        let current = self
            .active_session_id()
            .and_then(|id| self.sessions.iter().position(|session| session.id == id))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        let session_id = self.sessions[next].id;
        self.activate_session(session_id, cx);
    }

    fn next_connection_action(
        &mut self,
        _: &NextConnection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_connection(true, cx);
    }

    fn previous_connection_action(
        &mut self,
        _: &PreviousConnection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_connection(false, cx);
    }

    fn toggle_sidebar_action(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar(cx);
    }

    pub(crate) fn show_toast(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast {
            id,
            kind,
            message: message.into(),
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
        let lifetime = match kind {
            ToastKind::Error => std::time::Duration::from_secs(6),
            ToastKind::Info | ToastKind::Success => std::time::Duration::from_millis(3200),
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(lifetime).await;
            this.update(cx, |this, cx| this.dismiss_toast(id, cx))
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.toasts.len();
        self.toasts.retain(|toast| toast.id != id);
        if self.toasts.len() != before {
            cx.notify();
        }
    }

    /// Escape backs out of the connection picker to the connection that was
    /// already open, like dismissing a sheet.
    fn dismiss_connection_picker(&mut self) -> bool {
        let live_session = self
            .active_session()
            .is_some_and(|session| session.engine.is_some());
        if self.connection_picker_open && live_session {
            self.connection_picker_open = false;
            true
        } else {
            false
        }
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_hidden = !self.sidebar_hidden;
        cx.notify();
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
        if session.secondary_tabs.iter().any(|tab| matches!(&tab.kind, SecondaryTabKind::Data(data) if data.has_unsaved_cell_work() || data.busy)) {
            session.error = Some("Finish running table work and save or discard cell edits before switching databases".into());
            cx.notify();
            return;
        }
        if session.secondary_tabs.iter().any(|tab| matches!(&tab.kind, SecondaryTabKind::Query(query) if query.in_transaction || query.busy)) {
            session.error = Some("Finish or roll back open query transactions before switching databases".into());
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
                SecondaryTabKind::Data(data) => data.invalidate_request(),
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
                let switched = matches!(&result, Ok(Ok(_)));
                match result {
                    Ok(Ok(tables)) => {
                        session.set_tables(tables);
                        session.current_database = Some(database.clone());
                        session.schema_objects.clear();
                        session.schema_objects_error = None;
                        // Open tables belong to the previous database.
                        session.close_data_tabs();
                        session.completion_columns.clear();
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
                    }
                }
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
                if switched {
                    this.load_schema_objects_for(session_id, cx);
                    this.persist_query_workspace_for(session_id, cx);
                    this.persist_startup_workspace(cx);
                }
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
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let Some(columns) = self
            .data_tab(session_id, tab_id)
            .filter(|data| data.cell_editor.is_none())
            .map(|data| data.table_columns.clone())
        else {
            return;
        };
        let mut row_draft = RowDraftModel::new();
        for column in columns {
            row_draft.push(FieldRow::new_insert(column, None, window, cx));
        }
        self.watch_draft_fields_for(session_id, tab_id, &row_draft, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.error = None;
        let Some(data) = session.data_tab_mut(tab_id) else {
            return;
        };
        data.draft_mode = DraftMode::Insert;
        data.draft_insert = None;
        data.selected_row = None;
        data.inspector_open = true;
        data.clear_grid_selection(cx);
        data.row_draft = Some(row_draft);
        cx.notify();
    }

    fn watch_draft_fields_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
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
                        this.set_row_value_text_for(session_id, tab_id, field_id, value, cx);
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
                        this.set_row_value_text_for(session_id, tab_id, field_id, value, cx);
                    },
                ));
            }
            if let Some(selector) = field.state_selector.clone() {
                let field_id = field.id;
                subscriptions.push(cx.subscribe_in(
                    &selector,
                    window,
                    move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, window, cx| {
                        let SelectEvent::Confirm(value) = event;
                        let state = value
                            .as_ref()
                            .and_then(|value| FieldValueState::from_label(value.as_ref()));
                        if let Some(state) = state {
                            this.set_row_field_state_for(
                                session_id, tab_id, field_id, state, window, cx,
                            );
                        }
                    },
                ));
            }
        }
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.row_draft_subscriptions = subscriptions;
        }
    }

    fn set_row_value_text_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
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
        let Some(field) = find_data_tab_mut(&mut session.secondary_tabs, tab_id)
            .and_then(|data| data.row_draft.as_mut())
            .and_then(|draft| {
                draft
                    .fields_mut()
                    .iter_mut()
                    .find(|field| field.id == field_id)
            })
        else {
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
        tab_id: SecondaryTabId,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, TableEvent::SelectRow(_) | TableEvent::ClearSelection)
            && self
                .data_tab_mut(session_id, tab_id)
                .is_some_and(|data| std::mem::take(&mut data.suppress_next_grid_selection_event))
        {
            return;
        }
        match event {
            TableEvent::ColumnWidthsChanged(widths) => {
                if let Some(data) = self.data_tab_mut(session_id, tab_id) {
                    let delegate = data.data_grid.read(cx).delegate();
                    data.result_column_widths = delegate.widths_by_key(widths);
                    let widths = delegate.widths_by_name();
                    data.layout.widths.extend(widths);
                    self.store_table_layout_for(session_id, tab_id, cx);
                }
            }
            TableEvent::MoveColumn(..) => {
                if let Some(data) = self.data_tab_mut(session_id, tab_id) {
                    let names = data
                        .result
                        .as_deref()
                        .map(|result| {
                            data.data_grid
                                .read(cx)
                                .delegate()
                                .display_order()
                                .iter()
                                .map(|index| result.columns[*index].name.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    data.layout.order = names
                        .into_iter()
                        .filter(|name| !data.layout.pinned.contains(name))
                        .collect();
                    self.store_table_layout_for(session_id, tab_id, cx);
                }
            }
            TableEvent::SelectRow(row_index) => {
                self.select_row_for(session_id, tab_id, *row_index, cx);
            }
            // Selecting a cell also selects its row for the inspector.
            TableEvent::SelectCell(row_index, column_index) => {
                self.select_row_for(session_id, tab_id, *row_index, cx);
                if let Some(column) = self.data_grid_column(session_id, tab_id, *column_index, cx) {
                    self.select_column_for(session_id, tab_id, column, cx);
                }
            }
            TableEvent::DoubleClickedCell(row_index, column_index) => {
                if let Some(column) = self.data_grid_column(session_id, tab_id, *column_index, cx) {
                    self.begin_cell_edit_for(session_id, tab_id, *row_index, column, window, cx);
                }
            }
            TableEvent::SelectColumn(column_index) => {
                if let Some(column) = self.data_grid_column(session_id, tab_id, *column_index, cx) {
                    self.select_column_for(session_id, tab_id, column, cx);
                }
            }
            TableEvent::ClearSelection => {
                if let Some(data) = self.data_tab_mut(session_id, tab_id)
                    && data.draft_mode == DraftMode::Update
                {
                    data.selected_row = None;
                    data.row_draft = None;
                    data.row_draft_subscriptions.clear();
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// The result column a data grid column shows.
    pub(super) fn data_grid_column(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        col_ix: usize,
        cx: &App,
    ) -> Option<usize> {
        self.data_tab(session_id, tab_id)?
            .data_grid
            .read(cx)
            .delegate()
            .result_column(col_ix)
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
            TableEvent::DoubleClickedCell(row, column) if *column > 0 => {
                query_tab.inspected_value = query_tab
                    .result_grid
                    .read(cx)
                    .delegate()
                    .cell_value(
                        *row,
                        query_tab
                            .result_grid
                            .read(cx)
                            .delegate()
                            .result_column(*column)
                            .unwrap_or_default(),
                    )
                    .cloned();
            }
            TableEvent::ColumnWidthsChanged(widths) => {
                query_tab.result_column_widths = query_tab
                    .result_grid
                    .read(cx)
                    .delegate()
                    .widths_by_key(widths);
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
        tab_id: SecondaryTabId,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        let pending_draft = self.data_tab(session_id, tab_id).and_then(|data| {
            data.row_draft.as_ref()?;
            Some((data.selected_row, data.data_grid.clone()))
        });
        if let Some((selected_row, data_grid)) = pending_draft {
            if let Some(data) = self.data_tab_mut(session_id, tab_id) {
                data.suppress_next_grid_selection_event = true;
            }
            data_grid.update(cx, |table, cx| {
                if let Some(selected_row) = selected_row {
                    table.set_selected_row(selected_row, cx);
                } else {
                    table.clear_selection(cx);
                }
            });
            self.show_toast(
                ToastKind::Info,
                "Save or cancel the current row before selecting another",
                cx,
            );
            return;
        }
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(data) = find_data_tab_mut(&mut session.secondary_tabs, tab_id) else {
            return;
        };
        if data
            .result
            .as_ref()
            .is_none_or(|result| row >= result.rows.len() + data.pending_inserts.len())
        {
            return;
        }
        data.selected_row = Some(row);
        data.draft_mode = DraftMode::Update;
        data.inspector_open = true;
        data.row_draft = None;
        data.row_draft_subscriptions.clear();
        session.error = None;
        cx.notify();
    }

    fn begin_edit_selected_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let draft_data = self.data_tab(session_id, tab_id).and_then(|data| {
            if data.cell_editor.is_some() {
                return None;
            }
            let row = data.selected_row?;
            if data.pending_deletes.contains(&row) {
                return None;
            }
            let result = data.result.as_ref()?;
            let insert = data.insert_index(row);
            let values = match insert {
                Some(_) => Vec::new(),
                None => result.rows.get(row)?.values.clone(),
            };
            // The inspector opens on what the grid shows: staged values over
            // the loaded row, or a staged new row's values.
            let staged = (0..result.columns.len())
                .map(|column| match insert {
                    Some(index) => data.pending_inserts[index].values.get(&column).cloned(),
                    None => data.pending_edits.get(&(row, column)).cloned(),
                })
                .collect::<Vec<_>>();
            Some((
                data.table_columns.clone(),
                result.columns.clone(),
                values,
                staged,
                insert,
            ))
        });
        let Some((table_columns, result_columns, values, staged, insert)) = draft_data else {
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
                }
                cx.notify();
                return;
            };
            let original = match insert {
                Some(_) => None,
                None => match values.get(index).cloned() {
                    Some(original) => Some(original),
                    None => return,
                },
            };
            let field = match staged.get(index).cloned().flatten() {
                Some(MutationValue::Parameter(CellValue::Null)) => FieldRow::with_state(
                    column,
                    original,
                    String::new(),
                    FieldValueState::Null,
                    None,
                    window,
                    cx,
                ),
                Some(MutationValue::Parameter(value)) => FieldRow::with_state(
                    column,
                    original,
                    field_editor_text(&value),
                    FieldValueState::Value,
                    None,
                    window,
                    cx,
                ),
                Some(MutationValue::Expression(expression)) => FieldRow::with_state(
                    column,
                    original,
                    expression,
                    FieldValueState::Sql,
                    None,
                    window,
                    cx,
                ),
                None => match original {
                    Some(original) => FieldRow::new_update(column, original, window, cx),
                    None => FieldRow::new_insert(column, None, window, cx),
                },
            };
            draft.push(field);
        }
        self.watch_draft_fields_for(session_id, tab_id, &draft, window, cx);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.error = None;
        let Some(data) = session.data_tab_mut(tab_id) else {
            return;
        };
        data.draft_mode = if insert.is_some() {
            DraftMode::Insert
        } else {
            DraftMode::Update
        };
        data.draft_insert = insert;
        data.inspector_open = true;
        data.row_draft = Some(draft);
        cx.notify();
    }

    fn close_inspector_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.inspector_open = false;
            cx.notify();
        }
    }

    fn select_column_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.selected_column = column;
            cx.notify();
        }
    }

    fn set_row_field_state_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        field_id: FieldId,
        state: FieldValueState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(data) = find_data_tab_mut(&mut session.secondary_tabs, tab_id) else {
            return;
        };
        let draft_mode = data.draft_mode;
        let Some(field) = data.row_draft.as_mut().and_then(|draft| {
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
        if state == FieldValueState::Default && draft_mode != DraftMode::Insert {
            return;
        }
        field.set_state(state);
        // SQL mode edits the same text; show what Value mode will now save.
        if state == FieldValueState::Value {
            field.sync_value_selectors(window, cx);
        }
        session.error = None;
        cx.notify();
    }

    fn cancel_row_draft_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.error = None;
        let Some(data) = session.data_tab_mut(tab_id) else {
            return;
        };
        // Cancelling an edit of a staged new row keeps that row selected.
        let was_insert = data.draft_mode == DraftMode::Insert && data.draft_insert.is_none();
        data.draft_insert = None;
        data.row_draft = None;
        data.row_draft_subscriptions.clear();
        if was_insert {
            data.selected_row = None;
            data.clear_grid_selection(cx);
        }
        data.draft_mode = DraftMode::Update;
        cx.notify();
    }

    /// Stage the inspector's row: changed fields of a loaded row, or a new
    /// row. Nothing is written until the changeset is committed.
    fn save_draft_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(data) = session.data_tab(tab_id) else {
            return;
        };
        if session.busy || data.busy {
            return;
        }
        let draft_mode = data.draft_mode;
        if !session.kind.is_sql() {
            let return_focus = self
                .row_draft_focus_for(session_id, tab_id, None, cx)
                .or_else(|| window.focused(cx));
            self.show_mutation_error_for(
                session_id,
                tab_id,
                "Edit Redis keys from the command console.".into(),
                return_focus,
                window,
                cx,
            );
            return;
        }
        let (Some(_), Some(row_draft)) = (
            self.editable_table_for(session_id, tab_id),
            data.row_draft.as_ref(),
        ) else {
            return;
        };
        let selected_row = data.selected_row;
        let values = match draft_mode {
            DraftMode::Insert => row_draft.insert_values(cx),
            DraftMode::Update => row_draft.changed_fields(cx),
        }
        .map_err(|error| (error.to_string(), error.field_id()));
        let values = match values {
            Ok(values) => values,
            Err((error, field_id)) => {
                let return_focus = self
                    .row_draft_focus_for(session_id, tab_id, Some(field_id), cx)
                    .or_else(|| window.focused(cx));
                self.show_mutation_error_for(session_id, tab_id, error, return_focus, window, cx);
                return;
            }
        };
        if let Some(session) = self.session_mut(session_id) {
            session.error = None;
        }
        match (draft_mode, selected_row) {
            (DraftMode::Insert, _) => self.stage_insert_for(session_id, tab_id, values, cx),
            (DraftMode::Update, Some(row)) => {
                // Primary keys must still identify the row when committed.
                if let Some(row_data) = self
                    .data_tab(session_id, tab_id)
                    .and_then(|data| data.result.as_ref()?.rows.get(row).cloned())
                    && let Err(error) = self.identity_filters_for(session_id, tab_id, &row_data)
                {
                    self.show_mutation_error_for(session_id, tab_id, error, None, window, cx);
                    return;
                }
                self.stage_update_for(session_id, tab_id, row, values, cx)
            }
            (DraftMode::Update, None) => {}
        }
        if let Some(data) = self.data_tab(session_id, tab_id) {
            let grid = data.data_grid.read(cx).focus_handle(cx);
            grid.focus(window, cx);
        }
    }

    fn request_delete_selected_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .session(session_id)
            .is_some_and(|session| !session.kind.is_sql())
        {
            self.show_toast(
                ToastKind::Info,
                "Edit Redis keys from the command console",
                cx,
            );
            return;
        }
        self.delete_rows_for(session_id, tab_id, None, cx);
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
            sql: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn cancel_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.confirmation_dialog.take() else {
            return;
        };
        // A declined run must ask for its parameters again next time.
        if let ConfirmationAction::RunQuery { session_id, .. } = dialog.action
            && let Some(query) = self.active_query_tab_mut(session_id)
        {
            query.parameters_ready = false;
            query.prepared_parameters = None;
        }
        if let Some(return_focus) = dialog.return_focus {
            return_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn show_mutation_error_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        detail: String,
        return_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut draft_mode = DraftMode::Update;
        if let Some(session) = self.session_mut(session_id) {
            session.error = Some(detail.clone());
            if let Some(data) = session.data_tab_mut(tab_id) {
                data.busy = false;
                draft_mode = data.draft_mode;
            }
        }
        let title = match draft_mode {
            DraftMode::Insert => "Couldn’t insert row",
            DraftMode::Update => "Couldn’t update row",
        };
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
        let closes_query = matches!(
            dialog.action,
            ConfirmationAction::CloseQuery { .. } | ConfirmationAction::DiscardDataTab { .. }
        );
        match dialog.action {
            ConfirmationAction::DeleteProfile { id } => self.change_saved_profile(id, true, cx),
            ConfirmationAction::LockVault => {
                self.lock_vault(cx);
                self.vault_editors
                    .passphrase_editor
                    .read(cx)
                    .focus_handle()
                    .focus(window, cx);
                return;
            }
            ConfirmationAction::RunQuery {
                session_id,
                tab_id,
                run_all,
                query,
            } => {
                if self
                    .session(session_id)
                    .and_then(|session| session.active_secondary_tab)
                    != Some(tab_id)
                {
                    self.show_toast(
                        ToastKind::Info,
                        "Return to the query tab and run it again",
                        cx,
                    );
                    return;
                }
                if let Some(tab) = self.active_query_tab_mut(session_id) {
                    tab.execution_override = Some(query);
                }
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
            ConfirmationAction::Quit => {
                cx.quit();
                return;
            }
            ConfirmationAction::CommitChanges { session_id, tab_id } => {
                self.save_pending_edits_for(session_id, tab_id, window, cx)
            }
            ConfirmationAction::DiscardDataTab { session_id, tab_id } => {
                self.discard_pending_edits_for(session_id, tab_id, cx);
                self.close_secondary_tab_for(session_id, tab_id, cx);
            }
            ConfirmationAction::NativeRestore {
                session_id,
                path,
                database,
            } => {
                if self
                    .session(session_id)
                    .is_some_and(|session| session.current_database == database)
                {
                    self.run_native_backup(session_id, path, true, cx);
                } else {
                    self.show_toast(ToastKind::Info, "Restore target changed. Choose the backup again and review the new target.", cx);
                }
            }
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
                let toast = match result {
                    Ok((outcome, tables)) => {
                        session.set_tables(tables);
                        session.error = None;
                        Some(match action {
                            TableAction::Truncate => {
                                if let Some(tab_id) = session.data_tab_for_table(&target_table)
                                    && let Some(data) =
                                        find_data_tab_mut(&mut session.secondary_tabs, tab_id)
                                {
                                    data.invalidate_request();
                                    let mut result = data
                                        .result
                                        .as_deref()
                                        .cloned()
                                        .unwrap_or_else(|| QueryResult::empty(None, 0));
                                    result.rows.clear();
                                    result.rows_affected = Some(outcome.rows_affected);
                                    result.elapsed_ms = outcome.elapsed_ms;
                                    data.set_result(Some(result), &session.tables, cx);
                                    data.table_page = 0;
                                    data.table_has_next_page = false;
                                    data.result_table = Some(target_table.clone());
                                    data.selected_row = None;
                                    data.row_draft = None;
                                    data.row_draft_subscriptions.clear();
                                }
                                format!(
                                    "Truncated {} · {}",
                                    table.name,
                                    counted(outcome.rows_affected, "row", "rows")
                                )
                            }
                            TableAction::Drop => {
                                session.close_data_tabs_where(|data| data.table == target_table);
                                session
                                    .completion_columns
                                    .remove(&completion_table_key(&target_table));
                                format!("Deleted table {}", table.name)
                            }
                        })
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        None
                    }
                };
                match toast {
                    Some(message) => this.show_toast(ToastKind::Success, message, cx),
                    None => cx.notify(),
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn identity_filters_for(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: &RowData,
    ) -> Result<Vec<Filter>, String> {
        let data = self
            .data_tab(session_id, tab_id)
            .ok_or("No table is open")?;
        let result = data.result.as_ref().ok_or("No row result is loaded")?;
        let primary_keys: Vec<_> = data
            .table_columns
            .iter()
            .filter(|column| column.primary_key)
            .collect();
        if primary_keys.is_empty() {
            return Err("Rows without a primary key are read-only.".into());
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
        let sql = match kind.dialect() {
            DatabaseKind::PostgreSQL => {
                "CREATE TABLE public.new_table (\n    id BIGSERIAL PRIMARY KEY,\n    name TEXT NOT NULL\n);"
            }
            DatabaseKind::MySQL => {
                "CREATE TABLE new_table (\n    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,\n    name VARCHAR(255) NOT NULL\n);"
            }
            DatabaseKind::SQLite => {
                "CREATE TABLE new_table (\n    id INTEGER PRIMARY KEY,\n    name TEXT NOT NULL\n);"
            }
            DatabaseKind::DuckDB => "CREATE TABLE new_table (id BIGINT PRIMARY KEY, name VARCHAR);",
            DatabaseKind::BigQuery => "CREATE TABLE new_table (id INT64, name STRING);",
            DatabaseKind::ClickHouse => {
                "CREATE TABLE new_table (\n    id UInt64,\n    name String\n) ENGINE = MergeTree ORDER BY id;"
            }
            DatabaseKind::SqlServer => {
                "CREATE TABLE dbo.new_table (\n    id INT IDENTITY(1,1) PRIMARY KEY,\n    name NVARCHAR(200) NOT NULL\n);"
            }
            _ => kind.default_query(),
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
    }
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

fn tag_badge(tag: Option<&ConnectionTag>) -> Option<Div> {
    tag.map(|tag| {
        let color = gpui::rgb(tag.color);
        div()
            .flex_none()
            .px(px(6.))
            .py(px(2.))
            .rounded_full()
            .border_1()
            .border_color(color)
            .text_size(px(9.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .child(tag.name.clone())
    })
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
    (kind.dialect() == DatabaseKind::PostgreSQL
        && tables
            .iter()
            .any(|table| table.schema.as_deref() == Some("public")))
    .then(|| "public".to_owned())
}

fn schema_filter_options(kind: DatabaseKind, tables: &[TableInfo]) -> Vec<Option<String>> {
    if kind.dialect() != DatabaseKind::PostgreSQL {
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
    if kind.dialect() != DatabaseKind::PostgreSQL {
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
    (kind.dialect() == DatabaseKind::PostgreSQL)
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
    let mut document =
        diagram_document_for_selection(source_schema, diagram.selected_schemas.as_ref());
    document.place_nodes(&diagram.arranged_positions);
    let document = Arc::new(document);
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
    diagram.node_drag = None;
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
            kind.dialect() != DatabaseKind::PostgreSQL
                || schema_filter.is_none()
                || table.schema.as_deref() == schema_filter
        })
        .cloned()
        .collect()
}

fn table_is_visible_in(
    kind: DatabaseKind,
    schema_filter: Option<&str>,
    table_schema: Option<&str>,
) -> bool {
    kind.dialect() != DatabaseKind::PostgreSQL
        || schema_filter.is_none()
        || table_schema == schema_filter
}

fn can_mutate_result(
    kind: DatabaseKind,
    busy: bool,
    selected_table: Option<&TableRef>,
    result_table: Option<&TableRef>,
) -> bool {
    !busy
        && kind.supports_row_mutations()
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

const DIAGRAM_MINIMAP_MAX_WIDTH: f32 = 188.0;
const DIAGRAM_MINIMAP_MAX_HEIGHT: f32 = 124.0;
const DIAGRAM_MINIMAP_MARGIN: f32 = 14.0;
const DIAGRAM_MINIMAP_PADDING: f32 = 6.0;

/// The minimap drawing size: the document scaled to fit a fixed box.
fn diagram_minimap_size(document: &DiagramDocument) -> (f32, f32) {
    let width = document.width.max(1.0);
    let height = document.height.max(1.0);
    let scale = (DIAGRAM_MINIMAP_MAX_WIDTH / width).min(DIAGRAM_MINIMAP_MAX_HEIGHT / height);
    (width * scale, height * scale)
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
    fn connection_tag_is_single_and_updates_shared_colours(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.profile_store = Some(store.clone());
                app.connection_tags = store.tags().unwrap();
                app.vault_state = Some(VaultState::Unlocked);
                let prod = default_tags().remove(0);
                app.select_tag(prod.clone(), cx);
                assert_eq!(app.draft.tag.as_ref().map(|tag| tag.id), Some(prod.id));
                app.sessions.push(ConnectionSession::new(
                    Uuid::new_v4(),
                    None,
                    "Test".into(),
                    DatabaseKind::SQLite,
                    app.draft.tag.clone(),
                    window,
                    cx,
                ));
                app.edit_tag(Some(prod.clone()), cx);
                app.tag_editor.color.update(cx, |value, cx| {
                    *value = "#B48EAD".into();
                    cx.notify();
                });
                app.save_connection_tag(cx);
                assert_eq!(app.draft.tag.as_ref().unwrap().color, 0xb48ead);
                assert_eq!(app.sessions[0].tag.as_ref().unwrap().color, 0xb48ead);
                assert_eq!(store.tags().unwrap()[0].color, 0xb48ead);
                app.select_tag(prod.clone(), cx);
                assert!(app.draft.tag.is_none());
                app.select_tag(prod.clone(), cx);
                app.delete_connection_tag(prod.id, cx);
                assert!(app.draft.tag.is_none());
                assert!(app.sessions[0].tag.is_none());
                assert!(app.connection_tags.iter().all(|tag| tag.id != prod.id));
                assert_eq!(store.tags().unwrap(), app.connection_tags);
                let _ = gpui::Render::render(app, window, cx);
                app.compact_layout = true;
                let _ = gpui::Render::render(app, window, cx);
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
    fn named_query_drafts_are_flushed_before_vault_lock(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let profile_store = ProfileStore::at(directory.path().join("connections.json"));
        let vault = profile_store.vault().unwrap();
        vault.create("draft recovery passphrase").unwrap();
        let workspace = Arc::new(crate::workspace::WorkspaceStore::new(vault.clone()));
        let session_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        let key = crate::workspace::connection_key(&QueryHistoryConnection::profile(profile_id));
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.profile_store = Some(profile_store);
                app.workspace_store = Some(workspace.clone());
                app.workspace_documents
                    .insert(key.clone(), Default::default());
                app.vault_state = Some(VaultState::Unlocked);
                app.sessions.push(ConnectionSession::new(
                    session_id,
                    Some(profile_id),
                    "Recovery test".into(),
                    DatabaseKind::SQLite,
                    None,
                    window,
                    cx,
                ));
                app.active_session_id = Some(session_id);
                app.open_saved_query_for(
                    session_id,
                    crate::workspace::SavedQuery {
                        name: "Unfinished investigation".into(),
                        sql: "SELECT 'private unfinished draft'".into(),
                    },
                    window,
                    cx,
                );
                app.save_named_query_for(session_id, cx);
                app.lock_vault(cx);
                assert!(app.sessions.is_empty());
                assert!(app.workspace_documents.is_empty());
            })
        });
        vault.unlock("draft recovery passphrase").unwrap();
        let recovered = workspace.load(&key).unwrap();
        assert_eq!(recovered.drafts.len(), 1);
        assert_eq!(recovered.saved.len(), 1);
        assert_eq!(recovered.drafts[0].sql, "SELECT 'private unfinished draft'");
        assert_eq!(recovered.saved[0].name, "Unfinished investigation");
    }
    #[gpui::test]
    fn workbench_renders_transaction_controls_result_tabs_and_saved_queries(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Workbench test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            let mut query = QueryTab::new(DatabaseKind::SQLite, session_id, tab_id, window, cx);
            query.in_transaction = true;
            query.statement_results = vec![
                StatementResult {
                    statement: "SELECT 1".into(),
                    result: QueryResult::empty(None, 1),
                    error: None,
                },
                StatementResult {
                    statement: "SELECT 2".into(),
                    result: QueryResult {
                        columns: vec![ColumnInfo::result("second", 0, "INTEGER")],
                        rows: vec![dbx_core::RowData::new(vec![CellValue::Integer(2)])],
                        ..QueryResult::empty(None, 1)
                    },
                    error: None,
                },
            ];
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Query(Box::new(query)),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Query;
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app.connection_picker_open = false;
            app
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("save-named-query").is_some());
        assert!(cx.debug_bounds("query-workbench-more").is_some());
        assert!(cx.debug_bounds("commit-query").is_some());
        assert!(cx.debug_bounds("rollback-query").is_some());
        assert!(cx.debug_bounds("statement-result-0").is_some());
        assert!(cx.debug_bounds("statement-result-1").is_some());
        let second = cx.debug_bounds("statement-result-1").unwrap();
        cx.simulate_click(second.center(), gpui::Modifiers::none());
        cx.update(|_, cx| {
            let session = app.read(cx).session(session_id).unwrap();
            let SecondaryTabKind::Query(query) = &session.secondary_tabs[0].kind else {
                panic!("query tab")
            };
            assert_eq!(query.active_result, 1);
            assert_eq!(query.result.as_ref().unwrap().columns[0].name, "second");
            assert_eq!(
                query.result.as_ref().unwrap().rows[0].values[0],
                CellValue::Integer(2)
            );
        });
        cx.simulate_resize(gpui::size(px(720.), px(640.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for selector in [
            "save-named-query",
            "commit-query",
            "rollback-query",
            "query-workbench-more",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .expect("query control remains visible");
            assert!(
                bounds.origin.x >= px(0.) && bounds.right() <= px(720.),
                "{selector} is outside the compact window: {bounds:?}"
            );
        }
        let options = cx.debug_bounds("query-workbench-more").unwrap();
        cx.simulate_click(options.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|_, cx| assert!(app.read(cx).session(session_id).is_some()));
    }

    #[gpui::test]
    fn focused_redis_query_renders_runtime_catalog_completion_menu(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            let tab_id = Uuid::new_v4();
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Redis test".into(),
                DatabaseKind::Redis,
                Some(default_tags().remove(3)),
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
                    SecondaryTabKind::Data(_)
                    | SecondaryTabKind::Structure(_)
                    | SecondaryTabKind::Diagram(_) => None,
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
    fn sidebar_list_rebuilds_only_when_its_inputs_change() {
        let tables = vec![
            TableInfo::table("Events", Some("analytics".into())),
            TableInfo::table("users", Some("public".into())),
            TableInfo::table("accounts", Some("public".into())),
        ];
        let kind = DatabaseKind::PostgreSQL;
        let names = |list: &SidebarList| {
            list.visible
                .iter()
                .map(|table| table.name.clone())
                .collect::<Vec<_>>()
        };
        let mut list = SidebarList::default();

        assert!(list.refresh(kind, &tables, 1, None, ""));
        assert_eq!(names(&list), vec!["Events", "users", "accounts"]);
        let first_visible = list.visible.clone();
        let first_options = list.schema_options.clone();

        // Same inputs, including whitespace-only search noise: no rebuild.
        assert!(!list.refresh(kind, &tables, 1, None, "  "));
        assert!(Arc::ptr_eq(&first_visible, &list.visible));

        // Search is case-insensitive and keeps the schema options cached.
        assert!(list.refresh(kind, &tables, 1, None, "EVE"));
        assert_eq!(names(&list), vec!["Events"]);
        assert!(Arc::ptr_eq(&first_options, &list.schema_options));

        assert!(list.refresh(kind, &tables, 1, Some("public"), ""));
        assert_eq!(names(&list), vec!["users", "accounts"]);

        // A new table list bumps the revision and rebuilds the options too.
        let mut more = tables.clone();
        more.push(TableInfo::table("orders", Some("sales".into())));
        assert!(list.refresh(kind, &more, 2, Some("public"), ""));
        assert_eq!(list.schema_options.len(), 4);
    }

    #[test]
    fn sidebar_list_ignores_schema_filter_outside_postgres() {
        let tables = vec![TableInfo::table("t", None), TableInfo::table("u", None)];
        let mut list = SidebarList::default();
        assert!(list.refresh(DatabaseKind::SQLite, &tables, 1, None, ""));
        assert_eq!(list.visible.len(), 2);
        assert!(list.schema_options.is_empty());
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
        assert!(table_is_visible_in(
            DatabaseKind::PostgreSQL,
            Some("public"),
            Some("public")
        ));
        assert!(!table_is_visible_in(
            DatabaseKind::PostgreSQL,
            Some("public"),
            Some("analytics")
        ));
    }
}
