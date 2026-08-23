#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BUILD_SCRIPT="$SCRIPT_DIR/build-macos-app.sh"

fail() {
	printf 'PKCS#12 bootstrap check: %s\n' "$*" >&2
	exit 1
}

[[ -f "$BUILD_SCRIPT" ]] || fail "missing build script: $BUILD_SCRIPT"

# macOS Security rejects the empty-password PKCS#12 OpenSSL 3 produces. Keep
# this source-level check runnable on Linux, where the Keychain command itself
# is unavailable.
if grep -F -- '-passout pass:' "$BUILD_SCRIPT" >/dev/null || grep -F -- '-P ""' "$BUILD_SCRIPT" >/dev/null; then
	fail "empty PKCS#12 password would make Keychain import fail"
fi

grep -F -- 'openssl rand -hex 32' "$BUILD_SCRIPT" >/dev/null \
	|| fail "PKCS#12 password must be generated without printing it"
grep -F -- '-passout "pass:$bundle_password"' "$BUILD_SCRIPT" >/dev/null \
	|| fail "PKCS#12 export must use the generated password"
grep -F -- '-P "$bundle_password"' "$BUILD_SCRIPT" >/dev/null \
	|| fail "Keychain import must use the generated password"
grep -F -- '-keypbe PBE-SHA1-3DES' "$BUILD_SCRIPT" >/dev/null \
	|| fail "PKCS#12 key encryption must use Keychain-compatible PBE"
grep -F -- '-certpbe PBE-SHA1-3DES' "$BUILD_SCRIPT" >/dev/null \
	|| fail "PKCS#12 certificate encryption must use Keychain-compatible PBE"
grep -F -- '-macalg sha1' "$BUILD_SCRIPT" >/dev/null \
	|| fail "PKCS#12 MAC must use Keychain-compatible SHA-1"

command -v openssl >/dev/null 2>&1 || fail "openssl is required for the compatibility check"
temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/dbx-pkcs12-check.XXXXXX")"
trap 'rm -rf -- "$temp_dir"' EXIT
password="$(openssl rand -hex 32)"
openssl req -new -x509 -newkey rsa:2048 -nodes -sha256 -days 1 \
	-subj '/CN=DBX PKCS12 Check' \
	-keyout "$temp_dir/key.pem" \
	-out "$temp_dir/cert.pem" \
	>/dev/null 2>&1
openssl pkcs12 -export \
	-inkey "$temp_dir/key.pem" \
	-in "$temp_dir/cert.pem" \
	-out "$temp_dir/identity.p12" \
	-keypbe PBE-SHA1-3DES \
	-certpbe PBE-SHA1-3DES \
	-macalg sha1 \
	-passout "pass:$password" \
	>/dev/null 2>&1
metadata="$(openssl pkcs12 -in "$temp_dir/identity.p12" -passin "pass:$password" -info -noout 2>&1)"
[[ "$metadata" == *'MAC: sha1'* ]] || fail "generated PKCS#12 does not use SHA-1 MAC"
[[ "$metadata" == *'pbeWithSHA1And3-KeyTripleDES-CBC'* ]] \
	|| fail "generated PKCS#12 does not use 3DES PBE"

printf 'PKCS#12 bootstrap check: passed\n'
