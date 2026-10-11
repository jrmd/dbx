# DBX accessibility implementation

Implementation of the [11 October accessibility audit](accessibility-audit-2026-10-11.md)
is in progress. The audit describes the original checkout; this file tracks the
changes and the evidence for them. The full accessibility goal remains open.

## Current implementation

| Audit area | Current state | Remaining work |
| --- | --- | --- |
| Shared controls | Clickable primitives require names and publish button roles. Tabs, appearance choices, and assistant choices publish selection state. Icon library buttons receive explicit names. Settings switches have names and checked state. | Named connection/document/Settings tab lists, Workspace and Explorer landmarks, theme and connection-format groups, Explorer options and schema-object lists are implemented. Select triggers now have names, selected values and native open/dismiss actions. Option rows now have names, selected state, guarded native focus/activate actions, cursor feedback and named search. Continue remaining contextual controls and native coverage across screens, including row-inspector and non-searchable selectors. |
| Text fields and editor | The existing input focus node now exposes a text/password role and persistent name. Ordinary fields publish grapheme-aware text runs, selection and painted glyph geometry in physical window coordinates. Split runs retain same-line associations. Accessible set-value, replacement, selection and caret reveal use the real editor and undo behavior. Passwords publish neither plaintext values nor text runs. | Diagnostic descriptions now distinguish warnings from errors; only errors mark a field invalid, and protected fields never expose diagnostic excerpts. Native folded-text workflow, read-only details and actual screen reader editing checks remain. Line-number synchronization after caret reveal is corrected and verified. Geometry and native ordinary/offscreen caret requests are now checked. |
| Connection and vault | Connection, transport, password, tag and vault fields have names. New connection steps now focus their visible first field inside a named form. Orca verifies picker/form context and local test success/failure announcements. | Complete validation and connect/save workflows across supported readers/platforms. |
| Other fields | Search, find/replace, query naming, export, parameters, assistant setup, import mapping, row drafts and cell editors have names. Designer fields now use the standard input shell and change their accessible name with the selected operation. | Verify field descriptions and state feedback with assistive technology. |
| Contrast and hit areas | Text/status foregrounds now have composited contrast checks over tinted glass, shadows and control fills. Primary button interaction states stay opaque. Library input edges use the app palette with an independent opaque fill, and Base receives the resolved theme. Window controls and tab-close controls have keyboard access and larger targets. | Finish selected/custom-tag/diagram graphic states, remaining control targets and native compositor/platform coverage. The controlled Linux rendering and matrix establish the checked materials and foregrounds, not every screen or OS display setting. |
| Results grid | Grid, headers and cells publish names; rows publish positions and selection, with names on their row-header cells. Loaded counts, sort and staged/edit/read-only state are exposed. Catalog table keys survive sorting/reloads, while ambiguous records are scoped to snapshot/page and typed occurrence. Real component cell/header identities now follow stable rows and columns; new row identities survive edits/removal of preceding inserts. Keyboard column resizing and accessible cell selection use the existing grid. | Native Linux Table/TableCell interfaces, coordinates, spans, containing-grid references and header relationships are implemented. Live Orca now reads table dimensions and complete visible row values. Unpainted loaded cells and horizontally virtualized headers/cells now have stable semantic objects and guarded native reveal actions. Finish total-count semantics on each platform, reader editing/save workflows and large-grid reader performance. Native Linux sorting identity, duplicate paging/action rejection, loaded counts and final-page feedback are verified; other platforms remain unverified. |
| Dialogs and announcements | Dialogs publish titles/descriptions and modal state, including a wrapper that preserves the existing focus node. Background workspace, lower modals and notifications are hidden during modal browsing. Query/footer/validation/toast outcomes have stable live nodes. Import/export dialogs restore their opener. | Verify announcement order and cancellation/edit/save outcomes with screen readers on each supported platform. |
| Text scaling | Persistent interface size and additional SQL text size controls cover 100–200%. Ctrl/Command +, − and 0 adjust/reset the interface. Design metrics, fields, grid rows and text scale together. Larger settings rows stack controls and dialogs stay within the viewport. | Finish 200% native connection/dialog/inline-editing checks and focus reveal in other scrolling surfaces. Diagram list and toolbar scale; Explorer header now wraps to keep actions visible. |
| Diagram and column sizing | Ctrl/Command + Shift + Left/Right resizes the selected grid column; the same modifiers with Home fit its content. Diagram list exposes all columns and qualified/composite relationship descriptions, with keyboard selection/opening and named card movement controls. Ctrl/Command + Shift + arrows move the selected card through the real layout/routing model. | Screen reader workflow and larger schema performance checks; preserve canvas pan/zoom behavior. |
| Reduced motion | Saved System/Reduce/Full radio choices set GPUI’s global policy for animations and component transitions. System probes the Linux portal off the UI thread, AppKit on macOS, and Windows client-area animation preference; changes are checked every 15 seconds. Missing platform preference uses static presentation. | Actual macOS/Windows build and OS preference change verification; Linux backend reporting a supported preference. |

## Verification

The foundation changes passed `cargo test -p dbx-ui --locked -- --test-threads=1`:
367 passed and two existing ignored tests. The test run needed local socket and
process-control permissions for existing PostgreSQL-password/updater fixtures and
CLI cancellation tests. New tests cover Unicode and reversed text selections,
long text runs, real accessible editing with undo and stale-selection rejection,
protected-field metadata, named shared controls, and text/input-edge contrast.
Strict UI Clippy passed. Subsequent field-name/designer refinements passed 96
editor tests, 17 row-draft tests, four cell-edit tests and the designer regression.
Strict Clippy, formatting and diff checks also passed for those refinements.
Final full validation will run after the remaining changes.

A disposable native Linux DBX instance was inspected through libatspi. The
published tree exposed named Settings/window buttons, named protected vault
fields, the labelled device-unlock checkbox and Create vault. The protected
fields had no text children. This is native tree evidence for the initial vault
screen, not an Orca workflow or coverage of the workbench.

The locked Unix adapter does not expose an AT-SPI EditableText interface for
the protected field. A direct native set-text probe therefore could not exercise
the app's SetValue handler; ordinary screen reader keyboard input and the
platform-specific accessible editing interfaces still need validation.

