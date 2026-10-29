---
name: DBX Native Data Console
description: A dense, keyboard-oriented native database workbench with persistent operational context.
colors:
  canvas: "#0a0c10"
  panel: "#111318"
  panel-raised: "#171a20"
  rail: "#0d1016"
  grid-alternate: "#0e1116"
  border: "#1f232b"
  border-strong: "#343b47"
  text: "#f1f5f9"
  text-muted: "#94a3b8"
  accent: "#2563eb"
  accent-foreground: "#ffffff"
  accent-soft: "#10294d"
  focus-ring: "#60a5fa"
  success: "#22c55e"
  warning: "#f59e0b"
  danger: "#ef4444"
  sql-keyword: "#c792ea"
  sql-string: "#c3e88d"
  sql-comment: "#737e8c"
  sql-number: "#f78c6c"
  sql-parameter: "#ffcb6b"
  sql-identifier: "#82aaff"
  sql-type: "#89ddff"
  light-canvas: "#f7f9fc"
  light-panel: "#ffffff"
  light-panel-raised: "#f0f4f8"
  light-rail: "#ebf0f6"
  light-border: "#d8dee8"
  light-border-strong: "#b6c2d1"
  light-text: "#16202f"
  light-text-muted: "#52657b"
  light-accent: "#1d5fd1"
  light-accent-soft: "#e5f0ff"
typography:
  display:
    fontSize: "18px"
    fontWeight: 600
  title:
    fontSize: "15px"
    fontWeight: 700
  body:
    fontSize: "12px"
  label:
    fontSize: "10px"
    fontWeight: 500
rounded:
  editor: "5px"
  control: "7px"
  panel: "12px"
  glass: "16px"
  full: "9999px"
spacing:
  space-1: "4px"
  space-2: "8px"
  space-3: "12px"
  space-4: "16px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "0 12px"
    height: "30px"
  button-quiet:
    backgroundColor: "{colors.panel-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "0 12px"
    height: "30px"
  button-danger:
    backgroundColor: "{colors.panel-raised}"
    textColor: "{colors.danger}"
    rounded: "{rounded.control}"
    padding: "0 12px"
    height: "30px"
  connection-tab-active:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "0 12px"
    height: "32px"
  connection-tab-active-marker:
    backgroundColor: "{colors.accent}"
    height: "2px"
  document-tab-active:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.text}"
    rounded: "{rounded.editor}"
    padding: "0 11px"
    height: "35px"
  input:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.text}"
    rounded: "{rounded.editor}"
    padding: "7px"
    height: "32px"
---

# Design System: DBX Native Data Console

## Overview

**Creative North Star: "The Native Data Console"**

DBX is an operator's cockpit: a compact, near-black workbench where database identity, navigation, data, and row context stay simultaneously visible. It uses crisp one-pixel separators, small label-first typography, and a restrained blue action language to make a large amount of state immediately scannable instead of decorative.

The interface is intentionally native and dense. Surfaces are layered only enough to establish panes; the visual rhythm comes from alignment, a four-pixel spacing scale, and compact controls. Green is health, amber is work-in-progress or caution, and red is reserved for destructive actions. Structured SQL and JSON editors are the one place where a broader semantic spectrum earns its space.

**Key Characteristics:**

- Persistent context: a 48px app rail, 46px unified titlebar, 30px status bar, and visible explorer/inspector panes; the DBX logo anchors the disconnected titlebar and the vault gate.
- Deliberate density: 11–12px operational text, 26–30px controls, and four-pixel spacing increments.
- Blue is an interaction locator, not a decorative fill; it marks active navigation and primary commit actions.
- With native Liquid Glass enabled, navigation reveals the adaptive material while documents, tables, editors and forms remain opaque. Selection, focus and transient overlays keep stronger contrast. Solid mode uses opaque surfaces throughout.

## Colors

The default palette is a low-glare charcoal console: cool white text sits on layered black surfaces, with purposeful semantic color for action and state. A first-class light appearance maps the same roles onto cool paper, white panels, blue-slate dividers, and ink text; it is composed rather than mechanically inverted.

### Custom palettes

