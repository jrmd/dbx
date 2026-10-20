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