A second disposable native session under `/tmp/dbx-a11y-vbCm2A` reached the
SQLite workbench through AT-SPI Select and Connect controls. Its tree exposed
seven named headers, row headers, full cell values and the result summary. Native
Page Down navigation reached and published Row 250; the selected cell received
native focus. Profile and confirmation dialog trees exposed their titles and
controls while background controls were absent. The ordinary Connection name
field returned `QA SQLite` through the native Text interface. These are tree and
action assertions, not an Orca workflow.

AT-SPI keyboard synthesis reported acceptance but delivered no text on this
Wayland session. Unlock was completed with compositor keyboard input, guarded
by a check that the active window's PID matched the disposable DBX process.
The Unlock button and subsequent navigation used native accessible actions.
No fixture key was saved to the device keychain, and no import/export/delete or
database mutation was confirmed.

The second native process was stopped, and both accessibility service flags
were restored to false and read back successfully. The fixture trees remain in
that temporary directory. The QA vault seeder now accepts an optional config
path so repeated independent QA sessions can avoid existing state.

Before text scaling, the expanded full UI suite passed 372 tests with two existing
ignored tests, including grid focus/resize, virtualized navigation and profile
dialog focus restoration. The complete suite including scaling now passes 375 tests, with two existing
ignored tests. Strict UI Clippy, formatting and diff checks also pass. The 800×600, 200% settings and shortcut regression passes, as does
focus reveal for a control initially below the viewport. The Vim keyboard test
now locates its named switch rather than assuming it is the first setting.

Native 200% vault and settings layouts were visually inspected at 1261×688.
AT-SPI focus on Reduce transparency scrolled its settings row into view and the
published focused state was asserted. Screenshots and the focused tree are under
`/tmp/dbx-a11y-vbCm2A`. This native check establishes focus reveal in Settings,
not every scrolling surface or a 200% screen reader workflow. The QA process was
stopped and both accessibility flags were restored and read back as false.

The diagram and motion additions passed the full UI suite: 380 passed, two
existing ignored tests. Strict Clippy passed. A subsequent native check caught
production arrow-key bindings invoking canvas pan actions in list mode; the
handlers now select tables in list mode. The regression uses those production
bindings. Diagram layout tests cover bounded selection, actual card movement,
relationship rerouting, automatic layout restoration and columns beyond the
canvas’s 18-column limit at 200%.

Native Linux diagram tree/action checks published two tables, full column names,
primary-key/NULL state and the `projects (team_id) references teams (id)`
relationship. Accessible card movement updated the stable selection/position
status and enabled Auto layout. Open table reached the teams data tab and its
Platform fixture cell. With corrected production bindings, two Down keys
published native focus on teams; Enter opened its data at 200%. The enlarged
selected list row was visually inspected at 1261×688. Evidence is under
`/tmp/dbx-a11y-vbCm2A/diagram-*-tree.json` and
`/tmp/dbx-a11y-vbCm2A/diagram-selected-200.png`.

Motion controls published a radio group and named System/Reduce/Full options;
the native Reduce action persisted `motion: reduce` in the disposable settings.
GPUI tests verify OS changes, explicit override precedence and saving motion
alongside other preferences. The local portal returned Requested setting not
found for the standardized reduced-motion key, so this machine verifies the
unavailable-setting path, not a supported backend’s change notification.
Platform APIs follow [XDG Portal Settings](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html),
[NSWorkspace’s reduced motion property](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducemotion)
and [Windows SystemParametersInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfoa).
Only the Linux build was exercised here.

The 200% native check also caught Explorer header actions clipped outside the
sidebar. The header and its action row now wrap, with a rendered bounds
regression. The rendered bounds regression passed. A fresh native 200% session opened the
Explorer menu through its accessible action, and the menu remained inside the
window (`explorer-menu-200-tree.json` and `explorer-menu-200.png`). The Explorer
title text still needs layout coverage alongside the now-visible action row.
All disposable QA
processes were stopped, and both accessibility flags were restored and read
back as false. These remain native tree and keyboard/action checks, not an
Orca, VoiceOver or NVDA workflow.

Final validation after the production binding and Explorer header corrections:
`cargo test -p dbx-ui --locked -- --test-threads=1` passed 380 tests with two
existing ignored tests (80.59 seconds). Strict UI Clippy, formatting and diff
checks passed. No native QA process remains running, and both service flags
were restored to false. The overall audit goal remains open.

Editor text runs now publish bounds, character positions and advance widths
from the real shaped lines and the current paint origin, multiplied by the
physical window scale just as GPUI’s real nodes are. Unicode grapheme runs,
long lines split across runs, hard line breaks and same-line associations are
covered by a GPUI text-system regression. Geometry clears each frame; text
outside the painted/caret lines remains readable without invented glyph bounds.
The existing fold reconciliation opens hidden SQL when an accessible selection
puts its caret there, and the new regression checks fold opening and offscreen
caret reveal through the real editor action handler.

An isolated native Linux query editor held 616 known Unicode/multiline
characters. Once queued typing had finished, the native Text caret request to
zero published caret zero and valid first-line character rectangles, including
a zero-width newline rectangle. A request to offset 560 published caret 560 and
a revealed 8×19 rectangle at window coordinates 385,309. The first request made
while typing was still queued was superseded by continuing input; this was
retested after the character count stabilized. Native evidence is saved as
`text-first-line-extents.json`, `text-offscreen-caret.json` and
`text-offscreen-caret.png` under `/tmp/dbx-a11y-vbCm2A`. These establish native
Text geometry and SetTextSelection action behavior, not EditableText support or
an Orca workflow. The native screenshot also showed line numbers starting at one
beside scrolled text; investigate gutter synchronization before completion.

Validation after the editor geometry changes: 98 editor tests passed; the full
UI suite passed 382 tests with two existing ignored tests (83.28 seconds).
Strict UI Clippy, formatting and diff checks passed. The disposable native
process was stopped and both accessibility flags were restored and read back
as false. The full audit goal remains open.

