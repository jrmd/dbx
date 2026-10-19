#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("release_notes", Path(__file__).with_name("release-notes.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReleaseNotesTests(unittest.TestCase):
    def test_exact_version_and_section_boundary(self):
        text = "# Changelog\n\n## Unreleased\n\nFuture\n\n## 1.2.30 - 2026-10-19\n\nOther\n\n## 1.2.3 - 2026-10-18\n\n- Fixed.\n\n### Details\n\nPreserved.\n\n## 1.2.2\n\nOlder\n"
        self.assertEqual(module.release_notes(text, "v1.2.3"), "## What's new in v1.2.3\n\n- Fixed.\n\n### Details\n\nPreserved.\n")

    def test_missing_or_empty_notes_block_publication(self):
        for text, tag in [("## 1.0.0\n", "v1.0.0"), ("## 1.0.0\nNotes", "v1.0.1"), ("", "main")]:
            with self.assertRaises(ValueError):
                module.release_notes(text, tag)


if __name__ == "__main__":
    unittest.main()
