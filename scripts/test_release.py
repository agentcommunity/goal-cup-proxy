#!/usr/bin/env python3
"""Behavioral tests for scripts/release.sh, run in a throwaway git repository.

The repository holds a one-crate workspace that inherits its version, plus a copy of
release.sh, so the script's own checks and its cargo and git calls run for real.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

RELEASE = Path(__file__).with_name("release.sh")
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid"]


@unittest.skipUnless(shutil.which("cargo") and shutil.which("git"), "needs cargo and git")
class ReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.repo = Path(tmp.name)
        (self.repo / "crates/c/src").mkdir(parents=True)
        (self.repo / "scripts").mkdir()
        (self.repo / "Cargo.toml").write_text(
            '[workspace]\nresolver = "3"\nmembers = ["crates/c"]\n\n'
            '[workspace.package]\n# the only version\nversion = "0.6.0"\nedition = "2024"\n'
        )
        (self.repo / "crates/c/Cargo.toml").write_text(
            '[package]\nname = "c"\nversion.workspace = true\nedition.workspace = true\n'
        )
        (self.repo / "crates/c/src/lib.rs").write_text("")
        shutil.copy(RELEASE, self.repo / "scripts/release.sh")
        self.git("init", "-q", "-b", "main")
        self.cargo("generate-lockfile", "--offline")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "start")

    def git(self, *args: str) -> str:
        return subprocess.run(
            [*GIT, *args], cwd=self.repo, check=True, capture_output=True, text=True
        ).stdout

    def cargo(self, *args: str) -> None:
        subprocess.run(["cargo", *args], cwd=self.repo, check=True, capture_output=True)

    def release(self, *args: str) -> subprocess.CompletedProcess[str]:
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.invalid",
        }
        return subprocess.run(
            ["sh", "scripts/release.sh", *args],
            cwd=self.repo,
            capture_output=True,
            text=True,
            env=env,
        )

    def refused(self, *args: str) -> str:
        result = self.release(*args)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.git("tag", "--list"), "")
        self.assertEqual(self.git("rev-list", "--count", "HEAD").strip(), "1")
        return result.stderr

    def test_sets_the_version_updates_the_lock_commits_and_tags(self) -> None:
        result = self.release("0.7.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "0.7.0"', (self.repo / "Cargo.toml").read_text())
        self.assertIn('name = "c"\nversion = "0.7.0"', (self.repo / "Cargo.lock").read_text())
        self.assertEqual(self.git("tag", "--list").strip(), "v0.7.0")
        self.assertEqual(self.git("log", "-1", "--format=%s").strip(), "Release 0.7.0")
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertEqual(self.git("rev-parse", "v0.7.0^{commit}"), self.git("rev-parse", "HEAD"))

    def test_refuses_a_version_that_is_not_higher(self) -> None:
        self.assertIn("not higher", self.refused("0.6.0"))
        self.assertIn("not higher", self.refused("0.5.9"))

    def test_refuses_a_malformed_version(self) -> None:
        self.assertIn("x.y.z", self.refused("0.7"))
        self.assertIn("x.y.z", self.refused("v0.7.0"))

    def test_refuses_a_dirty_tree_and_a_branch_that_is_not_main(self) -> None:
        (self.repo / "stray.txt").write_text("x")
        self.assertIn("not clean", self.refused("0.7.0"))
        (self.repo / "stray.txt").unlink()
        self.git("switch", "-q", "-c", "topic")
        self.assertIn("run this on main", self.refused("0.7.0"))


if __name__ == "__main__":
    unittest.main()
