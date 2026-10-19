# Natural-language query assistance

Use an installed CLI or a direct API/local model from Settings → Query assistant.

## Installed CLI setup

1. Install and authenticate your preferred CLI in a terminal: Claude Code (`claude`), Cursor (`cursor-agent`), Codex (`codex`), OpenCode (`opencode`), or GitHub Copilot (`copilot`).
2. Open **Settings → Query assistant** and choose a CLI. DBX checks its installation automatically; **Check CLI** lets you repeat that check. This checks the version, not authentication.
3. Enter an optional model, or leave it blank to inherit the CLI's default. **Advanced settings** contains the executable name or absolute path and, for Codex and OpenCode, a provider ID. Codex accepts an existing configured provider ID; OpenCode accepts a provider ID plus model, or a combined `provider/model`. Other CLIs use their own provider configuration.
4. Changes save automatically. Selecting a CLI makes it the assistant used for generation. Each CLI retains its own settings.
5. Open a connection and query tab, then choose **Ask AI** (Ctrl+K on Linux, Cmd+K on macOS). Describe the result you want, or start with an example request, and choose **Generate query**. Enter generates; Shift+Enter adds a line. Stop or Escape cancels a running generation.
6. Review the query and explanation. **Copy** copies the draft, **Insert query** replaces the editor's query for further review, and **Insert & run** replaces and executes it through the normal DBX query workflow. Replacing a query can be undone in the editor. Regeneration keeps the previous draft available if generation fails or is stopped.

In CLI mode, DBX uses the CLI's existing authentication and launches it locally. Direct API modes are described below. Desktop launches may have a different PATH from your terminal; use an absolute executable path if needed. A wrapper executable can configure provider environment variables before invoking the CLI.

SQL context includes a fresh snapshot of the open database's tables, schemas, column types, primary keys and foreign keys. Other connectors use entity names and any column metadata already loaded in the explorer. They do not sample records for generation; unknown fields may need to be described in your request. The prompt includes the native query format expected by DBX.

The schema and your description are sent to the chosen agent and its configured provider. DBX excludes connection URLs, passwords and sampled row values from the automatically collected schema context. Identifiers and declared enum values are schema metadata; review whether your schema is appropriate to share. Do not include secrets in your description.

After a failed query, **Ask AI to fix** opens an editable request containing the editor query and the database error. Review and redact it before choosing **Generate query**: SQL literals and database errors can contain row values or sensitive details. Opening the request does not send it; generation and applying or running a proposed correction use the same controls described above.

Requests run in disposable directories, with CLI tool permissions restricted where supported. Existing CLI authentication and global provider configuration remain available. This is not an operating-system isolation boundary for an arbitrary executable you configure. No database connection or execution tool is given to the CLI.

Generation never automatically executes a query. Prefer read-only requests; an explicitly requested write can still produce a write query. Review it before running. Cancelling, closing its tab/connection, or switching databases cancels generation; database changes also discard previews. Unix cancellation terminates the request's process group.

Generation has a three-minute CLI timeout, bounded output, and a 96 KiB prompt limit. Oversized schemas produce a clear error rather than silently omitting metadata. Schema loading and inference together are limited to 200 seconds.

Adapter references: [Claude CLI](https://code.claude.com/docs/en/cli-reference), [Cursor permissions](https://cursor.com/docs/cli/reference/permissions), [Codex non-interactive mode](https://developers.openai.com/codex/noninteractive/), [OpenCode CLI](https://opencode.ai/docs/cli/), and [Copilot programmatic mode](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-programmatic-reference). CLI flags and output formats may change; check your installed version when diagnosing a failure.

Run fixture and native render checks with `cargo test -p dbx-ui agents`. An opt-in paid inference smoke test creates a disposable SQLite schema, generates a count query and explicitly runs it:

```bash
DBX_AGENT_SMOKE_CLI=codex cargo test -p dbx-ui live_cli_generates_from_sqlite_schema -- --ignored --nocapture
```

Use `claude`, `cursor`, `opencode` or `copilot` to select a different CLI for this smoke test.

## Direct API and local models

Settings → Query assistant also offers **OpenAI API**, **Anthropic API**, and
**Ollama**, without an installed agent CLI. Select a provider and enter a model
name available to your account or local Ollama installation. Hosted API requests
use your own provider account and billing.

For OpenAI or Anthropic, unlock the DBX vault, enter the key in the masked field,
and click **Save key**. Keys are encrypted in the vault, never written into the
settings JSON, and can be deleted with **Remove key**. The input clears after
saving. OpenAI requests use the [Responses API](https://developers.openai.com/api/docs/guides/text)
with response storage disabled. Anthropic requests use the
[Messages API](https://platform.claude.com/docs/en/api/messages/create).

For Ollama, run your chosen model locally and enter its installed model name.
The default server is `http://127.0.0.1:11434`; Advanced settings can change it.
HTTP is accepted only on loopback; remote endpoints require HTTPS. DBX uses
Ollama's [generate endpoint](https://docs.ollama.com/api/generate) without streaming.
No hosted-provider fallback occurs if your local model fails.

These adapters send only the editable request and schema context. They expose
no database execution tools. SQL/error text included in your request may contain
values: review it before clicking Generate. Output remains a draft for Insert
or explicit Insert & run, subject to the normal query safeguards. Responses are
bounded to 1 MiB, redirects are refused, cancellation stops waiting for a result,
and errors do not echo provider response bodies or API keys.
