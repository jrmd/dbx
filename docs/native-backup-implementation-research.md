# Native backup and restore implementation research

**Scope.** This is a design note for a DBX wrapper around installed PostgreSQL and MySQL client tools. It uses only upstream PostgreSQL, MySQL, and OpenSSH documentation. It is not proof against the particular client binaries DBX will ship; the wrapper must record executable and server versions and run the stated compatibility checks.

## Decision

* For PostgreSQL over SSH with certificate hostname verification, use an owned loopback **TCP** forward. Pass the original DNS name as `host`, `127.0.0.1` as `hostaddr`, and the forwarded port as `port`. Do not use a Unix-domain socket: libpq ignores `sslmode` for socket connections.
* For MySQL, a Unix-socket SSH forward plus `--protocol=SOCKET --socket=... --host=<original DNS> --ssl-mode=VERIFY_IDENTITY` is a documented composition, but MySQL does not document this exact combination as an example. Gate it behind an end-to-end test against the distributed client before exposing it as verified. A loopback TCP forward is safer when that test is unavailable, but needs a verified way to retain the original TLS identity.
* Start PostgreSQL dumps in custom format and restore with `pg_restore`; write MySQL dumps as SQL and feed them to `mysql` through process stdin. Never invoke a shell, put a password in argv, or log an option file / conninfo containing secrets.

## PostgreSQL: authenticated connection through an SSH TCP forward

libpq permits `host` and `hostaddr` together: `hostaddr` selects the network address, while `host` remains available where a host name is required, including password-file matching. `verify-full` requires a host name. The documented contract supports this invocation shape:

```text
host=<original DNS> hostaddr=127.0.0.1 port=<owned forward port>
dbname=<database> user=<user> sslmode=verify-full
sslrootcert=<CA path> [sslcert=<client cert> sslkey=<client key>]
```

Pass the resulting libpq conninfo as one `--dbname` argument using a dedicated encoder for libpq key/value quoting; launch through a process API with discrete arguments, never a shell. The connection remains TCP to the local forward while certificate identity is checked against the original DNS name. This is the required path for `verify-full`; PostgreSQL states that `sslmode` is ignored for Unix-domain sockets.

Create an owned temporary `PGPASSFILE` with mode `0600`, set `PGPASSFILE` only on the child process, and remove it after completion. The one line must use the original host and forwarded port:

```text
<escaped-original-host>:<forwarded-port>:<escaped-database>:<escaped-user>:<escaped-password>
```

In `.pgpass`, a colon and backslash are escaped with a backslash, the first matching line wins, and the `host` field is matched when it is supplied (otherwise `hostaddr` is used). Do not use an ambient password file. Pass `--no-password` so an unavailable secret fails rather than prompting in a noninteractive UI.