Navigation now publishes named connection/document/Settings tab lists and
Workspace/Database Explorer landmarks. The virtualized Explorer publishes a
named list box, fully qualified object options, selected state and an active
keyboard descendant. Native accessible option focus updates the real cursor;
accessible Scroll Up/Down requests move by 20 objects through the existing
virtual list. Object identities are scoped to their connection, and stale
requests do not navigate a hidden connection. A regression reaches object 99, then filters the list and checks
paging clamps to the remaining matches. Connection format and Theme choices
publish named radio groups and selected state; schema-object lists expose their
object type, schema and definition action. New connection/query controls have
context-specific names.

Native Linux trees asserted the tab groups, landmarks, Explorer list/options,
Settings sections, Theme and Motion groups. Focusing Database objects through
AT-SPI and pressing Home then Down published native focus on teams. Evidence:
`navigation-tree.json`, `explorer-cursor-tree.json` and
`settings-groups-tree.json` under the disposable fixture. The 200% Explorer
heading is now retained alongside the wrapped actions (`explorer-title-200.png`)
and the rendered regression also checks the title's width.

The initial attempt to notify the editor after committing a caret-reveal offset
still left the native gutter stale. A temporary trace confirmed the gutter
painted with offset zero before text committed the reveal offset, and no follow-up
frame painted the gutter. The fix schedules a refresh with `on_next_frame`, since
a refresh during painting can be cleared by the current draw. Temporary tracing
was removed. Native caret offset 560 at 200% now shows lines 50–56 with the caret
on line 56, matching the document (`text-gutter-final.json` and
`text-gutter-final.png`). The strengthened regression checks the scroll offset
stays committed through subsequent frames. All disposable processes were stopped
and both accessibility flags were restored and read back as false.

The six Explorer tests and strengthened caret/200% diagram regressions passed.
Strict UI Clippy is clean. The full UI suite passed 383 tests with two existing ignored tests (83.24
seconds), followed by six passing Explorer checks after connection-scoping hardening.
Strict UI Clippy, formatting and diff checks also passed on the current sources. Screen reader workflows, select-trigger labels and the
remaining audit gates are still open.

Select triggers now attach their name, current value/description and native Click
open/dismiss action to the library's original ComboBox node. This covers database
and schema selection, filter column/operator selection, and enum/boolean/value
state controls in the row inspector. Keyboard selection and trigger focus still
use the original library control. The adapter uses checked downcasts of the
pinned stateless wrapper and StatefulDiv root, applies metadata between layout
and accessibility prepaint, and forwards the original prepaint/paint state. It
adds no layout container or focus handle. Its regression is a dependency contract:
if a component update changes those concrete types, update the adapter before
shipping that dependency update.

Native Linux AT-SPI checks found exactly one named Filter column and Filter
operator ComboBox each. Their descriptions reported `Selected: id` and
`Selected: Equals`. The native Click action opened the original menu; a second
Click dismissed it. Evidence is in `select-filter-tree.json`,
`select-open-tree.json` and `select-dismissed-tree.json` in the disposable fixture.
The open menu exposed unnamed library ListItem rows and no active-option focus;
those are newly confirmed gaps in the pinned list implementation and remain
open. The filter value field also still reports the generic name `Text` and needs
contextual naming. This is partial native selector evidence, not a complete
screen reader workflow. A compositor keyboard probe was refused by its PID guard
when another window was active, so no native keyboard-selection result is claimed.

Editor descriptions now report diagnostic severity and messages. Warnings alone
no longer mark the field invalid. Protected fields publish only generic invalid
feedback, even if a future validator includes input excerpts in a diagnostic.
Regression coverage checks ordinary error/warning descriptions and protected
field redaction. The selector keyboard/focus and pinned-root contract tests pass.
The initial full suite caught a test requesting layout outside GPUI's allowed
frame phases; the test now inspects the root from a real frame prepaint callback.
The final full UI suite passes 386 tests with two existing ignored tests
(84.25 seconds). Strict UI Clippy, formatting and diff checks pass. The disposable
native process was stopped, and both accessibility service flags were restored
and read back as false.

The component UI crate is now retained from upstream commit
`c6ffd3e166abb43bff8845f0aa61711adb128dcf` under
`vendor/gpui-component`, selected by the root Cargo patch. Its Apache license,
source provenance and update/test instructions are retained in
`vendor/gpui-component/DBX-PATCHES.md`. The root patch file
`patches/gpui-component-accessibility.patch` reproduces all seven modified source
files from that upstream source; five contain production accessibility changes
and two adjust compile-time fixture paths. Base, macros and assets remain locked
Git dependencies. Both app and component-test lockfiles resolve GPUI to
`fe9556a11e9cc9dbc78686041aa524d6932879db` and base to the original component
commit. The component has a separate lockfile for its standalone tests.

Select names propagate into the existing list and its search field. Options now
publish names, positions/counts, selected/disabled state and guarded native focus
and activate actions. Search/data replacements change option node identities,
and old handlers check the revision and current label/enabled state. Equal
labels therefore do not reuse old option nodes after a data/search change.
Disabled options cannot activate; arrow navigation skips them. Non-searchable
lists use the existing active-descendant mechanism. Searchable lists retain real
search-field focus and publish a stable polite, atomic cursor status, including
filtered position/count and empty-result feedback.

Native checks exposed two additional problems during this repair. Dispatching
Confirm immediately after changing focus could use the previous frame's path,
so native select activation now invokes the same live menu transition as pointer
activation. Deferred popup views also retained stale accessibility state after
an entity notification; accessible option actions explicitly refresh the window.
Finally, the library input wrapper tracked a frame-only focus handle while its
editor used another handle. It now shares the actual editor handle. Keyboard
input remains in the original editor, and a Tab/Shift-Tab regression proves this
adds no duplicate Tab stop.

The rebuilt disposable Linux app verified native menu opening without a separate
keypress, actual search-field focus, accessible focus on Contains with immediate
`Contains, option 3 of 11` feedback, and native activation changing the selected
value to Contains while restoring trigger focus. Guarded compositor typing of
`greater` filtered to two named options and published
`Greater than, option 1 of 2`; Down then Enter selected Greater than or equal and
restored trigger focus. The filter value field is now named for its selected
column and updates when that column changes. These checks ran with the fixture's
persisted interface scale at 200% and editor scale at 100%; they establish tree,
focus and action behavior, not dropdown typography/layout compliance.

