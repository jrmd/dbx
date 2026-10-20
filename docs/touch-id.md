# Vault unlock with Touch ID

The vault gate offers **Use Touch ID** on a Mac where biometrics are available
and the running app has a data-protection Keychain entitlement. Select it, then
unlock with the vault passphrase to enroll this vault. On subsequent locks or
launches, choose **Unlock with Touch ID**. Launching DBX does not request
biometric authentication automatically.

The passphrase remains available. Cancelling Touch ID leaves the vault locked.
Changing the enrolled fingerprints can invalidate the saved key; unlock with
the passphrase to enroll again. Turning off **Use Touch ID** takes effect on
the next successful passphrase unlock.

Touch ID and **Unlock automatically on this device** are separate options.
Automatic unlock lives in **Settings → Connections → Vault**; changing it
updates the system keychain immediately while the vault is unlocked.
Enabling Touch ID switches off automatic unlock. Before enrolling the protected
key, DBX removes its ordinary remembered key; if removal fails, enrollment fails.
The passphrase unlock still succeeds and a toast reports the device-key failure.

## Key protection

Only the derived vault key is saved, never the passphrase. The Touch ID item
uses a separate Keychain service and a per-vault account. It lives in the macOS
data-protection Keychain with `biometryCurrentSet` access control and
`WhenPasscodeSetThisDeviceOnly` protection. Each read uses a fresh
`LAContext` with no authentication reuse interval. The Keychain enforces the
biometric requirement when releasing the key.

## Signed-build prerequisite

The normal local signing setup does not supply data-protection Keychain
entitlements. Touch ID stays hidden on those builds, including unsigned
`cargo run` builds. Ordinary passphrase and remembered-device unlock continue
to work.

A release intended to offer Touch ID must be signed and provisioned for a
valid application identifier / Keychain access group. Keep that identity stable
between updates. Adding arbitrary entitlement strings to an ad-hoc signature
does not establish access to a provisioned group. Enrollment still handles
Keychain rejection safely if a signature passes preflight but cannot store
the item.

See Apple's [Mac Keychain implementations](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains),
[Keychain access groups](https://developer.apple.com/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps),
and [`biometryCurrentSet`](https://developer.apple.com/documentation/security/secaccesscontrolcreateflags/biometrycurrentset).

## Native verification

Use a disposable vault under `DBX_CONFIG_DIR`, never a user's real vault.
Tests do not request fingerprints or operate on the system Keychain.
On an appropriately provisioned build with enrolled Touch ID, verify:

1. Opt in, unlock with a passphrase, lock, and unlock with Touch ID.
2. Lock again and confirm a new authentication is required.
3. Cancel the prompt and unlock using the passphrase.
4. Relaunch and confirm no authentication prompt appears until requested.
5. Disable Touch ID and unlock with a passphrase; confirm the protected item
   is removed and automatic unlock remains off unless separately selected.
6. Confirm missing/invalidated items and unavailable biometrics return to the
   passphrase path without unlocking.

The native setup, unlock, failure, and appearance flows can also be checked
without biometric hardware; see [isolated desktop QA](desktop-qa.md).
