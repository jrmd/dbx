#!/usr/bin/env bash
# Build a downloadable bundle on a Mac or GitHub-hosted macOS runner.
set -Eeuo pipefail
umask 077
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
[[ $(uname -s) == Darwin ]] || { echo 'A macOS runner is required.' >&2; exit 1; }
mode=${1:-signed}
[[ $mode == signed || $mode == unsigned ]] || { echo 'Usage: release-macos.sh [signed|unsigned]' >&2; exit 1; }
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
export DBX_VERSION="$version"
export DBX_APP_DIR="$root/target/macos/DBX.app"
mkdir -p target/macos
temporary=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/dbx-release.XXXXXX")
keychain="$temporary/signing.keychain-db"
cleanup() {
  if [[ -f $keychain ]]; then security delete-keychain "$keychain" >/dev/null 2>&1 || true; fi
  rm -rf -- "$temporary"
}
trap cleanup EXIT

if [[ $mode == signed ]]; then
  for name in APPLE_CERTIFICATE_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_ID APPLE_APP_SPECIFIC_PASSWORD APPLE_TEAM_ID; do
    [[ -n ${!name:-} ]] || { echo "Missing GitHub secret: $name" >&2; exit 1; }
  done
  keychain_password=$(openssl rand -hex 32)
  printf '%s' "$APPLE_CERTIFICATE_BASE64" | base64 --decode > "$temporary/certificate.p12"
  security create-keychain -p "$keychain_password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$keychain_password" "$keychain"
  security import "$temporary/certificate.p12" -P "$APPLE_CERTIFICATE_PASSWORD" -k "$keychain" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
  security list-keychains -d user -s "$keychain" "$HOME/Library/Keychains/login.keychain-db"
  export DBX_KEYCHAIN="$keychain"
  # Select exactly one matching Developer ID identity; never fall back to local signing.
  DBX_SIGNING_NAME=$(security find-identity -v -p codesigning "$keychain" | python3 -c '
import os,re,sys
names=re.findall(r"\"(Developer ID Application: [^\"]+)\"", sys.stdin.read())
names=[n for n in names if n.endswith("("+os.environ["APPLE_TEAM_ID"]+")")]
if len(names)!=1: sys.exit("Expected one Developer ID Application identity for the configured team.")
print(names[0])')
  export DBX_SIGNING_NAME DBX_SIGNING_MODE=developer-id
else
  export DBX_SIGNING_NAME=- DBX_SIGNING_MODE=local
fi

bash scripts/build-macos-app.sh build
if [[ $mode == signed ]]; then
  codesign -dv --verbose=4 "$DBX_APP_DIR" 2> "$temporary/signature.txt"
  grep -Fqx "TeamIdentifier=$APPLE_TEAM_ID" "$temporary/signature.txt"
  grep -Fq 'Authority=Developer ID Application:' "$temporary/signature.txt"
  xcrun notarytool store-credentials dbx-release --keychain "$keychain" \
    --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" >/dev/null
  ditto -c -k --keepParent "$DBX_APP_DIR" "$temporary/submission.zip"
  # A timeout or rejected submission stops the job before any release artifact is uploaded.
  xcrun notarytool submit "$temporary/submission.zip" --keychain-profile dbx-release \
    --keychain "$keychain" --wait --timeout 30m --output-format json > target/macos/notarization.json
  python3 -c 'import json; r=json.load(open("target/macos/notarization.json")); assert r.get("status")=="Accepted", r'
  xcrun stapler staple "$DBX_APP_DIR"
  xcrun stapler validate "$DBX_APP_DIR"
  codesign --verify --deep --strict --verbose=2 "$DBX_APP_DIR"
  spctl --assess --type execute --verbose=2 "$DBX_APP_DIR"
fi
archive="DBX-$version-macos-$(uname -m)"
[[ $mode == signed ]] || archive="$archive-unsigned"
ditto -c -k --keepParent "$DBX_APP_DIR" "target/macos/$archive.zip"
(cd target/macos && shasum -a 256 "$archive.zip" > "$archive.zip.sha256")
echo "Ready: target/macos/$archive.zip"
