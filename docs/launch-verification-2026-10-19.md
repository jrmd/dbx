# Launch backlog verification — 19 October 2026

Implementation is on `feature/launch-safety-workflows`; website copy is in
[jrmd/dbx-site#1](https://github.com/jrmd/dbx-site/pull/1). These checks do not
constitute a published release. Scope and remaining limits are recorded in the
[backlog ledger](launch-backlog-2026-10-19.md).

## Passed locally

- `cargo fmt --all -- --check` and `git diff --check`.
- `cargo test --locked --workspace`: 470 passed, no failures. Tests requiring
  other external services or signing fixtures retain their explicit ignore markers.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`.
- `scripts/test-release-notes.py`: two tests; `scripts/test-macos-release.py`:
  five tests, using Python 3.13. The latter uses fixture signing tools, not a
  new notarization.
- Disposable PostgreSQL 16: existing workbench-safety integration plus the new
  isolated ANALYZE/watch test. Confirmed database error message/code/position,
  actual plan metrics, server-enforced read-only mode, rollback on error,
  cancellation, and preservation of the original tab's uncommitted transaction.
- Disposable MongoDB 8: canonical BSON editing, immutable identity, conflicts
  including newly added fields, and protected-write rejection.
- Disposable Redis 7: structured types, streams, expiry preservation, expired
  and changed-key conflicts, malformed/oversized input rejection before changes,
  logical-database identity and protected-write rejection.
- External-driver fixtures: the runnable SQLite driver browses metadata, binds
  parameters and enforces row caps; oversized/malformed responses and timeouts
  fail; cancellation stops descendants. Imported executable paths and native
  backup routing are rejected, and hostless URLs/passwords round-trip through
  profile storage without exposing credentials.
- Native GPUI test coverage includes completion Tab/Enter behavior, failed-query
  presentation, first-click Dark persistence, staged bulk editing/undo/redo,
  command/destination palette separation, and API-key clearing including undo.
- Website `pnpm lint` and `pnpm build`; desktop 1280px and mobile 390px browser
  inspection, including contained horizontal scrolling of the comparison table.

The default macOS debug test linker reports a large unwind-table warning;
Clippy succeeds with warnings denied. A preexisting `block 0.1.6` dependency
also emits Cargo's future-compatibility notice.

## External and manual boundaries

The intermittent first-click Dark issue was not reproduced; its regression
passes, so a causal fix is not claimed. Direct API request/response handling is
covered by fixtures, including an actual loopback Ollama HTTP request; no
user's hosted API key or cloud database account was used. Other server engines,
physical Linux desktop interactions and a signed release install were not
revalidated in this pass. CI remains responsible for its Linux/macOS build and
full disposable-connector matrix.

All 24 existing published GitHub releases have version-specific notes. Topics
and Discussions are enabled. The old HTTPS domain was verified returning a
308 redirect to `https://dbx.bonusobjective.com/`.

Benchmarking was deferred by the user. No performance measurements or
competitor speed/RAM claims were produced, and no launch post was submitted.