Palettes live in the Theme group at the top of Settings → Appearance, beside
the System/Light/Dark choice; there is no separate Themes page. DBX, Ocean,
Forest, Plum, Sand, and Graphite each have light and dark variants and appear
as 104px thumbnails that sketch rail, active row, and syntax in that palette;
the selected one gets a Command Blue ring. **Your themes** is a second row of
the same thumbnails for imported and edited themes, each revealing a small ×
inside its corner on hover or focus, red when targeted (removal confirms), ending in a dashed **Import…** tile, so
add and remove sit with the themes they affect. **Edit…** expands a searchable
semantic-colour editor inline (colour picker, hex/alpha entry, explicit Apply,
Reset) above a live mini result grid and query line. Editing a preset adds a
new theme; preferences and both custom variants persist across relaunches.

VS Code JSON/JSONC imports map UI colours and common TextMate syntax scopes to
DBX's semantic roles. A self-contained DBX export preserves every role in both
appearances. The importer validates before applying and never follows include
paths. See `docs/themes.md` for format details and compatibility limits.

Grid active, grouped selection, and hover fills are independent semantic roles.
The active row uses a stronger opaque fill; the active cell adds a two-pixel
focus outline without painting over its value. Both the component colour table
and resolved tokens must receive the palette, including header, footer, stripes,
menus, scrollbars, controls, and focus states.

### Appearance modes

- **Dark:** Black Canvas, Instrument Panel, Raised Utility Surface, and Navigation Rail remain the default low-glare working environment.
- **Light:** Cool Paper (`light-canvas`) holds the workspace, white (`light-panel`) carries panes, Pale Utility (`light-panel-raised`) distinguishes controls, and Blue-Slate dividers preserve the pane hierarchy.
- **Parity:** blue still means action/location, green still means health, amber still means caution, and red still means destructive. Grid alternation, focus visibility, editor syntax, overlays, and disabled states must be checked in both appearances.
- **System:** the default. DBX follows the operating system's light/dark setting live.
- **Control:** Settings → Appearance offers System, Light, and Dark plus **Liquid Glass** on macOS or **Reduce transparency** elsewhere; every choice applies immediately and persists.

### Primary

- **Command Blue** (`accent`): primary buttons, active tabs, active rail entries, and authored navigation icons.
- **Blue Selection Well** (`accent-soft`): selected table, engine, filter chip, and hover treatment where a filled selection must remain quiet.
- **Focus Blue** (`focus-ring`): the brighter focus reference; preserve it for keyboard-visible focus treatment.

### Neutral

- **Black Canvas** (`canvas`): the workspace and input interior; it is the deepest operational plane.
- **Instrument Panel** (`panel`): standard sidebars, tab strips, status surfaces, and connection containers.
- **Raised Utility Surface** (`panel-raised`): quiet buttons, menus, and hover states.
- **Navigation Rail** (`rail`): the app rail and top bar, distinct from the main canvas without visual weight.
- **Alternating Grid Ink** (`grid-alternate`): quiet row alternation in data grids.
- **Hairline Divider** (`border`) and **Assertive Divider** (`border-strong`): the only default separators between panes and controls.
- **Operational White** (`text`) and **Muted Metadata** (`text-muted`): readable primary content and secondary labels/URLs respectively.

### Tertiary

- **Connection Green** (`success`): live connection health and trusted persistence notices.
- **Caution Amber** (`warning`): busy status and destructive-but-recoverable operations.
- **Destructive Red** (`danger`): irreversible operations and error emphasis.
- **Structured-editor spectrum** (`sql-keyword`, `sql-string`, `sql-comment`, `sql-number`, `sql-parameter`, `sql-identifier`, `sql-type`): lexical meaning in SQL editors and the corresponding JSON token roles.

**The Sparse Accent Rule.** Use Command Blue to identify an action or current location; never use it as a general panel fill.

## Typography

**Display Font:** GPUI/platform default UI font.
**Body Font:** GPUI/platform default UI font.
**Label/Mono Font:** GPUI/platform default UI font; SQL color, not a separate typeface, carries lexical differentiation.

**Character:** The type system is deliberately utilitarian: compact enough for dense database work, but with sufficient contrast between 18px section titles, 12px working text, and 9–10px metadata to preserve scanning order.

### Hierarchy

- **Display** (600, 18px): connection-screen title such as “New connection.”
- **Title** (700, 15px): DBX wordmark in the top bar.
- **Body** (normal, 12px): fields, controls, table labels, connection tabs, and SQL editor text.
- **Label** (500, 10px): panel detail, status, captions, and compact data labels; use 9px only for the smallest navigation metadata.

**The Label-First Rule.** State the object or mode in a compact muted label, then reserve brighter text for actionable or selected content.

## Layout