Verified native evidence is in `options-final-open-tree.json`,
`options-final-cursor-tree.json`, `options-final-selected-tree.json`,
`options-final-search-tree.json` and
`options-final-keyboard-selected-tree.json` under `/tmp/dbx-a11y-vbCm2A`.
The earlier captures that still showed the previous state are diagnostic evidence
for the repaint/focus-path failures and are superseded by those rebuilt-app
checks. A New row activation probe did not expose the inspector; its cause and
row-inspector/native non-searchable selector workflows remain unverified. No
insert or other database mutation was confirmed. All native QA processes were
stopped, and both accessibility flags were restored and read back as false.

The component suite passes 452 tests, including stale/equal-label option identity,
filtered cursor/empty feedback, disabled navigation, live menu toggle/focus
restoration and duplicate Tab-stop checks. DBX's complete UI suite passes 386
with two existing ignored tests (82.01 seconds). Strict DBX UI Clippy and strict
component-library Clippy pass. Source patch reproduction, formatting and diff
checks pass. Actual Orca/VoiceOver/NVDA workflows and the remaining audit areas
are still required before completion.

## Compact row inspector and table actions

The earlier native New row probe now has a confirmed cause. Activation created
an insert draft, but `render_data` omitted every inspector when the workspace
was narrower than the scaled 1180px threshold. The same draft appeared after
returning the same QA window from 200% to 100%; native captures
`inspector-before-200-tree.json` and `inspector-before-100-tree.json` record that
comparison. This was a layout gate rather than a missing button handler.

Explicit new/edit-row and row-detail requests now use a full-width inspector in
compact workspaces. Ordinary grid selection keeps the grid available. The
explorer is temporarily hidden while that inspector is displayed, preserving
its stored visibility preference. Focus moves into the first draft field or the
named row-details region; closing preserves the draft, and Continue row draft
resumes it without resetting values. Cancelling and successfully staging return
to the grid. Focus already inside wide-pane details is preserved when a resize
or scale change makes the layout compact, before the main layout is computed.

The inspector has a named region and column/type/required/primary-key field
metadata. Individual text, SQL and selector controls reveal themselves on focus,
including the final offscreen field. Multiline editor height is bounded by the
available scrolling viewport. Read-only fields expose complete values, with
explicit NULL and empty-text descriptions, and Copy actions name their column.
A separate compact-toolbar failure was reproduced: at 200% in an 800px test
window, New row began at x=896 and ended at x=1027. Toolbar actions now wrap,
and the narrow grid can shrink to the available content width.

A regression renders the actual DbxApp at 800×600 and 200%, rather than a copied
layout. It verifies action bounds, opening by a real button click, ordinary
selection versus explicit details, full inspector bounds, offscreen editor
reveal, retained draft contents after closing/resuming, staging and cancellation
with existing staged changes, grid focus, and focused wide-to-compact reflow.

The rebuilt disposable Linux app verified native opening and first-field focus
at 200%; opening its non-searchable state menu, selecting Value and exposing the
named notes editor; native focus revealing the last selector and editor; native
text reading after guarded fixture typing; preserving that text across close
and resume; cancellation and Back to table returning native focus to the grid;
and explicit row details exposing full values. Ordinary native selection keeps
the grid available. Native component geometry places New row at
`(796,253,131,56)` and Row details at `(745,185,161,56)`, fully within the
app's actual minimum window of 960×640. A requested 800×600 native resize is
clamped to that minimum, so 800×600 is harness evidence only.

Evidence files under `/tmp/dbx-a11y-vbCm2A` include
`inspector-fixed-focused-tree.json`, `inspector-fixed-state-menu-tree.json`,
`inspector-fixed-value-tree.json`, `inspector-fixed-closed-tree.json`,
`inspector-fixed-resumed-tree.json`, `inspector-fixed-minimum-details-final-tree.json`,
`inspector-fixed-notes-state-minimum-final-tree.json`,
`inspector-fixed-notes-entry-minimum-final-tree.json`,
`inspector-fixed-minimum-cancelled-final-tree.json`, and
`inspector-fixed-minimum-actions-geometry.json`. Earlier captures made while the
compositor retiled or moved the window partly offscreen are diagnostic captures;
they do not establish a stable focus-restoration defect or layout compliance.
Final native checks used a floating QA window fully on its monitor. Screenshots
verify this inspector slice, not every enlarged app layout or composited-glass
contrast. No native insert/update was staged or committed to the database.

The final full DBX UI suite passes 387 tests with two existing ignored tests
(91.13 seconds). Its existing loopback and subprocess fixtures require an
unsandboxed run: the restricted run had seven fixture/process failures, and the
permitted complete rerun passes. Strict UI Clippy, formatting and diff checks
pass. Component code was unchanged in this slice; its previously verified
452-test result remains the latest component evidence. Native QA processes were
stopped and both desktop accessibility flags restored and read back as false.
Actual Orca/VoiceOver/NVDA workflows and the other completion gates remain open.

## Grid identity, catalog metadata and native paging

Two source regressions demonstrated node aliasing: identical keyless rows on
successive pages received the same keys, and a query's catalog primary-key flags
could alias distinct join matches. Table identity now trusts only complete,
non-NULL, non-repeated catalog keys on data tabs. Driver result metadata did not
carry SQLite primary-key flags in the real fixture; DataTab now passes its
catalog columns to the grid. That metadata also supplies header type/key
information. Queries do not trust catalog keys as unique query-row identities.
Ambiguous rows use typed values and occurrence keys scoped to the source
snapshot and page. A retained source reused for layout changes keeps its scope;
a new ambiguous snapshot gets new nodes. Valid table keys retain identity across
value changes. Changing the page offset invalidates cached keys.

Focus/click/reveal handlers capture a source revision and resolve their row and
column keys against the current delegate before selecting anything. A replaced
or resorted snapshot rejects queued old callbacks even when values or keys still
match. Missing/NULL catalog-key values fall back to ambiguous identity. Full row
payloads are fingerprinted only when needed, avoiding that work for keyed table
records. Column headers and cells now include readable position/sort context,
with stable author IDs and row/column index text. The grid description reports
loaded rows and data-column counts, and the existing live footer supplies the
known exact/estimated total. These descriptions are a fallback for native bridges
that do not expose table indices/counts.

