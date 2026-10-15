#!/usr/bin/env python3
"""Behavioral tests for scripts/release.sh, run in throwaway git repositories.

A bare repository stands in for GitHub's `origin`; the working clone holds a one-crate workspace
that inherits its version and a copy of release.sh, so the script's checks and its cargo and git
calls run for real. `gh` is a stub that logs its arguments.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

RELEASE = Path(__file__).with_name("release.sh")
IDENTITY = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.invalid",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.invalid",
}


@unittest.skipUnless(shutil.which("cargo") and shutil.which("git"), "needs cargo and git")
class ReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.origin = root / "origin.git"
        self.repo = root / "clone"
        self.gh_log = root / "gh.log"
        self.gh_log.touch()
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(self.origin)], check=True)
        self.repo.mkdir()
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
        self.set_version_check(passes=True)
        bindir = root / "bin"
        bindir.mkdir()
        gh = bindir / "gh"
        gh.write_text(f'#!/bin/sh\necho "$*" >> "{self.gh_log}"\n[ -z "$GH_FAIL" ]\n')
        gh.chmod(gh.stat().st_mode | stat.S_IXUSR)
        self.env = {**os.environ, **IDENTITY, "PATH": f"{bindir}{os.pathsep}{os.environ['PATH']}"}
        self.git("init", "-q", "-b", "main")
        self.cargo("generate-lockfile", "--offline")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "start")
        self.git("remote", "add", "origin", str(self.origin))
        self.git("push", "-q", "-u", "origin", "main")

    def set_version_check(self, passes: bool) -> None:
        (self.repo / "scripts/check-version.py").write_text(f"import sys\nsys.exit({0 if passes else 1})\n")

    def git(self, *args: str, cwd: Path | None = None) -> str:
        return subprocess.run(
            ["git", *args], cwd=cwd or self.repo, check=True, capture_output=True, text=True, env=self.env
        ).stdout

    def origin_git(self, *args: str) -> str:
        return self.git("--git-dir", str(self.origin), *args)

    def cargo(self, *args: str) -> None:
        subprocess.run(["cargo", *args], cwd=self.repo, check=True, capture_output=True)

    def release(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", "scripts/release.sh", *args],
            cwd=self.repo, capture_output=True, text=True, env=self.env,
        )

    def merge_version_into_origin_main(self, version: str) -> None:
        """What merging the release pull request does: origin/main gets the new version."""
        text = (self.repo / "Cargo.toml").read_text().replace('version = "0.6.0"', f'version = "{version}"')
        (self.repo / "Cargo.toml").write_text(text)
        self.cargo("update", "--workspace", "--offline")
        self.git("commit", "-q", "-am", f"bump {version}")
        self.git("push", "-q", "origin", "main")

    # prepare

    def test_prepare_opens_a_pull_request_from_a_release_branch(self) -> None:
        result = self.release("prepare", "0.7.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        branch = "release/v0.7.0"
        self.assertEqual(self.origin_git("log", "-1", "--format=%s", branch).strip(), "Release 0.7.0")
        self.assertIn('version = "0.7.0"', self.origin_git("show", f"{branch}:Cargo.toml"))
        self.assertIn('name = "c"\nversion = "0.7.0"', self.origin_git("show", f"{branch}:Cargo.lock"))
        self.assertEqual(self.origin_git("rev-parse", "main"), self.origin_git("rev-parse", f"{branch}~1"))
        self.assertIn("pr create --base main --head release/v0.7.0", self.gh_log.read_text())
        # main is never pushed, and the clone is back where it started, clean and at 0.6.0.
        self.assertEqual(self.git("branch", "--show-current").strip(), "main")
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertIn('version = "0.6.0"', (self.repo / "Cargo.toml").read_text())

    def test_prepare_does_nothing_when_origin_main_already_has_the_version(self) -> None:
        result = self.release("prepare", "0.6.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already at version 0.6.0", result.stdout)
        self.assertEqual(self.origin_git("branch", "--list", "release/*"), "")
        self.assertEqual(self.gh_log.read_text(), "")

    def refused(self, *args: str) -> None:
        result = self.release(*args)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.origin_git("branch", "--list", "release/*"), "")
        self.assertEqual(self.origin_git("tag", "--list"), "")
        self.assertEqual(self.gh_log.read_text(), "")

    def test_prepare_refuses_a_lower_or_malformed_version(self) -> None:
        self.refused("prepare", "0.5.9")
        self.refused("prepare", "0.7")
        self.refused("prepare", "v0.7.0")

    def test_prepare_refuses_a_dirty_tree(self) -> None:
        (self.repo / "stray.txt").write_text("x")
        self.refused("prepare", "0.7.0")

    def test_a_failing_version_check_leaves_no_branch_and_no_change(self) -> None:
        self.set_version_check(passes=False)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "failing check")
        self.git("push", "-q", "origin", "main")
        self.refused("prepare", "0.7.0")
        self.assertEqual(self.git("branch", "--list", "release/*"), "")
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertEqual(self.git("branch", "--show-current").strip(), "main")

    def test_a_failed_pull_request_leaves_the_owner_back_on_their_branch_with_the_next_step(self) -> None:
        self.env["GH_FAIL"] = "1"
        result = self.release("prepare", "0.7.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gh pr create --base main --head release/v0.7.0", result.stderr)
        self.assertEqual(self.git("branch", "--show-current").strip(), "main")
        self.assertEqual(self.git("status", "--porcelain"), "")

    # tag

    def test_tag_puts_the_tag_on_origin_main_and_pushes_only_the_tag(self) -> None:
        main_before = self.origin_git("rev-parse", "main")
        # A local commit that is not on origin/main must not be tagged.
        (self.repo / "local.txt").write_text("x")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "local only")
        result = self.release("tag", "0.6.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.origin_git("tag", "--list"), "v0.6.0\n")
        self.assertEqual(self.origin_git("rev-parse", "v0.6.0^{commit}"), main_before)
        self.assertEqual(self.origin_git("rev-parse", "main"), main_before)

    def test_tag_refuses_when_origin_main_has_another_version(self) -> None:
        result = self.release("tag", "0.7.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("prepare 0.7.0", result.stderr)
        self.assertEqual(self.origin_git("tag", "--list"), "")
        self.assertEqual(self.git("tag", "--list"), "")

    def test_tag_follows_a_merged_release_and_refuses_a_second_tag(self) -> None:
        self.merge_version_into_origin_main("0.7.0")
        self.assertEqual(self.release("tag", "0.7.0").returncode, 0)
        again = self.release("tag", "0.7.0")
        self.assertNotEqual(again.returncode, 0)
        self.assertIn("already exists", again.stderr)


if __name__ == "__main__":
    unittest.main()
