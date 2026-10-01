# macOS release candidates

DBX can be built, signed, and notarized on GitHub's hosted Mac runners without a personal Mac. The manual candidate workflow produces **Apple Silicon (arm64)** candidates. The tag-triggered release workflow produces the same Apple Silicon bundle alongside Linux x86_64. Intel Macs are not supported.

## Signing setup

Create an RSA 2048-bit private key and SHA-256 certificate signing request using OpenSSL on a trusted machine. Keep the key outside the repository in an owner-only directory. Submit the request through [Apple's certificate portal](https://developer.apple.com/account/resources/certificates/list) for a **Developer ID Application** certificate, selecting the G2 intermediate if prompted. The final certificate's name follows the enrolled Apple account, rather than the request's common name.

Given a DER `.cer` downloaded from Apple and its matching `private-key.pem`, run:

```bash
python3 scripts/configure-macos-certificate.py /path/to/developerID_application.cer \
  --team YOUR_TEAM_ID \
  --directory /private/path/to/signing-material \
  --repo OWNER/REPO
```

This helper requires OpenSSL, Python 3, and an authenticated `gh` with repository access. It verifies the certificate's team, Developer ID Application extension, expiration, public-key match, and signature against Apple's intermediate. It creates an encrypted PKCS#12 bundle and installs three repository secrets:

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE_BASE64` | Certificate, intermediate, and private key in encrypted PKCS#12 form |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting the PKCS#12 bundle |
| `APPLE_TEAM_ID` | Developer Program team identifier |
| `APPLE_ID` | Enrolled Apple Account email used for notarization |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password for notarization |

The last two secrets are configured separately. Generate a password labelled **DBX GitHub Notarization** at [Apple Account → Sign-In and Security → App-Specific Passwords](https://account.apple.com/). Use `gh secret set APPLE_ID` and `gh secret set APPLE_APP_SPECIFIC_PASSWORD` in an interactive terminal; enter the values when prompted. Never use your normal Apple Account password. [Apple's password instructions](https://support.apple.com/102654).

The helper stores `private-key.pem`, `identity.p12`, and `identity-password.txt` outside the repository with owner-only access. Back up that directory securely. Do not upload it as a workflow artifact or attach it to an issue. GitHub receives secrets through stdin, and the helper never prints their contents.

## Build a candidate

The [macOS release candidate workflow](../.github/workflows/macos-release.yml) is manually dispatched:

```bash
# Signed and notarized candidate; all five secrets must be configured.
gh workflow run macos-release.yml --ref main -f signed=true

# Platform build validation before credentials are available.
gh workflow run macos-release.yml --ref main -f signed=false
```

The workflow installs the Rust/native toolchain, runs workspace tests, and builds the version declared in `Cargo.toml`. For signed candidates it imports the certificate into a temporary runner Keychain, selects the Developer ID identity for the configured team, applies hardened-runtime signatures and secure timestamps, submits the bundle to Apple's notary service, and requires **Accepted** before proceeding. It then staples and validates the ticket, verifies the code signature, and assesses Gatekeeper acceptance.

Downloads are Actions artifacts, retained for 14 days:

- `DBX-VERSION-macos-arm64.zip`, containing `DBX.app` with its stapled ticket.
- A corresponding SHA-256 checksum file.
- Unsigned candidates have `-unsigned` in their filenames and are not notarized releases.

The private Keychain and temporary certificate are cleaned up at job exit. Rejected or timed-out notarization stops the workflow before the download upload. First-time submissions may exceed the 30-minute wait; use Apple's submission history to inspect the outcome before retrying.

Artifacts are **release candidates**. The workflow does not create or publish a public GitHub Release. Before publication, download and extract the final archive on a Mac, confirm Gatekeeper launch, and smoke-test vault creation/device unlock, connection testing, table browsing, row editing, and queries. Automated Mac tests and signature checks do not establish that hands-on UI evidence.

## Publish a release with updates

The [release workflow](../.github/workflows/release.yml) builds all platforms from
one version tag. Set the workspace version in `Cargo.toml`, update `Cargo.lock`,
commit the release changes, and push a matching `vVERSION` tag. A manual workflow
rerun must also select that tag. All five Apple secrets above must be available.
The workflow rejects mismatched tags and prerelease versions, runs tests and
Clippy, builds Linux x86_64 plus Apple Silicon, and requires successful
notarization for both Mac bundles. It verifies archive checksums, uploads all
assets to a draft release, then publishes it as latest. A failed build publishes
nothing. If upload fails after draft creation, inspect/remove that draft before
retrying; existing published releases are never overwritten by this workflow.

Each release supplies `DBX-VERSION-linux-x86_64.AppImage`,
`DBX-VERSION-linux-x86_64.tar.gz`, and `DBX-VERSION-macos-arm64.zip`, each with its own
`.sha256` file. The updater accepts only exact platform/version filenames from
`jrmd/dbx`, stable versions newer than the running version, and checksum-matching
downloads. Unsigned candidate filenames cannot be selected.

Mac updates require an installed Developer ID signed `DBX.app` in a writable
location, outside a mounted disk image. Local self-signed/ad-hoc development
builds cannot install updates. Before replacement, the downloaded bundle must
match the running app's signing team and `dev.jrmd.dbx` identifier, pass code
signature and Gatekeeper checks, and have the expected bundle version. The app
bundle is exchanged atomically on the same filesystem using `renamex_np`.
Linux extracts only the regular-file binary entry and replaces the executable
with an atomic rename; root-owned installations require manual/package updates.
An AppImage (detected through `APPIMAGE`) downloads the new AppImage and replaces
that file the same way.
Neither platform replaces the vault or user configuration. Restart is explicit;
DBX does not save or restore open queries when restarting.

macOS uses opaque surfaces and an opaque window backdrop. Light/dark/system
appearance remains available, while the transparency toggle is hidden on Mac.
Verify the final appearance and update/restart journey on both Mac architectures
before calling them device-verified.

## Build reuse

Both release workflows run tests and Clippy in the release profile, matching
the packaging scripts. This avoids compiling the shared dependencies once in
debug mode and again in release mode. GPUI's test-support feature still needs
a separate test variant; the distributed binary keeps its production features.

The shared preparation action caches Cargo downloads and `target/release`
dependency artifacts per operating system, architecture, compiler, and Cargo
inputs. Packaged apps and notarization output are excluded. The
[release build check](../.github/workflows/build-check.yml) runs on relevant
`main` pushes and can be dispatched manually to warm the default-branch cache.
GitHub allows new release tags to restore that cache; a cache saved only on an
older tag is unavailable to a new tag. For fastest releases, let the build check
finish on `main` before pushing the version tag. Cold caches or toolchain/native
dependency changes can still require a full build. Signing, notarization, and
the signed updater installation check always run for each release.

## Local verification

```bash
bash -n scripts/build-macos-app.sh scripts/release-macos.sh
python3 scripts/test-macos-release.py
```

The Python suite exercises acceptance/rejection, wrong-team, cleanup, and unsigned-label gates with fixture Mac tools. It does not prove real Apple signing or notarization. `scripts/release-macos.sh` needs macOS and Python 3.11 or newer; run it with `signed` or `unsigned` on a Mac runner.

Sources: [Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/), [Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow), and [GitHub's signing-Keychain guidance](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications).
