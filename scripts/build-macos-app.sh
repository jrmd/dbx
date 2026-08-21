#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

APP_DIR="${DBX_APP_DIR:-$ROOT_DIR/target/macos/DBX.app}"
BUNDLE_ID="${DBX_BUNDLE_ID:-dbx.jrmd.app}"
SIGNING_NAME="${DBX_SIGNING_NAME:-DBX Local Development}"
KEYCHAIN="${DBX_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
CARGO_BIN="${CARGO:-cargo}"

readonly BINARY_PATH="$ROOT_DIR/target/release/dbx"
readonly INFO_PLIST="$ROOT_DIR/packaging/macos/Info.plist"

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

	[[ -f "$INFO_PLIST" ]] || die "missing bundle metadata: $INFO_PLIST"
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

	local temp_dir key_file cert_file bundle_file
	temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/dbx-signing.XXXXXX")"
	key_file="$temp_dir/dbx-local.key.pem"
	cert_file="$temp_dir/dbx-local.cert.pem"
	bundle_file="$temp_dir/dbx-local.identity.p12"

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
		-passout pass: \
		>/dev/null 2>&1 \
		|| die "could not package the local code-signing identity"

	security import "$bundle_file" \
		-k "$KEYCHAIN" \
		-P "" \
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

build_bundle() {
	log "building release binary"
	(cd "$ROOT_DIR" && "$CARGO_BIN" build --release --package dbx-ui)
	[[ -x "$BINARY_PATH" ]] || die "release binary was not produced: $BINARY_PATH"

	mkdir -p "$APP_DIR/Contents/MacOS"
	cp "$BINARY_PATH" "$APP_DIR/Contents/MacOS/dbx"
	cp "$INFO_PLIST" "$APP_DIR/Contents/Info.plist"
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
