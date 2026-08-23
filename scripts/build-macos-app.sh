#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

APP_DIR="${DBX_APP_DIR:-$ROOT_DIR/target/macos/DBX.app}"
BUNDLE_ID="${DBX_BUNDLE_ID:-dev.jrmd.dbx}"
SIGNING_NAME="${DBX_SIGNING_NAME:-DBX Local Development}"
KEYCHAIN="${DBX_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
CARGO_BIN="${CARGO:-cargo}"

readonly BINARY_PATH="$ROOT_DIR/target/release/dbx"
readonly INFO_PLIST="$ROOT_DIR/packaging/macos/Info.plist"
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

check_host() {
	[[ "$(uname -s)" == "Darwin" ]] || die "make build/run is for macOS; use make cargo-build or make cargo-run on this host"
	require_command "$CARGO_BIN"
	require_command codesign
	require_command security
	require_command open
	require_command iconutil
	require_command sips

	[[ -f "$INFO_PLIST" ]] || die "missing bundle metadata: $INFO_PLIST"
	[[ -f "$LOGO_SVG" ]] || die "missing vector logo: $LOGO_SVG"
	[[ -f "$LOGO_PNG" ]] || die "missing raster logo fallback: $LOGO_PNG"
	[[ -f "$KEYCHAIN" ]] || die "login keychain not found: $KEYCHAIN"
}

identity_is_available() {
	security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null \
		| grep -Fq "\"$SIGNING_NAME\""
}

print_manual_identity_instructions() {
	cat >&2 <<EOF
DBX: Could not find a usable '$SIGNING_NAME' identity.

Create it once in Keychain Access:
  1. Open Keychain Access.
  2. Keychain Access > Certificate Assistant > Create a Certificate.
  3. Name: $SIGNING_NAME
  4. Identity Type: Self Signed Root
  5. Certificate Type: Code Signing
  6. Save it in the login keychain.

Then run make run again. This identity is only for this Mac's local builds;
it is not an Apple Developer or distribution certificate.
EOF
}

create_local_identity() {
	if ! command -v openssl >/dev/null 2>&1; then
		print_manual_identity_instructions
		die "openssl is unavailable, so the local identity must be created in Keychain Access"
	fi

	local temp_dir key_file cert_file bundle_file bundle_password
	temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/dbx-signing.XXXXXX")"
	key_file="$temp_dir/dbx-local.key.pem"
	cert_file="$temp_dir/dbx-local.cert.pem"
	bundle_file="$temp_dir/dbx-local.identity.p12"
	bundle_password="$(openssl rand -hex 32)"
	[[ -n "$bundle_password" ]] || die "could not generate a PKCS#12 password"

	cleanup_temp_identity() {
		rm -rf -- "$temp_dir"
	}
	trap cleanup_temp_identity EXIT

	log "creating local code-signing identity '$SIGNING_NAME'"
	openssl req -new -x509 -newkey rsa:2048 -nodes -sha256 \
		-days 3650 \
		-subj "/CN=$SIGNING_NAME" \
		-addext "basicConstraints=critical,CA:false" \
		-addext "keyUsage=critical,digitalSignature" \
		-addext "extendedKeyUsage=critical,1.3.6.1.5.5.7.3.3" \
		-keyout "$key_file" \
		-out "$cert_file" \
		>/dev/null 2>&1 \
		|| {
			print_manual_identity_instructions
			die "could not generate the local code-signing certificate"
		}

	openssl pkcs12 -export \
		-name "$SIGNING_NAME" \
		-inkey "$key_file" \
		-in "$cert_file" \
		-out "$bundle_file" \
		-keypbe PBE-SHA1-3DES \
		-certpbe PBE-SHA1-3DES \
		-macalg sha1 \
		-passout "pass:$bundle_password" \
		>/dev/null 2>&1 \
		|| die "could not package the local code-signing identity"

	security import "$bundle_file" \
		-k "$KEYCHAIN" \
		-P "$bundle_password" \
		-T /usr/bin/codesign \
		>/dev/null \
		|| {
			print_manual_identity_instructions
			die "could not import the local identity into the login keychain"
		}

	if ! identity_is_available; then
		print_manual_identity_instructions
		die "the imported identity was not available to codesign"
	fi

	cleanup_temp_identity
	trap - EXIT
	log "local identity is ready"
}

ensure_signing_identity() {
	if [[ "$SIGNING_NAME" == "-" ]]; then
		log "using ad-hoc signing; Keychain permissions may not survive rebuilds"
		return
	fi

	identity_is_available || create_local_identity
}

