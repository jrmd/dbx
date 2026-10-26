# Isolated desktop QA

On Linux or macOS, run `scripts/run-ui-test.sh` from any directory. It builds DBX and launches the real native app with a separate config directory and working directory under `target/ui-test/`.

- `config/dbx/credentials.vault`: disposable encrypted QA vault.
- `config/dbx/connections.json`: four QA profiles, without embedded passwords.
- `config/dbx/settings.json` and `query-history.json`: test appearance and query history.
- `workbench.sqlite`: 250 projects and three teams, including foreign keys, fractional numeric values, and NULLs.
- `artifacts/`: screenshots and run evidence.

The launcher creates only missing fixtures. Relaunching preserves the vault, saved credentials, data edits, and appearance. Normal user config is not read or modified. The working directory is isolated too, so the default relative SQLite path cannot open the repository's database. All generated state stays in the ignored `target/` tree.

`DBX_CONFIG_DIR` overrides the complete DBX state directory on every platform, including the vault, settings, history, keymap, registered drivers, library preferences, and default SQL library. An overridden profile is labelled “DBX — isolated profile” in the native window title. Use an absolute path for repeatable QA sessions.

The launcher provisions the vault through DBX's own Rust vault implementation. Unlock it using the deliberately public, test-only passphrase `dbx-ui-test-vault`. Never put real credentials in this vault. Fresh PostgreSQL and MySQL profiles receive `dbx_test_password` in the encrypted vault. Existing credentials are retained. A vault with a different passphrase causes the launcher to stop rather than overwrite it.

## Docker databases

From the repository root:

```bash
docker compose -f docker-compose.test.yml up -d --wait
```

Use `sudo` if your machine requires it. The app accesses loopback TCP ports and does not need permission to use the Docker socket. PostgreSQL uses 55432, MySQL 53306, and Redis 56379. All profiles are marked Local. The Compose database storage is disposable; stopping/removing containers can discard it.

To test already-running databases without starting or stopping Docker:

```bash
DBX_TEST_POSTGRES_URL=postgres://dbx_test:dbx_test_password@127.0.0.1:55432/dbx_test \
DBX_TEST_MYSQL_URL=mysql://dbx_test:dbx_test_password@127.0.0.1:53306/dbx_test \
DBX_TEST_REDIS_URL=redis://127.0.0.1:56379/0 \
DBX_TEST_SQLITE_URL="sqlite://$PWD/target/ui-test/integration.sqlite?mode=rwc" \
cargo test -p dbx-core --test integration --locked -- --ignored --test-threads=1
```

Run the launcher first to create `target/ui-test`. These tests perform CRUD against their integration tables and must only target disposable databases.

## Native checks

### Mac Liquid Glass

On macOS 26 or later, open Settings → Appearance and toggle Liquid Glass. Check
the window against a colourful desktop in light and dark mode, including an
explicit DBX appearance that differs from macOS. The native Explorer and
toolbar should reveal the material; document tabs and settings belong to the
opaque content layer. Tables, SQL editors,
forms and settings content must stay opaque and stable as the desktop changes.
Check text, alternating rows, selection, focus and open menus
for readability. Pay particular attention to muted metadata, SQL comments,
inactive tabs and status text over bright and dark backgrounds. Pane edges,
row separators and buttons should have no painted white rims; keyboard focus
and input outlines must remain visible. Turning glass off must make the surfaces solid immediately.
Restart and verify both saved
choices. Toggle macOS Reduce Transparency and Increase Contrast while DBX is
open: either preference should make the window solid within a second without
changing the saved Liquid Glass switch. Restore both preferences afterwards.
Also check typing, menus, window dragging, resizing, fullscreen and closing
after repeatedly toggling glass. On macOS versions before 26, verify the
explanatory setting and solid fallback without a crash.

Build with Xcode 26 / Swift 6.2 or newer for the native shell; older compilers
build the solid fallback. Run `scripts/test-native-shell.sh` to exercise real
AppKit lifecycle, repeated toggles, selection events, AccessKit-root retention,
scroll coordinate alignment and restoration without a database or visible window.

Use an Explorer with more rows than fit onscreen. Scroll underneath its search
accessory and check Apple's native edge treatment, then independently scroll
the grid, a long SQL editor and Settings. Search, clear search, change schema or
database, select several tables and open the context menu. Confirm read-only and
view capabilities still gate mutations. Test individual connection tabs (including closing an inactive connection), refresh,
new query, sidebar toggle and keyboard focus (`Cmd-Shift-E` / Return). Open a
destructive-action confirmation and cancel it: the native controls must be
blocked while the modal is open and restored afterwards. Verify screen-reader
access to both native navigation and GPUI content after toggling several times. Lock the vault and verify app toolbar controls are absent, not merely disabled; unlock and verify the tabs return. Check for a colour seam across the titlebar above the split.

Create the vault, reject a mismatched confirmation, then unlock it again after restarting DBX. Save the Docker passwords through the form and reconnect after relaunch without re-entering them. Check Data, Query, Structure, foreign-key navigation, row editing, filters, and tab switching in dark and light appearances. Use both a narrow and wide window. Inspect the actual data cells as well as connection success: unit/build results alone do not prove native rendering.

`DBX_UI_TEST_SKIP_BUILD=1 scripts/run-ui-test.sh` reuses a binary already built for this checkout. For Linux X11 automation, launch with `env -u WAYLAND_DISPLAY`; otherwise the launcher uses the desktop's normal backend. macOS window behavior still requires a Mac.
