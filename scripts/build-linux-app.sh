#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

APP_DIR="${DBX_LINUX_DIR:-$ROOT_DIR/target/linux/DBX}"
ARCHIVE_PATH="${DBX_LINUX_ARCHIVE:-$ROOT_DIR/target/linux/dbx-linux.tar.gz}"
CARGO_BIN="${CARGO:-cargo}"

readonly BINARY_PATH="$ROOT_DIR/target/release/dbx"
readonly DESKTOP_FILE="$ROOT_DIR/packaging/linux/dbx.desktop"
readonly LOGO_SVG="$ROOT_DIR/logo.svg"

log() {
	printf 'DBX: %s\n' "$*"
}

die() {
	printf 'DBX: %s\n' "$*" >&2
	exit 1
}

require_command() {
	command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

check_inputs() {
	require_command "$CARGO_BIN"
	require_command tar
	[[ -f "$DESKTOP_FILE" ]] || die "missing Linux desktop entry: $DESKTOP_FILE"
	[[ -f "$LOGO_SVG" ]] || die "missing vector logo: $LOGO_SVG"
}

build_package() {
	log "building release binary"
	(cd "$ROOT_DIR" && "$CARGO_BIN" build --release --package dbx-ui)
	[[ -x "$BINARY_PATH" ]] || die "release binary was not produced: $BINARY_PATH"

	log "staging Linux application with desktop metadata and SVG icon"
	mkdir -p \
		"$APP_DIR/usr/bin" \
		"$APP_DIR/usr/share/applications" \
		"$APP_DIR/usr/share/icons/hicolor/scalable/apps"
	cp "$BINARY_PATH" "$APP_DIR/usr/bin/dbx"
	cp "$DESKTOP_FILE" "$APP_DIR/usr/share/applications/dbx.desktop"
	cp "$LOGO_SVG" "$APP_DIR/usr/share/icons/hicolor/scalable/apps/dbx.svg"

	mkdir -p "$(dirname -- "$ARCHIVE_PATH")"
	tar -C "$APP_DIR" -czf "$ARCHIVE_PATH" .
	log "ready: $APP_DIR"
	log "archive: $ARCHIVE_PATH"
}

main() {
	local action="${1:-build}"
	check_inputs

	case "$action" in
	build|package)
		build_package
		;;
	run)
		build_package
		exec "$BINARY_PATH"
		;;
	*)
		die "usage: $0 [build|package|run]"
		;;
	esac
}

main "$@"
