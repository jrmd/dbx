#!/usr/bin/env bash
# Build a downloadable bundle on a Mac or GitHub-hosted macOS runner.
set -Eeuo pipefail
umask 077
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
[[ $(uname -s) == Darwin ]] || { echo 'A macOS runner is required.' >&2; exit 1; }
mode=${1:-signed}
[[ $mode == signed || $mode == unsigned || $mode == local || $mode == check-local ]] || {
  echo 'Usage: release-macos.sh [signed|unsigned|local|check-local]' >&2; exit 1;
}
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
export DBX_VERSION="$version"
# Distributions always use the shipping profile, whatever the caller exported.
export DBX_CARGO_PROFILE=release
# Never pick up the developer-machine default profile; only the secret below applies.
export DBX_NO_PROVISIONING_PROFILE=1
export DBX_APP_DIR="$root/target/macos/DBX.app"
mkdir -p target/macos
temporary=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/dbx-release.XXXXXX")
keychain="$temporary/signing.keychain-db"
original_keychains=()
restore_keychains=false
cleanup() {
  if $restore_keychains; then
    security list-keychains -d user -s "${original_keychains[@]}" >/dev/null 2>&1 || true
  fi
  if [[ -f $keychain ]]; then security delete-keychain "$keychain" >/dev/null 2>&1 || true; fi
  rm -rf -- "$temporary"
}
trap cleanup EXIT

