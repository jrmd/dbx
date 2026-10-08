# Transport and profile implementation research

_Research date: 8 October 2026. Evidence is local locked SQLx source plus first-party documentation/source. No tunnel or imported file was exercised._

## Decision

Enable identity-verifying TLS over SSH for PostgreSQL and MySQL on Unix by using an OpenSSH local Unix-domain-socket forward. Keep the original database DNS name in SQLx for certificate/SNI verification; tell SQLx to dial the owned local socket. Do not rewrite the database URL host to 127.0.0.1 and do not weaken verify-full or verify_identity.

Keep the current rejection on non-Unix platforms pending a separately verified transport that can split dial address from TLS name. Do not use a global hosts-file override or a private SQLx fork.

## Exact SQLx 0.8.6 evidence

DBX locks sqlx 0.8.6 at [Cargo.lock](../Cargo.lock#L7890). The relevant local dependency source is the installed, checksum-locked crate source.

| Driver | Dial path | TLS identity path | Consequence |
| --- | --- | --- | --- |
| PostgreSQL | [stream.rs](/home/jrmd/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sqlx-postgres-0.8.6/src/connection/stream.rs#L44) selects `connect_uds` when `fetch_socket` returns a path. | [tls.rs](/home/jrmd/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sqlx-postgres-0.8.6/src/connection/tls.rs#L48) gives `options.host` to TLS and verifies hostnames in VerifyFull. | Keep the original host and give PgConnectOptions a local socket directory. |
| MySQL | [establish.rs](/home/jrmd/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sqlx-mysql-0.8.6/src/connection/establish.rs#L16) selects `connect_uds` when `options.socket` is set. | [tls.rs](/home/jrmd/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sqlx-mysql-0.8.6/src/connection/tls.rs#L59) gives `options.host` to TLS and verifies hostnames in VerifyIdentity. | Keep the original host and give MySqlConnectOptions the exact local socket file. |

DBX already maps ConnectionConfig.socket into both public SQLx options builders in [sqlx_engine.rs](../crates/dbx-core/src/sqlx_engine.rs#L29). Its present SSH preparation changes the URL host to loopback at [transport.rs](../crates/dbx-core/src/transport.rs#L171), after correctly rejecting identity-verifying modes at [transport.rs](../crates/dbx-core/src/transport.rs#L112).

OpenSSH officially documents -L local_socket:host:hostport, forwarding a local Unix socket to a remote TCP endpoint. [OpenSSH ssh(1)](https://man.openbsd.org/ssh#L)

## Safe implementation shape

1. Preserve the entered URL host and remote database port as the TLS identity and remote forwarding destination.
2. Create a private short-lived tunnel directory that remains owned by Tunnel supervision. Retain existing strict host-key checking, ExitOnForwardFailure, reconnect behavior and kill-on-drop.
3. For PostgreSQL/CockroachDB allocate a local port identifier, create a forward at <dir>/.s.PGSQL.<local-port> to original-host:remote-port, set SQLx socket to <dir>, and set URL port to the local identifier. Do not alter URL host.
4. For MySQL create a forward at <dir>/mysql.sock to original-host:remote-port and set the SQLx socket to that exact path. Do not alter URL host.
5. Remove stale sockets only inside the owned directory before SSH restarts; fail early on Unix-socket path-length overflow. Keep loopback TCP forwarding for Redis and existing unsupported families.

Required release proof: PostgreSQL and MySQL certs must contain the original DNS SAN but exclude 127.0.0.1; direct and SSH+UDS verify-full/verify_identity must succeed, and a wrong original host must fail. Repeat through a jump host and after SSH child restart. Check cleanup, private permissions and that reconnect never replays a statement.

## Profile import boundary

### TablePlus

Official TablePlus docs define .tableplusconnection export/import: the export can include database/server passwords if the user chooses and protects the file with a password; group names/icons are included, while SSH and SSL private keys are excluded. Folder sync keeps passwords in the local Keychain. [TablePlus connections](https://tableplus.com/docs/gui-tools/manage-connections)

TablePlus does not publish the container schema, encryption, KDF or versioning. Its own issue leaves the format proprietary/undocumented. [Issue 2018](https://github.com/TablePlus/TablePlus/issues/2018)

Recommendation: treat .tableplusconnection as opaque. Do not parse, decrypt or guess it. Support explicit TablePlus URL migration as plaintext secret input, and a separately labelled macOS local-profile migration only with user action. TablePro's official importer offers implementation evidence for that migration (Connections.plist, groups plist and Keychain-mode handling), but it is not TablePlus format documentation. [TablePro importer](https://github.com/TableProApp/TablePro/blob/main/TablePro/Core/Services/Export/ForeignApp/TablePlusImporter.swift)

### TablePro

TablePro's official source defines .tablepro. Plain exports are a versioned JSON envelope; its export service can include Keychain credentials. [Envelope](https://github.com/TableProApp/TablePro/blob/main/Packages/TableProCore/Sources/TableProImport/ConnectionExportEnvelope.swift) [export service](https://github.com/TableProApp/TablePro/blob/main/TablePro/Core/Services/Export/ConnectionExportService.swift)

Encrypted files have ASCII TPRO, version byte 1, 32-byte salt, 12-byte AES-GCM nonce, ciphertext and 16-byte tag; the published implementation derives a 32-byte key using PBKDF2-HMAC-SHA256 with 600,000 iterations. [Crypto source](https://github.com/TableProApp/TablePro/blob/main/Packages/TableProCore/Sources/TableProImport/ConnectionExportCrypto.swift#L22-L124)

Recommendation: bounded-read the file, detect TPRO before JSON parse, reject unknown versions/invalid lengths, prompt only then for a passphrase, authenticate before decoding, and put any credentials directly in DBX vault storage. For plaintext JSON, warn before import. Validate the known versioned envelope, present a field-by-field review, import as disabled drafts, and never execute imported startup/tunnel commands. Test valid plaintext/encrypted exports, credential-bearing files, wrong passphrases/tampered tags, malformed lengths, missing TLS/key paths and repeated imports.

### Plaintext envelope schema and DBX mapping

The current TablePro decoder accepts ISO-8601 JSON, has `currentFormatVersion = 1`, and rejects a later version. It encodes sorted, pretty-printed JSON. [Decoder source](https://raw.githubusercontent.com/TableProApp/TablePro/main/Packages/TableProCore/Sources/TableProImport/ConnectionImportTypes.swift#L205-L253)

The required top-level fields are `formatVersion` (integer), `exportedAt`
(ISO-8601 timestamp), `appVersion` (string), and `connections` (array).
Optional top-level fields are `groups`, `tags`, `credentials` (a map keyed by
the connection's zero-based array index as a string), and
`credentialProfiles`. [Envelope source](https://raw.githubusercontent.com/TableProApp/TablePro/main/Packages/TableProCore/Sources/TableProImport/ConnectionExportEnvelope.swift#L43-L70)

Each connection requires `name`, `host`, `port` (integer), `database`,
`username`, and `type`. The optional fields are `sshConfig`, `sslConfig`,
`color`, `tagName`, `tagNames`, `groupName`, `sshProfileId`,
`sshProfileName`, `credentialProfileName`, `safeModeLevel`, `aiPolicy`,
`connectTimeoutSeconds`, `queryTimeoutSeconds`, `additionalFields`,
`redisDatabase`, `startupCommands`, `localOnly`, and `tunnelCommand`.
[Connection definition](https://raw.githubusercontent.com/TableProApp/TablePro/main/Packages/TableProCore/Sources/TableProImport/ConnectionExportEnvelope.swift#L91-L197)

`sshConfig` has `enabled`, `host`, optional integer `port`, `username`,
`authMethod`, `privateKeyPath`, `agentSocketPath`, optional `jumpHosts`, and
optional TOTP/remote-file fields. Each jump host has `host`, optional `port`,
`username`, `authMethod`, and `privateKeyPath`. `sslConfig` has `mode` and
optional CA, client-certificate, and client-key paths. Credentials are never
part of the connection object: the optional index-keyed credentials value has
`password`, `sshPassword`, `keyPassphrase`, `sslClientKeyPassphrase`,
`totpSecret`, and `pluginSecureFields`.
[Nested definitions](https://raw.githubusercontent.com/TableProApp/TablePro/main/Packages/TableProCore/Sources/TableProImport/ConnectionExportEnvelope.swift#L319-L445)

DBX can map `host`, `port`, `database`, and `username` directly into its
connection fields; map `sshConfig` only after validating host/port/user/key
reference/jump hosts; map `sslConfig.mode` and existing certificate paths;
and store credential-map values only in the vault. Whitelist and map `type`
to a DBX `DatabaseKind` case-insensitively, warning and leaving unsupported
entries unselected. Preserve `name`/group/tags as display metadata. Treat
`startupCommands`, `tunnelCommand`, `additionalFields`, AI policy, TOTP, and
unknown type-specific fields as non-executable review-only data.

Here is a minimal interoperable fixture for DBX's plaintext parser. TablePro's
published type constants make the PostgreSQL value exactly `PostgreSQL`.
[Type constants](https://raw.githubusercontent.com/TableProApp/TablePro/main/Packages/TableProCore/Sources/TableProCoreTypes/DatabaseType.swift#L9-L41)

```json
{
  "formatVersion": 1,
  "exportedAt": "2026-10-08T12:00:00Z",
  "appVersion": "fixture",
  "connections": [
    {
      "name": "Production PostgreSQL",
      "host": "db.example.test",
      "port": 5432,
      "database": "app",
      "username": "app_reader",
      "type": "PostgreSQL",
      "sshConfig": {
        "enabled": true,
        "host": "bastion.example.test",
        "port": 22,
        "username": "deploy",
        "authMethod": "Private Key",
        "privateKeyPath": "~/.ssh/id_ed25519",
        "agentSocketPath": "",
        "jumpHosts": null,
        "totpMode": null,
        "totpAlgorithm": null,
        "totpDigits": null,
        "totpPeriod": null,
        "remoteFilePath": null,
        "remoteFileAccess": null
      },
      "sslConfig": {
        "mode": "Verify Identity",
        "caCertificatePath": "~/.config/dbx/ca.pem",
        "clientCertificatePath": null,
        "clientKeyPath": null
      },
      "safeModeLevel": "Read-Only",
      "connectTimeoutSeconds": 10,
      "queryTimeoutSeconds": 30,
      "additionalFields": null,
      "redisDatabase": null,
      "startupCommands": null,
      "localOnly": true,
      "tunnelCommand": null
    }
  ]
}
```

The fixture intentionally omits `credentials`. Add a separate, encrypted
fixture with a `"credentials": { "0": { "password": "fixture-only" } }` map to prove that DBX routes it to the vault and never serializes it with profiles.

## Evidence boundary

TablePlus behavior is vendor documentation, while its unexported container remains unknown. TablePro format details are first-party open-source implementation evidence. The TLS recommendation rests on exact local SQLx 0.8.6 behavior and OpenSSH documented forwarding syntax, not a guessed SQLx API.
