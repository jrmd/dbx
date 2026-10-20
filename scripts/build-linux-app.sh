#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

APP_DIR="${DBX_LINUX_DIR:-$ROOT_DIR/target/linux/DBX}"
VERSION="$(python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["workspace"]["package"]["version"])' "$ROOT_DIR/Cargo.toml")"
ARCHIVE_PATH="${DBX_LINUX_ARCHIVE:-$ROOT_DIR/target/linux/DBX-$VERSION-linux-$(uname -m).tar.gz}"
CARGO_BIN="${CARGO:-cargo}"

# Cargo profile. Direct invocation builds the shipping `release` profile; `make`
# sets DBX_CARGO_PROFILE=release-local for routine local builds.
CARGO_PROFILE="${DBX_CARGO_PROFILE:-release}"
[[ "$CARGO_PROFILE" =~ ^[A-Za-z0-9_-]+$ ]] || { printf 'DBX: invalid DBX_CARGO_PROFILE: %s\n' "$CARGO_PROFILE" >&2; exit 1; }
case "$CARGO_PROFILE" in
release) CARGO_PROFILE_ARGS=(--release) ;;
*) CARGO_PROFILE_ARGS=(--profile "$CARGO_PROFILE") ;;
esac
# Cargo writes dev and test to debug/ and bench to release/; custom profiles use their own name.
case "$CARGO_PROFILE" in
dev | test) CARGO_PROFILE_DIR=debug ;;
bench) CARGO_PROFILE_DIR=release ;;
*) CARGO_PROFILE_DIR="$CARGO_PROFILE" ;;
esac
# CARGO_TARGET_DIR is relative to the directory cargo runs in, which is ROOT_DIR.
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
[[ "$TARGET_DIR" == /* ]] || TARGET_DIR="$ROOT_DIR/$TARGET_DIR"
BINARY_PATH="$TARGET_DIR/$CARGO_PROFILE_DIR/dbx"
ARCH="$(uname -m)"
APPDIR="${DBX_APPDIR:-$ROOT_DIR/target/linux/DBX.AppDir}"
APPIMAGE_PATH="${DBX_APPIMAGE:-$ROOT_DIR/target/linux/DBX-$VERSION-linux-$ARCH.AppImage}"
APPIMAGETOOL="${APPIMAGETOOL:-$ROOT_DIR/target/linux/tools/appimagetool-$ARCH.AppImage}"
readonly APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"

readonly DESKTOP_FILE="$ROOT_DIR/packaging/linux/dbx.desktop"
readonly LOGO_SVG="$ROOT_DIR/logo.svg"
readonly LOGO_PNG="$ROOT_DIR/logo.png"

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

build_binary() {
	log "building $CARGO_PROFILE binary"
	(cd "$ROOT_DIR" && "$CARGO_BIN" build --locked "${CARGO_PROFILE_ARGS[@]}" --package dbx-ui)
	[[ -x "$BINARY_PATH" ]] || die "$CARGO_PROFILE binary was not produced: $BINARY_PATH"
}

build_package() {
	build_binary

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
	(cd "$(dirname -- "$ARCHIVE_PATH")" && sha256sum "$(basename -- "$ARCHIVE_PATH")" > "$(basename -- "$ARCHIVE_PATH").sha256")
	log "ready: $APP_DIR"
	log "archive: $ARCHIVE_PATH"
}

fetch_appimagetool() {
	[[ -x "$APPIMAGETOOL" ]] && return
	require_command curl
	log "downloading appimagetool"
	mkdir -p "$(dirname -- "$APPIMAGETOOL")"
	curl -fsSL --retry 3 -o "$APPIMAGETOOL.part" "$APPIMAGETOOL_URL"
	chmod +x "$APPIMAGETOOL.part"
	mv "$APPIMAGETOOL.part" "$APPIMAGETOOL"
}

build_appimage() {
	log "assembling AppDir"
	rm -rf "$APPDIR"
	mkdir -p "$APPDIR"
	cp -a "$APP_DIR/usr" "$APPDIR/"
	cp "$DESKTOP_FILE" "$APPDIR/dbx.desktop"
	cp "$LOGO_SVG" "$APPDIR/dbx.svg"
	cp "$LOGO_PNG" "$APPDIR/.DirIcon"

	# No libraries are bundled. xkbcommon must match the host's X11 compose
	# tables, and libxcb, Wayland, Vulkan, and fonts come from the desktop.
	cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/dbx" "$@"
APPRUN
	chmod +x "$APPDIR/AppRun"

	fetch_appimagetool
	log "building AppImage"
	rm -f "$APPIMAGE_PATH"
	# Extract-and-run avoids needing FUSE on CI runners.
	APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$APPIMAGETOOL" --no-appstream "$APPDIR" "$APPIMAGE_PATH"
	(cd "$(dirname -- "$APPIMAGE_PATH")" && sha256sum "$(basename -- "$APPIMAGE_PATH")" > "$(basename -- "$APPIMAGE_PATH").sha256")
	log "AppImage: $APPIMAGE_PATH"
}

main() {
	local action="${1:-build}"
	check_inputs

	case "$action" in
	build|package)
		build_package
		;;
	appimage)
		build_package
		build_appimage
		;;
	run)
		build_binary
		exec "$BINARY_PATH"
		;;
	*)
		die "usage: $0 [build|package|appimage|run]"
		;;
	esac
}

main "$@"