if [[ $mode == signed ]]; then
  # Optional: a Developer ID provisioning profile enables Touch ID vault unlock.
  if [[ -n ${APPLE_PROVISIONING_PROFILE_BASE64:-} ]]; then
    printf '%s' "$APPLE_PROVISIONING_PROFILE_BASE64" | base64 --decode > "$temporary/embedded.provisionprofile"
    export DBX_PROVISIONING_PROFILE="$temporary/embedded.provisionprofile"
  fi
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
  security list-keychains -d user | python3 -c 'import shlex,sys; print("\n".join(shlex.split(sys.stdin.read())))' > "$temporary/keychains"
  while IFS= read -r existing; do
    [[ -z $existing ]] || original_keychains+=("$existing")
  done < "$temporary/keychains"
  restore_keychains=true
  security list-keychains -d user -s "$keychain" "${original_keychains[@]}"
  export DBX_KEYCHAIN="$keychain"
  # Select exactly one matching Developer ID identity; never fall back to local signing.
  DBX_SIGNING_NAME=$(security find-identity -v -p codesigning "$keychain" | python3 -c '
import os,re,sys
names=re.findall(r"\"(Developer ID Application: [^\"]+)\"", sys.stdin.read())
names=[n for n in names if n.endswith("("+os.environ["APPLE_TEAM_ID"]+")")]
if len(names)!=1: sys.exit("Expected one Developer ID Application identity for the configured team.")
print(names[0])')
  export DBX_SIGNING_NAME DBX_SIGNING_MODE=developer-id
  xcrun notarytool store-credentials dbx-release --keychain "$keychain" \
    --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" >/dev/null
  notary_args=(--keychain-profile dbx-release --keychain "$keychain")
elif [[ $mode == local || $mode == check-local ]]; then
  # Use the installed distribution identity and saved notary credentials without
  # importing keys, changing the keychain search list, or passing passwords.
  export DBX_KEYCHAIN="${DBX_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
  DBX_SIGNING_NAME=$(security find-identity -v -p codesigning "$DBX_KEYCHAIN" | python3 -c '
import os,re,sys
identities=re.findall(r"([A-Fa-f0-9]{40}) \"(Developer ID Application: [^\"]+)\"", sys.stdin.read())
choice=os.environ.get("DBX_SIGNING_NAME", "")
team=os.environ.get("APPLE_TEAM_ID", "")
matches=[name for fingerprint,name in identities
         if (not choice or choice==name or choice.upper()==fingerprint.upper())
         and (not team or name.endswith("("+team+")"))]
if len(matches)!=1:
    sys.exit("Install one Developer ID Application certificate with its private key, or select it with DBX_SIGNING_NAME/APPLE_TEAM_ID.")
print(matches[0])')
  APPLE_TEAM_ID="${DBX_SIGNING_NAME##*(}"
  export APPLE_TEAM_ID="${APPLE_TEAM_ID%)}"
  export DBX_SIGNING_NAME DBX_SIGNING_MODE=developer-id
  notary_args=(--keychain-profile "${DBX_NOTARY_PROFILE:-dbx-release}"
    --keychain "${DBX_NOTARY_KEYCHAIN:-$DBX_KEYCHAIN}")
  xcrun notarytool history "${notary_args[@]}" --output-format json >/dev/null
  if [[ $mode == check-local ]]; then
    echo "Local signing identity and notarization credentials are ready."
    exit 0
  fi
else
  export DBX_SIGNING_NAME=- DBX_SIGNING_MODE=local
fi

# A profile is only meaningful with a distribution signature.
if [[ -n ${DBX_PROVISIONING_PROFILE:-} && $mode != unsigned ]]; then
  [[ -s $DBX_PROVISIONING_PROFILE ]] || { echo "Provisioning profile is missing or empty: $DBX_PROVISIONING_PROFILE" >&2; exit 1; }
fi
bash scripts/build-macos-app.sh build
if [[ $mode != unsigned ]]; then
  codesign -dv --verbose=4 "$DBX_APP_DIR" 2> "$temporary/signature.txt"
  grep -Fqx "TeamIdentifier=$APPLE_TEAM_ID" "$temporary/signature.txt"
  grep -Fq 'Authority=Developer ID Application:' "$temporary/signature.txt"
  if [[ -n ${DBX_PROVISIONING_PROFILE:-} ]]; then
    # The signed app must carry the validated profile and the entitlements Touch ID relies on.
    [[ -f $DBX_APP_DIR/Contents/embedded.provisionprofile ]] || { echo 'Provisioning profile was not embedded.' >&2; exit 1; }
    cmp -s "$DBX_PROVISIONING_PROFILE" "$DBX_APP_DIR/Contents/embedded.provisionprofile" || {
      echo 'Embedded provisioning profile differs from the validated profile.' >&2; exit 1; }
    codesign -d --entitlements :- "$DBX_APP_DIR" > "$temporary/entitlements.plist" 2>/dev/null
    python3 -I - "$temporary/entitlements.plist" "$APPLE_TEAM_ID.${DBX_BUNDLE_ID:-dev.jrmd.dbx}" <<'PY'
import plistlib, sys
path, expected = sys.argv[1:3]
try:
    with open(path, "rb") as f:
        entitlements = plistlib.load(f)
except Exception as error:
    sys.exit("Signed entitlements are unreadable: %s" % error)
if entitlements.get("com.apple.application-identifier") != expected:
    sys.exit("Signed application-identifier is not %s." % expected)
groups = entitlements.get("keychain-access-groups")
# Touch ID stores its key in the default (first) access group.
if not isinstance(groups, list) or not groups or groups[0] != expected:
    sys.exit("First signed keychain-access-groups entry must be %s." % expected)
PY
  fi
  ditto -c -k --keepParent "$DBX_APP_DIR" "$temporary/submission.zip"
  # A timeout or rejected submission stops the job before any release artifact is uploaded.
  xcrun notarytool submit "$temporary/submission.zip" "${notary_args[@]}" \
    --wait --timeout "${DBX_NOTARY_TIMEOUT:-30m}" --output-format json > target/macos/notarization.json
  python3 -c 'import json; r=json.load(open("target/macos/notarization.json")); assert r.get("status")=="Accepted", r'
  xcrun stapler staple "$DBX_APP_DIR"
  xcrun stapler validate "$DBX_APP_DIR"
  codesign --verify --deep --strict --verbose=2 "$DBX_APP_DIR"
  spctl --assess --type execute --verbose=2 "$DBX_APP_DIR"
fi
archive="DBX-$version-macos-$(uname -m)"
[[ $mode != unsigned ]] || archive="$archive-unsigned"
ditto -c -k --keepParent "$DBX_APP_DIR" "target/macos/$archive.zip"
(cd target/macos && shasum -a 256 "$archive.zip" > "$archive.zip.sha256")
echo "Ready: target/macos/$archive.zip"
