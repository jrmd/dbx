# DBX accessibility audit 11 October 2026

DBX can become substantially accessible with its existing GPUI stack. Keyboard
operation has a useful foundation, but custom text editing, the results grid,
and much of the app chrome currently lack the information assistive technology
needs. Screen reader support should be the next major accessibility investment.

This audit covers checkout `21a1569e44c754fd63d1b5b97734553b57679b29`, source
inspection of the actual locked dependencies, dispatched keyboard tests on the
GPUI test platform, and calculated theme contrast. It does not establish actual
Orca, VoiceOver, NVDA, voice control, or desktop magnifier behavior. WCAG criteria
are used as practical design benchmarks, not a native-app compliance certificate.

## What GPUI provides

The locked GPUI revision is `fe9556a11e9cc9dbc78686041aa524d6932879db`, with
AccessKit and Linux, macOS, and Windows accessibility adapters. The version label
`0.2.2` alone would give a misleading impression of this dependency's capabilities.
The locked gpui-component revision is `c6ffd3e166abb43bff8845f0aa61711adb128dcf`.

GPUI requires stable element IDs and semantic roles for reported nodes. Keyboard
focus and click handlers alone do not supply those roles. Custom elements can
provide synthetic text children and selection information. These APIs are already
present in DBX's locked source; an upgrade is not a prerequisite for starting.
See the [GPUI accessibility guide](https://github.com/zed-industries/zed/blob/fe9556a11e9cc9dbc78686041aa524d6932879db/crates/gpui/src/_accessibility.rs).

## Findings and priority

P1 blocks a core workflow for a group of users. P2 makes a workflow harder or
less discoverable. Findings below are confirmed source gaps; their exact spoken
presentation still requires testing with the operating system accessibility API.

### P1 Custom text fields and SQL editor have no accessible text interface

`crates/dbx-ui/src/editor.rs:3080` gives the shared input shell an ID and focus
handle, but no text-input role, accessible name, or value. `TextEditorText` at
line 2500 paints glyphs directly, returns no element ID, and implements no
accessibility hooks. This affects connection fields, vault passphrases, search,
import/export fields, row editing, and SQL editing.

Give each field a stable semantic identity, a persistent label and appropriate
text role. Publish value, selection, caret, read-only/invalid state, and text
runs; handle accessible focus and editing requests. Use protected/password
semantics and never publish the underlying passphrase. Verify Unicode selection,
multiline text, folded SQL, and offscreen caret movement through the real bridge.
IME support and ordinary clipboard operation do not prove this interface exists.

### P1 The virtualized results grid has no table accessibility model

DBX uses gpui-component `DataTable`/`TableState` with `ResultTableDelegate`
(`crates/dbx-ui/src/app/result_table.rs:1037`). That dependency path builds ordinary
divs for the grid, rows, headers, and cells, without accessibility roles or row
and column metadata. DBX's delegate supplies no semantic replacement. The
dependency's separate static `Table` components do have semantics, but DBX's
virtualized grid does not use them.

Expose a grid/table with total row and column counts, header relationships, cell
values, selected cell, sort direction, and edit state. Keep row identity stable
across paging/sorting and announce NULL explicitly. Virtualization must let an
assistive user reach data beyond the painted viewport without rendering the
entire dataset. Connect accessible selection/navigation requests to the existing
grid actions. Preserve the row inspector as a complementary reading/editing path.

### P1 Custom navigation and icon controls are missing roles or names

`Pressable::pressable` (`theme.rs:675`) adds keyboard traversal and a visible ring,
but no semantic role or name. Connection/document tabs, appearance choices,
custom icon buttons, and document close controls use it. For example,
`app/view/chrome.rs:1261` renders document tabs without tab/selected semantics;
the close control at line 920 has an 18px hit area and no accessible label.

Library `Button` publishes a role and derives its accessible name from `.label`.
An icon passed through `.child(...)` plus `.tooltip(...)` does not provide that
name in the locked implementation. Examples include the confirmation close
button (`app/view/overlays.rs:430`) and export close button (line 182).
Settings switches publish checked state through the library, but switches such
as Reduce transparency (`app/view/settings.rs:232`) have no `.label`; the
adjacent visual settings label is not a programmatic association.

Make roles, names, disabled state, and selected/checked state part of shared
control helpers. Name close actions with their document or dialog context.
Give tab groups and appearance choices appropriate group and selection semantics.
Add meaningful labels to settings switches and context to repeated Export buttons.

### P1 Dialogs and asynchronous outcomes are not exposed semantically

Confirmation and export overlays have keyboard focus containment, but no dialog
role, modal state, or title/description association
(`app/view/overlays.rs:398` and line 151). The locked focus-trap wrapper also
does not forward accessibility hooks, so adding a role only to its wrapped
element needs verification; put semantics on an exposed outer node or fix
forwarding. Keyboard containment alone does not hide the background from a
screen reader's browse navigation.

The status footer (`app/view/chrome.rs:1183`) renders outcomes and errors as raw
string children without a status/alert node. Raw string elements in this GPUI
revision have neither an ID nor a role. Similar text must be exposed explicitly
through accessible text elements or labelled semantic parents.

Expose dialog titles and descriptions, make the background unavailable while a
modal is active, and restore both keyboard and accessibility focus on dismissal.
Announce query completion, cancellation, connection failure, validation errors,
and staged/save outcomes. Prefer concise announcements and avoid announcing
every render or every row streamed into a result.

### P1 There is no app wide text scaling

`settings.rs:22` has appearance and transparency preferences but no font/UI scale.
The shared editor uses fixed 12px text and a fixed 32px single-line field height.
Grid headers and metadata use 8–11px text (`result_table.rs:1101–1139`); controls
also use fixed sizes. Diagram zoom changes only the diagram, and window zoom
maximizes the window.

Add persistent UI and editor text-size controls with keyboard shortcuts, and
scale control heights, spacing, grid rows, and editor metrics with them. Test
200% text size in a small window, including connection setup, Settings, dialogs,
and inline editing. Operating system display scaling is a useful complement,
but does not establish independent text resizing without lost content.
See [W3C Resize Text](https://www.w3.org/WAI/WCAG22/Understanding/resize-text.html).

### P2 Dark blue text and input boundaries need stronger contrast

Calculated from the opaque sRGB tokens in `theme.rs:87` and line 130:

| Pair | Dark | Light |
| --- | ---: | ---: |
| Main text on canvas | 17.865:1 | 15.534:1 |
| Muted text on canvas | 7.633:1 | 5.679:1 |
| Accent text on canvas | **3.787:1** | 5.516:1 |
| Accent text on raised panel | **3.372:1** | 5.264:1 |
| SQL comments on canvas | 4.746:1 | 4.512:1 |
| White text on primary accent | 5.169:1 | 5.817:1 |
| Strong border against canvas | **1.736:1** | **1.713:1** |
| Focus ring against canvas | 7.698:1 | 5.396:1 |

The small-text benchmark is 4.5:1. Dark-theme accent text fails it, including
the Vim mode/help indicator on the query canvas (`app/view/query.rs:558`).
Use a lighter blue foreground token for dark-theme text independently of the
primary button fill. Essential field boundaries use `border_strong` and are
below the 3:1 non-text benchmark; assess the complete field against its adjacent
surface rather than treating every decorative separator as a failure.

The existing focus ring and main/muted opaque text are good foundations.
Glass surfaces need composited contrast checks against light and dark desktop
backdrops; the table does not certify translucent surfaces. Keep the opaque
surface option and platform transparency preferences.
See [W3C Text Contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)
and [Non-text Contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).

### P2 Some precision interactions need alternatives and larger hit areas

Column resizing uses a 7px drag handle with pointer handlers and no keyboard
resize action (`result_table.rs:1150–1198`). Document close controls use 18px
targets. Add keyboard column resizing or an explicit width control, and expand
hit areas while retaining the compact visual icon.

The diagram already has keyboard pan, zoom, fit, and refresh actions
(`app/view/diagram.rs:993`). Its node selection/opening and rearrangement use
pointer handlers, and its relationships have no accessible reading model.
Provide a named table/relationship list and keyboard selection/opening of nodes;
provide an alternative to dragging for meaningful layout operations.

DBX has no reduced-motion preference. The locked component library animates
switch and checkbox changes. This is a follow-up improvement with less urgency
than text/grid semantics: honor the platform preference and allow animation to
be disabled, without removing state feedback.

## Keyboard behavior checked

These commands passed in this audit on the GPUI test platform:

```sh
rtk cargo test -p dbx-ui --locked app::focus::tests -- --test-threads=1
# 13 passed
rtk cargo test -p dbx-ui --locked popups::tests -- --test-threads=1
# 6 passed
rtk cargo test -p dbx-ui --locked app::view::pane_resize::tests -- --test-threads=1
# 1 passed
```

They exercise connection-form forward/reverse traversal, engine activation,
modal containment, tab-close focus continuation, hidden-handle recovery, SQL
indent/F6 exit, grid focus exit, Vim preference operation, dropdown/popover
activation and focus restoration, select navigation, scroll shielding, and
explorer splitter resizing/reset. The earlier
[keyboard audit](keyboard-a11y-audit-2026-10-11.md) covers the previous fix pass.
These 20 passing tests establish those keyboard behaviors, not full app coverage
or screen reader support.

## Implementation order and release verification

1. Establish accessible shared controls and text-field labels. Include the vault
   and connection setup so users can reach the workbench independently.
2. Implement accessible editing and the virtualized grid. Treat these as core
   product interfaces with real text/selection and navigation tests.
3. Expose dialogs, app regions, relationships, and status/error announcements.
4. Add text scaling, adjust contrast, and improve precision interactions.

For each stage, capture the actual accessibility tree and assert names, roles,
state, focus, and action behavior. GPUI exposes `Window::debug_a11y_tree_json`
once assistive technology activates the bridge. Add those checks alongside
the existing keyboard tests, including a check that protected fields never
expose their plaintext value.

Before claiming screen reader support, run an isolated demo session with Orca
on Linux, VoiceOver on macOS, and NVDA on Windows for each supported release
platform. Complete unlock, connection setup, query editing/execution, result
navigation, row editing/save, errors, and cancellation without a pointer.
Check modal browse containment, announcement order, 200% text size, opaque/glass
contrast, and focus visibility in a narrow window. Use the existing
[isolated desktop QA](desktop-qa.md) fixtures to avoid real credentials and data.

This audit records the original application state. Follow the
[implementation tracker](accessibility-implementation.md) for subsequent changes
and their validation. The audit itself does not establish screen reader support.
