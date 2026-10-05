# Natural-language queries with your CLI

1. Install and authenticate your preferred CLI in a terminal: Claude Code (`claude`), Cursor (`cursor-agent`), Codex (`codex`), OpenCode (`opencode`), or GitHub Copilot (`copilot`).
2. Open **Settings → Query assistant** and choose a CLI. DBX checks its installation automatically; **Check CLI** lets you repeat that check. This checks the version, not authentication.
3. Enter an optional model, or leave it blank to inherit the CLI's default. **Advanced settings** contains the executable name or absolute path and, for Codex and OpenCode, a provider ID. Codex accepts an existing configured provider ID; OpenCode accepts a provider ID plus model, or a combined `provider/model`. Other CLIs use their own provider configuration.
4. Changes save automatically. Selecting a CLI makes it the assistant used for generation. Each CLI retains its own settings.
5. Open a connection and query tab, then choose **Ask AI** (Ctrl+K on Linux, Cmd+K on macOS). Describe the result you want, or start with an example request, and choose **Generate query**. Enter generates; Shift+Enter adds a line. Stop or Escape cancels a running generation.
6. Review the query and explanation. **Copy** copies the draft, **Insert query** replaces the editor's query for further review, and **Insert & run** replaces and executes it through the normal DBX query workflow. Replacing a query can be undone in the editor. Regeneration keeps the previous draft available if generation fails or is stopped.

DBX uses your CLI's existing authentication. It does not store provider API keys or call a hosted inference API itself. Desktop launches may have a different PATH from your terminal; use an absolute executable path if needed. A wrapper executable can configure provider environment variables before invoking the CLI.

SQL context includes a fresh snapshot of the open database's tables, schemas, column types, primary keys and foreign keys. Other connectors use entity names and any column metadata already loaded in the explorer. They do not sample records for generation; unknown fields may need to be described in your request. The prompt includes the native query format expected by DBX.

The schema and your description are sent to the chosen agent and its configured provider. DBX excludes connection URLs, passwords and row values from that context. Identifiers and declared enum values are schema metadata; review whether your schema is appropriate to share. Do not include secrets in your description.

Requests run in disposable directories, with CLI tool permissions restricted where supported. Existing CLI authentication and global provider configuration remain available. This is not an operating-system isolation boundary for an arbitrary executable you configure. No database connection or execution tool is given to the CLI.

Generation never automatically executes a query. Prefer read-only requests; an explicitly requested write can still produce a write query. Review it before running. Cancelling, closing its tab/connection, or switching databases cancels generation; database changes also discard previews. Unix cancellation terminates the request's process group.

Generation has a three-minute CLI timeout, bounded output, and a 96 KiB prompt limit. Oversized schemas produce a clear error rather than silently omitting metadata. Schema loading and inference together are limited to 200 seconds.

Adapter references: [Claude CLI](https://code.claude.com/docs/en/cli-reference), [Cursor permissions](https://cursor.com/docs/cli/reference/permissions), [Codex non-interactive mode](https://developers.openai.com/codex/noninteractive/), [OpenCode CLI](https://opencode.ai/docs/cli/), and [Copilot programmatic mode](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-programmatic-reference). CLI flags and output formats may change; check your installed version when diagnosing a failure.

Run fixture and native render checks with `cargo test -p dbx-ui agents`. An opt-in paid inference smoke test creates a disposable SQLite schema, generates a count query and explicitly runs it:

```bash
DBX_AGENT_SMOKE_CLI=codex cargo test -p dbx-ui live_cli_generates_from_sqlite_schema -- --ignored --nocapture
```

Use `claude`, `cursor`, `opencode` or `copilot` to select a different CLI for this smoke test.
