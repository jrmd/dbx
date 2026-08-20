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
  accent: "#3b82f6"
  accent-soft: "#10294d"
  focus-ring: "#60a5fa"
  success: "#22c55e"
  warning: "#f59e0b"
  danger: "#ef4444"
  sql-keyword: "#c792ea"
  sql-string: "#c3e88d"
  sql-comment: "#6b7482"
  sql-number: "#f78c6c"
  sql-parameter: "#ffcb6b"
  sql-identifier: "#82aaff"
  sql-type: "#89ddff"
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
  control: "6px"
  panel: "10px"
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
    height: "31px"
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

The interface is intentionally native and dense. Surfaces are layered only enough to establish panes; the visual rhythm comes from alignment, a four-pixel spacing scale, and compact controls. Green is health, amber is work-in-progress or caution, and red is reserved for destructive actions. SQL is the one place where a broader semantic spectrum earns its space.

**Key Characteristics:**

- Persistent context: a 46px app rail, 42px top bar, 26px status bar, and visible explorer/inspector panes; the supplied DBX logo asset anchors the rail and top-bar identity.
- Deliberate density: 10–12px operational text, 30–36px controls, and four-pixel spacing increments.
- Blue is an interaction locator, not a decorative fill; it marks active navigation and primary commit actions.
- Borders and tonal planes, rather than shadows, separate work areas.

## Colors

The palette is a low-glare charcoal console: cool white text sits on layered black surfaces, with purposeful semantic color for action and state.

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
- **SQL Spectrum** (`sql-keyword`, `sql-string`, `sql-comment`, `sql-number`, `sql-parameter`, `sql-identifier`, `sql-type`): lexical meaning in the query editor only.

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

The desktop shell is fixed-context and pane-based: the 46px rail remains on the left, the top bar is 42px, primary connection tabs occupy the remaining top-bar width with horizontal overflow, and the status line is 26px. The supplied DBX logo asset anchors both the rail and top-bar branding. While disconnected, the shell retains that identity but hides rail controls that require a database context. In a live workspace, the explorer is 224px wide (180px in compact layout); the inspector remains alongside the grid when space permits. Connection setup is a centered, scrollable form capped at 900px, containing a 170px engine chooser (124px in compact layout).

Spacing follows the exact 4px, 8px, 12px, 16px token rhythm. Do not introduce a competing scale. Data and query panes use 36–38px headers; the SQL editor reserves 224px including its shell; multiline fields are 204px high.

At widths below 1180px, DBX enters its narrow-workspace behavior and removes the row inspector to protect the primary data canvas. At widths below 900px, the compact layout narrows the explorer and engine chooser, reduces top-bar identity copy, removes the refresh control, and reduces connection-form padding from 24px to 12px. Connection tabs remain horizontally scrollable rather than wrapping or sacrificing their identity.

## Elevation & Depth

DBX is flat by default: it defines depth through the ordered canvas, panel, raised-panel, and rail tones plus one-pixel borders. There are no drop-shadow tokens. Menus, popovers, selected tabs, and controls earn distinction through `panel-raised` or `border-strong`, never floating-card effects.

**The Earned Separation Rule.** Add a new tonal plane only when it clarifies a pane, selection, or transient utility; default content stays on the canvas with a single border boundary.

## Shapes

Forms are compact and lightly softened. Standard controls use a 6px radius, panel containers use 10px, text editors use 5px, and compact icon affordances may use 4–5px corners. Status dots and badges are fully rounded. Borders are one pixel and never replaced with thick outlines; selected state comes from blue tint and text/icon color, not excessive rounding.

## Components

### Buttons

The button family is compact, square-shouldered, and action-ranked.

- **Shape:** gently softened control corners (6px), 30px height, 12px horizontal padding, one-pixel border.
- **Primary:** Command Blue fill with Operational White text for commits such as Connect, Run, and New connection.
- **Quiet:** Raised Utility Surface with a Hairline Divider and white text for secondary operations such as Save, Test Connection, or Choose file. Test Connection validates the currently entered configuration only: it must not save a profile, open a workspace, or change database state.
- **Danger:** Raised Utility Surface with a red border and red label; reserve it for irreversible actions.
- **Hover / Focus:** quiet icon controls lift only to `panel-raised`; quiet form buttons may change their border to Command Blue. Keyboard focus must use the brighter Focus Blue rather than relying on hover.

