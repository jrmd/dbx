SHELL := /bin/bash

MACOS_APP_SCRIPT := scripts/build-macos-app.sh
LINUX_APP_SCRIPT := scripts/build-linux-app.sh
MACOS_APP := target/macos/DBX.app
INSTALL_DIR ?= /Applications

.PHONY: build run install macos-build macos-run linux-build linux-run linux-package cargo-build cargo-run

# Local Mac workflow. The helper creates a stable, self-signed development
# identity once, packages the Rust binary as DBX.app, signs it, and launches it.
build:
	bash $(MACOS_APP_SCRIPT) build

run:
	bash $(MACOS_APP_SCRIPT) run

# Build the signed bundle and replace the installed copy. Quits a running
# DBX first so the old bundle is not swapped out from under it.
install: build
	chmod +x $(MACOS_APP)/Contents/MacOS/dbx
	-osascript -e 'quit app id "dev.jrmd.dbx"' 2>/dev/null
	rm -rf "$(INSTALL_DIR)/DBX.app"
	cp -R $(MACOS_APP) "$(INSTALL_DIR)/DBX.app"
	@echo "DBX: installed $(INSTALL_DIR)/DBX.app"

macos-build: build

macos-run: run

linux-build:
	bash $(LINUX_APP_SCRIPT) build

linux-package: linux-build

linux-run:
	bash $(LINUX_APP_SCRIPT) run

# Keep the raw Cargo entry points available for non-Mac development and tests.
cargo-build:
	cargo build --release --package dbx-ui

cargo-run:
	cargo run --release --package dbx-ui
