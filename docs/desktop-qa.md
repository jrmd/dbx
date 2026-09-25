# Isolated desktop QA

On Linux, run `scripts/run-ui-test.sh` from any directory. It builds DBX and launches the real native app with a separate config directory and working directory under `target/ui-test/`.

- `config/dbx/credentials.vault`: disposable encrypted QA vault.
- `config/dbx/connections.json`: four QA profiles, without embedded passwords.
- `config/dbx/settings.json` and `query-history.json`: test appearance and query history.
- `workbench.sqlite`: 250 projects and three teams, including foreign keys, fractional numeric values, and NULLs.
- `artifacts/`: screenshots and run evidence.

The launcher creates only missing fixtures. Relaunching preserves the vault, saved credentials, data edits, and appearance. Normal user config is not read or modified. The working directory is isolated too, so the default relative SQLite path cannot open the repository's database. All generated state stays in the ignored `target/` tree.

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

Create the vault, reject a mismatched confirmation, then unlock it again after restarting DBX. Save the Docker passwords through the form and reconnect after relaunch without re-entering them. Check Data, Query, Structure, foreign-key navigation, row editing, filters, and tab switching in dark and light appearances. Use both a narrow and wide window. Inspect the actual data cells as well as connection success: unit/build results alone do not prove native rendering.

`DBX_UI_TEST_SKIP_BUILD=1 scripts/run-ui-test.sh` reuses a binary already built for this checkout. For Linux X11 automation, launch with `env -u WAYLAND_DISPLAY`; otherwise the launcher uses the desktop's normal backend. macOS window behavior still requires a Mac.
