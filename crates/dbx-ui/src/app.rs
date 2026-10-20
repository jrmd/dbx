//! THESIS: DBX is a dense native database cockpit; it rejects the centered-card utility shell.
//! OWN-WORLD: Near-black layered panes, hairline borders, blue navigation, green health, 6px controls.
//! STORY: Pick or open a connection, keep its tab, browse context, then inspect or query without losing place.
//! FIRST VIEWPORT: A 46px rail, 40px primary connection tabs, explorer, data canvas, and row inspector.
//! FORM: Reference-led operator console, user-supplied DBX screen map; seed key: dbx-native-console.
//! FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, and DESIGN.md.
//! IMPLEMENTATION: `DbxApp` coordinates shared session state; focused workflows and rendering live in
//! the private `app/` module tree documented in `docs/architecture.md`.

use crate::accessibility::DialogElement as _;
use crate::accessibility::SelectAccessibility as _;

mod agents;
mod cell_edits;
mod designer;
mod query_actions;
mod schema_objects;
mod session;
mod tabs;
mod theme_settings;
use session::*;
mod backups;
mod connection;
mod data_clipboard;
mod data_import;
mod diagnostics;
mod explorer;
mod find;
mod focus;
pub use focus::focus_key_bindings;
mod commands;
mod driver_plugins;
mod mcp;
mod nonrelational;
mod profile_transfer;
mod query_language;
mod query_parameters;
mod quick_open;
mod redis_completion;
mod result_table;
mod row_count;
mod sql_completion;
mod sql_library;
mod table_commands;
mod table_layout;
mod transfer;
pub(crate) mod value_view;
mod view;
mod workspace;

