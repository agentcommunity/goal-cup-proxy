#!/usr/bin/env python3
"""Behavioral tests for scripts/linux-check.sh, run against a stub `docker` on PATH.

The stub logs its argv and succeeds, so the script walks every step; each test reads the
`docker run` line to see what the container would have mounted.
"""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("linux-check.sh")
GIT_ENV = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@t",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@t",
}


class LinuxCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name).resolve()
        self.log = self.base / "docker.log"
        self.log.touch()
        bindir = self.base / "bin"
        bindir.mkdir()
        stub = bindir / "docker"
        stub.write_text('#!/bin/sh\necho "$*" >> "$STUB_LOG"\nexit 0\n')
        stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
        (self.base / "cargo/registry").mkdir(parents=True)
        self.env = {
            **os.environ,
            **GIT_ENV,
            "PATH": f"{bindir}:{os.environ['PATH']}",
            "STUB_LOG": str(self.log),
            "CARGO_HOME": str(self.base / "cargo"),
        }

    def git(self, cwd: Path, *args: str) -> None:
        subprocess.run(["git", *args], cwd=cwd, check=True, env=self.env, capture_output=True)

    def checkout(self, name: str) -> Path:
        """A git checkout holding a copy of linux-check.sh under scripts/."""
        root = self.base / name
        (root / "scripts").mkdir(parents=True)
        (root / "scripts/linux-check.sh").write_text(SCRIPT.read_text())
        self.git(root, "init", "-q")
        self.git(root, "add", "-A")
        self.git(root, "-c", "commit.gpgsign=false", "commit", "-q", "-m", "x")
        return root

    def run_check(self, root: Path) -> tuple[subprocess.CompletedProcess, str]:
        run = subprocess.run(
            ["sh", "scripts/linux-check.sh"], cwd=root, env=self.env, capture_output=True, text=True
        )
        run_lines = [x for x in self.log.read_text().splitlines() if x.startswith("run ")]
        self.assertEqual(len(run_lines), 1, run.stdout + self.log.read_text())
        return run, run_lines[0]

    def test_a_plain_checkout_mounts_only_itself(self) -> None:
        root = self.checkout("plain")
        run, docker_run = self.run_check(root)
        self.assertTrue(run.stdout.endswith("linux-check: PASS\n"), run.stdout)
        self.assertIn(f"-v {root}:/src", docker_run)
        self.assertNotIn(":ro", docker_run)

    def test_a_plain_checkout_is_named_safe_for_git_in_the_container(self) -> None:
        # Docker Desktop may show the mounted files owned by root; git as `dev` then refuses them.
        root = self.checkout("plain-safe")
        _, docker_run = self.run_check(root)
        self.assertIn("-e GIT_CONFIG_COUNT=1 ", docker_run)
        self.assertIn("-e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0=/src ", docker_run)
        self.assertNotIn("GIT_CONFIG_KEY_1", docker_run)

    def test_a_linked_worktree_mounts_the_main_git_directory_read_only(self) -> None:
        main = self.checkout("main")
        linked = self.base / "linked"
        self.git(main, "worktree", "add", "-q", str(linked))
        self.assertTrue((linked / ".git").is_file())
        run, docker_run = self.run_check(linked)
        self.assertTrue(run.stdout.endswith("linux-check: PASS\n"), run.stdout)
        common = main / ".git"
        self.assertIn(f"-v {common}:{common}:ro", docker_run)
        self.assertIn(f"-v {linked}:/src", docker_run)

    def test_a_linked_worktree_and_its_mounted_git_directory_are_named_safe(self) -> None:
        main = self.checkout("main-safe")
        linked = self.base / "linked-safe"
        self.git(main, "worktree", "add", "-q", str(linked))
        _, docker_run = self.run_check(linked)
        self.assertIn("-e GIT_CONFIG_COUNT=2 ", docker_run)
        self.assertIn("-e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0=/src ", docker_run)
        self.assertIn(
            f"-e GIT_CONFIG_KEY_1=safe.directory -e GIT_CONFIG_VALUE_1={main / '.git'} ", docker_run
        )


if __name__ == "__main__":
    unittest.main()
