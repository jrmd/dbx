# Launch backlog — 19 October 2026

This tracks implementation in `feature/launch-safety-workflows`, including
checks of already-shipped behavior. Checked items describe the branch, not a
published app release or authenticated cloud-provider verification.

## P0

- [x] 1. Failed query presentation and server message/position.
- [x] 2. Tab and Enter completion acceptance.
- [x] 3. SQLite paths, home expansion, picker and file drop.
- [x] 4. First-click appearance regression test passes; the original intermittent failure was not reproduced, so no causal fix is claimed.
- [x] 5. Readable, hover-paused keychain notifications.
- [x] 6. ER obstacle routing and persistent manual layout.
- [x] 7. Grid header name priority.
- [x] 8. MIT LICENSE added to checkout.
- [x] 9. All 24 published GitHub release bodies populated; workflow extracts changelog sections.
- [x] 10. GitHub topics and Discussions enabled; contribution and issue/PR templates added.
- [x] 11. Stale schema-diff claim corrected and October 8 audits explicitly marked historical.
- [x] 12. Live old-domain permanent redirect verified (308 to dbx.bonusobjective.com).

## P1

- [x] 13. Action command palette.
- [x] 14. Shortcut screen and editable JSON bindings.
- [x] 15. Table context actions and production typed confirmation.
- [x] 16. Production/protected unbounded UPDATE/DELETE warning.
- [x] 17. Production surface tint and connection folders.
- [x] 18. Staged grid undo/redo.
- [x] 19. Markdown and XLSX export of loaded result rows (full exports retain CSV/TSV/SQL).
- [x] 20. Reviewed DBeaver, Compose and .env imports, with import entry on the connection screen. TablePro bundles and TablePlus connection URLs are supported; proprietary TablePlus files are not parsed.
- [x] 21. Terminal file/URI opening.
- [x] 22. Ask AI to fix errors.
- [x] 23. More themes and Zed theme import.
- [x] 24. Hosted database presets.
- [x] 25. Shareable saved SQL files and favourites.
- [x] 26. Empty/overview connection landing.
- [x] 27. Appearance as Settings entry point.

## P2

- [x] 32. Visual PostgreSQL/MySQL plan trees; PostgreSQL-only opt-in ANALYZE in a separate rolled-back read-only transaction.
- [x] 33. Two-open-connection schema comparison and migration draft, within the same SQL dialect; ambiguous/destructive changes remain review comments.
- [x] 34. MongoDB document replacement and Redis string/hash/list/set/zset/stream editors, with bounded UTF-8 Redis values and explicit review.
- [x] 35. Direct OpenAI/Anthropic/Ollama query drafts with vault credentials; protocol/loopback fixtures tested, authenticated hosted models not exercised.
- [x] 36. Foreign-key JOIN suggestions.
- [x] 37. Verify grid multi-selection and add remaining bulk/context editing.
- [x] 38. Safety-led website and launch draft. Website draft PR: https://github.com/jrmd/dbx-site/pull/1; no launch post submitted.
- Deferred by user: 39. Desktop startup/RAM/scrolling measurements and competitor runs (19 October 2026).
- [x] 40. Linux-focused positioning.
- [x] 41. All-free feature comparison with dated primary sources.
- [x] 42. PostgreSQL read-only query watch and clearly labelled loaded-row column statistics; numeric summaries are approximate.
- [x] 43. Explicitly registered local SQL/Mongo-compatible driver process protocol, documented with a runnable SQLite example. Stateless protocol does not claim native atomic edits, persistent sessions, SSH or backup support.

Benchmarking was explicitly deferred by the user. No performance comparison is
included in the launch work.
