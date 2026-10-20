#!/usr/bin/env python3
"""Check which Cargo profile each local and shipping build entry point selects."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parent.parent

FAKE_CARGO = '''#!/usr/bin/env bash
echo "$*" >> "$CALL_LOG"
profile=release
args=("$@")
for i in "${!args[@]}"; do
  [[ ${args[$i]} == --profile ]] && profile=${args[$((i+1))]}
done
case $profile in dev|test) dir=debug ;; bench) dir=release ;; *) dir=$profile ;; esac
out=${CARGO_TARGET_DIR:-target}/$dir
mkdir -p "$out"
printf '#!/bin/sh\\n' > "$out/dbx"
chmod +x "$out/dbx"
'''

FAKE_UNAME = '#!/bin/sh\necho Darwin\n'
FAKE_NOOP = '#!/bin/sh\n:\n'
FAKE_OPENSSL = '#!/bin/sh\n[ "$1" = rand ] && echo deadbeef\nexit 0\n'
FAKE_CODESIGN = '#!/bin/sh\necho "codesign $*" >> "$CALL_LOG"\n'
# A canned keychain: `find-identity` prints $IDENTITIES; `import` adds a local identity.
FAKE_SECURITY = '''#!/bin/sh
echo "security $*" >> "$CALL_LOG"
case $1 in
find-identity) echo "  Policy: Code Signing"; cat "$IDENTITIES"; echo "     valid identities found" ;;
import) echo '  9) CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC "DBX Local Development"' >> "$IDENTITIES" ;;
esac
'''
FAKE_ICONUTIL = '#!/bin/sh\necho icns > "$4"\n'
FAKE_SIPS = '#!/bin/sh\nwhile [ $# -gt 1 ]; do [ "$1" = --out ] && echo png > "$2"; shift; done\n'
FAKE_RSVG = '#!/bin/sh\nwhile [ $# -gt 1 ]; do [ "$1" = -o ] && echo png > "$2"; shift; done\n'

DEV_ID = "A" * 40
DEV_ID_2 = "B" * 40
APPLE_DEV = "1" * 40
APPLE_DEV_2 = "E" * 40
LOCAL_ID = "C" * 40

STUB = '#!/usr/bin/env bash\necho "$1 profile=${DBX_CARGO_PROFILE-unset}" >> "$CALL_LOG"\n'


class LinuxScriptTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "scripts").mkdir()
        (self.root / "packaging/linux").mkdir(parents=True)
        shutil.copy(REPO / "scripts/build-linux-app.sh", self.root / "scripts")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion="0.1.0"\n')
        (self.root / "packaging/linux/dbx.desktop").write_text("[Desktop Entry]\n")
        (self.root / "logo.svg").write_text("<svg/>")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        for name, body in (("cargo", FAKE_CARGO), ("sha256sum", "#!/bin/sh\necho fixture  \"$1\"\n")):
            (bin_dir / name).write_text(body)
            (bin_dir / name).chmod(0o755)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("DBX_", "CARGO"))}
        self.env.update(PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", CALL_LOG=str(self.root / "calls.log"))

    def build(self, action="build", **env):
        result = subprocess.run(["bash", str(self.root / "scripts/build-linux-app.sh"), action],
                                env={**self.env, **env}, capture_output=True, text=True, cwd=self.root)
        return result

    def cargo_call(self):
        return (self.root / "calls.log").read_text().strip()

    def staged(self):
        return (self.root / "target/linux/DBX/usr/bin/dbx").exists()

    def test_default_is_shipping_release(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.cargo_call(), "build --locked --release --package dbx-ui")
        self.assertTrue((self.root / "target/release/dbx").exists())
        self.assertTrue(self.staged())

    def test_custom_profile_uses_its_own_directory(self):
        result = self.build(DBX_CARGO_PROFILE="release-local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.cargo_call(), "build --locked --profile release-local --package dbx-ui")
        self.assertTrue((self.root / "target/release-local/dbx").exists())
        self.assertTrue(self.staged())

    def test_dev_profile_uses_debug_directory(self):
        result = self.build(DBX_CARGO_PROFILE="dev")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "target/debug/dbx").exists())
        self.assertTrue(self.staged())

    def test_invalid_profile_is_rejected_before_cargo(self):
        result = self.build(DBX_CARGO_PROFILE="x; touch pwned")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "calls.log").exists())

    def test_cargo_target_dir_is_honoured(self):
        for target_dir in ("custom-target", str(self.root / "abs-target")):
            with self.subTest(target_dir=target_dir):
                result = self.build(DBX_CARGO_PROFILE="release-local", CARGO_TARGET_DIR=target_dir)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue((self.root / target_dir / "release-local/dbx").exists())
                self.assertTrue(self.staged())

    def test_run_builds_binary_without_packaging(self):
        result = self.build("run", DBX_CARGO_PROFILE="release-local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "target/linux").exists())


def identity_line(fingerprint, name, index=1):
    return f'  {index}) {fingerprint} "{name}"\n'


class MacosSigningTests(unittest.TestCase):
    """Drive build-macos-app.sh with fake mac tools; no keychain or native build is touched."""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "scripts").mkdir()
        (self.root / "packaging/macos").mkdir(parents=True)
        # Keep the fixture portable so Linux CI exercises the Mac signing logic.
        script = (REPO / "scripts/build-macos-app.sh").read_text()
        (self.root / "scripts/build-macos-app.sh").write_text(
            script.replace("/usr/libexec/PlistBuddy", "PlistBuddy"))
        shutil.copy(REPO / "packaging/macos/Info.plist", self.root / "packaging/macos")
        (self.root / "logo.svg").write_text("<svg/>")
        (self.root / "logo.png").write_text("png")
        self.keychain = self.root / "home/Library/Keychains/login.keychain-db"
        self.keychain.parent.mkdir(parents=True)
        self.keychain.write_text("")
        self.identities = self.root / "identities.txt"
        self.identities.write_text("")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        fakes = {"cargo": FAKE_CARGO, "uname": FAKE_UNAME, "security": FAKE_SECURITY, "codesign": FAKE_CODESIGN,
                 "openssl": FAKE_OPENSSL, "iconutil": FAKE_ICONUTIL, "sips": FAKE_SIPS, "open": FAKE_NOOP,
                 "rsvg-convert": FAKE_RSVG, "PlistBuddy": FAKE_NOOP}
        for name, body in fakes.items():
            (bin_dir / name).write_text(body)
            (bin_dir / name).chmod(0o755)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("DBX_", "CARGO"))}
        self.env.update(PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", HOME=str(self.root / "home"),
                        CALL_LOG=str(self.root / "calls.log"), IDENTITIES=str(self.identities))

    def install(self, *identities):
        self.identities.write_text("".join(identity_line(fp, name, i) for i, (fp, name) in enumerate(identities, 1)))

    def build(self, **env):
        return subprocess.run(["bash", str(self.root / "scripts/build-macos-app.sh"), "build"],
                              env={**self.env, **env}, capture_output=True, text=True, cwd=self.root)

    def calls(self):
        log = self.root / "calls.log"
        return log.read_text().splitlines() if log.exists() else []

    def cargo_ran(self):
        return any(line.startswith("build ") for line in self.calls())

    def imported(self):
        return any(line.startswith("security import") for line in self.calls())

    def sign_calls(self):
        return [line for line in self.calls() if line.startswith("codesign") and "--sign" in line]

    def assert_signed_with(self, fingerprint, timestamp):
        signs = self.sign_calls()
        self.assertEqual(len(signs), 2, signs)
        for line in signs:
            self.assertIn(f"--sign {fingerprint} ", line + " ")
            self.assertEqual("--timestamp" in line, timestamp, line)
            self.assertIn("--options runtime", line)

    def assert_rejected_before_cargo(self, result, *fragments):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertFalse(self.cargo_ran(), self.calls())
        self.assertEqual(self.sign_calls(), [])
        self.assertFalse(self.imported())
        for fragment in fragments:
            self.assertIn(fragment, result.stderr)

    def test_auto_prefers_developer_id_over_apple_development(self):
        self.install((APPLE_DEV, "Apple Development: Dev (TEAM1)"), (DEV_ID, "Developer ID Application: Org (TEAM2)"))
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_signed_with(DEV_ID, timestamp=False)

    def test_auto_discovers_apple_development_without_timestamp(self):
        self.install((LOCAL_ID, "DBX Local Development"), (APPLE_DEV, "Apple Development: Dev (TEAM1)"))
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_signed_with(APPLE_DEV, timestamp=False)
        self.assertIn(f"--keychain {self.keychain}", self.sign_calls()[0])

    def test_auto_without_apple_identity_fails_before_cargo(self):
        self.install((LOCAL_ID, "DBX Local Development"))
        result = self.build()
        self.assert_rejected_before_cargo(result, "DBX_SIGNING_MODE=local", "DBX_SIGNING_NAME=-")

    def test_auto_with_multiple_identities_fails_before_cargo(self):
        self.install((APPLE_DEV, "Apple Development: One (TEAM1)"), (APPLE_DEV_2, "Apple Development: Two (TEAM2)"))
        result = self.build()
        self.assert_rejected_before_cargo(result, APPLE_DEV, APPLE_DEV_2, "fingerprint")

    def test_explicit_name_or_fingerprint_disambiguates(self):
        self.install((APPLE_DEV, "Apple Development: One (TEAM1)"), (APPLE_DEV_2, "Apple Development: Two (TEAM2)"))
        for choice in ("Apple Development: Two (TEAM2)", APPLE_DEV_2.lower()):
            with self.subTest(choice=choice):
                (self.root / "calls.log").unlink(missing_ok=True)
                result = self.build(DBX_SIGNING_NAME=choice)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_signed_with(APPLE_DEV_2, timestamp=False)

    def test_auto_explicit_custom_identity_works_but_is_never_created(self):
        self.install((LOCAL_ID, "My DBX Development"))
        self.assertEqual(self.build(DBX_SIGNING_NAME="My DBX Development").returncode, 0)
        self.assert_signed_with(LOCAL_ID, timestamp=False)
        (self.root / "calls.log").unlink()
        result = self.build(DBX_SIGNING_NAME="Missing Identity")
        self.assert_rejected_before_cargo(result, "Missing Identity", "refusing to create")

    def test_local_mode_creates_identity_only_when_opted_in(self):
        result = self.build(DBX_SIGNING_MODE="local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.imported())
        self.assert_signed_with(LOCAL_ID, timestamp=False)

    def test_local_mode_reuses_existing_identity(self):
        self.install((DEV_ID_2, "DBX Local Development"), (APPLE_DEV, "Apple Development: Dev (TEAM1)"))
        result = self.build(DBX_SIGNING_MODE="local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.imported())
        self.assert_signed_with(DEV_ID_2, timestamp=False)

    def test_local_mode_custom_name(self):
        self.install((APPLE_DEV, "My DBX Development"))
        result = self.build(DBX_SIGNING_MODE="local", DBX_SIGNING_NAME="My DBX Development")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.imported())
        self.assert_signed_with(APPLE_DEV, timestamp=False)

    def test_adhoc_is_explicit_opt_in(self):
        for mode in ("auto", "local"):
            with self.subTest(mode=mode):
                (self.root / "calls.log").unlink(missing_ok=True)
                result = self.build(DBX_SIGNING_MODE=mode, DBX_SIGNING_NAME="-")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_signed_with("-", timestamp=False)
                self.assertFalse(self.imported())

    def test_developer_id_discovers_identity_and_adds_timestamp(self):
        self.install((APPLE_DEV, "Apple Development: Dev (TEAM1)"), (DEV_ID, "Developer ID Application: Org (TEAM2)"))
        result = self.build(DBX_SIGNING_MODE="developer-id")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_signed_with(DEV_ID, timestamp=True)

    def test_developer_id_rejects_apple_development_and_never_creates(self):
        self.install((APPLE_DEV, "Apple Development: Dev (TEAM1)"))
        for env in ({}, {"DBX_SIGNING_NAME": "Apple Development: Dev (TEAM1)"}, {"DBX_SIGNING_NAME": APPLE_DEV},
                    {"DBX_SIGNING_NAME": "Developer ID Application: Missing (TEAM9)"}, {"DBX_SIGNING_NAME": "-"}):
            with self.subTest(env=env):
                (self.root / "calls.log").unlink(missing_ok=True)
                self.assert_rejected_before_cargo(self.build(DBX_SIGNING_MODE="developer-id", **env))

    def test_developer_id_explicit_name_with_selected_keychain(self):
        custom = self.root / "release.keychain-db"
        custom.write_text("")
        self.install((DEV_ID, "Developer ID Application: Org (TEAM2)"), (DEV_ID_2, "Developer ID Application: Other (TEAM3)"))
        self.assert_rejected_before_cargo(self.build(DBX_SIGNING_MODE="developer-id", DBX_KEYCHAIN=str(custom)),
                                          "Developer ID Application: ")
        (self.root / "calls.log").unlink()
        result = self.build(DBX_SIGNING_MODE="developer-id", DBX_KEYCHAIN=str(custom),
                            DBX_SIGNING_NAME="Developer ID Application: Other (TEAM3)")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_signed_with(DEV_ID_2, timestamp=True)
        calls = self.calls()
        self.assertIn(f"security find-identity -v -p codesigning {custom}", calls)
        self.assertIn(f"--keychain {custom}", self.sign_calls()[0])

    def test_apple_development_mode_requires_apple_development(self):
        self.install((DEV_ID, "Developer ID Application: Org (TEAM2)"), (APPLE_DEV, "Apple Development: Dev (TEAM1)"))
        result = self.build(DBX_SIGNING_MODE="apple-development")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_signed_with(APPLE_DEV, timestamp=False)
        (self.root / "calls.log").unlink()
        self.assert_rejected_before_cargo(self.build(DBX_SIGNING_MODE="apple-development", DBX_SIGNING_NAME=DEV_ID))
        self.install((DEV_ID, "Developer ID Application: Org (TEAM2)"))
        self.assert_rejected_before_cargo(self.build(DBX_SIGNING_MODE="apple-development"))

    def test_unknown_mode_fails_before_cargo(self):
        self.assert_rejected_before_cargo(self.build(DBX_SIGNING_MODE="bogus"), "unknown signing mode")


class MakefileTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.stub = Path(temp.name) / "stub.sh"
        self.stub.write_text(STUB)
        self.log = Path(temp.name) / "calls.log"
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("DBX_", "MAKE"))}
        self.env["CALL_LOG"] = str(self.log)

    def make(self, *args, **env):
        result = subprocess.run(
            ["make", "-s", f"MACOS_APP_SCRIPT={self.stub}", f"LINUX_APP_SCRIPT={self.stub}", *args],
            env={**self.env, **env}, capture_output=True, text=True, cwd=REPO)
        self.assertEqual(result.returncode, 0, result.stderr)
        return self.log.read_text().strip().splitlines() if self.log.exists() else []

    def test_local_targets_use_release_local(self):
        self.assertEqual(self.make("run"), [f"run profile=release-local"])

    def test_linux_run_is_local_but_packaging_is_release(self):
        self.assertEqual(self.make("linux-run"), ["run profile=release-local"])
        self.log.unlink()
        self.assertEqual(self.make("linux-build"), ["build profile=release"])
        self.log.unlink()
        self.assertEqual(self.make("linux-package"), ["build profile=release"])
        self.log.unlink()
        self.assertEqual(self.make("linux-appimage"), ["appimage profile=release"])

    def test_explicit_override_applies_everywhere(self):
        self.assertEqual(self.make("run", "DBX_CARGO_PROFILE=dev"), ["run profile=dev"])
        self.log.unlink()
        self.assertEqual(self.make("linux-appimage", DBX_CARGO_PROFILE="dev"), ["appimage profile=dev"])

    def test_cargo_targets_follow_profile(self):
        out = subprocess.run(["make", "-n", "cargo-build", "cargo-run"], env=self.env,
                             capture_output=True, text=True, cwd=REPO).stdout
        self.assertIn("cargo build --locked --profile release-local --package dbx-ui", out)
        self.assertIn("cargo run --locked --profile release-local --package dbx-ui", out)


if __name__ == "__main__":
    unittest.main()
