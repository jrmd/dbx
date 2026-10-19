# Contributing to DBX

Bug reports, fixes, documentation and database integration tests are welcome.
Use [Discussions](https://github.com/jrmd/dbx/discussions) for questions and feature
ideas, and [issues](https://github.com/jrmd/dbx/issues) for reproducible bugs.

## Develop

Follow the platform prerequisites in [README](README.md#build-from-source).
Use Rust 1.97 or newer and the checked-in Cargo.lock:

```sh
cargo run --locked --release -p dbx-ui
cargo fmt --all -- --check
cargo test --locked --profile ci --workspace
cargo clippy --locked --profile ci --workspace --all-targets -- -D warnings
python3 scripts/test-macos-release.py
python3 scripts/test-release-notes.py
```

Start with focused tests while iterating. The full build includes native database
and GPUI dependencies and can take substantial time on a fresh checkout.
Add a test when it protects a meaningful behavior or reproduces a bug; do not
test every helper, getter, label or reversible cosmetic change. Extend an
existing scenario when it already exercises the behavior, and use table-driven
cases for related input variations. Keep separate tests for distinct failure
modes so failures remain easy to diagnose. Preserve coverage for data loss,
transaction boundaries, cancellation, credentials and protected writes.
Run `bash scripts/test-integration.sh` for the disposable Docker database suite;
read the script and [desktop QA guide](docs/desktop-qa.md) before running platform
or live-provider checks. Never use production databases for tests.

## CI feedback

Pull requests run one Linux job: formatting, Clippy, the workspace tests and
release-script checks. The `ci` profile skips release optimisation and debug
symbols, with a separate dependency cache. It keeps debug assertions and
overflow checks enabled. It does not build a shipping binary or run benchmarks.

Full Linux/macOS release builds run nightly and on manual dispatch. Live
database/SSH/TLS checks run after relevant pushes to `main`, nightly and on
manual dispatch. Both can be dispatched against a feature branch when changing
platform or connector behavior. Tag releases still run their full validation
before packaging and publishing. Cold PR caches will still take longer than
warm runs; the individual CI steps expose compile and test timings separately.

## Make a change

- Keep a pull request focused on one problem and describe the resulting behavior.
- Add regression coverage for behavior that could silently corrupt or lose data.
- Preserve staged SQL review, protected profiles, cancellation, conflict checks,
  and credential redaction. AI drafts must never execute SQL themselves.
- Describe connector limitations honestly; read [capabilities](docs/capability-matrix.md)
  and [workbench safety](docs/workbench-safety.md) when changing database behavior.
- Include screenshots for visible UI changes and note what you tested on each OS.
- Add a user-facing changelog entry for a shipped behavior change.

Do not include database contents, connection passwords, API keys, vault files,
or customer information in reports or fixtures. For a security vulnerability,
use GitHub's private vulnerability reporting if available rather than posting
credentials or exploit details in a public issue.

Contributions are licensed under the repository's [MIT license](LICENSE).
Vendored dependencies retain their own licenses.
