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
cargo test --locked --release --workspace
cargo clippy --locked --release --workspace --all-targets -- -D warnings
python3 scripts/test-macos-release.py
python3 scripts/test-release-notes.py
```

Start with focused tests while iterating. The full build includes native database
and GPUI dependencies and can take substantial time on a fresh checkout.
Run `bash scripts/test-integration.sh` for the disposable Docker database suite;
read the script and [desktop QA guide](docs/desktop-qa.md) before running platform
or live-provider checks. Never use production databases for tests.

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