The desktop shell is fixed-context and pane-based: the 46px rail remains on the left, the top bar is 42px, primary connection tabs occupy the remaining top-bar width with horizontal overflow, and the status line is 26px. The supplied DBX logo asset anchors the top-bar identity while disconnected and the rail identity in a live workspace, avoiding duplicate marks. While disconnected, the shell retains that identity but hides rail controls that require a database context. In a live workspace, the explorer is 224px wide (180px in compact layout); the inspector remains alongside the grid when space permits. Connection setup is a centered, scrollable form capped at 900px, containing a 170px engine chooser (124px in compact layout).

Spacing follows the exact 4px, 8px, 12px, 16px token rhythm. Do not introduce a competing scale. Data and query panes use 36–38px headers; the query editor initially reserves 224px including its shell and can be resized vertically between a compact working minimum and roughly half the workspace; multiline row-value fields are 204px high.

At widths below 1180px, DBX enters its narrow-workspace behavior and removes the row inspector to protect the primary data canvas. At widths below 900px, the compact layout narrows the explorer and engine chooser, reduces top-bar identity copy, removes the refresh control, and reduces connection-form padding from 24px to 12px. Connection tabs remain horizontally scrollable rather than wrapping or sacrificing their identity.

## Elevation & Depth

DBX separates navigation from content following [Apple's Materials guidance](https://developer.apple.com/design/human-interface-guidelines/materials):

1. **Native navigation** (macOS 26+, Liquid Glass on): `NSToolbar` owns the toolbar controls and their glass groups. A sidebar-behaviour `NSSplitViewItem` owns the Explorer glass; an `NSTableView` uses native vibrant labels and selection. Its search, database/schema filters and actions live in a top-aligned `NSSplitViewItemAccessoryViewController`. Do not add an `NSVisualEffectView` or a second glass layer inside it.
2. **Scroll edges**: AppKit's `NSScrollView` supplies the automatic edge effects under the toolbar and Explorer accessory. The Explorer scrolls natively. The main host mirrors the active GPUI grid/editor/settings scroll range and offset into a native scroll view while pinning the existing renderer to window coordinates, preserving virtualized content, hit testing, IME and accessibility geometry. Internal GPUI document controls remain fixed content; they do not become native floating accessories.
3. **Content sheet** (`canvas`, `RADIUS_PANEL`): grids, editors, forms, document tabs, settings and status bars use opaque surfaces, including imported translucent themes. Table striping, selection and staged-change colours belong to the content palette and do not sample the desktop. No painted white rims or extra glass shadows.

Turning **Liquid Glass** off, or enabling macOS Reduce Transparency or Increase Contrast, restores the GPUI shell with solid surfaces. The saved app preference survives OS accessibility changes. Older macOS versions and builds made with older Swift SDKs use the solid fallback. Other platforms retain their existing material handling.

**The Earned Separation Rule.** Inside the content sheet, depth stays tonal and border-led. Rows, cells and fields do not receive Liquid Glass effects. The content split adjusts its safe area and uses AppKit’s `NSBackgroundExtensionView` to extend the content colours beneath the toolbar and sidebar, avoiding unrelated desktop and opaque backdrops at the split. Connection tabs stay individually visible with close actions. Explorer actions use equal 32px targets without nested bezels or a redundant menu arrow. The locked vault hides the app toolbar entirely. Native labels use AppKit semantic colours and vibrancy; custom content text and syntax retain their contrast correction. Essential focus and input boundaries remain distinct from decorative separators.

**Hosting contract.** `native/NativeShell.swift` owns native views through a retained, main-thread-only handle. Rust sends state snapshots and drains queued intents; no delegate re-enters GPUI. Existing handlers validate session identity, capabilities and destructive confirmations. GPUI's original content root is retained with its AccessKit adapter, and its logical canvas stays aligned with the whole window even inside the clipped main split. Main-scroller geometry is a projection of GPUI state, not a second scroll owner. While a modal is open the native sidebar stays in place under a tint in the modal scrim's own colour (it fades over about 0.18s, or switches instantly with Reduce Motion), and native actions are disabled. Detaching restores the original content root, toolbar, appearance and sizing. See [Apple's AppKit guidance](https://developer.apple.com/videos/play/wwdc2025/310/).

## Shapes

Selectable and actionable controls are capsules: buttons, connection tabs, document tabs, segmented options, chips, and titlebar icon controls. List rows use a 10px radius, the content sheet 12px, floating glass 16px, and raised dialogs 20px. In the solid shell, navigation panels beside the content sheet (Explorer, connection list) share the sheet radius so neighbouring panels match. Status dots and badges are fully rounded. Borders are one pixel. Selection comes from a lifted glass pill or a blue tint, not heavier outlines.

## Components

### Buttons

The button family is compact, capsule-shaped, and action-ranked.

- **Shape:** full capsule, 28px height, 14px horizontal padding, one-pixel hairline border, 12px medium label. Always build buttons through `theme::button` (or `Size::XSmall`); gpui-component otherwise sizes labels at 16px.
- **Primary:** Command Blue fill with Operational White text and a specular top rim, for commits such as Connect, Run, and Unlock.
- **Quiet:** Raised Utility Surface with a Hairline Divider and white text for secondary operations such as Save, Test Connection, or Choose file. Test Connection validates the currently entered configuration only: it must not save a profile, open a workspace, or change database state.
- **Danger:** Raised Utility Surface with a red border and red label; reserve it for irreversible actions.
- **Hover / Focus:** quiet icon controls lift only to `panel-raised`; quiet form buttons may change their border to Command Blue. Keyboard focus must use the brighter Focus Blue rather than relying on hover.

### Inputs / Fields

- **Style:** Black Canvas fill, Assertive Divider border, 5px radius, 32px high with 7px padding for single-line fields.
- **Connection modes:** Details and Connection String are equal modes, not a primary form with an escape hatch. Details exposes labeled Host, Port, User, Password, and Database fields. SQLite replaces network details with a database-file field and a native Choose file action.
- **Saved connections:** Save requires the user-visible connection name. Persist password-free metadata under that name; DBX Vault encrypts normal saved credentials with Argon2id + XChaCha20-Poly1305 and is unlocked once in-app per launch. The passphrase is never stored or recoverable, and selecting a saved connection eagerly hydrates its credential. Only **Import old system passwords** reads Keychain/Secret Service; imports are non-destructive and never overwrite a vault entry.
- **Multiline SQL:** same field treatment at 204px high with 10px padding, horizontal/vertical scrolling, selection tint, and a 2px blue caret.
- **Row value modes:** one compact selector switches between a bound Value, an explicit single SQL expression, nullable SQL NULL, and insert-only database Default. Ordinary values remain parameterized; SQL is visually and behaviorally explicit.
- **Typed row controls:** Boolean fields use a native true/false selector. JSON and JSONB values open in a multiline editor, pretty-print existing documents, and retain valid incomplete input while typing.
- **Syntax:** SQL token colors distinguish keyword, string, comment, number, parameter, identifier, and type. JSON reuses that semantic spectrum for property names, strings, numbers, booleans, and null while base text stays Operational White.

### Dialogs

- **Mutation errors:** insert and update failures use one focus-trapped dialog with the exact database or validation detail, a clear recovery action, and explicit confirmation that the draft remains open. The dialog never discards entered values.
- **Hierarchy:** blocking errors and destructive confirmations are centered transient surfaces; routine field guidance stays inline in the inspector.

### Navigation

- **App rail:** 46px wide with the supplied DBX logo asset, embedded compile-time 16px SVG line icons including dedicated Structure and Refresh glyphs, 24px icon hit areas, and a persistent green/offline status dot at the bottom. Icons inherit their semantic color from the consuming control. While disconnected, hide controls that operate on a database rather than presenting unavailable actions.
- **Top bar:** 42px rail-toned strip; show the supplied DBX logo with the DBX title while disconnected, and retain connection tabs to preserve multi-connection context once a workspace is active.
- **Appearance action:** a single compact sun/moon action sits with window-level controls, is available before and after connection, and names the appearance it will switch to in its tooltip.
- **Connection tabs:** 32px high, 6px top corners, icon + health dot + engine badge + muted metadata. Active tabs use the standard panel and stronger border, with a full-width 2px Command Blue bottom indicator; inactive tabs use the rail.
- **Document tabs:** each connection owns a 36px, horizontally scrollable row containing the persistent Data document plus independently closable Query, table-bound Structure, and database Diagram documents. Active documents use the canvas, a single blue bottom separator, and Command Blue icon; the compact add action opens another query without replacing existing work.
- **Explorer:** the header keeps Refresh and New Table visible for quick routine work. Diagram, Export, and Import live in one compact overflow menu rather than competing for header width. Database and schema filter rows remain labelled, horizontally scrollable chip rows; they never wrap into the table list. Active entries use Blue Selection Well with Command Blue icon/text; unselected entries stay muted until hover. When a PostgreSQL schema filter is active, table labels omit that redundant schema prefix; the All view remains qualified.

### Cards / Containers

- **Connection setup:** a flat configuration pane beside the saved-connection list, separated by a single vertical border. Form labels use an 11px minimum. Details and Connection String receive equal visual weight; Details uses labeled Host, Port, User, Password, and Database fields, while SQLite presents a database-file field with a native Choose file action.
- **Menus:** Raised Utility Surface, strong border, 8px corners, 6px internal padding, and compact 8px × 7px menu rows.
- **Grid and inspector:** adjacent border-separated panes, not independently elevated cards.
- **Structure metadata:** columns and foreign keys form separate dense sections. Foreign-key rows show constraint name, local columns, qualified target columns, and update/delete actions without introducing nested cards.

### Status Badges

- **Style:** fully rounded Raised Utility Surface with 8px horizontal and 4px vertical padding, 10px medium text.
- **Meaning:** color the label by semantic state; do not make every badge blue.

### Query Workbench

- **Execution scope:** Run executes the selection when present, otherwise the SQL statement at the caret (or the current Redis command line). Run All is an explicit secondary action. The editor and result grid share a vertically resizable split so either can become the primary working surface.
- **Safety:** destructive SQL and non-atomic multi-statement scripts require a focused confirmation before execution. Cancelling, switching databases, closing a query document, or closing its connection invalidates the active request so a late result cannot overwrite newer state.
- **Outcome:** keep the previous result visible while a newer request runs or fails, but mark it as stale. The result header distinguishes returned rows, affected rows, elapsed time, database provenance, truncation, cancellation, and full database errors without relying on the global status line.
- **Result interaction:** cells, rows, and columns are keyboard-selectable and copy with explicit NULL versus empty-string semantics. Complete results can be copied or exported as TSV, CSV, or lossless positional JSON.
- **History and recovery:** executed queries are stored locally in a bounded, connection-scoped, credential-free history. Loading history never executes it. Non-empty query documents ask before closing and recently closed text can be reopened within the connection session.
- **Keyboard model:** Cmd/Ctrl+Enter runs the current scope, Cmd/Ctrl+Shift+Enter runs the document, Escape cancels an active request, and ordinary editor undo/redo remains available. Shortcut labels use platform-neutral copy unless the platform is known.
- **Dialect truthfulness:** SQL connections use SQL syntax, completion, formatting, and statement scope. Redis presents a command editor, disables SQL-only affordances, and executes the selected text or current command line.

### Table Designer

The Structure title and compact **Design table…** / **Close designer** control
share one toolbar row. The designer aligns with the left edge of the metadata
pane and is capped at 860px. A 148px navigation column groups Columns, Indexes
and Constraints; selected actions use the blue selection well. The form uses
11px labels above 32px inputs, with help and 24px type/column shortcuts directly
under the relevant field. Nullability and uniqueness use labelled checkboxes.
The footer keeps the queued count and Clear action together, with Add to plan
and Review SQL plan side by side; actions wrap in compact panes. Hiding the
form retains its state. All changes continue through the SQL review workflow.

### Database Diagram

- **Scope:** the database diagram is a connection-scoped, independently closable document tab for relational engines. It loads the current database schema without adding an app-rail destination.
- **Diagram language:** deterministic table cards show the table name, a muted schema line, the column count, amber PK and blue FK markers, and compact type spellings (`timestamptz`, `varchar(160)`) with a `?` suffix for nullable columns. Cards carry no decorative colour coding. Relationships use crow's-foot notation — a fork on the referencing side with a bar (mandatory) or ring (all referencing columns nullable), and a double bar on the referenced side — drawn as rounded orthogonal routes over a quiet dot grid. Vertical runs sit in the gap beside the referenced table and relationships sharing a corridor fan out into separate lanes. Large tables use bounded detail, preserving relationship-bearing columns and summarising the remainder instead of creating an unbounded canvas.
- **Navigation:** the diagram scrolls in both directions, uses grab-to-pan, and supplies zoom, Fit, and refresh controls; every toolbar control shares the 28px capsule height. Ctrl/⌘+wheel zooms around the pointer. When the canvas is focused, arrows pan, Shift+arrows move farther, +/− zoom, F fits, 0 resets, and R refreshes. A minimap appears in the bottom-right corner whenever the scene overflows the viewport; clicking or dragging it recentres the view. Selecting a card lifts it with a blue glow, highlights its relationships, and dims unrelated cards; clicking empty canvas clears the selection, and double-clicking a card drills into that table's existing data workflow.
- **Arrangement:** cards can be dragged to new positions; relationships reroute live. Hand-arranged positions survive refreshes and schema changes until **Auto layout** (shown only while positions are customised) restores the deterministic layout. Exports use the arranged positions.
- **Schema scope:** PostgreSQL diagrams start from the Explorer's active schema and provide a compact multi-select schema picker. Changing it projects the retained metadata snapshot in memory, preserves selected-to-selected cross-schema relationships, and applies identically to the canvas, SVG, and PNG exports without re-querying the database.
- **State:** retain a previously loaded scene while refresh work is pending and clearly identify loading, stale, empty, and error states. Schema-changing actions invalidate the document rather than presenting stale metadata as current.
- **Export:** the on-screen scene is the canonical SVG scene. SVG and PNG exports are generated from that same scene, use the active light/dark palette, and preserve the table and relationship content shown to the operator.

## Do's and Don'ts

### Do:

- **Do** preserve the 4px, 8px, 12px, 16px spacing rhythm and favor 30–36px controls for dense desktop work.
- **Do** keep database context visible through persistent rail, top-level connection tabs, explorer, and status feedback.
- **Do** use the blue active-state system and green connection health consistently across rail, tabs, tables, and connection setup.
- **Do** keep the DBX logo asset visible in the contextual rail or top-bar identity while hiding database-only rail controls until a connection is active.
- **Do** treat Details and Connection String as equal setup modes, and keep Test Connection non-mutating.
- **Do** scope document tabs to their connection and preserve the Data document while Query and Structure documents are independently closable.
- **Do** keep Explorer routine actions compact: Refresh and New Table visible, Diagram/Export/Import in overflow, and labelled database/schema filter rows horizontally scrollable.
- **Do** keep diagram cards, relationship routing, and exports deterministic; use one SVG scene for on-screen rendering and SVG/PNG output.
- **Do** preserve query result provenance and clearly label stale, limited, failed, and cancelled outcomes.
- **Do** let narrow layouts hide secondary inspection before shrinking primary data and query work below usable density.
- **Do** validate semantic contrast and selected, hover, disabled, error, grid, editor, menu, and dialog states in both light and dark appearances.

### Don't:

- **Don't** put glass inside the content sheet, stack translucent layers over data, or add decorative gradients. Glass is for chrome and transient layers only.
- **Don't** use blue as ambient decoration or apply semantic success/warning/danger colors without a state meaning.
- **Don't** wrap multi-connection tabs into ambiguous rows; retain horizontal scrolling and concise metadata.
- **Don't** turn the Explorer header into a row of text actions or make the diagram a competing global navigation destination.
- **Don't** repeat an active PostgreSQL schema prefix in explorer table labels; retain qualification only in the All view.
- **Don't** replace the authored geometric icon vocabulary with emoji, mixed icon sets, or browser-shaped controls.
- **Don't** save a connection password in profile JSON or use Test Connection to persist, open, or change a database.

### Native desktop chrome

With native Liquid Glass enabled, AppKit supplies the titlebar, toolbar controls, connection tabs and sidebar toggle. Otherwise the 46px GPUI titlebar spans the window and is transparent over the backdrop. The whole empty stretch is a drag area: double-click zooms and right-click opens the compositor window menu. macOS keeps native traffic lights (inset 16px) with reserved leading space. Elsewhere DBX draws circular minimize, maximize/restore, and close controls, showing only those the compositor supports. Client-decorated windows get invisible edge and corner resize handles. Connection tabs are capsules; the selected one lifts into a glass pill. When connected, a sidebar toggle sits beside the window identity.

Standard shortcuts use ⌘ on macOS and Ctrl elsewhere: New Connection (N), New Query (T), Close Tab (W; on Data it closes the connection), Toggle Explorer (B), Refresh (R, F5), and next/previous connection (Ctrl-Tab / Ctrl-Shift-Tab, ⌘⇧] / ⌘⇧[). Escape backs out of the connection picker to the open connection. macOS gets a full menu bar (DBX, File, Edit, View, Query, Window) wired to the same actions. If focus is lost (for example, a focused tab closes), the root view reclaims it so shortcuts keep working.
