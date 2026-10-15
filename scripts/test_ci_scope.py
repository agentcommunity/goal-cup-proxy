#!/usr/bin/env python3
"""Behavioral tests for the CI scope classifier, scripts/ci-scope.py."""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("ci-scope.py")
SPEC = importlib.util.spec_from_file_location("ci_scope", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
scope = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(scope)

# Fixture repositories ignore the caller's Git configuration, so a developer's hooks, signing
# or templates cannot change what the tests see. Every inherited Git control (GIT_DIR,
# GIT_WORK_TREE, GIT_CONFIG_COUNT and any future one) is dropped first.
GIT_ENV = {
    **{name: value for name, value in os.environ.items() if not name.startswith("GIT_")},
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_TERMINAL_PROMPT": "0",
}


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=repo, env=GIT_ENV, check=True, capture_output=True, text=True
    ).stdout.strip()


def write_files(root: Path, files: dict[str, bytes]) -> None:
    for name, body in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)


def make_history(before: dict[str, bytes], after: dict[str, bytes]) -> tuple[Path, str, str]:
    """Commit `before`, then replace the whole tree with `after`, in a fresh temporary repo."""
    holder = tempfile.TemporaryDirectory()
    repo = Path(holder.name)
    unittest.addModuleCleanup(holder.cleanup)
    git(repo, "init", "-q")
    git(repo, "config", "user.name", "ci-scope test")
    git(repo, "config", "user.email", "ci-scope@example.invalid")
    git(repo, "config", "commit.gpgsign", "false")
    write_files(repo, before)
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "--allow-empty", "-m", "before")
    base = git(repo, "rev-parse", "HEAD")
    git(repo, "rm", "-q", "-r", "--ignore-unmatch", ".")
    write_files(repo, after)
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "--allow-empty", "-m", "after")
    head = git(repo, "rev-parse", "HEAD")
    return repo, base, head


def run_scope(repo: Path, base: str, head: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--base", base, "--head", head],
        cwd=repo,
        env=GIT_ENV,
        capture_output=True,
        text=True,
        timeout=60,
    )


class ClassifierTests(unittest.TestCase):
    def test_docs_only(self) -> None:
        self.assertFalse(scope.requires_rust(["README.md", "docs/OPERATING.md"]))
        self.assertFalse(scope.requires_rust([".github/PULL_REQUEST_TEMPLATE.md"]))
        self.assertFalse(scope.requires_rust([]))

    def test_code_and_gate_inputs(self) -> None:
        for path in ["crates/x/src/lib.rs", "connectors/x/harness.toml", "Cargo.toml",
                     "Cargo.lock", "rust-toolchain.toml", "scripts/ci-scope.py",
                     "packaging/README.md", ".github/workflows/ci.yml"]:
            self.assertTrue(scope.requires_rust([path]), path)

    def test_mixed(self) -> None:
        self.assertTrue(scope.requires_rust(["README.md", "crates/x/src/lib.rs"]))


class DiffTests(unittest.TestCase):
    def test_deleted_and_renamed(self) -> None:
        for after in [{}, {"docs/moved.md": b"x"}]:
            repo, base, head = make_history({"crates/x/src/lib.rs": b"x"}, after)
            result = run_scope(repo, base, head)
            self.assertEqual((result.returncode, result.stdout), (0, "rust=true\n"))

    def test_unknown_diff_runs_rust(self) -> None:
        repo, base, head = make_history({"README.md": b"old"}, {"README.md": b"new"})
        for bad in ["", "0" * 40, "missing-ref"]:
            result = run_scope(repo, bad, head)
            self.assertEqual((result.returncode, result.stdout), (0, "rust=true\n"))

    def test_malformed_diff_is_unknown(self) -> None:
        for raw in [b"README.md", b"\xff\0"]:
            with mock.patch.object(scope.subprocess, "check_output", return_value=raw):
                self.assertIsNone(scope.changed_paths("1" * 40, "2" * 40))

    def test_nul_paths(self) -> None:
        with mock.patch.object(scope.subprocess, "check_output",
                               return_value=b"crates/x\nname.rs\0"):
            self.assertEqual(scope.changed_paths("1" * 40, "2" * 40), ["crates/x\nname.rs"])

    def test_git_failure_is_unknown(self) -> None:
        failure = subprocess.CalledProcessError(128, ["git", "diff"])
        with mock.patch.object(scope.subprocess, "check_output", side_effect=failure):
            self.assertIsNone(scope.changed_paths("1" * 40, "2" * 40))

    def test_cli_docs_and_code(self) -> None:
        repo, base, head = make_history({"README.md": b"old"},
                                        {"README.md": b"new", "docs/x.md": b"x"})
        result = run_scope(repo, base, head)
        self.assertEqual((result.returncode, result.stdout), (0, "rust=false\n"))
        repo, base, head = make_history({"README.md": b"old"},
                                        {"README.md": b"old", "crates/x/src/lib.rs": b"x"})
        result = run_scope(repo, base, head)
        self.assertEqual((result.returncode, result.stdout), (0, "rust=true\n"))


if __name__ == "__main__":
    unittest.main()
