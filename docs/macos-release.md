# macOS release candidates

DBX can be built, signed, and notarized on GitHub's hosted Mac runners without a personal Mac. The workflow currently produces **Apple Silicon (arm64)** candidates. Intel and universal binaries are not supplied by this workflow.

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

## Local verification

```bash
bash -n scripts/build-macos-app.sh scripts/release-macos.sh
python3 scripts/test-macos-release.py
```

The Python suite exercises acceptance/rejection, wrong-team, cleanup, and unsigned-label gates with fixture Mac tools. It does not prove real Apple signing or notarization. `scripts/release-macos.sh` needs macOS and Python 3.11 or newer; run it with `signed` or `unsigned` on a Mac runner.

Sources: [Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/), [Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow), and [GitHub's signing-Keychain guidance](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications).
