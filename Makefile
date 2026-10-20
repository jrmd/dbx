SHELL := /bin/bash

MACOS_APP_SCRIPT := scripts/build-macos-app.sh
LINUX_APP_SCRIPT := scripts/build-linux-app.sh
MACOS_APP := target/macos/DBX.app
INSTALL_DIR ?= /Applications

# Routine local targets use the faster `release-local` Cargo profile. Shipping
# targets (linux-build, linux-package, linux-appimage) stay on `release`.
# Override either with DBX_CARGO_PROFILE=<profile>, e.g. `make run DBX_CARGO_PROFILE=dev`.
# Captured before the local default is applied so packaging only follows an
# explicit override.
DBX_PACKAGE_PROFILE := $(or $(DBX_CARGO_PROFILE),release)
DBX_CARGO_PROFILE ?= release-local
export DBX_CARGO_PROFILE

linux-build linux-package linux-appimage: DBX_CARGO_PROFILE := $(DBX_PACKAGE_PROFILE)

.PHONY: build run install macos-build macos-run linux-build linux-run linux-package linux-appimage cargo-build cargo-run

# Local Mac workflow. The helper packages the Rust binary as DBX.app, signs it
# with an installed Developer ID Application or Apple Development certificate
# (found automatically; it fails if none or several match), and launches it.
# DBX_SIGNING_MODE=local opts in to a self-signed identity; DBX_SIGNING_NAME=-
# opts in to ad-hoc signing. Uses $(DBX_CARGO_PROFILE) (release-local by default).
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

linux-appimage:
	bash $(LINUX_APP_SCRIPT) appimage

linux-run:
	bash $(LINUX_APP_SCRIPT) run

# Keep the raw Cargo entry points available for non-Mac development and tests.
# For a production binary run `cargo build --locked --release --package dbx-ui`.
cargo-build:
	cargo build --locked --profile $(DBX_CARGO_PROFILE) --package dbx-ui

cargo-run:
	cargo run --locked --profile $(DBX_CARGO_PROFILE) --package dbx-ui
