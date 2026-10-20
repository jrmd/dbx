#!/usr/bin/env python3
"""Exercise local publication gates without compiling or contacting GitHub."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("release_local", Path(__file__).with_name("release-local.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
EXTRACT_SPEC = importlib.util.spec_from_file_location("extract_appimage", Path(__file__).with_name("extract-appimage.py"))
extractor = importlib.util.module_from_spec(EXTRACT_SPEC)
EXTRACT_SPEC.loader.exec_module(extractor)


class AppImageExtractionTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.image = self.root / "tool.AppImage"
        self.destination = self.root / "extracted"
        self.image.write_bytes(b"\x7fELF\x02\x01\x01\x00AI\x02" + b"hsqs false marker hsqs filesystem")

    def test_ignores_false_magic_inside_runtime(self):
        with patch.object(extractor.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 1),
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0),
        ]) as run:
            extractor.extract(self.image, self.destination)
        probes = run.call_args_list
        self.assertEqual(len(probes), 3)
        self.assertEqual(probes[1].args[0][3], str(self.image.read_bytes().rfind(b"hsqs")))
        self.assertIn("-d", probes[2].args[0])
        self.assertTrue(probes[2].kwargs["check"])

    def test_invalid_image_is_rejected_without_executing_tools(self):
        self.image.write_bytes(b"not an AppImage")
        with patch.object(extractor.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "type-2"):
                extractor.extract(self.image, self.destination)
            run.assert_not_called()

    def test_missing_filesystem_is_rejected(self):
        with patch.object(extractor.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
            with self.assertRaisesRegex(ValueError, "No valid SquashFS"):
                extractor.extract(self.image, self.destination)

    def test_existing_contents_are_not_overwritten(self):
        self.destination.mkdir()
        existing = self.destination / "keep"
        existing.write_text("preserve")
        with self.assertRaisesRegex(ValueError, "must be empty"):
            extractor.extract(self.image, self.destination)
        self.assertEqual(existing.read_text(), "preserve")


class ReleaseLookupTests(unittest.TestCase):
    def missing_tag(self):
        return subprocess.CalledProcessError(1, ["gh", "api"], stderr="gh: Not Found (HTTP 404)")

    def test_published_release_uses_tag_lookup(self):
        info = {"tag_name": "v1.2.3", "draft": False, "assets": []}
        with patch.object(release, "run", return_value=json.dumps(info)) as run:
            self.assertEqual(release.release_info("v1.2.3"), info)
        self.assertEqual(run.call_count, 1)

    def test_draft_is_found_on_later_authenticated_list_page(self):
        draft = {"tag_name": "v1.2.3", "draft": True, "assets": []}
        pages = [[{"tag_name": "v1.2.4", "draft": True}], [draft]]
        with patch.object(release, "run", side_effect=[self.missing_tag(), json.dumps(pages)]) as run:
            self.assertEqual(release.release_info("v1.2.3"), draft)
        self.assertIn("--paginate", run.call_args.args)
        self.assertIn("--slurp", run.call_args.args)

    def test_absent_release_returns_none(self):
        with patch.object(release, "run", side_effect=[self.missing_tag(), "[[]]"]):
            self.assertIsNone(release.release_info("v1.2.3"))

    def test_authentication_failure_is_not_treated_as_missing(self):
        error = subprocess.CalledProcessError(1, ["gh", "api"], stderr="HTTP 401")
        with patch.object(release, "run", side_effect=error) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                release.release_info("v1.2.3")
        self.assertEqual(run.call_count, 1)


class LocalReleaseTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.output = self.root / "target/releases/v1.2.3"
        self.output.mkdir(parents=True)
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n')
        self.commit = "a" * 40
        self.calls = []
        self.published = False
        self.corrupt_download = False
        self.fail_upload = False
        self.remote_commit = self.commit
        for name in release.asset_names("1.2.3"):
            (self.output / name).write_bytes(b"fixture artifact")
            (self.output / (name + ".sha256")).write_text(f"{release.digest(self.output / name)}  {name}\n")
        (self.output / "release-notes.md").write_text("Release notes\n")
        self.manifest = {
            "format": 1, "tag": "v1.2.3", "commit": self.commit,
            "sha256": {name: release.digest(self.output / name) for name in release.manifest_files("1.2.3")},
        }
        self.save_manifest()
        for name, value in (("ROOT", self.root), ("RELEASES", self.root / "target/releases")):
            patcher = patch.object(release, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def save_manifest(self):
        (self.output / "manifest.json").write_text(json.dumps(self.manifest))

    def fake_run(self, *args, **kwargs):
        args = tuple(map(str, args))
        self.calls.append(args)
        if args[:2] == ("git", "rev-parse"):
            return self.commit
        if args[:2] == ("git", "remote"):
            return "git@github.com:jrmd/dbx.git"
        if args[:2] == ("git", "ls-remote"):
            return f"{self.remote_commit}\trefs/tags/v1.2.3" if self.remote_commit else ""
        if args[:2] == ("git", "push"):
            self.remote_commit = self.commit
        if args[:2] == ("gh", "api"):
            return json.dumps({"draft": not self.published, "assets": []})
        if args[:3] == ("gh", "release", "upload") and self.fail_upload:
            raise subprocess.CalledProcessError(1, args)
        if args[:3] == ("gh", "release", "download"):
            destination = Path(args[args.index("--dir") + 1])
            for name in release.manifest_files("1.2.3"):
                if name != "release-notes.md":
                    shutil.copy2(self.output / name, destination)
            if self.corrupt_download:
                (destination / release.asset_names("1.2.3")[0]).write_bytes(b"truncated")
        return ""

    def publish(self):
        with patch.object(release, "run", self.fake_run), patch("builtins.print"):
            release.publish("1.2.3", "v1.2.3")

    def assert_not_published(self):
        self.assertFalse(any("--draft=false" in call for call in self.calls))

    def test_publish_happens_only_after_download_verification(self):
        self.publish()
        self.assertIn("--draft=false", self.calls[-1])
        self.assertIn("--latest", self.calls[-1])
        self.assertTrue(any(call[:3] == ("gh", "release", "download") for call in self.calls[:-1]))
        self.assertFalse(any(call[:2] == ("git", "push") for call in self.calls))

    def test_missing_remote_tag_is_pushed_explicitly(self):
        self.remote_commit = None
        self.publish()
        self.assertIn(("git", "push", "origin", "refs/tags/v1.2.3:refs/tags/v1.2.3"), self.calls)

    def test_published_release_cannot_be_overwritten(self):
        self.published = True
        with self.assertRaisesRegex(ValueError, "already published"):
            self.publish()
        self.assertFalse(any(call[:2] == ("gh", "release") for call in self.calls))

    def test_mismatched_remote_tag_cannot_publish(self):
        self.remote_commit = "b" * 40
        with self.assertRaisesRegex(ValueError, "another commit"):
            self.publish()
        self.assertFalse(any(call[:2] == ("gh", "release") for call in self.calls))

    def test_failed_upload_stays_draft(self):
        self.fail_upload = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish()
        self.assert_not_published()

    def test_corrupt_download_stays_draft(self):
        self.corrupt_download = True
        with self.assertRaisesRegex(ValueError, "Uploaded asset failed"):
            self.publish()
        self.assert_not_published()

    def test_modified_asset_or_notes_rejected_before_network(self):
        for name in (release.asset_names("1.2.3")[0], "release-notes.md"):
            with self.subTest(name=name):
                original = (self.output / name).read_bytes()
                (self.output / name).write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "changed after validation"):
                    self.publish()
                self.assertFalse(any(call[0] == "gh" for call in self.calls))
                (self.output / name).write_bytes(original)

    def test_old_commit_manifest_rejected(self):
        self.manifest["commit"] = "b" * 40
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, "does not match"):
            self.publish()
        self.assert_not_published()

    def test_missing_asset_checksum_and_extra_manifest_entry_rejected(self):
        name = release.asset_names("1.2.3")[0] + ".sha256"
        (self.output / name).unlink()
        with self.assertRaises(FileNotFoundError):
            release.verify_manifest(self.output, "1.2.3", "v1.2.3", self.commit)
        self.manifest["sha256"]["unexpected.zip"] = "a" * 64
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, "exactly the expected"):
            release.verify_manifest(self.output, "1.2.3", "v1.2.3", self.commit)

    def test_checksum_cannot_name_another_file(self):
        name = release.asset_names("1.2.3")[0]
        (self.output / (name + ".sha256")).write_text(f"{release.digest(self.output / name)}  other.zip\n")
        with self.assertRaisesRegex(ValueError, "Invalid checksum"):
            release.verify_assets(self.output, "1.2.3")

    def test_dirty_tree_fails_before_doctor_or_build(self):
        with patch.object(release, "run", return_value=" M Cargo.toml"), patch.object(release, "doctor") as doctor:
            with self.assertRaisesRegex(ValueError, "working tree is dirty"):
                release.build("1.2.3", "v1.2.3")
            doctor.assert_not_called()

    def test_version_mismatch_and_prerelease_rejected(self):
        self.assertEqual(release.version_tag(), ("1.2.3", "v1.2.3"))
        with self.assertRaisesRegex(ValueError, "must match"):
            release.version_tag("v1.2.4")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion="1.2.3-beta.1"\n')
        with self.assertRaisesRegex(ValueError, "Only stable"):
            release.version_tag()


if __name__ == "__main__":
    unittest.main()