use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    rc::Rc,
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
use gpui::UniformListScrollHandle;
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, CursorStyle, Decorations, Div, ElementId,
    Entity, FocusHandle, Focusable as _, FontWeight, Image, ImageFormat, IntoElement, KeyDownEvent,
    MouseButton, PathPromptOptions, Pixels, Point, Render, ResizeEdge, Rgba, ScrollHandle,
    SharedString, Stateful, StatefulInteractiveElement, Subscription, Window, WindowControlArea,
    WindowHandle, anchored, deferred, div, img, point, prelude::*, px, relative, uniform_list,
};
use gpui_component::{
    Disableable as _, FocusTrapElement as _, IndexPath, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    resizable::ResizableState,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    table::{TableEvent, TableState},
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
        Appearance, ButtonAccessibility as _, ButtonKind, FollowCorners, GLASS_INSET, Icon,
        Pressable as _, RADIUS_CONTROL, RADIUS_GLASS, RADIUS_PANEL, appearance, badge, button,
        connection_tab, database_logo, glass, glass_icon_button, glass_raised, glass_shadow, icon,
        panel_header, reduce_transparency, segment, segmented_track, set_appearance,
        set_reduce_transparency, set_system_appearance, settings_group, settings_row,
        settings_switch, shortcut, sync_component_theme, theme, tip, window_background,
    },
    vault::{VaultError, VaultState},
};
use redis_completion::redis_completion_items;
use result_table::{ResultTableDelegate, foreign_key_target_table};
use sql_completion::{
    SqlCompletionItem, SqlCompletionRequest, StatementSource, completion_table_key,
    sql_completion_items_with_relations, statement_sources,
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
        CompletionTab,
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
        FocusNext,
        FocusPrevious,
        NewConnection,
        NewQuery,
        CloseTab,
        NextConnection,
        PreviousConnection,
        ToggleSidebar,
        FocusExplorer,
        NextTab,
        PreviousTab,
        ExplorerNext,
        ExplorerPrevious,
        ExplorerFirst,
        ExplorerLast,
        ExplorerOpen,
        ExplorerContextMenu,
        TableMenuNext,
        TableMenuPrevious,
        TableMenuFirst,
        TableMenuLast,
        TableMenuConfirm,
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
        UndoGridEdits,
        RedoGridEdits,
        CopyDataSelection,
        PasteRows,
        OpenQuickOpen,
        OpenCommandPalette,
        OpenSqlFile,
        ChooseTheme,
        LightAppearance,
        DarkAppearance,
        SystemAppearance,
        BrowseSqlFiles,
        FavoriteSqlFiles,
        SaveSqlFile,
        ChooseSqlFolder,
        ExplainQuery,
        AnalyzeQuery,
        QueryStatistics,
        TableStatistics,
        FullTableStatistics,
        WatchQuery,
        StopWatch,
        InspectSchema,
        CaptureSchema,
        ExportDatabase,
        ExportQueryCsv,
        ExportQueryXlsx,
        ExportFullQueryXlsx,
        ExportFullTableXlsx,
        OpenSettings,
        OpenKeyboardShortcuts,
        SearchQueryHistory,
        QuickOpenNext,
        QuickOpenPrevious,
        QuickOpenConfirm,
        SubmitTableName,
        ConfirmAction,
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
    Tab,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum QueryResultExportFormat {
    Tsv,
    Csv,
    Json,
    Markdown,
    Xlsx,
    Insert,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum QueryResultExportAction {
    Copy,
    Export,
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
            Self::Markdown => "md",
            Self::Xlsx => "xlsx",
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
    hovered: bool,
    remaining: std::time::Duration,
}

impl Toast {
    fn elapse(&mut self, elapsed: std::time::Duration) -> bool {
        if !self.hovered {
            self.remaining = self.remaining.saturating_sub(elapsed);
        }
        self.remaining.is_zero()
    }
}

const MAX_TOASTS: usize = 3;

fn toast_reading_time(message: &str) -> std::time::Duration {
    std::time::Duration::from_millis((6000 + 40 * message.chars().count() as u64).min(20_000))
}

struct ConnectionDraft {
    driver_manifest: Option<PathBuf>,
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
    /// The confirmed folder; only changed by the folder menu.
    folder: Entity<String>,
    folder_focus: FocusHandle,
    /// The inline "New folder" form is open.
    creating_folder: bool,
    new_folder: Entity<String>,
    new_folder_editor: Entity<TextEditor>,
    folder_error: Option<String>,
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
        let mut field = |label: &str, initial: &str| {
            let value = cx.new(|_| initial.to_owned());
            let editor = cx.new(|cx| {
                TextEditor::new(value.clone(), false, window, cx)
                    .accessibility_label(label.to_owned())
            });
            (value, editor)
        };
        let (socket, socket_editor) = field("Unix socket path", "");
        let (ssh_host, ssh_host_editor) = field("SSH host", "");
        let (ssh_port, ssh_port_editor) = field("SSH port", "22");
        let (ssh_user, ssh_user_editor) = field("SSH username", "");
        let (ssh_key, ssh_key_editor) = field("SSH private key path", "");
        let (ssh_jump, ssh_jump_editor) = field("SSH jump host", "");
        let ssh_password = cx.new(|_| String::new());
        let ssh_password_editor = cx.new(|cx| {
            TextEditor::new(ssh_password.clone(), false, window, cx)
                .accessibility_label("SSH password")
                .password()
        });
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
        let name_editor = cx.new(|cx| {
            TextEditor::new(name.clone(), false, window, cx).accessibility_label("Tag name")
        });
        let color_editor = cx.new(|cx| {
            TextEditor::new(color.clone(), false, window, cx).accessibility_label("Tag colour")
        });
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
        let passphrase_editor = cx.new(|cx| {
            TextEditor::new(passphrase.clone(), false, window, cx)
                .accessibility_label("Vault passphrase")
                .password()
        });
        let confirmation_editor = cx.new(|cx| {
            TextEditor::new(confirmation.clone(), false, window, cx)
                .accessibility_label("Confirm vault passphrase")
                .password()
        });

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
        let folder = cx.new(|_| String::new());
        let new_folder = cx.new(|_| String::new());
        let new_folder_editor = cx.new(|cx| {
            TextEditor::new(new_folder.clone(), false, window, cx)
                .accessibility_label("New folder name")
        });
        let import_url = cx.new(|_| String::new());
        let import_url_editor = cx.new(|cx| {
            TextEditor::new(import_url.clone(), false, window, cx)
                .accessibility_label("Paste a connection URL or file path")
        });
        let fields = ConnectionFields::from_url("sqlite://dbx.db?mode=rwc")
            .expect("default SQLite connection URL is valid");
        let connection_url = cx.new(|_| fields.connection_string.clone());
        let host = cx.new(|_| fields.host.clone());
        let port = cx.new(|_| fields.port.clone());
        let username = cx.new(|_| fields.username.clone());
        let password = cx.new(|_| fields.password.clone());
        let database = cx.new(|_| fields.database.clone());
        let connection_name_editor = cx.new(|cx| {
            TextEditor::new(connection_name.clone(), false, window, cx)
                .accessibility_label("Connection name")
        });
        let connection_editor = cx.new(|cx| {
            TextEditor::new(connection_url.clone(), false, window, cx)
                .accessibility_label("Database file or connection string")
        });
        let host_editor = cx
            .new(|cx| TextEditor::new(host.clone(), false, window, cx).accessibility_label("Host"));
        let port_editor = cx
            .new(|cx| TextEditor::new(port.clone(), false, window, cx).accessibility_label("Port"));
        let username_editor = cx.new(|cx| {
            TextEditor::new(username.clone(), false, window, cx).accessibility_label("Username")
        });
        let password_editor = cx.new(|cx| {
            TextEditor::new(password.clone(), false, window, cx)
                .accessibility_label("Password or API token")
                .password()
        });
        let database_editor = cx.new(|cx| {
            TextEditor::new(database.clone(), false, window, cx).accessibility_label("Database")
        });

        Self {
            kind: DatabaseKind::SQLite,
            driver_manifest: None,
            read_only: false,
            cloud_auth: None,
            mode: ConnectionFormMode::Details,
            selected_profile: None,
            choosing_kind: true,
            import_url,
            import_url_editor,
            tag: Some(default_tags().remove(3)),
            folder,
            folder_focus: cx.focus_handle(),
            creating_folder: false,
            new_folder,
            new_folder_editor,
            folder_error: None,
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
    tables: Vec<TableInfo>,
    position: Point<gpui::Pixels>,
    /// Highlighted item (an index into the menu's rows); `None` until the
    /// keyboard is used, so a right-click doesn't pre-select anything.
    cursor: Option<usize>,
    focus: FocusHandle,
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
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
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
    RemoveDriver {
        path: PathBuf,
    },
    DeleteProfile {
        id: uuid::Uuid,
    },
    RunQuery {
        session_id: SessionId,
        tab_id: SecondaryTabId,
        run_all: bool,
        query: query_actions::ExecutionSource,
    },
    AnalyzeQuery {
        session_id: SessionId,
        tab_id: SecondaryTabId,
        sql: String,
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
        tables: Vec<TableInfo>,
        cascade: bool,
    },
    CommitChanges {
        session_id: SessionId,
        tab_id: SecondaryTabId,
    },
    Quit,
    RemoveCustomTheme {
        index: usize,
    },
    DiscardDataTab {
        session_id: SessionId,
        tab_id: SecondaryTabId,
    },
    DiscardEdits {
        session_id: SessionId,
        tab_id: SecondaryTabId,
    },
    DeleteSavedQuery {
        session_id: SessionId,
        name: String,
    },
    DeleteSavedFilter {
        session_id: SessionId,
        tab_id: SecondaryTabId,
        index: usize,
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
            Self::LockVault
            | Self::Quit
            | Self::RemoveCustomTheme { .. }
            | Self::RemoveDriver { .. }
            | Self::DeleteProfile { .. } => None,
            Self::RunQuery { session_id, .. }
            | Self::AnalyzeQuery { session_id, .. }
            | Self::CloseQuery { session_id, .. }
            | Self::ClearQueryHistory { session_id }
            | Self::Table { session_id, .. }
            | Self::CommitChanges { session_id, .. }
            | Self::DiscardDataTab { session_id, .. }
            | Self::DiscardEdits { session_id, .. }
            | Self::DeleteSavedQuery { session_id, .. }
            | Self::DeleteSavedFilter { session_id, .. }
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
    Editor,
    QueryAgent,
    Shortcuts,
    Connections,
    Tags,
    Updates,
}

/// Key bindings for explorer, table-menu and tab navigation.
pub fn explorer_key_bindings() -> Vec<gpui::KeyBinding> {
    explorer::key_bindings()
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
    touch_id_enabled: bool,
    touch_id_available: bool,
    touch_id_enrolled: bool,
    touch_id_probe_generation: u64,
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
    sql_library: crate::query_library::SqlLibrary,
    sql_library_refreshing: bool,
    sql_library_generation: u64,
    sql_library_favorite_pending: bool,
    workspace_documents: HashMap<String, crate::workspace::WorkspaceDocument>,
    startup_recovery_started: bool,
    sessions: Vec<ConnectionSession>,
    active_session_id: Option<SessionId>,
    connection_picker_open: bool,
    compact_connection_form_open: bool,
    /// UI-only disclosure state; `None` follows the draft's content.
    connection_drivers_open: Option<bool>,
    connection_network_open: Option<bool>,
    table_context_menu: Option<TableContextMenu>,
    /// Physical Control state, before macOS rewrites Control-click as right-click.
    explorer_control_held: bool,
    table_name_prompt: Option<table_commands::TableNamePrompt>,
    nonrelational_editor: Option<nonrelational::NonrelationalEditor>,
    nonrelational_generation: u64,
    database_export_dialog: Option<DatabaseExportDialog>,
    confirmation_dialog: Option<ConfirmationDialog>,
    table_confirmation_input: Option<(String, Entity<TextEditor>, Subscription)>,
    quick_open: Option<quick_open::QuickOpen>,
    copied_table_data: Option<Arc<dbx_core::data_import::ImportData>>,
    data_import_dialog: Option<data_import::DataImportDialog>,
    profile_transfer_dialog: Option<profile_transfer::ProfileTransferDialog>,
    mutation_error_dialog: Option<MutationErrorDialog>,
    shortcut_editor: Option<commands::ShortcutEditor>,
    driver_registry: driver_plugins::DriverRegistry,
    settings_open: bool,
    settings_focus: FocusHandle,
    settings_scroll: gpui::ScrollHandle,
    settings_return_focus: Option<FocusHandle>,
    settings_section: SettingsSection,
    tag_editor: TagEditor,
    appearance: Appearance,
    theme_preferences: crate::theme::catalog::ThemePreferences,
    theme_editor: Option<theme_settings::ThemeEditor>,
    reduce_transparency: bool,
    ui_scale: u16,
    editor_scale: u16,
    font_family: Option<String>,
    mono_font_family: Option<String>,
    row_font_family: Option<String>,
    font_size: u16,
    mono_font_size: u16,
    row_font_size: u16,
    font_size_inputs: Option<[Entity<gpui_component::input::InputState>; 3]>,
    font_pickers: Option<[Entity<SelectState<SearchableVec<SharedString>>>; 3]>,
    font_names_loading: bool,
    drag_row_selection: bool,
    row_drag: Option<(SessionId, SecondaryTabId, usize)>,
    vim_mode: bool,
    settings_store: Option<SettingsStore>,
    #[cfg(test)]
    _settings_directory: tempfile::TempDir,
    agent_setup: agents::AgentSetup,
    compact_layout: bool,
    narrow_workspace: bool,
    sidebar_hidden: bool,
    /// Widths the user dragged the explorer and row inspector to; `None`
    /// keeps the defaults.
    explorer_width: Option<f32>,
    inspector_width: Option<f32>,
    pane_resizing: Option<view::PaneResize>,
    /// Owns keyboard focus whenever no control does, so app shortcuts keep
    /// working after the focused element (e.g. a closed tab's editor) is gone.
    focus_handle: FocusHandle,
    /// The topmost modal as of the last render, to notice when one opens.
    last_modal_trap: Option<FocusHandle>,
    /// A modal opened and hasn't been checked for focus since it painted.
    modal_focus_pending: bool,
    connection_form_focus_pending: bool,
    /// Focus was outside the app's tree on the previous render.
    stray_focus_seen: bool,
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

        #[cfg(not(test))]
        let settings_store = SettingsStore::new().ok();
        // UI tests must not inherit or overwrite the developer's live preferences.
        #[cfg(test)]
        let settings_directory = tempfile::tempdir().expect("test settings directory");
        #[cfg(test)]
        let settings_store = Some(SettingsStore::at(
            settings_directory.path().join("settings.json"),
        ));
        let saved_settings = settings_store.as_ref().and_then(|store| store.load().ok());
        let mut this = Self {
            runtime: Arc::new(tokio::runtime::Runtime::new().expect("create DBX Tokio runtime")),
            update_state: crate::updater::UpdateState::Idle,
            logo: Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_BYTES.to_vec())),
            draft,
            vault_editors,
            vault_state,
            vault_busy: false,
            remember_device: saved_settings
                .as_ref()
                .is_none_or(|settings| settings.remember_device && !settings.touch_id_enabled),
            touch_id_enabled: saved_settings
                .as_ref()
                .is_some_and(|settings| settings.touch_id_enabled),
            touch_id_available: false,
            touch_id_enrolled: false,
            touch_id_probe_generation: 0,
            saving_connection: false,
            vault_generation: 0,
            credential_hydrating: false,
            credential_hydration_generation: 0,
            credential_connect_window: None,
            sql_library: Default::default(),
            sql_library_refreshing: false,
            sql_library_generation: 0,
            sql_library_favorite_pending: false,
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
            connection_drivers_open: None,
            connection_network_open: None,
            table_context_menu: None,
            explorer_control_held: false,
            table_name_prompt: None,
            nonrelational_editor: None,
            nonrelational_generation: 0,
            database_export_dialog: None,
            confirmation_dialog: None,
            table_confirmation_input: None,
            quick_open: None,
            profile_transfer_dialog: None,
            data_import_dialog: None,
            copied_table_data: None,
            mutation_error_dialog: None,
            shortcut_editor: None,
            driver_registry: driver_plugins::DriverRegistry::default(),
            settings_open: false,
            settings_focus: cx.focus_handle(),
            settings_scroll: gpui::ScrollHandle::new(),
            settings_return_focus: None,
            settings_section: SettingsSection::Appearance,
            tag_editor,
            appearance: appearance(),
            theme_preferences: saved_settings
                .as_ref()
                .map(|s| s.theme.clone())
                .unwrap_or_default(),
            theme_editor: None,
            reduce_transparency: reduce_transparency(),
            ui_scale: saved_settings
                .as_ref()
                .map_or(100, |settings| settings.ui_scale.clamp(100, 200)),
            editor_scale: saved_settings
                .as_ref()
                .map_or(100, |settings| settings.editor_scale.clamp(100, 200)),
            font_family: saved_settings.as_ref().and_then(|s| s.font_family.clone()),
            mono_font_family: saved_settings
                .as_ref()
                .and_then(|s| s.mono_font_family.clone()),
            row_font_family: saved_settings
                .as_ref()
                .and_then(|s| s.row_font_family.clone()),
            font_size: saved_settings
                .as_ref()
                .map_or(12, |s| s.font_size.clamp(10, 20)),
            mono_font_size: saved_settings.as_ref().map_or(12, |s| {
                s.mono_font_size.unwrap_or(s.font_size).clamp(10, 20)
            }),
            row_font_size: saved_settings
                .as_ref()
                .map_or(12, |s| s.row_font_size.unwrap_or(s.font_size).clamp(10, 20)),
            font_size_inputs: None,
            font_pickers: None,
            font_names_loading: false,
            drag_row_selection: saved_settings.as_ref().is_none_or(|s| s.drag_row_selection),
            row_drag: None,
            vim_mode: saved_settings
                .as_ref()
                .is_some_and(|settings| settings.vim_mode),
            settings_store,
            #[cfg(test)]
            _settings_directory: settings_directory,
            agent_setup: agents::AgentSetup::new(window, cx),
            compact_layout: false,
            narrow_workspace: false,
            sidebar_hidden: false,
            explorer_width: saved_settings
                .as_ref()
                .and_then(|settings| settings.explorer_width)
                .map(|width| width as f32),
            inspector_width: saved_settings
                .as_ref()
                .and_then(|settings| settings.inspector_width)
                .map(|width| width as f32),
            pane_resizing: None,
            focus_handle: cx.focus_handle(),
            last_modal_trap: None,
            modal_focus_pending: false,
            connection_form_focus_pending: false,
            stray_focus_seen: false,
            toasts: Vec::new(),
            next_toast_id: 0,
            window_drag_armed: false,
            test_generation: 0,
            testing_connection: false,
            connection_test_abort: AbortOnDrop::default(),
            _subscriptions: subscriptions,
            error: profile_error,
        };
        crate::theme::catalog::set_preferences(&this.theme_preferences);
        this.refresh_touch_id(cx);
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

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_section(SettingsSection::Appearance, window, cx);
    }

    fn open_settings_section(
        &mut self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        self.settings_section = section;
        self.settings_scroll.set_offset(point(px(0.), px(0.)));
        if !self.settings_open {
            self.settings_return_focus = window.focused(cx);
            self.settings_focus.focus(window, cx);
            self.settings_open = true;
        }
        self.check_agent_cli(cx);
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.settings_open) {
            self.edit_tag(None, cx);
            self.settings_return_focus
                .take()
                .unwrap_or_else(|| self.focus_handle.clone())
                .focus(window, cx);
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
        self.refresh_theme_color_input(window, cx);
        cx.refresh_windows();
    }

    fn persist_settings(&mut self, cx: &mut Context<Self>) {
        let mut settings = Settings::new(self.appearance)
            .with_reduce_transparency(self.reduce_transparency)
            .with_remember_device(self.remember_device);
        settings.touch_id_enabled = self.touch_id_enabled;
        settings.theme = self.theme_preferences.clone();
        settings.agents = self.agent_setup.preferences.clone();
        settings.vim_mode = self.vim_mode;
        settings.motion = crate::motion::preference(cx);
        settings.ui_scale = self.ui_scale;
        settings.editor_scale = self.editor_scale;
        settings.font_family = self.font_family.clone();
        settings.mono_font_family = self.mono_font_family.clone();
        settings.row_font_family = self.row_font_family.clone();
        settings.font_size = self.font_size;
        settings.mono_font_size = Some(self.mono_font_size);
        settings.row_font_size = Some(self.row_font_size);
        settings.drag_row_selection = self.drag_row_selection;
        settings.explorer_width = self.explorer_width.map(|width| width.round() as u32);
        settings.inspector_width = self.inspector_width.map(|width| width.round() as u32);
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

    pub(super) fn set_text_scale(
        &mut self,
        scale: u16,
        editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scale = scale.clamp(100, 200);
        let preference = if editor {
            &mut self.editor_scale
        } else {
            &mut self.ui_scale
        };
        if *preference == scale {
            return;
        }
        let ratio = scale as f32 / *preference as f32;
        *preference = scale;
        crate::theme::set_text_scales(self.ui_scale, self.editor_scale);
        window.set_rem_size(gpui::px(16. * crate::theme::ui_scale()));
        if !editor {
            // Existing column widths must follow the text immediately; their
            // persisted values remain in screen pixels, as before.
            for session in &mut self.sessions {
                for tab in &mut session.secondary_tabs {
                    let grid = match &mut tab.kind {
                        SecondaryTabKind::Data(data) => &data.data_grid,
                        SecondaryTabKind::Query(query) => &query.result_grid,
                        _ => continue,
                    };
                    grid.update(cx, |table, cx| {
                        table.delegate_mut().scale_column_widths(ratio);
                        table.refresh(cx);
                        result_table::emit_widths(table, cx);
                    });
                }
            }
        }
        self.persist_settings(cx);
        window.refresh();
        cx.notify();
    }

    fn change_text_scale_on_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if !modifiers.secondary() || modifiers.alt {
            return;
        }
        let scale = match event.keystroke.key.as_str() {
            "+" | "=" => self.ui_scale.saturating_add(25),
            "-" => self.ui_scale.saturating_sub(25),
            "0" if !modifiers.shift => 100,
            _ => return,
        };
        cx.stop_propagation();
        self.set_text_scale(scale, false, window, cx);
    }

    fn toggle_vim_mode(&mut self, cx: &mut Context<Self>) {
        self.vim_mode = !self.vim_mode;
        for session in &self.sessions {
            for tab in &session.secondary_tabs {
                if let SecondaryTabKind::Query(query) = &tab.kind {
                    query
                        .query_editor
                        .update(cx, |editor, cx| editor.set_vim_mode(self.vim_mode, cx));
                }
            }
        }
        self.persist_settings(cx);
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
        let dismissed = if self.confirmation_dialog.is_some() {
            self.cancel_confirmation(window, cx);
            true
        } else if self.workspace_screen_drawn() && self.dismiss_mutation_error_dialog(window, cx) {
            true
        } else if self
            .data_import_dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.busy)
        {
            self.close_data_import(window, cx);
            true
        } else if self
            .profile_transfer_dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.busy)
        {
            self.close_profile_transfer(window, cx);
            true
        } else if self.quick_open.is_some() {
            self.close_quick_open(window, cx);
            true
        } else if self.nonrelational_editor.is_some() {
            self.close_nonrelational_editor(window, cx);
            true
        } else if self.table_name_prompt.is_some() {
            self.close_table_name_prompt(window, cx);
            true
        } else if self.workspace_screen_drawn() && self.cancel_database_export(window, cx) {
            true
        } else if self.settings_open {
            self.close_settings(window, cx);
            true
        } else {
            self.dismiss_table_context_menu(window, cx) || self.dismiss_connection_picker()
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
        self.invalidate_nonrelational_for(Some(session_id));
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
                row.editor.update(cx, |editor, cx| {
                    editor.set_accessibility_label(format!("Filter value for {column}"), cx)
                });
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
            .is_none_or(|engine| !engine.capabilities().row_edits)
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
            if !query_tab.query_editor.read(cx).accepts_insert_input() {
                return None;
            }
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
                relational_schema: session
                    .completion_schema
                    .as_ref()
                    .map_or(0, |schema| Arc::as_ptr(schema) as usize),
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
                    editor::sql_completion_context_for(&query_text, cursor, Some(session.kind)).map(
                        |context| {
                            let items = sql_completion_items_with_relations(
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
                                session.completion_schema.as_deref(),
                            );
                            (context.replacement_range, items)
                        },
                    )
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
                CompletionAction::Tab => {
                    query_editor.update(cx, |editor, cx| editor.indent_selection(cx));
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
            CompletionAction::Enter | CompletionAction::Tab => {
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
            let schema = engine.relational_schema().await.ok().map(Arc::new);
            let columns = schema
                .as_ref()
                .map(|schema| {
                    schema
                        .tables
                        .iter()
                        .filter(|entry| {
                            matches!(entry.table.kind, EntityKind::Table | EntityKind::View)
                        })
                        .map(|entry| {
                            (
                                completion_table_key(&table_ref(&entry.table)),
                                entry.structure.columns.clone(),
                            )
                        })
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();
            (columns, schema)
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }

        cx.spawn(async move |this, cx| {
            let (metadata, schema) = task.await?;

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
                session.completion_schema = schema;
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

    fn toggle_sidebar_action(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.inspector_page() {
            if let Some((session, tab)) = self
                .active_session()
                .and_then(|session| session.active_data_tab_id().map(|tab| (session.id, tab)))
            {
                self.close_inspector_for(session, tab, window, cx);
            }
        } else {
            self.toggle_sidebar(cx);
        }
    }

    pub(crate) fn show_toast(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        let message = message.into();
        let remaining = toast_reading_time(&message);
        self.toasts.push(Toast {
            id,
            kind,
            message,
            hovered: false,
            remaining,
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
        cx.spawn(async move |this, cx| {
            let tick = std::time::Duration::from_millis(250);
            loop {
                cx.background_executor().timer(tick).await;
                let keep = this
                    .update(cx, |this, cx| {
                        let Some(toast) = this.toasts.iter_mut().find(|toast| toast.id == id)
                        else {
                            return false;
                        };
                        if toast.elapse(tick) {
                            this.dismiss_toast(id, cx);
                            return false;
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
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
                        session.sidebar.selected.clear();
                        session.sidebar.cursor = None;
                        session.schema_objects.clear();
                        session.schema_objects_error = None;
                        // Open tables belong to the previous database.
                        session.close_data_tabs();
                        session.completion_columns.clear();
                        session.completion_schema = None;
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
        if self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| data.row_draft.is_some())
        {
            self.open_inspector_for(session_id, tab_id, window, cx);
            return;
        }
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
        data.inspector_expanded = true;
        data.clear_grid_selection(cx);
        data.row_draft = Some(row_draft);
        self.focus_inspector_for(session_id, tab_id, window, cx);
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
            TableEvent::DoubleClickedCell(row, column) => {
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
            let mut field = match staged.get(index).cloned().flatten() {
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
            if crate::row_drafts::field_value_kind(&field.column)
                == crate::row_drafts::FieldValueKind::Bytes
            {
                let mut format = self
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .data_grid
                    .read(cx)
                    .delegate()
                    .binary_format(index);
                let value = staged
                    .get(index)
                    .cloned()
                    .flatten()
                    .and_then(|value| {
                        if let MutationValue::Parameter(value) = value {
                            Some(value)
                        } else {
                            None
                        }
                    })
                    .or_else(|| field.original.clone());
                if matches!(&value, Some(CellValue::Bytes(bytes)) if bytes.len() != 16)
                    && matches!(
                        format,
                        value_view::BinaryFormat::Uuid | value_view::BinaryFormat::UuidSwapped
                    )
                {
                    format = value_view::BinaryFormat::Hex;
                }
                if field.state == FieldValueState::Value
                    && let Some(value @ CellValue::Bytes(_)) = value
                {
                    field.editor.update(cx, |editor, cx| {
                        editor.set_text(format.editor_text(&value), cx)
                    });
                }
                field.binary_format = Some(format);
            }
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
        data.inspector_expanded = true;
        data.row_draft = Some(draft);
        self.focus_inspector_for(session_id, tab_id, window, cx);
        cx.notify();
    }

    pub(super) fn inspector_page(&self) -> bool {
        self.narrow_workspace
            && self.active_session().is_some_and(|session| {
                session.pane == Pane::Data
                    && session.active_data_tab().is_some_and(|data| {
                        data.inspector_open && (data.inspector_expanded || data.row_draft.is_some())
                    })
            })
    }

    fn focus_inspector_for(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        let focus = data
            .row_draft
            .as_ref()
            .and_then(|draft| draft.fields().first())
            .map(|field| {
                if let Some(selector) = &field.state_selector {
                    selector.focus_handle(cx)
                } else if let Some(selector) = field
                    .boolean_selector
                    .as_ref()
                    .or(field.enum_selector.as_ref())
                {
                    selector.focus_handle(cx)
                } else {
                    field.editor.read(cx).focus_handle()
                }
            })
            .unwrap_or_else(|| data.inspector_focus.clone());
        focus.focus(window, cx);
    }

    fn open_inspector_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.inspector_open = true;
        data.inspector_expanded = true;
        self.focus_inspector_for(session_id, tab_id, window, cx);
        cx.notify();
    }

    fn close_inspector_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(data) = self.data_tab_mut(session_id, tab_id) {
            data.inspector_open = false;
            data.inspector_expanded = false;
            data.data_grid.focus_handle(cx).focus(window, cx);
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
        window: &mut Window,
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
        data.inspector_expanded = false;
        data.data_grid.focus_handle(cx).focus(window, cx);
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
                "Use the structured document/key editor or the native command console.".into(),
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
        if let Some(data) = self.data_tab_mut(session_id, tab_id)
            && data.row_draft.is_none()
        {
            data.inspector_expanded = false;
            data.data_grid.focus_handle(cx).focus(window, cx);
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
                "Use the structured document/key editor or the native command console",
                cx,
            );
            return;
        }
        self.delete_rows_for(session_id, tab_id, None, cx);
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
        self.confirm_table_actions(action, session_id, vec![table], window, cx);
    }

    fn confirm_table_actions(
        &mut self,
        action: TableAction,
        session_id: SessionId,
        tables: Vec<TableInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tables.is_empty() {
            return;
        }
        let return_focus = window.focused(cx);
        self.table_context_menu = None;
        let names = tables
            .iter()
            .map(|table| table_sidebar_label(table, None))
            .collect::<Vec<_>>()
            .join(", ");
        let subject = if tables.len() == 1 {
            names.clone()
        } else {
            format!("{} tables", tables.len())
        };
        let (title, detail, confirm_label) = match action {
            TableAction::Truncate => (
                format!("Truncate {subject}?"),
                format!(
                    "Every row in the selected tables will be permanently deleted. Table structures remain.\n\n{names}"
                ),
                if tables.len() == 1 {
                    "Truncate table"
                } else {
                    "Truncate tables"
                },
            ),
            TableAction::Drop => (
                format!("Delete {subject}?"),
                format!(
                    "The selected tables, their rows, indexes, and constraints will be permanently removed.\n\n{names}"
                ),
                if tables.len() == 1 {
                    "Delete table"
                } else {
                    "Delete tables"
                },
            ),
        };
        let detail = if tables.len() > 1 {
            format!(
                "{detail}\n\nExecution stops on the first error. Tables already completed remain changed."
            )
        } else {
            detail
        };
        self.table_confirmation_input = None;
        if self
            .session(session_id)
            .is_some_and(|session| session.tag.as_ref().is_some_and(|tag| tag.is_production()))
        {
            let input = cx.new(|cx| {
                TextEditor::empty(false, window, cx)
                    .accessibility_label("Type the table name to confirm")
            });
            let subscription = cx.observe(&input, |_, _, cx| cx.notify());
            self.table_confirmation_input =
                Some((table_action_confirmation_text(&tables), input, subscription));
        }
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title,
            detail,
            confirm_label,
            tone: ConfirmationTone::Danger,
            action: ConfirmationAction::Table {
                action,
                session_id,
                tables,
                cascade: false,
            },
            focus: focus.clone(),
            return_focus,
            sql: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    fn cancel_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.table_confirmation_input = None;
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

    fn table_confirmation_matches(&self, cx: &App) -> bool {
        let Some(ConfirmationDialog {
            action: ConfirmationAction::Table {
                session_id, tables, ..
            },
            ..
        }) = &self.confirmation_dialog
        else {
            return true;
        };
        if !self
            .session(*session_id)
            .is_some_and(|session| session.tag.as_ref().is_some_and(|tag| tag.is_production()))
        {
            return true;
        }
        self.table_confirmation_input
            .as_ref()
            .is_some_and(|(name, input, _)| {
                name == &table_action_confirmation_text(tables) && input.read(cx).text(cx) == *name
            })
    }

    fn confirm_pending_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The action path is guarded as well as the button, including Enter.
        if !self.table_confirmation_matches(cx) {
            return;
        }
        self.table_confirmation_input = None;
        let Some(dialog) = self.confirmation_dialog.take() else {
            return;
        };
        let return_focus = dialog.return_focus.clone();
        let closes_query = matches!(
            dialog.action,
            ConfirmationAction::CloseQuery { .. } | ConfirmationAction::DiscardDataTab { .. }
        );
        match dialog.action {
            ConfirmationAction::AnalyzeQuery {
                session_id,
                tab_id,
                sql,
            } => {
                self.analyze_query_for(session_id, tab_id, sql, cx);
            }
            ConfirmationAction::DeleteProfile { id } => self.change_saved_profile(id, true, cx),
            ConfirmationAction::RemoveDriver { path } => self.remove_driver_registration(path, cx),
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
                tables,
                cascade,
            } => self.execute_table_actions(action, session_id, tables, cascade, cx),
            ConfirmationAction::Quit => {
                cx.quit();
                return;
            }
            ConfirmationAction::RemoveCustomTheme { index } => {
                self.remove_custom_theme(index, window, cx)
            }
            ConfirmationAction::CommitChanges { session_id, tab_id } => {
                self.save_pending_edits_for(session_id, tab_id, window, cx)
            }
            ConfirmationAction::DiscardDataTab { session_id, tab_id } => {
                self.discard_pending_edits_for(session_id, tab_id, cx);
                self.close_secondary_tab_for(session_id, tab_id, cx);
            }
            ConfirmationAction::DiscardEdits { session_id, tab_id } => {
                self.discard_pending_edits_for(session_id, tab_id, cx);
            }
            ConfirmationAction::DeleteSavedQuery { session_id, name } => {
                self.delete_saved_query_for(session_id, &name, cx);
            }
            ConfirmationAction::DeleteSavedFilter {
                session_id,
                tab_id,
                index,
            } => self.delete_saved_filter_for(session_id, tab_id, index, cx),
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

    fn execute_table_actions(
        &mut self,
        action: TableAction,
        session_id: SessionId,
        tables: Vec<TableInfo>,
        cascade: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(engine) = session.engine.clone() else {
            return;
        };
        if session.busy
            || engine.is_read_only()
            || !session.kind.is_sql()
            || tables.is_empty()
            || tables.iter().any(|table| table.kind != EntityKind::Table)
        {
            return;
        }
        let targets = tables.iter().map(table_ref).collect::<Vec<_>>();
        // Cascading can touch any dependent table, including an unselected tab.
        let affected_tabs = session
            .secondary_tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                SecondaryTabKind::Data(data) if cascade || targets.contains(&data.table) => {
                    Some(tab.id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for &tab_id in &affected_tabs {
            if self.pending_edits_block(session_id, tab_id, cx) {
                return;
            }
            if self
                .data_tab(session_id, tab_id)
                .is_some_and(|data| data.busy)
            {
                self.show_toast(
                    ToastKind::Info,
                    "Wait for table work to finish before deleting or truncating",
                    cx,
                );
                return;
            }
        }
        let statements = match action {
            TableAction::Truncate => {
                dbx_core::build_truncate_tables(engine.kind(), &targets, cascade)
            }
            TableAction::Drop => dbx_core::build_drop_tables(engine.kind(), &targets),
        };
        let statements = match statements {
            Ok(statements) => statements,
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
        let runtime = self.runtime.clone();
        let session = self.session_mut(session_id).expect("session checked above");
        for &tab_id in &affected_tabs {
            if let Some(data) = session.data_tab_mut(tab_id) {
                data.busy = true;
            }
        }
        session.busy = true;
        session.error = None;
        session.status = format!(
            "{} {}…",
            if action == TableAction::Truncate {
                "Truncating"
            } else {
                "Deleting"
            },
            counted(targets.len(), "table", "tables")
        );
        session.request_generation += 1;
        let generation = session.request_generation;
        let task = runtime.spawn(async move {
            let grouped = statements.len() == 1;
            let mut completed = Vec::new();
            let mut failure = None;
            for (index, statement) in statements.iter().enumerate() {
                match engine.execute(statement).await {
                    Ok(_) => {
                        if grouped {
                            completed.extend(targets.iter().cloned());
                        } else {
                            completed.push(targets[index].clone());
                        }
                    }
                    Err(error) => {
                        failure = Some(format!(
                            "{}: {error}",
                            if grouped {
                                "Selected tables".to_owned()
                            } else {
                                table_ref_label(&targets[index])
                            }
                        ));
                        break;
                    }
                }
            }
            // Refresh even after partial failure, without masking mutation results.
            let refreshed = engine.list_tables().await;
            (completed, failure, refreshed)
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
                if session.request_generation != generation {
                    return;
                }
                session.busy = false;
                for &tab_id in &affected_tabs {
                    if let Some(data) = session.data_tab_mut(tab_id) {
                        data.busy = false;
                    }
                }
                let (completed, failure, refreshed) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        session.error = Some(format!(
                            "Table action interrupted; refresh to check the database: {error}"
                        ));
                        cx.notify();
                        return;
                    }
                };
                let mut errors = Vec::new();
                if let Some(error) = failure {
                    errors.push(format!(
                        "Stopped after completing {}. {error}",
                        counted(completed.len(), "table", "tables")
                    ));
                }
                match refreshed {
                    Ok(tables) => session.set_tables(tables),
                    Err(error) => errors.push(format!("Could not refresh the table list: {error}")),
                }
                let mut reload = Vec::new();
                let mut close = Vec::new();
                match action {
                    TableAction::Drop => {
                        close = session
                            .secondary_tabs
                            .iter()
                            .filter_map(|tab| match &tab.kind {
                                SecondaryTabKind::Data(data) if completed.contains(&data.table) => {
                                    Some(tab.id)
                                }
                                SecondaryTabKind::Structure(structure)
                                    if completed.contains(&structure.table) =>
                                {
                                    Some(tab.id)
                                }
                                _ => None,
                            })
                            .collect();
                        session.completion_schema = None;
                        for target in &completed {
                            session
                                .completion_columns
                                .remove(&completion_table_key(target));
                        }
                    }
                    TableAction::Truncate => {
                        for tab in &mut session.secondary_tabs {
                            if let SecondaryTabKind::Data(data) = &mut tab.kind
                                && (completed.contains(&data.table)
                                    || (cascade && !completed.is_empty()))
                            {
                                data.invalidate_request();
                                data.set_result(None, &session.tables, cx);
                                data.table_page = 0;
                                data.table_has_next_page = false;
                                data.selected_row = None;
                                data.row_draft = None;
                                data.row_draft_subscriptions.clear();
                                reload.push(tab.id);
                            }
                        }
                    }
                }
                session.error = (!errors.is_empty()).then(|| errors.join("\n"));
                let message = format!(
                    "{} {}{}",
                    if action == TableAction::Truncate {
                        "Truncated"
                    } else {
                        "Deleted"
                    },
                    counted(completed.len(), "table", "tables"),
                    if cascade && !completed.is_empty() {
                        " and dependent tables"
                    } else {
                        ""
                    }
                );
                session.status = if errors.is_empty() {
                    message.clone()
                } else {
                    "Table action needs attention".into()
                };
                for tab_id in close {
                    this.close_secondary_tab_for(session_id, tab_id, cx);
                }
                for tab_id in reload {
                    this.refresh_table_for(session_id, tab_id, cx);
                }
                if errors.is_empty() {
                    this.show_toast(ToastKind::Success, message, cx);
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

fn table_action_confirmation_text(tables: &[TableInfo]) -> String {
    if tables.len() == 1 {
        tables[0].name.clone()
    } else {
        tables
            .iter()
            .map(|table| table_sidebar_label(table, None))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn table_sidebar_id(table: &TableInfo) -> String {
    // Structured components avoid selecting a different table when a
    // schema or name contains the separator (e.g. a-b.c versus a.b-c).
    format!(
        "table-{}",
        serde_json::to_string(&(&table.schema, &table.name))
            .expect("table identity is serializable")
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

    #[gpui::test]
    fn large_setup_can_scroll_to_submit(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (_app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Uninitialized);
            app.ui_scale = 150;
            app.font_size = 18;
            app
        });
        cx.simulate_resize(gpui::size(px(960.), px(640.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let gate = cx.debug_bounds("vault-gate").unwrap();
        let before = cx.debug_bounds("vault-submit").unwrap();
        assert!(before.bottom() > gate.bottom(), "fixture must overflow");
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: gate.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1200.))),
            ..Default::default()
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after = cx.debug_bounds("vault-submit").unwrap();
        assert!(
            after.top() >= gate.top() && after.bottom() <= gate.bottom(),
            "submit must scroll into view: before={before:?}, after={after:?}, gate={gate:?}"
        );
    }

    #[gpui::test]
    fn touch_id_preference_excludes_silent_unlock(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.touch_id_available = true;
                app.remember_device = true;
                app.toggle_touch_id(cx);
                assert!(app.touch_id_enabled);
                assert!(!app.remember_device);
                app.toggle_remember_device(cx);
                assert!(!app.remember_device);
                let saved = app.settings_store.as_ref().unwrap().load().unwrap();
                assert!(saved.touch_id_enabled);
                assert!(!saved.remember_device);

                app.vault_busy = true;
                app.toggle_touch_id(cx);
                assert!(app.touch_id_enabled);
                app.vault_busy = false;
                // Removing a fingerprint or losing the sensor must not trap
                // the user in a preference they can no longer turn off.
                app.touch_id_available = false;
                app.toggle_touch_id(cx);
                assert!(!app.touch_id_enabled);
                assert!(!app.remember_device);
                app.toggle_touch_id(cx);
                assert!(!app.touch_id_enabled);
            });
        });
    }

    #[gpui::test]
    fn touch_id_requires_explicit_ready_locked_state(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.vault_state = Some(VaultState::Locked);
                app.touch_id_enabled = true;
                app.remember_device = true; // Inconsistent legacy preferences.
                app.try_device_unlock(cx);
                assert!(!app.vault_busy);
                app.touch_id_available = false;
                app.touch_id_enrolled = true;
                app.unlock_with_touch_id(cx);
                assert!(!app.vault_busy);
                assert_eq!(app.vault_state, Some(VaultState::Locked));
                assert_eq!(app.vault_generation, 0);
            });
        });
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
        let startup = workspace.load_startup().unwrap();
        assert_eq!(startup.connections.len(), 1);
        assert_eq!(startup.connections[0].profile_id, profile_id);
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
    fn display_url_redacts_passwords_and_invalid_input() {
        let displayed = display_url("postgres://user:secret@example.test:5432/app");
        assert!(!displayed.contains("secret"));
        assert!(displayed.contains("user@example.test"));
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
}
