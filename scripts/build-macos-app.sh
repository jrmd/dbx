#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

APP_DIR="${DBX_APP_DIR:-$ROOT_DIR/target/macos/DBX.app}"
BUNDLE_ID="${DBX_BUNDLE_ID:-dev.jrmd.dbx}"
# Signing modes (DBX_SIGNING_MODE):
#   auto               installed Developer ID Application, else Apple Development (default)
#   apple-development  requires an Apple Development identity
#   developer-id       requires a Developer ID Application identity; adds a secure timestamp
#   local              self-signed identity, created if missing (explicit opt-in)
# DBX_SIGNING_NAME selects an identity by exact name or SHA-1 fingerprint; "-" is ad-hoc.
SIGNING_NAME="${DBX_SIGNING_NAME:-}"
SIGNING_MODE="${DBX_SIGNING_MODE:-auto}"
LOCAL_IDENTITY_NAME="DBX Local Development"
# The codesign --sign argument (a fingerprint, or "-") and a label for logs.
SIGNING_IDENTITY=""
SIGNING_LABEL=""
# Optional DBX_PROVISIONING_PROFILE=<.provisionprofile>: embed it and sign with the
# keychain entitlements Touch ID needs. An explicit value is validated strictly.
# When it is unset, the default profile below is used only if it exists, the
# identity is Developer ID Application and the bundle ID is the default. Set
# DBX_PROVISIONING_PROFILE= (empty) or DBX_NO_PROVISIONING_PROFILE=1 to opt out.
PROVISIONING_PROFILE="${DBX_PROVISIONING_PROFILE:-}"
readonly DEFAULT_BUNDLE_ID="dev.jrmd.dbx"
readonly DEFAULT_PROVISIONING_PROFILE="$HOME/Library/Developer/DBX/dbx-developer-id.provisionprofile"
PROVISION_DIR=""
PROVISION_ENTITLEMENTS=""
KEYCHAIN="${DBX_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
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

readonly INFO_PLIST="$ROOT_DIR/packaging/macos/Info.plist"
readonly ENTITLEMENTS_TEMPLATE="$ROOT_DIR/packaging/macos/DBX.entitlements"
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

IDENTITY_IDS=()
IDENTITY_NAMES=()
MATCH_IDS=()
MATCH_NAMES=()

load_identities() {
	local output line
	local pattern='^[[:space:]]*[0-9]+\)[[:space:]]+([0-9A-Fa-f]{40})[[:space:]]+"([^"]*)"'
	IDENTITY_IDS=()
	IDENTITY_NAMES=()
	output="$(security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null)" \
		|| die "could not list code-signing identities in $KEYCHAIN"
	while IFS= read -r line; do
		if [[ "$line" =~ $pattern ]]; then
			IDENTITY_IDS+=("${BASH_REMATCH[1]}")
			IDENTITY_NAMES+=("${BASH_REMATCH[2]}")
		fi
	done <<<"$output"
}