A rendered component regression found a separate identity defect: cell wrapper
IDs and the horizontal list used row positions, and header wrapper IDs used
column positions. Stable delegate row IDs therefore did not preserve descendant
state or native cell identity. Cell/header wrappers now use column keys, and each
row's horizontal list uses a constant ID scoped within that row. The regression
checks fixed and scrolling cell state across row reversal, then cell/header state
across column reordering. It went red for all cells, then only scrolling cells,
then headers as the three positional scopes were repaired, and now passes.
The retained component source patch reproduces all eight modified files, including
`src/table/state.rs`; its provenance/update notes were updated.

The rebuilt Linux app, using only the disposable QA SQLite fixture and 200% UI
scale, verified a server-backed sort of three keyed records. Native accessible
cell objects and author IDs remained identical while zeta moved from row 1 to
row 3. Header descriptions exposed the catalog primary key and current sort;
cell descriptions exposed current row/column positions. Evidence is
`/tmp/dbx-a11y-vbCm2A/grid-sorting-native-identities.json`.

A second native fixture has 1,100 identical keyless rows. Native Next reached
rows 1,001–1,100; its cell IDs and native objects were disjoint from the first
page, and an action interface retained from a removed first-page cell rejected
activation. The grid reported 100 loaded rows and the footer reported the exact
1,100-row total. The saved probe explicitly confirms that the locked Linux bridge
has no native Table interface. Evidence is
`/tmp/dbx-a11y-vbCm2A/grid-paging-native-identities.json`. Initial probe-only
failures were an omitted evidence-writer import and a count expectation that
omitted the UI's thousands separators; corrected complete probes pass. Transient
null child references during a tree replacement are skipped before inspection.
Two temporary tables were created only in the known QA database; no native data
edit, stage or commit workflow was exercised in this slice.

The locked `accesskit_atspi_common` 0.18.1 source exports roles but no Table or
TableCell interface; its relation set exports ControllerFor only. AccessKit 0.24.1
also has no dedicated cell/header relationship properties. Native table
navigation and explicit header relationships are therefore still open bridge
work; role/index metadata, readable context and identity checks do not prove
those interfaces or an Orca/VoiceOver/NVDA workflow.

The complete UI suite passes 391 tests with two existing ignored tests
(89.36 seconds). The component suite passes 453 tests. Strict UI Clippy and
strict component-library Clippy pass, as do app formatting, changed component
formatting, diff checks and source-patch reproduction. The native QA process exited normally with status zero before cleanup, with no
crash output; its absence was confirmed. The desktop accessibility flags were
restored to their recorded pre-probe values (both false) and read back. The
broader goal remains open.

## Live Orca presentation and connection navigation

Orca 50.2 was downloaded with its two missing Python dependencies into
`/tmp/dbx-orca-50.2`. All three Arch packages passed detached-signature verification
against the installed Arch keyring before extraction. Nothing was installed into
system directories. Orca used temporary XDG directories and the in-memory
GSettings backend.

The test ran Orca's real AT-SPI event loop, scripts and speech generation against
the disposable DBX process. Its final speech output was intercepted by a silent
sink; no synthesizer was initialized. Debug logging was disabled, object events
were restricted to the QA process, and protected-field utterances were discarded.
For the import dialog only, two known static public labels were extracted from
protected-field utterances; no protected values or dynamic excerpts were retained.
This verifies reader presentation, not audible synthesis, Braille or a human
reader's complete workflow. The temporary harness is
`/tmp/dbx-orca-50.2/run-dbxtest.py`; its raw filtered journal is
`/tmp/dbx-orca-50.2/dbx-presentation.jsonl`. Selected evidence is retained in
[accessibility-orca-2026-10-11.json](accessibility-orca-2026-10-11.json).

Two actual presentation failures were reproduced and repaired:

- Opening New connection and choosing SQLite left focus on the app root. Orca
  announced only DBX. These explicit transitions now request focus on the URL
  input or connection name after the pane changes. The form has a named semantic
  container whose identity distinguishes the picker from the selected engine's
  form. Orca now announces the picker and **New SQLite connection**, followed by
  the appropriate named entry. Ordinary redraws and Tab navigation retain the
  user's focus. The production-render regression covers these transitions over
  multiple frames, forward traversal and returning to the picker.
- Toasts placed their dismissal button inside the live region. Orca announced
  **Dismiss notification** instead of the outcome. Only the message is now live;
  dismissal is a sibling control. A real local SQLite connection test now
  announces **Connection succeeded · 0 tables**. A nonexistent directory under
  the QA fixture produces the actual redacted database connection failure,
  which Orca also announces. No profile was saved; the successful test used the
  fixture's default SQLite path, entirely under `/tmp`.

Additional verified behavior:

- Focus changes announce named controls, roles and checked state. The vault was
  unlocked only with its known QA passphrase; it was absent from the journal.
- The SQL editor exposed edited multiline Unicode text through native Text.
  Orca received actual caret events at offsets 0, 10 and 30, and its context
  command read the Unicode SQL comment. Its native caret-navigation commands
  moved between the lines. Automatic arrow-key line speech was **not** established:
  this Wayland setup did not provide Orca the key notifications used to classify
  caret movement. Do not count this as complete screen reader editing coverage.
- The import dialog exposed **Review connection import**, hid background
  controls, and Orca's generated protected-field presentation contained the
  public dialog title and field label. Keyboard activation followed by native
  Cancel restored focus to **Import connection profiles**. No chooser was opened
  and no file or profile was imported. This is one modal cancellation workflow,
  not every dialog or save flow.

The full UI suite passed **392 tests, two existing ignored** (90.07 seconds).
All 15 focus tests passed again after the final form identity adjustment.
Strict UI Clippy, app build, formatting and diff checks passed. The temporary
reader and QA DBX processes were stopped explicitly; both desktop accessibility
flags were restored to their recorded false values and read back successfully.
The native table/EditableText bridge, audible/key-driven reader workflows,
remaining layout/contrast work, and macOS/Windows verification remain open.

## Composited contrast and resolved control materials