### Inputs / Fields

- **Style:** Black Canvas fill, Assertive Divider border, 5px radius, 32px high with 7px padding for single-line fields.
- **Connection modes:** Details and Connection String are equal modes, not a primary form with an escape hatch. Details exposes labeled Host, Port, User, Password, and Database fields. SQLite replaces network details with a database-file field and a native Choose file action.
- **Saved connections:** Save requires the user-visible connection name. Persist connection metadata under that name and use it for the OS-keyring lookup; do not place the password in the saved JSON profile.
- **Multiline SQL:** same field treatment at 204px high with 10px padding, horizontal/vertical scrolling, selection tint, and a 2px blue caret.
- **Syntax:** SQL token colors distinguish keyword, string, comment, number, parameter, identifier, and type while base text stays Operational White.

### Navigation

- **App rail:** 46px wide with the supplied DBX logo asset, embedded compile-time 16px SVG line icons including dedicated Structure and Refresh glyphs, 24px icon hit areas, and a persistent green/offline status dot at the bottom. Icons inherit their semantic color from the consuming control. While disconnected, hide controls that operate on a database rather than presenting unavailable actions.
- **Top bar:** 42px rail-toned strip; repeat the supplied DBX logo asset with the DBX title, and retain connection tabs to preserve multi-connection context.
- **Connection tabs:** 32px high, 6px top corners, icon + health dot + engine badge + muted metadata. Active tabs use the standard panel and stronger border, with a full-width 2px Command Blue bottom indicator; inactive tabs use the rail.
- **Document tabs:** each connection owns a 36px, horizontally scrollable row containing the persistent Data document plus independently closable Query and table-bound Structure documents. Active documents use the canvas, strong border, and Command Blue icon; the compact add action opens another query without replacing existing work.
- **Pane tabs and explorer entries:** active state uses Blue Selection Well with Command Blue icon/text; unselected entries stay muted until hover. When a PostgreSQL schema filter is active, table labels omit that redundant schema prefix; the All view remains qualified.

### Cards / Containers

- **Connection setup:** a 10px-radius Instrument Panel with a single Hairline Divider. Its engine list and configuration pane are separated by a vertical border rather than nested cards. Details and Connection String receive equal visual weight; Details uses labeled Host, Port, User, Password, and Database fields, while SQLite presents a database-file field with a native Choose file action.
- **Menus:** Raised Utility Surface, strong border, 8px corners, 6px internal padding, and compact 8px × 7px menu rows.
- **Grid and inspector:** adjacent border-separated panes, not independently elevated cards.
- **Structure metadata:** columns and foreign keys form separate dense sections. Foreign-key rows show constraint name, local columns, qualified target columns, and update/delete actions without introducing nested cards.

### Status Badges

- **Style:** fully rounded Raised Utility Surface with 8px horizontal and 4px vertical padding, 10px medium text.
- **Meaning:** color the label by semantic state; do not make every badge blue.

## Do's and Don'ts

### Do:

- **Do** preserve the 4px, 8px, 12px, 16px spacing rhythm and favor 30–36px controls for dense desktop work.
- **Do** keep database context visible through persistent rail, top-level connection tabs, explorer, and status feedback.
- **Do** use the blue active-state system and green connection health consistently across rail, tabs, tables, and connection setup.
- **Do** keep the DBX logo asset visible in the rail and top-bar identity while hiding database-only rail controls until a connection is active.
- **Do** treat Details and Connection String as equal setup modes, and keep Test Connection non-mutating.
- **Do** scope document tabs to their connection and preserve the Data document while Query and Structure documents are independently closable.
- **Do** let narrow layouts hide secondary inspection before shrinking primary data and query work below usable density.

### Don't:

- **Don't** introduce shadows, glass effects, oversized cards, or decorative gradients; depth is tonal and border-led.
- **Don't** use blue as ambient decoration or apply semantic success/warning/danger colors without a state meaning.
- **Don't** wrap multi-connection tabs into ambiguous rows; retain horizontal scrolling and concise metadata.
- **Don't** repeat an active PostgreSQL schema prefix in explorer table labels; retain qualification only in the All view.
- **Don't** replace the authored geometric icon vocabulary with emoji, mixed icon sets, or browser-shaped controls.
- **Don't** save a connection password in profile JSON or use Test Connection to persist, open, or change a database.