# Fill MATCH_* with valid identities whose name starts with $1 or, when $2 is
# "exact", whose name or SHA-1 fingerprint equals $1.
find_identities() {
	local value="$1" exact="${2:-}" fingerprint i
	fingerprint="$(printf '%s' "$value" | tr 'a-f' 'A-F')"
	MATCH_IDS=()
	MATCH_NAMES=()
	load_identities
	for ((i = 0; i < ${#IDENTITY_IDS[@]}; i++)); do
		if [[ -n "$exact" ]]; then
			[[ "${IDENTITY_NAMES[$i]}" == "$value" || "${IDENTITY_IDS[$i]}" == "$fingerprint" ]] || continue
		else
			[[ "${IDENTITY_NAMES[$i]}" == "$value"* ]] || continue
		fi
		MATCH_IDS+=("${IDENTITY_IDS[$i]}")
		MATCH_NAMES+=("${IDENTITY_NAMES[$i]}")
	done
}

# Use the single match found by find_identities; $1 describes what was searched.
use_single_match() {
	local i
	if [[ "${#MATCH_IDS[@]}" -ne 1 ]]; then
		printf 'DBX: found %s matching identities for %s in %s:\n' "${#MATCH_IDS[@]}" "$1" "$KEYCHAIN" >&2
		for ((i = 0; i < ${#MATCH_IDS[@]}; i++)); do
			printf '  %s  %s\n' "${MATCH_IDS[$i]}" "${MATCH_NAMES[$i]}" >&2
		done
		die "set DBX_SIGNING_NAME to the full certificate name or SHA-1 fingerprint to choose one"
	fi
	SIGNING_IDENTITY="${MATCH_IDS[0]}"
	SIGNING_LABEL="${MATCH_NAMES[0]}"
}

no_apple_identity() {
	cat >&2 <<EOF
DBX: No Developer ID Application or Apple Development identity was found in
$KEYCHAIN

Install one (Xcode > Settings > Accounts > Manage Certificates), or choose
explicitly:
  DBX_SIGNING_MODE=local   self-signed '$LOCAL_IDENTITY_NAME', created if missing
  DBX_SIGNING_NAME=-       ad-hoc signing (Keychain permissions may not survive rebuilds)
  DBX_SIGNING_NAME=<name or SHA-1 fingerprint> and/or DBX_KEYCHAIN=<keychain>
EOF
	exit 1
}

print_manual_identity_instructions() {
	cat >&2 <<EOF
DBX: Could not find a usable '$1' identity.

Create it once in Keychain Access:
  1. Open Keychain Access.
  2. Keychain Access > Certificate Assistant > Create a Certificate.
  3. Name: $1
  4. Identity Type: Self Signed Root
  5. Certificate Type: Code Signing
  6. Save it in the login keychain.

Then run make run again. This identity is only for this Mac's local builds;
it is not an Apple Developer or distribution certificate.
EOF
}

create_local_identity() {
	local name="$1"
	if ! command -v openssl >/dev/null 2>&1; then
		print_manual_identity_instructions "$name"
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

	log "creating local code-signing identity '$name'"
	openssl req -new -x509 -newkey rsa:2048 -nodes -sha256 \
		-days 3650 \
		-subj "/CN=$name" \
		-addext "basicConstraints=critical,CA:false" \
		-addext "keyUsage=critical,digitalSignature" \
		-addext "extendedKeyUsage=critical,1.3.6.1.5.5.7.3.3" \
		-keyout "$key_file" \
		-out "$cert_file" \
		>/dev/null 2>&1 \
		|| {
			print_manual_identity_instructions "$name"
			die "could not generate the local code-signing certificate"
		}

	openssl pkcs12 -export \
		-name "$name" \
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
			print_manual_identity_instructions "$name"
			die "could not import the local identity into the login keychain"
		}

	cleanup_temp_identity
	trap - EXIT
	log "local identity is ready"
}

# Resolve SIGNING_IDENTITY before the expensive build. Only local mode ever
# creates a certificate; every other mode fails if nothing suitable is installed.
resolve_signing_identity() {
	local prefix="" name candidate
	case "$SIGNING_MODE" in
	auto | local) ;;
	apple-development) prefix="Apple Development: " ;;
	developer-id) prefix="Developer ID Application: " ;;
	*) die "unknown signing mode: $SIGNING_MODE (expected auto, apple-development, developer-id or local)" ;;
	esac

	if [[ "$SIGNING_NAME" == "-" ]]; then
		[[ -z "$prefix" ]] || die "$SIGNING_MODE mode requires a '$prefix' identity, not ad-hoc signing"
		log "using ad-hoc signing; Keychain permissions may not survive rebuilds"
		SIGNING_IDENTITY="-"
		SIGNING_LABEL="ad-hoc"
		return
	fi

	if [[ "$SIGNING_MODE" == "local" ]]; then
		name="${SIGNING_NAME:-$LOCAL_IDENTITY_NAME}"
		find_identities "$name" exact
		if [[ "${#MATCH_IDS[@]}" -eq 0 ]]; then
			create_local_identity "$name"
			find_identities "$name" exact
			if [[ "${#MATCH_IDS[@]}" -eq 0 ]]; then
				print_manual_identity_instructions "$name"
				die "the imported identity was not available to codesign"
			fi
		fi
		use_single_match "'$name'"
		return
	fi

	if [[ -n "$SIGNING_NAME" ]]; then
		find_identities "$SIGNING_NAME" exact
		[[ "${#MATCH_IDS[@]}" -gt 0 ]] || die "no valid code-signing identity '$SIGNING_NAME' in $KEYCHAIN; refusing to create a replacement"
		use_single_match "'$SIGNING_NAME'"
		[[ -z "$prefix" || "$SIGNING_LABEL" == "$prefix"* ]] \
			|| die "$SIGNING_MODE mode requires a '$prefix' identity; '$SIGNING_LABEL' is not one"
		return
	fi

	# Discover: prefer Developer ID Application, then Apple Development.
	local candidates=("Developer ID Application: " "Apple Development: ")
	[[ -z "$prefix" ]] || candidates=("$prefix")
	for candidate in "${candidates[@]}"; do
		find_identities "$candidate"
		if [[ "${#MATCH_IDS[@]}" -gt 0 ]]; then
			use_single_match "'$candidate'"
			return
		fi
	done
	[[ -z "$prefix" ]] || die "no valid '$prefix' identity found in $KEYCHAIN; refusing to create a self-signed replacement"
	no_apple_identity
}

cleanup_provisioning() {
	[[ -z "$PROVISION_DIR" ]] || rm -rf -- "$PROVISION_DIR"
}

# Validate DBX_PROVISIONING_PROFILE against the signing identity and bundle ID,
# then render the entitlements from the profile's own team and app ID. Runs
# before the build so a bad profile fails fast.
prepare_provisioning() {
	if [[ -z "${DBX_PROVISIONING_PROFILE+set}" && -z "${DBX_NO_PROVISIONING_PROFILE:-}" \
		&& "$SIGNING_LABEL" == "Developer ID Application: "* && "$BUNDLE_ID" == "$DEFAULT_BUNDLE_ID" \
		&& -f "$DEFAULT_PROVISIONING_PROFILE" ]]; then
		PROVISIONING_PROFILE="$DEFAULT_PROVISIONING_PROFILE"
	fi
	[[ -n "$PROVISIONING_PROFILE" ]] || return 0
	local decoded team label_team
	[[ -f "$PROVISIONING_PROFILE" ]] || die "provisioning profile not found: $PROVISIONING_PROFILE"
	[[ "$SIGNING_IDENTITY" != "-" ]] || die "a provisioning profile cannot be used with ad-hoc signing; sign with a Developer ID Application identity"
	[[ "$SIGNING_MODE" != "local" ]] || die "a provisioning profile cannot be used with a self-signed local identity; use a Developer ID Application identity"
	[[ -f "$ENTITLEMENTS_TEMPLATE" ]] || die "missing entitlements template: $ENTITLEMENTS_TEMPLATE"
	PROVISION_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dbx-provision.XXXXXX")"
	trap cleanup_provisioning EXIT

	decoded="$PROVISION_DIR/profile.plist"
	security cms -D -i "$PROVISIONING_PROFILE" >"$decoded" 2>/dev/null \
		|| die "could not decode provisioning profile: $PROVISIONING_PROFILE"

	# Profiles are Developer ID distribution profiles, so the identity must be a
	# Developer ID Application certificate whose "(TEAMID)" suffix names the team.
	[[ "$SIGNING_LABEL" == "Developer ID Application: "*"("*")" ]] \
		|| die "a provisioning profile requires a 'Developer ID Application: ... (TEAMID)' identity; '$SIGNING_LABEL' is not one"
	label_team="${SIGNING_LABEL##*(}"
	label_team="${label_team%)}"
	[[ "$label_team" =~ ^[A-Z0-9]+$ ]] || die "could not determine the team ID from '$SIGNING_LABEL'"
	[[ "$BUNDLE_ID" =~ ^[A-Za-z0-9.-]+$ ]] || die "bundle ID contains characters unsuitable for entitlements: $BUNDLE_ID"
	require_command plutil

	team="$(python3 -I - "$decoded" "$BUNDLE_ID" "$label_team" "$SIGNING_IDENTITY" <<'PY'
import datetime, hashlib, plistlib, sys

path, bundle_id, expected_team, fingerprint = sys.argv[1:5]

def fail(message):
    sys.exit("provisioning profile rejected: " + message)

try:
    with open(path, "rb") as f:
        profile = plistlib.load(f)
except Exception as error:
    fail("not a readable profile (%s)" % error)

teams = profile.get("TeamIdentifier") or []
entitlements = profile.get("Entitlements") or {}
app_id = entitlements.get("com.apple.application-identifier") or ""
if not teams or "." not in app_id:
    fail("missing TeamIdentifier or com.apple.application-identifier")
team = teams[0]
if app_id.split(".", 1)[0] != team:
    fail("application-identifier %s does not belong to team %s" % (app_id, team))
if expected_team and team != expected_team:
    fail("profile team %s does not match signing identity team %s" % (team, expected_team))

# Developer ID profiles list no devices; development and ad hoc profiles do.
if "ProvisionedDevices" in profile:
    fail("profile is bound to specific devices; a Developer ID distribution profile is required")
platform = profile.get("Platform") or []
if not any(p in ("OSX", "macOS") for p in platform):
    fail("profile is not a macOS profile (Platform=%s)" % list(platform))
# The signing certificate must be one the profile was issued for.
certs = profile.get("DeveloperCertificates") or []
digests = {hashlib.sha1(bytes(c)).hexdigest().upper() for c in certs}
if fingerprint.upper() not in digests:
    fail("signing certificate %s is not in the profile's DeveloperCertificates" % fingerprint.upper())

full_id = team + "." + bundle_id
if app_id.endswith("*"):
    matches = full_id.startswith(app_id[:-1])
else:
    matches = app_id == full_id
if not matches:
    fail("application-identifier %s does not match %s" % (app_id, full_id))

expires = profile.get("ExpirationDate")
if expires is None:
    fail("missing ExpirationDate")
if expires.tzinfo is not None:
    expires = expires.astimezone(datetime.timezone.utc).replace(tzinfo=None)
if expires <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None):
    fail("expired on %s" % expires.isoformat())

groups = entitlements.get("keychain-access-groups")
if not groups:
    fail("keychain-access-groups is not allowed by this profile")
def allows(group):
    return group == full_id or (group.endswith("*") and full_id.startswith(group[:-1]))
if not any(allows(group) for group in groups):
    fail("keychain-access-groups %s does not allow %s" % (list(groups), full_id))

print(team)
PY
)" || die "invalid provisioning profile: $PROVISIONING_PROFILE"
	[[ -n "$team" ]] || die "could not read the team from $PROVISIONING_PROFILE"

	local rendered
	rendered="$(<"$ENTITLEMENTS_TEMPLATE")"
	rendered="${rendered//'$(TEAM)'/$team}"
	rendered="${rendered//'$(BUNDLE_ID)'/$BUNDLE_ID}"
	PROVISION_ENTITLEMENTS="$PROVISION_DIR/DBX.entitlements"
	printf '%s\n' "$rendered" >"$PROVISION_ENTITLEMENTS"
	plutil -convert xml1 "$PROVISION_ENTITLEMENTS" || die "could not normalise the rendered entitlements"
	plutil -lint "$PROVISION_ENTITLEMENTS" >/dev/null || die "rendered entitlements are not a valid property list"
	log "using provisioning profile $PROVISIONING_PROFILE for $team.$BUNDLE_ID"
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
	log "building $CARGO_PROFILE binary"
	(cd "$ROOT_DIR" && "$CARGO_BIN" build --locked "${CARGO_PROFILE_ARGS[@]}" --package dbx-ui)
	[[ -x "$BINARY_PATH" ]] || die "$CARGO_PROFILE binary was not produced: $BINARY_PATH"

	mkdir -p "$APP_DIR/Contents/MacOS"
	mkdir -p "$APP_DIR/Contents/Resources"
	cp "$BINARY_PATH" "$APP_DIR/Contents/MacOS/dbx"
	cp "$INFO_PLIST" "$APP_DIR/Contents/Info.plist"
	/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $BUNDLE_ID" "$APP_DIR/Contents/Info.plist"
	if [[ -n "${DBX_VERSION:-}" ]]; then
		/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $DBX_VERSION" "$APP_DIR/Contents/Info.plist"
		/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $DBX_VERSION" "$APP_DIR/Contents/Info.plist"
	fi
	cp "$LOGO_SVG" "$APP_DIR/Contents/Resources/DBX.svg"
	build_icon
	[[ -s "$APP_DIR/Contents/Resources/DBX.icns" ]] || die "macOS icon was not produced"
}