The opaque-token checks missed actual failures when glass, interaction washes
and shadows were combined. The new tests initially reproduced dark error text
at 3.35:1 on raised controls, dark input edges below 3:1 on some glass surfaces,
and light primary-button pressed text at 3.99:1. A native rendering then showed
that GPUI paints the pane shadow beneath its translucent fill; including that
layer exposed further light-theme muted/blue/status text and edge failures.

The checks use the audit's practical benchmarks:
[4.5:1 for ordinary text](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)
and [3:1 for essential control information](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
These are design checks for the app, not a native-app compliance certificate.
Decorative hairlines do not supply the essential input boundary.

Changes:

- Error, success and warning text have foreground tokens appropriate to their
  surfaces. Light muted/blue text is stronger, and text consumers use these
  tokens for query/setup/update outcomes, errors, warnings and key badges.
  Boolean true and NULL result values use full-contrast text.
- Primary hover/pressed fills are opaque shades. Destructive outline controls
  retain red text with an interaction wash toward the canvas.
- Input edges are stronger in both palettes. The retained component's optional
  input-fill override keeps the fill independent of the edge; its upstream
  default remains when the override is absent. This prevents stronger borders
  from brightening dark fields and weakening their placeholder text.
- The styled component and Base theme snapshots are both synchronized after
  DBX overrides their colors. Custom settings switches use the input-edge token.

The matrix checks both glass/opaque appearances over black, white and primary
RGB desktop colors. It includes the root tint, ordinary/raised panes, full pane
shadows, control washes and opaque content surfaces. Additional checks exercise
the actual resolved component tokens for primary/destructive interaction states
and input fill, value, placeholder and inside/outside boundary contrast. Base
input/focus colors are asserted against the resolved component colors.

The retained native harness,
[`accessibility_contrast.rs`](../crates/dbx-ui/examples/accessibility_contrast.rs),
renders real shared buttons, switches, checkboxes and selectors. An opaque QA
window contains its own black/white backdrops, so captures expose no user desktop
content. Both themes were rendered and visually inspected at 1200×760 on Linux.
Flat background pixels were sampled, then compared with the nominal foreground
tokens; subpixel glyph antialiasing was not treated as a different text color.
The harness has no vault, profiles, user settings or database access; its buttons
are visual fixtures and do not establish an editing/connect workflow.

Selected native sample minima:

| Foreground | Dark | Light |
| --- | ---: | ---: |
| Muted text | 6.0594:1 | 5.4467:1 |
| Blue text | 7.1631:1 | 5.8323:1 |
| Error text | 6.4803:1 | 5.3393:1 |
| Success text | 6.8184:1 | 5.1630:1 |
| Warning text | 7.2341:1 | 5.4549:1 |
| Input edge | 4.2770:1 | 3.7967:1 |

These minima cover the sampled root/glass/raised surfaces over black and white,
not every matrix surface. Exact sampled RGB values, ratios and the theme source
hash are in [accessibility-contrast-2026-10-11.json](accessibility-contrast-2026-10-11.json).
The temporary captures are `/tmp/dbx-contrast-dark.png` and
`/tmp/dbx-contrast-light.png`. Re-run the native harness with:

```sh
rtk cargo run -p dbx-ui --example accessibility_contrast --locked -- dark
rtk cargo run -p dbx-ui --example accessibility_contrast --locked -- light
```

The full UI suite passes **396 tests, two existing ignored** (89.43 seconds),
including all 12 theme tests. The retained component suite passes **454 tests**;
strict app/all-targets Clippy and component-library Clippy pass, as do formatting
and diff checks. The retained patch reproduces all nine modified source files.
Both final native QA processes were stopped. This slice changed no desktop
accessibility flags or user settings. Selected/tag/diagram states, remaining
targets, whole-screen/native compositor coverage and the broader completion
gates remain open.

## Completion gates

Every remaining audit area must be implemented and verified before the goal is
complete. Add actual accessibility-tree/action checks alongside the keyboard
regressions; run the full UI suite, strict Clippy, formatting and diff checks;
verify 200% text size and narrow layouts; and complete isolated screen reader
workflows on each supported release platform. No release or deployment is part
of the current changes.


## Native Linux table interfaces and Orca row presentation

The previously locked AccessKit Unix adapter mapped Grid/Cell roles but did not
advertise or register `org.a11y.atspi.Table` or `TableCell`. DBX now vendors the
same `accesskit_atspi_common` 0.18.1 and `accesskit_unix` 0.21.1 releases and adds
these interfaces. Coordinates and extents resolve actual cells under the current
consumer tree lock. Hidden subtrees and nested tables are excluded; declared
counts include unpainted loaded rows, but absent cells are never fabricated.
Header queries return real row/column header objects. Selection distinguishes a
selected cell from a whole row/column and honors explicit selected row wrappers.
Unsupported atomic whole-axis selection actions return false.

The legacy index contract requires a real index usable by `GetChildAtIndex`.
Nested row cells therefore return `-1` through that legacy method while modern
coordinate, containing-table and header queries remain usable.
[GNOME's index contract](https://gnome.pages.gitlab.gnome.org/at-spi2-core/libatspi/method.Table.get_index_at.html)
defines this constraint.

Native verification caught two wire-format issues during implementation. The
containing-table property must be a single `(so)` object reference: an extra
wrapper caused libatspi's libdbus decoder to abort in the **test client**. The
span method must return four integer outputs for the native GNOME C client.
The current
[GNOME XML](https://github.com/GNOME/at-spi2-core/blob/main/xml/TableCell.xml)
and generated Rust proxy include a leading boolean, whereas the
[client implementation](https://github.com/GNOME/at-spi2-core/blob/main/atspi/atspi-table-cell.c)
still requests `=>iiii`. DBX follows the working C contract and schema tests guard
both formats. A client generated from that inconsistent XML may need correction.

A disposable native DBX instance exposed a 4×3 grid for three data rows plus its
header row, including the row-number column. Native queries verified positions,
both span APIs, containing-table references, both header APIs, null caption and
summary, actual cell selection and unsupported whole-row selection. Sorting
retained all three accessible cell objects and updated their positions and row
headers. The reports use fixture values only.

Live Orca 50.2 processing initially read only “Row 1”, “Row 2” and “Row 3”. Its
full-row generator uses a named row wrapper as the complete row content. Removing
the redundant wrapper label, while retaining each actual row-header name, now
makes the unmodified reader present row values “2, alpha”, “3, mu” and “1, zeta”
and their row/column/type descriptions. The GPUI regression failed before that
change and passes after it. This was real native Component focus and Orca object
processing with a silent DBX-only speech sink, not hardware audio or Braille.

At that stage, the 1000-row fixture reported 1001×2 through Table, but
`GetAccessibleAt(500, 1)` returned null before that row was painted. The next
implementation slice below addresses loaded offscreen cells and headers. This extension establishes the native
protocol and visible-row presentation; it does not complete the grid audit or
cross-platform screen reader workflows.

Evidence is in [accessibility-native-table-2026-10-11.json](accessibility-native-table-2026-10-11.json).
The pinned versions, licenses, compatibility choice and seven-file reproducible
patch are documented in [vendor/ACCESSKIT-PATCHES.md](../vendor/ACCESSKIT-PATCHES.md).
All temporary reader/app processes were stopped and both accessibility service
flags were restored to false and read back successfully.

After the native table and row-presentation fixes, the complete UI suite passed
**397 tests**, with two existing ignored tests (88.25 seconds). The adapter suites
passed 11 consumer/query regressions and two D-Bus introspection/schema regressions.
Strict all-target Clippy passed for UI and both adapters, workspace/adapter format
checks and `git diff --check` passed, and the seven-file adapter patch reproduced
the final vendored sources byte for byte. Source hashes are retained with the
native report. The full accessibility goal remains open.

## Loaded virtualized grid and native focus reveal

The grid now publishes a logical model of every loaded row and visible data
column independently of the renderer's row/column virtualization. Stable keys
identify headers, rows and cells. The original visual table continues to paint
only the viewport; its inline editors and contextual controls remain real
children. This does not load additional database pages or pretend that the
loaded page is the entire result.

Cell/header Component focus and scroll actions use the actual TableState
selection and scroll model. Targets validate the current page revision and
stable row/column keys before acting. Header focus supports keyboard sort,
Left/Right/Home/End navigation and Down into the first data row. Bound table
navigation actions also recognize header focus; the production-binding
regression caught raw key capture running too late and now passes.

Bounds come from the actual painted cell/header containers and current clipping
mask. Unpainted objects return invalid extents and do not report Showing; Visible
keeps them discoverable. Scrolling clears old geometry. The grid's active
descendant follows actual cell/header selection without a separate focus handle.

Native Linux libatspi checks at 150% interface scale establish:

- A 1,000-row loaded page exposes 1,001×2, including its header/row-number
  column. Row 500 is accessible before paint. Component focus reveals/selects
  it, preserves its native object, supplies actual bounds and clears the old
  first-row bounds. Native Focused and Showing follow the reveal.
- A five-row, 40-data-column fixture exposes 6×41. Column 40's header and first
  cell are accessible before paint. Native header ScrollTo reveals both,
  preserves their objects and clears the old first-column bounds.
- The three-key fixture retains its native cell objects through server sorting,
  with updated coordinates, row headers and column relationships.

The model regression covers 1,000 rows and 40 data columns, including the last
cell, unique/reachable IDs and absent unpainted bounds. This is model coverage;
the large 40-column fixture has not yet had a full native reader performance
run. Actual rendering remains virtualized.

Orca 50.2's real table navigator, invoked programmatically with its supported
per-cell speech preference in an isolated memory settings backend, reaches and
reads **item 1000**. Its first-cell command focuses and presents **Row number
column header**. The output sink is silent and restricted to the disposable DBX
process; this is not physical-key delivery, audible synthesis or Braille proof.

Remaining reader behavior is explicit:

- The programmatic default full-row command can present before the offscreen
  row's Showing update, omitting its target value. Per-cell presentation reads
  the target; full-row timing remains open.
- Orca's last-cell command passes row/column counts into a formatter that adds
  one, announcing Row 1002/column 4 for native zero-based (1000, 2). Native
  counts/coordinates are independently verified; they are not changed to
  conceal the reader error.
- Orca appends **blank** after valid cell speech. A narrowly filtered diagnostic
  of the unchanged generator found its result starts with an empty context
  list, followed by the correct value; `_generate_real_table_cell` tests only
  the first item. Extra label leaves did not help and were removed. The trace
  is `/tmp/dbx-orca-50.2/dbx-virtualized-generator-trace.jsonl`; the relevant
  packaged sources are `speech_generator.py:2058` and `table_navigator.py:557`.
- A cold isolated reader took roughly 18 seconds to become ready against the
  1,000-row model in one probe. The native adapter also lacks AT-SPI Cache.
  Neither observation alone identifies the delay's cause. Reader startup,
  larger grids and navigation latency need investigation.

Fixture-only evidence is retained in
[accessibility-virtualized-grid-2026-10-11.json](accessibility-virtualized-grid-2026-10-11.json).
The earlier native protocol and visible full-row evidence remain in the separate
[native table report](accessibility-native-table-2026-10-11.json). Total database
counts, reader edit/save workflows, remaining audit areas and actual
macOS/Windows reader verification remain open.

Final validation after removing the unsuccessful extra-label experiment passed
**401 UI tests**, two existing ignored (87.55 seconds), and **15 adapter tests**
(13 common consumer/query and two Unix schema regressions). Strict all-target
Clippy for UI and both adapters, workspace/adapter formatting and diff checks
passed. The seven-file adapter patch reproduces the final vendored sources byte
for byte. Final native checks re-established table protocol/sorting identity,
row 500 and horizontal column 40 reveal, visible default Orca row speech and
programmatic far-cell/header presentation. Source hashes accompany both reports.
The final QA app and isolated reader were stopped, and both desktop accessibility
flags were restored to their recorded false values and read back successfully.
The broader accessibility goal remains active; these checks complete this grid
slice, not the remaining audit areas or cross-platform workflows.

## Table-query cost and large-grid startup

Repeated native lookup and position requests rebuilt the entire table snapshot
on every call. The focused regression reproduced **40 snapshot builds for eight
sets of queries**. Queries now share one immutable snapshot while the consumer
tree is unchanged. Each context owns its cache; the tree field is private, and
all semantic writes and host-focus changes invalidate the cache under the tree
write lock. A second regression warms the cache, then verifies changed counts,
positions, selection, headers, hidden rows and removal through the write path.
The common suite passes 15 tests; the two Unix schema tests also pass.

In the same generated 1,000-row × 40-data-column Linux debug fixture, the median
lookup/name/position sequence fell from **738.77 ms** to **1.19 ms** on the cache-only
build. The final cache/row-width build measured **1.22 ms**. These are eight local
warm repetitions, not a production benchmark or reader-startup guarantee. Native
last-row/last-column focus preserves the object, reveals actual bounds and updates
selection; its measured action-to-reveal time was about 0.9 seconds.

Cold Orca startup remains a real failure of responsiveness. With the far cell
selected, the isolated reader took **73.94 seconds** to become ready and present
it. Changing only selection to the first data cell on the same model reduced
ready time to **1.68 seconds**. Position therefore strongly affects cold focus
discovery. The current adapter still returns UnknownObject for AT-SPI Cache.
[GNOME's Cache interface](https://github.com/GNOME/at-spi2-core/blob/main/xml/Cache.xml)
provides bulk metadata to avoid individual object/property queries. Implementing
and measuring that path is the next investigation; the current internal table
cache does not implement that protocol or prove its effect on reader startup.

Native rendering also exposed unscaled row-number columns: at 150%, three- and
four-digit positions rendered as ellipses. Their width now follows the interface
scale and the largest absolute row number, including page offsets. Scaling also
updates column minimum/maximum widths with their widths. The native **Interface
size 200%** radio action persisted its setting, and the settled 1280×900 QA
capture shows the complete **1000** alongside the selected final cell.

The scaling transition is still open. A capture immediately after the 200%
change showed headers **27/28/29** over data columns **37/38/39**. A later
refocus/redraw showed matching headers **37/38/39**. OCR verified both captures;
native synthetic-header geometry alone passed and failed to detect the actual
visual mismatch. No settling duration was measured. Candidate causes are an
unscaled scroll offset, previous viewport bounds in header virtualization, or
retained body item sizes. Preserve the original logical exposure while fixing
this; reducing the model to painted cells would lose the accessible data path.

The final full UI suite passes **401 tests**, two existing ignored (88.11 seconds).
Strict all-target UI/adapter Clippy, build, formatting and diff checks pass. The
adapter patch now has nine changed/new Rust files and reproduces them byte for
byte. The 200% rendering is evidence for row-number visibility; it does not close
the scaling transition, remaining small-window workflows or cross-platform gates.
Exact timings, speech, captures, scope and source hashes are retained in
[accessibility-performance-2026-10-11.json](accessibility-performance-2026-10-11.json).

The QA app was later found absent from the host process table and accessibility
registry; its empty log does not establish why it exited. The attempted final
small-table reader rerun therefore did not execute. Earlier speech evidence is
retained with its earlier source hashes. Both accessibility flags were restored
to their recorded false values and read back successfully. Large-grid cold
startup, the scaling transition and the broader accessibility goal remain open.


## Native bulk metadata and the cold-reader boundary

The pinned Unix adapter now implements version 1 of `org.a11y.atspi.Cache`.
The endpoint is installed before the app is announced and returns current public
metadata with real object/parent references, indices, child counts, names,
descriptions, roles, interfaces and states. It adds no separate text, value or
editing payload. Add/remove signals follow registration and removal. Hidden or
removed queued nodes are not announced. Native live signal coherence across
modal hiding, sorting and interface changes still needs direct coverage.

Bulk snapshots drop the app lock before acquiring a window tree lock. Exact
filter results are memoized only within that read lock, then discarded. Metadata
parity, selection/name updates, hidden subtrees and protected-field omissions pass
the common-adapter regression. Schema tests cover the modern GetItems and
Add/RemoveAccessible signatures. Version 2 and asynchronous Ready are not claimed.

The large fixture returned 42,104 objects with unique references and complete
parent relationships. Its cached final-cell reference matched the actual native
Table accessor. With that cell focused, a direct bulk reply took 1.024 seconds,
down from the pre-memoization 2.321-second reply. Python decoding adds separate
cost and is excluded from these wire timings. These are local debug-build
observations, not release-build or cross-platform performance guarantees.

The earlier cache-use probe ran outside libatspi's event loop. Its false cache
predicates were interpreted incorrectly: libatspi 2.60.6 intentionally bypasses
most cached fields unless its event loop or explicit caching is enabled. Repeating
the check inside `atspi_event_main` made all seven default metadata cache flags
true on the real final cell. An Orca helper's complete focused-descendant traversal
inside that loop took 0.607 seconds and returned no focused descendant in that
particular diagnostic. This establishes cache use and traversal cost, not a
successful focused-cell cold startup. The gating condition is in
[GNOME's pinned client implementation](https://github.com/GNOME/at-spi2-core/blob/2.60.6/atspi/atspi-accessible.c).

Actual cold Orca 50.2 startup with the final loaded cell selected still took
75.296 seconds. Its unmodified speech generator read the 1,001-by-41 table,
Row 1000, visible values and `large-r999c39`. Reducing the bulk reply time did not
resolve the initial scan before the event loop starts. A provider-side query for
focused descendants is the next bounded investigation; do not claim cold-reader
performance is fixed or reduce logical grid coverage to conceal this delay.

Final validation for this change: 401 UI tests passed, with two existing ignores
(97.98 seconds); 16 common-adapter tests and three Unix schema tests passed.
Strict all-target UI/adapter Clippy and formatting checks passed. The ten-file
AccessKit patch reproduced the vendored Rust sources byte for byte. Source and
binary hashes, native timings, actual reader speech and the probe correction are
recorded in `accessibility-performance-2026-10-11.json` under `native_bulk_cache`.
Earlier receipts retain their original source scopes.

One unsupervised QA app disappeared with no own log or coredump; its exit cause
is unknown. The final app used a persistent supervisor and was stopped explicitly;
its exit code was -15. Both service flags were restored to false and read back.
No QA app or reader remains running. The overall accessibility goal stays open,
including the earlier scale-transition mismatch and reader editing/save workflows.