render_icon_png() {
	local output="$1"
	local size="$2"

	if command -v rsvg-convert >/dev/null 2>&1; then
		rsvg-convert -w "$size" -h "$size" "$LOGO_SVG" -o "$output"
	else
		# sips is part of macOS. The supplied PNG is only used to make the
		# Finder icon fallback when librsvg is not installed; the app UI still
		# embeds and renders the vector SVG.
		sips -s format png -z "$size" "$size" "$LOGO_PNG" --out "$output" >/dev/null
	fi
}

build_icon() {
	local resources_dir="$APP_DIR/Contents/Resources"
	local iconset_dir="$resources_dir/DBX.iconset"

	mkdir -p "$iconset_dir"
	rm -f \
		"$iconset_dir/icon_16x16.png" \
		"$iconset_dir/icon_16x16@2x.png" \
		"$iconset_dir/icon_32x32.png" \
		"$iconset_dir/icon_32x32@2x.png" \
		"$iconset_dir/icon_128x128.png" \
		"$iconset_dir/icon_128x128@2x.png" \
		"$iconset_dir/icon_256x256.png" \
		"$iconset_dir/icon_256x256@2x.png" \
		"$iconset_dir/icon_512x512.png" \
		"$iconset_dir/icon_512x512@2x.png"

	render_icon_png "$iconset_dir/icon_16x16.png" 16
	render_icon_png "$iconset_dir/icon_16x16@2x.png" 32
	render_icon_png "$iconset_dir/icon_32x32.png" 32
	render_icon_png "$iconset_dir/icon_32x32@2x.png" 64
	render_icon_png "$iconset_dir/icon_128x128.png" 128
	render_icon_png "$iconset_dir/icon_128x128@2x.png" 256
	render_icon_png "$iconset_dir/icon_256x256.png" 256
	render_icon_png "$iconset_dir/icon_256x256@2x.png" 512
	render_icon_png "$iconset_dir/icon_512x512.png" 512
	render_icon_png "$iconset_dir/icon_512x512@2x.png" 1024

	rm -f "$resources_dir/DBX.icns"
	iconutil -c icns -o "$resources_dir/DBX.icns" "$iconset_dir"
	# The generated iconset is an iconutil input, not a runtime resource.
	rm -f \
		"$iconset_dir/icon_16x16.png" \
		"$iconset_dir/icon_16x16@2x.png" \
		"$iconset_dir/icon_32x32.png" \
		"$iconset_dir/icon_32x32@2x.png" \
		"$iconset_dir/icon_128x128.png" \
		"$iconset_dir/icon_128x128@2x.png" \
		"$iconset_dir/icon_256x256.png" \
		"$iconset_dir/icon_256x256@2x.png" \
		"$iconset_dir/icon_512x512.png" \
		"$iconset_dir/icon_512x512@2x.png"
	rmdir "$iconset_dir" 2>/dev/null || true
}

build_bundle() {
	log "building release binary"
	(cd "$ROOT_DIR" && "$CARGO_BIN" build --release --package dbx-ui)
	[[ -x "$BINARY_PATH" ]] || die "release binary was not produced: $BINARY_PATH"

	mkdir -p "$APP_DIR/Contents/MacOS"
	mkdir -p "$APP_DIR/Contents/Resources"
	cp "$BINARY_PATH" "$APP_DIR/Contents/MacOS/dbx"
	cp "$INFO_PLIST" "$APP_DIR/Contents/Info.plist"
	cp "$LOGO_SVG" "$APP_DIR/Contents/Resources/DBX.svg"
	build_icon
	[[ -s "$APP_DIR/Contents/Resources/DBX.icns" ]] || die "macOS icon was not produced"
}

sign_bundle() {
	local identity="$SIGNING_NAME"
	log "signing $APP_DIR"

	# Sign nested code first. Do not use --deep for signing.
	codesign --force --options runtime \
		--sign "$identity" \
		--identifier "$BUNDLE_ID" \
		"$APP_DIR/Contents/MacOS/dbx"
	codesign --force --options runtime \
		--sign "$identity" \
		--identifier "$BUNDLE_ID" \
		"$APP_DIR"
	codesign --verify --deep --strict --verbose=2 "$APP_DIR"
}

build_app() {
	build_bundle
	ensure_signing_identity
	sign_bundle
	log "ready: $APP_DIR"
}

run_app() {
	build_app
	if [[ "${DBX_FOREGROUND:-0}" == "1" ]]; then
		exec "$APP_DIR/Contents/MacOS/dbx"
	fi
	open "$APP_DIR"
}

main() {
	local action="${1:-run}"
	check_host

	case "$action" in
	build)
		build_app
		;;
	run)
		run_app
		;;
	*)
		die "usage: $0 [build|run]"
		;;
	esac
}

main "$@"