sign_bundle() {
	local identity="$SIGNING_IDENTITY"
	local sign_options=(--force --options runtime --keychain "$KEYCHAIN")
	if [[ "$SIGNING_MODE" == "developer-id" ]]; then
		sign_options+=(--timestamp)
	fi
	local main_options=("${sign_options[@]}") bundle_options=("${sign_options[@]}")
	if [[ -n "$PROVISION_ENTITLEMENTS" ]]; then
		cp "$PROVISIONING_PROFILE" "$APP_DIR/Contents/embedded.provisionprofile"
		main_options+=(--entitlements "$PROVISION_ENTITLEMENTS")
		bundle_options+=(--entitlements "$PROVISION_ENTITLEMENTS")
	else
		# Never leave a profile from an earlier build in an unprovisioned bundle.
		rm -f "$APP_DIR/Contents/embedded.provisionprofile"
	fi
	log "signing $APP_DIR with $SIGNING_LABEL"

	# Sign nested code first. Do not use --deep for signing.
	codesign "${main_options[@]}" \
		--sign "$identity" \
		--identifier "$BUNDLE_ID" \
		"$APP_DIR/Contents/MacOS/dbx"
	codesign "${bundle_options[@]}" \
		--sign "$identity" \
		--identifier "$BUNDLE_ID" \
		"$APP_DIR"
	codesign --verify --deep --strict --verbose=2 "$APP_DIR"
}

build_app() {
	# Fail on a missing or ambiguous identity before the long cargo build.
	resolve_signing_identity
	prepare_provisioning
	build_bundle
	sign_bundle
	cleanup_provisioning
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