Sources: [libpq connection parameters](https://www.postgresql.org/docs/current/libpq-connect.html), [password-file matching and permissions](https://www.postgresql.org/docs/current/libpq-pgpass.html).

## PostgreSQL dump and restore contract

Create the archive with a current-enough `pg_dump`:

```text
pg_dump --dbname=<non-secret conninfo> --no-password --format=custom --file=<archive path>
```

Custom format is compressed and is intended for `pg_restore`. Before starting, run `pg_dump --version` and compare its major version with the selected server version. PostgreSQL documents that `pg_dump` can dump an older server but cannot dump a server newer than its own major version; its output is not guaranteed to load into an older server. Refuse the known-invalid first case and show the executable/server version evidence in the job detail.

Normal restore, with failures made terminal, is:

```text
pg_restore --dbname=<non-secret conninfo> --no-password --exit-on-error <archive path>
```

`--single-transaction` is a useful explicit option when the requested restore is compatible with it; it implies `--exit-on-error`, and PostgreSQL says it cannot be combined with parallel jobs for custom or directory archives. Do not silently add `--clean` or `--create`: they drop objects / databases. Require a separate destructive confirmation after DBX shows the target and the reviewed restore plan.

Treat an imported archive as executable input. PostgreSQL specifically warns that a non-plain dump can cause arbitrary code execution through restore and recommends inspecting it with `pg_restore --file` before restore. DBX should offer `pg_restore --list` and a review-SQL export as a preflight, then record the archive hash and tool version.

Sources: [pg_dump formats and version limits](https://www.postgresql.org/docs/current/app-pgdump.html), [pg_restore errors, transactions, clean/create, and archive safety](https://www.postgresql.org/docs/current/app-pgrestore.html).

## MySQL credentials and option-file isolation

MySQL option files are plaintext. Use a DBX-created, mode-`0600` temporary file and supply it as the **first** client argument:

```text
mysqldump --defaults-file=<private option file> --no-login-paths ...
mysql     --defaults-file=<private option file> --no-login-paths ...
```

Place only credentials needed by both tools in a `[client]` group, encoded with MySQL option-file quoting and escapes:

```ini
[client]
user=<encoded user>
password=<encoded password>
```

`--defaults-extra-file` is unsuitable for deterministic secret isolation: it augments the standard files, which can later override it. `--defaults-file` limits normal option-file processing to the named file, while `--no-login-paths` excludes the separately processed encrypted `.mylogin.cnf` login paths. The wrapper must keep this option first, use a process API, scrub its own diagnostic fields, and delete the file after the child exits. MySQL's option-file syntax does not use `--`, allows quoted values, and defines backslash escapes; implement an encoder rather than interpolating raw credentials.

Sources: [option-file command options and ordering](https://dev.mysql.com/doc/refman/8.4/en/option-file-options.html), [option precedence](https://dev.mysql.com/doc/refman/8.4/en/program-options.html#option-precedence), [option-file syntax](https://dev.mysql.com/doc/refman/8.4/en/option-files.html#option-file-syntax).

## MySQL over SSH and TLS identity

MySQL documents Unix socket transport as local-only and selects it explicitly with `--protocol=SOCKET --socket=<path>`. It also says SSL connection options apply to TCP/IP and Unix socket connections. `--ssl-mode=VERIFY_IDENTITY` verifies the CA and the host name used by the client against the certificate; it requires `--ssl-ca` or `--ssl-capath`.

OpenSSH documents local forwarding to a Unix-domain socket. The candidate command contract is therefore:

```text
mysqldump --defaults-file=<private cnf> --no-login-paths
  --protocol=SOCKET --socket=<owned local socket>
  --host=<original DNS> --ssl-mode=VERIFY_IDENTITY --ssl-ca=<CA path>
  <database>
```

For restore, run the same authenticated connection options with `mysql <database>` and attach the selected dump file as the process's stdin handle; do not model shell redirection. This keeps the password out of argv and preserves cancellation / exit-status handling.

The manual documents each option but does not explicitly demonstrate `SOCKET` transport with a distinct remote DNS `--host` under `VERIFY_IDENTITY`. Therefore DBX must mark this approach **validation required**, not advertised as proven, until an integration test proves: the TLS peer certificate contains the original DNS SAN but not `127.0.0.1`; the SSH socket connection succeeds for that original DNS; and changing the `--host` value fails. If that test fails on a shipped client, use the platform's supported loopback TCP forwarding path and validate the same hostname behavior before enabling identity mode.

Sources: [MySQL transport protocols](https://dev.mysql.com/doc/refman/8.4/en/transport-protocols.html), [protocol option](https://dev.mysql.com/doc/refman/8.4/en/connection-options.html#option_general_protocol), [TLS modes and identity verification](https://dev.mysql.com/doc/refman/8.4/en/connection-options.html#option_general_ssl-mode), [OpenSSH local forwarding](https://man.openbsd.org/ssh#L).

## MySQL dump / restore safety contract

Use `mysqldump` to write a SQL file through an owned stdout file handle, and `mysql` to read that file through an owned stdin file handle. Resolve and display the exact executable version before a job; do not claim cross-version restore compatibility without a tested version matrix for the shipped client and target server.

MySQL restores execute SQL supplied by the dump. The UI must identify the source file and target database, require the user to choose object/data scope, and present a destructive confirmation before starting a restore. Preserve the tool's stderr and exit code in the job record without exposing option-file contents. A cancellation is not a rollback promise; MySQL DDL can commit implicitly, so the result needs reconnect-and-inspect recovery guidance.

Sources: [mysqldump option syntax](https://dev.mysql.com/doc/refman/8.4/en/mysqldump.html#mysqldump-option-syntax), [mysql client invocation](https://dev.mysql.com/doc/refman/8.4/en/mysql.html).

## TablePro import: exact current SSL mode strings

For the TablePro `.tablepro` plaintext export envelope, `sslConfig.mode` is the `TableProPluginKit.SSLMode` raw value, not a libpq keyword. The current exact strings are `Disabled`, `Preferred`, `Required`, `Verify CA`, and `Verify Identity`. DBX should accept these strings case-sensitively for interoperable import, normalize them to DBX's internal TLS model, and reject unknown strings with an import diagnostic. In particular, the TablePro import fixture should use `"Verify Identity"`; `verify-full` is the PostgreSQL driver mapping for that mode, not the export value.

Sources: [TablePro SSLMode raw values](https://raw.githubusercontent.com/TableProApp/TablePro/main/Plugins/TableProPluginKit/SSLConfiguration.swift), [TablePro export mapping](https://raw.githubusercontent.com/TableProApp/TablePro/main/TablePro/Core/Services/Export/ConnectionExportService.swift), [TablePro PostgreSQL mapping](https://raw.githubusercontent.com/TableProApp/TablePro/main/Plugins/PostgreSQLDriverPlugin/LibPQSSLMapping.swift).

## Implementation acceptance evidence

1. A PostgreSQL server certificate with only `db.example.test` SAN succeeds directly and through an SSH TCP forward using `host=db.example.test hostaddr=127.0.0.1`; `host=127.0.0.1` and a wrong DNS name fail under `verify-full`.
2. The Postgres wrapper has a test with passwords containing `:` and `\\`, confirms the generated mode-`0600` pgpass file is used, and confirms child argv / UI logs contain no password.
3. The MySQL shipped client passes or fails the documented UDS + `VERIFY_IDENTITY` probe deterministically. Enable the feature only on a pass; retain the exact tool, OS, server, and certificate evidence.
4. Both dump formats have a cancel test, a nonzero-exit test, and a restore preflight / destructive-confirmation test. PostgreSQL archive review is exercised before restore.
5. TablePro import accepts the five documented raw SSL strings and maps `Verify Identity` to PostgreSQL verify-full semantics only for PostgreSQL targets.
