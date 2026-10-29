# Vault unlock with Touch ID

On a Mac where biometrics are available and the running app has a
data-protection Keychain entitlement, **Settings → Connections → Vault** offers
**Unlock with Touch ID** while the vault is unlocked. Turning it on enrols this
vault immediately; turning it off removes the saved key immediately. Once
enabled, the lock screen offers **Unlock with Touch ID** as its primary action.
Launching DBX does not request biometric authentication automatically.

The passphrase remains available. Cancelling Touch ID leaves the vault locked.
Changing the enrolled fingerprints can invalidate the saved key; unlock with
the passphrase and DBX enrols again, or turn Touch ID off and on in Settings.

Touch ID and **Unlock automatically on this device** are separate options.
Automatic unlock lives in **Settings → Connections → Vault**; changing it
updates the system keychain immediately while the vault is unlocked.
Enabling Touch ID switches off automatic unlock. Before enrolling the protected
key, DBX removes its ordinary remembered key; if removal fails, enrollment fails
and a toast reports it.

## Key protection

Only the derived vault key is saved, never the passphrase. The Touch ID item
uses a separate Keychain service and a per-vault account. It lives in the macOS
data-protection Keychain with `biometryCurrentSet` access control and
`WhenPasscodeSetThisDeviceOnly` protection. Each read uses a fresh
`LAContext` with no authentication reuse interval. The Keychain enforces the
biometric requirement when releasing the key.

## Signed-build prerequisite

Touch ID needs the restricted `com.apple.application-identifier` and
`keychain-access-groups` entitlements. Outside the App Store macOS only honours
them when a matching provisioning profile is embedded at
`Contents/embedded.provisionprofile` and the app is signed by the profile's
team; otherwise the system kills the app at launch. DBX therefore applies the
entitlements **only** when a profile is used. Builds without one (ad-hoc,
Apple Development, self-signed and unsigned `cargo run` builds) are unchanged,
hide Touch ID, and keep passphrase and remembered-device unlock.

### Default profile for local builds

If `DBX_PROVISIONING_PROFILE` is unset, `make build`, `make run` and `make
install` fall back to `~/Library/Developer/DBX/dbx-developer-id.provisionprofile`
when all of these hold: the file exists, the resolved identity is a
`Developer ID Application` certificate, and the bundle ID is the default
(`dev.jrmd.dbx`). Otherwise the build proceeds silently without a profile. One
log line names the profile when it is used. The default goes through the same
validation as an explicit profile, so a stale or mismatched file fails the
build; opt out with `DBX_NO_PROVISIONING_PROFILE=1` or `DBX_PROVISIONING_PROFILE=`
(set but empty). An explicitly set non-empty `DBX_PROVISIONING_PROFILE` is always
strict. `scripts/release-macos.sh` never uses the default; it only uses the
profile from `APPLE_PROVISIONING_PROFILE_BASE64`.

When `DBX_PROVISIONING_PROFILE=/path/to/DBX.provisionprofile` is set,
`scripts/build-macos-app.sh` decodes the profile with `security cms -D` and
fails before compiling unless all of these hold:

- the signing identity is a `Developer ID Application: ... (TEAMID)`
  certificate (ad-hoc, self-signed, Apple Development and unparseable names are
  rejected, so the team is always known);
- the profile's team equals that `(TEAMID)`;
- the SHA-1 of the signing certificate appears among the profile's
  `DeveloperCertificates`;
- it is a macOS Developer ID distribution profile: `Platform` includes `OSX`
  and it has no `ProvisionedDevices` list (development and ad hoc profiles are
  bound to devices; Developer ID profiles are not);
- its application identifier is `TEAM.dev.jrmd.dbx` (or a matching wildcard);
- it has not expired and allows `keychain-access-groups` for that identifier.

It then embeds the profile and renders `packaging/macos/DBX.entitlements` with
the profile's team and the bundle ID. The rendered file is normalised with
`plutil -convert xml1` and checked with `plutil -lint`; the template has no XML
comments because Apple rejects them in distribution entitlements. The
executable and bundle are signed with those entitlements and the usual hardened
runtime. The entitlements are applied only when a profile is supplied, because
without a matching embedded profile the system kills the app at launch.
Keep the bundle ID and team stable between updates so the Keychain access
group does not change.

### One-time Apple setup

1. At [Certificates, Identifiers & Profiles → Identifiers](https://developer.apple.com/account/resources/identifiers/list),
   with team `DQG6RVCCWV` selected, register an **explicit** App ID of type
   *App* with Bundle ID `dev.jrmd.dbx`. No capabilities are required: the
   `keychain-access-groups` entitlement under the team prefix needs none.
2. Under **Profiles**, create a new profile of type **Developer ID** (distribution
   outside the Mac App Store), select the `dev.jrmd.dbx` App ID and your
   Developer ID Application certificate, and download the `.provisionprofile`.
3. Locally, set it for release builds:
   `export DBX_PROVISIONING_PROFILE=/private/path/DBX.provisionprofile`
   then run `make release` (or `DBX_SIGNING_MODE=developer-id make build`).
4. For GitHub Actions, store it as a secret:
   `base64 -i DBX.provisionprofile | gh secret set APPLE_PROVISIONING_PROFILE_BASE64`.
   The signed release workflows decode it to a temporary file; the secret is optional.

Profiles expire (check the date in the portal) and become invalid if the
certificate is revoked. Renew and replace the file and the secret before then;
an expired profile fails the build rather than shipping a broken app.
After signing, the release script parses the signed entitlements and requires
`com.apple.application-identifier` and the **first** `keychain-access-groups`
entry (the default group Touch ID uses) to equal `TEAM.dev.jrmd.dbx`. It also
requires the embedded profile to be byte-identical to the validated one, and
stops otherwise. Enrollment still handles Keychain rejection safely if a
signature passes preflight but cannot store the item.

See Apple's [Mac Keychain implementations](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains),
[Keychain access groups](https://developer.apple.com/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps),
and [`biometryCurrentSet`](https://developer.apple.com/documentation/security/secaccesscontrolcreateflags/biometrycurrentset).

## Native verification

Use a disposable vault under `DBX_CONFIG_DIR`, never a user's real vault.
Tests do not request fingerprints or operate on the system Keychain.
On an appropriately provisioned build with enrolled Touch ID, verify:

1. Turn on Touch ID in Settings → Connections → Vault, lock, and unlock with Touch ID.
2. Lock again and confirm a new authentication is required.
3. Cancel the prompt and unlock using the passphrase.
4. Relaunch and confirm no authentication prompt appears until requested.
5. Turn Touch ID off in Settings; confirm the protected item
   is removed and automatic unlock remains off unless separately selected.
6. Confirm missing/invalidated items and unavailable biometrics return to the
   passphrase path without unlocking.

The native setup, unlock, failure, and appearance flows can also be checked
without biometric hardware; see [isolated desktop QA](desktop-qa.md).
