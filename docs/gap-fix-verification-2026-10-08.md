# Product gap fixes — 8 October 2026

The implementation follows the [dated competitor audit](product-gap-audit-2026-10-08.md). It remains unreleased. Verification uses disposable local databases and hosted CI as requested. No production database profiles, provider credentials or physical Mac are used.

## Implementation map

| Item | Delivered scope | Acceptance boundary |
| --- | --- | --- |
| G01 | Literal quoting for separated schema/table/column components, including dotted names | Dialect regressions and actual SQLite CRUD |
| G02 | Atomic loaded-result exports, explicit truncation notice and separate cancellable full-query stream export | Native read-only snapshot, one read statement, fresh results; existing transactions are excluded |
| G03 | Rename, duplicate, guarded delete, password-free export, authenticated DBX bundles and TablePro plaintext/encrypted imports | Imported profiles protected; no startup commands or automatic connection; TablePlus proprietary files require URL migration |
| G04 | Native PostgreSQL/MySQL backup/restore, version checks, progress, logs, cancellation and private credentials | PostgreSQL transactional restore; MySQL DDL can commit before failure. Native MySQL verified TLS requires direct TCP because its CLI skips TLS on sockets |
| G05 | Column properties, PK/FK/CHECK drafts, metadata selectors, type suggestions, combined SQL review and open-tab refresh | MySQL attributes preserved from metadata; generated/unknown attributes and unsupported SQLite operations require manual SQL |
| G06 | Visible connector capabilities; independent SQL Server clients and atomic checked changesets | Stateless analytics/provider APIs retain explicit limits; no document write editor added |
| G07 | Original TLS identity over SSH and automatic supported CLI credential refresh | Local wrong-host/CA fixtures establish transport behavior; authenticated RDS/Entra accounts remain untested |
| G08 | Vault-encrypted draft changesets with connection/database/table/column identity and original values | Review and fresh conflict validation required; results and transaction state never recover; workspace limit 2 MiB |
| G09 | Fixed 1001-table/100000-row workload, deep paging, eight concurrent readers, full-query export and reconnect | Core latency report on local/CI hardware; no comparative GUI benchmark or physical input/accessibility claim |
| G10 | Text/date/outcome/connection history search, draft-only opening, recording opt-out, retention and clear | Local plaintext disclosure; searchable connections must be open |
| G11 | Exact previewed CSV/TSV/JSON/JSONL data, destination mapping, atomic append, bounded capture/copy and keyed data diff | 64 MiB/100000 rows; no guessing conversion, no overwrite/sync. XLSX/full sync remain demand-dependent |
| G12 | Explicit ephemeral local MCP pairing, fixed database scope, metadata/read tools, auth, budgets, cancellation and activity | No arbitrary SQL/write tools; CLI pairing configuration is private and revocable |
| G13 | Disposable demo connection, switching guide, capability matrix, changelog and working-feature site content | Website additions describe baseline shipped workflows; new application behavior remains unreleased |
| G14 | Intel Mac test/candidate/release matrix and updater asset selection | Published Intel package/signing and physical QA require release evidence; Windows, roles/grants UI, Oracle and mobile remain demand-dependent |

## Evidence

Verification is in progress. Final local and hosted results, exact commit and run links are recorded here after the gates finish. Earlier v0.5.0 hosted runs in the audit are baseline evidence only.

The website production build and lint pass. Collaborative browser checks at 1280×800 and 390×844 found no horizontal overflow, the staged-change tile and correct task-oriented documentation links.

## Deferred environment and demand checks

Following the explicit environment choice, physical Mac keyboard/IME/screen-reader/scaling and authenticated cloud-provider checks are deferred. The implementation adds repeatable CI checks and preserves those acceptance boundaries. Customer switching trials, platform demand and full synchronization/administration scope need product evidence before expansion; this is not a declaration of complete competitor parity.
