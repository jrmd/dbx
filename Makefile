SHELL := /bin/bash

MACOS_APP_SCRIPT := scripts/build-macos-app.sh

.PHONY: build run macos-build macos-run cargo-build cargo-run

# Local Mac workflow. The helper creates a stable, self-signed development
# identity once, packages the Rust binary as DBX.app, signs it, and launches it.
build:
	bash $(MACOS_APP_SCRIPT) build

run:
	bash $(MACOS_APP_SCRIPT) run

macos-build: build

macos-run: run

# Keep the raw Cargo entry points available for non-Mac development and tests.
cargo-build:
	cargo build --release --package dbx-ui

cargo-run:
	cargo run --release --package dbx-ui
