# Keyboard accessibility audit — 11 October 2026

This pass covers keyboard operation and focus in the native GPUI interface.
Evidence comes from dispatched keystrokes, rendered controls and focus handles
in the GPUI test platform. Physical macOS/Linux desktop and screen-reader
validation are separate from these tests.

## Confirmed issues and fixes

| Issue | Result |
| --- | --- |
| Focused dropdown menu ignored Enter | Enter/Space open menus; Escape returns to the trigger. Diagram and query-assistant menus use the shared adapter. |
| Closing a focused document/connection tab restarted traversal | Focus continues to the next surviving control; closing the final document reaches New query. |
| Quick Open dismissal focused the app root | The opener is remembered and restored. |
| Settings retained focus on hidden workspace controls | Opening Settings focuses its close control; closing it restores the opener. |
| Tab left a custom schema-filter popover | Tab/Shift-Tab reach and wrap within popup controls; Escape restores trigger focus. |
| Explorer and inspector splitters were pointer-only | Both are tab stops with visible focus and splitter labels; Left/Right resize, Home resets, and widths persist. |

## Regression coverage

- Connection-form traversal and reverse traversal; Enter activation on engine
  tiles; confirmation-dialog containment; recovery from an unpainted handle.
- Dropdown keyboard activation, reopening, Escape restoration, background
  scroll blocking, and scrolling inside popovers.
- Native select opening, arrow navigation, confirmation and cancellation.
- Document and connection close controls, including the final document tab.
- Quick Open and Settings opener restoration; keyboard-operated Vim setting.
- SQL indentation, grid focus exit, explorer context-menu keyboard operation.
- Vim modes, word/line operators and counts, Visual selection, yank/put,
  grouped undo/redo, Unicode input, input rejection in Normal mode, completion
  only in Insert mode, preserved query shortcuts, and F6 focus exit.
- Backward-compatible preferences, persistence, live application to SQL tabs,
  and keeping other query languages on their native keymaps.

Vim's supported binding set and scope are documented in the
[user guide](user-guide.md#vim-bindings). Motion/operator behavior was checked
against the [Vim reference](https://github.com/vim/vim/blob/master/runtime/doc/motion.txt).

Validation: `cargo test -p dbx-ui` (361 passed, two existing ignored tests),
`cargo clippy -p dbx-ui --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, and `git diff --check`. Network fixture tests run
outside the restricted sandbox so their loopback listeners can bind.
