#!/usr/bin/env python3
"""Every path in .github/CODEOWNERS must exist, or code-owner review silently stops covering it."""

from __future__ import annotations

import glob
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CODEOWNERS = ROOT / ".github" / "CODEOWNERS"


def owned_paths(text: str) -> list[str]:
    """The path column of each rule line, comments and blank lines skipped."""
    paths = []
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            paths.append(line.split()[0])
    return paths


def missing(paths: list[str], root: Path) -> list[str]:
    """Paths that match nothing under root. A trailing slash must name a directory."""
    gone = []
    for path in paths:
        rel = path.lstrip("/")
        if any(ch in rel for ch in "*?["):
            if not glob.glob(str(root / rel)):
                gone.append(path)
            continue
        target = root / rel
        if rel.endswith("/") and not target.is_dir():
            gone.append(path)
        elif not target.exists():
            gone.append(path)
    return gone


class CodeownersTests(unittest.TestCase):
    def test_every_codeowners_path_exists(self) -> None:
        paths = owned_paths(CODEOWNERS.read_text(encoding="utf-8"))
        self.assertTrue(paths, "CODEOWNERS has no rules")
        self.assertEqual(missing(paths, ROOT), [], "CODEOWNERS names paths that do not exist")

    def test_a_removed_path_is_reported(self) -> None:
        paths = owned_paths("# comment\n\n/scripts/   @x\n/scripts/no-such-file.sh  @x\n/no-dir-*/ @x\n")
        self.assertEqual(missing(paths, ROOT), ["/scripts/no-such-file.sh", "/no-dir-*/"])

    def test_a_file_named_as_a_directory_is_reported(self) -> None:
        self.assertEqual(missing(["/README.md/"], ROOT), ["/README.md/"])


if __name__ == "__main__":
    unittest.main()
