#!/usr/bin/env python3
"""Behavioral tests for scripts/check-version.py, run on small throwaway git repositories."""

from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-version.py")
spec = importlib.util.spec_from_file_location("check_version", SCRIPT)
check_version = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_version)

CARGO = '[workspace]\nmembers = ["crates/c"]\n\n[workspace.package]\nversion = "0.6.0"\n'
CRATE = '[package]\nname = "goalcup-c"\nversion.workspace = true\n'
LOCK = 'version = 4\n\n[[package]]\nname = "goalcup-c"\nversion = "0.6.0"\n'


class CheckVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.write("Cargo.toml", CARGO)
        self.write("crates/c/Cargo.toml", CRATE)
        self.write("Cargo.lock", LOCK)
        self.write("docs/GUIDE.md", "Install the latest release.\n")

    def write(self, name: str, text: str) -> None:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def findings(self) -> list[str]:
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True)
        return check_version.check(self.root)

    def test_a_tree_with_one_version_is_clean(self) -> None:
        self.assertEqual(self.findings(), [])

    def test_this_repository_is_clean(self) -> None:
        self.assertEqual(check_version.check(SCRIPT.parent.parent), [])

    def test_a_tree_git_cannot_list_fails_instead_of_passing(self) -> None:
        # A linked worktree whose gitdir is missing (the case inside a container that does not
        # mount the main repository): `git ls-files` exits 128. It must not pass and not crash.
        (self.root / ".git").write_text("gitdir: /nonexistent/main/.git/worktrees/x\n")
        with self.assertRaises(check_version.NoFileList):
            check_version.check(self.root)
        run = subprocess.run(
            ["python3", str(SCRIPT), "--root", str(self.root)], capture_output=True, text=True
        )
        self.assertEqual(run.returncode, 2, run.stderr)
        self.assertIn("cannot list the tracked files", run.stderr)
        self.assertNotIn("Traceback", run.stderr)

    def test_a_linked_worktree_is_checked_like_any_checkout(self) -> None:
        self.assertEqual(self.findings(), [])
        env = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t",
               "GIT_COMMITTER_EMAIL": "t@t"}
        git = ["git", "-c", "commit.gpgsign=false"]
        subprocess.run([*git, "commit", "-q", "-m", "x"], cwd=self.root, check=True, env=env)
        linked = self.root / "linked"
        subprocess.run(["git", "worktree", "add", "-q", str(linked)], cwd=self.root, check=True)
        (linked / "docs/GUIDE.md").write_text("goalcup 0.6.0 is out.\n")
        subprocess.run(["git", "add", "-A"], cwd=linked, check=True)
        found = check_version.check(linked)
        self.assertIn("docs/GUIDE.md:1: writes the current version 0.6.0", found)

    def test_a_crate_with_its_own_version_is_flagged(self) -> None:
        self.write("crates/c/Cargo.toml", '[package]\nname = "goalcup-c"\nversion = "0.6.0"\n')
        found = self.findings()
        self.assertTrue(any("version.workspace = true" in f for f in found), found)
        self.assertTrue(any("version of its own" in f for f in found), found)

    def test_a_stale_lock_file_is_flagged(self) -> None:
        self.write("Cargo.lock", LOCK.replace("0.6.0", "0.5.0"))
        self.assertEqual(
            self.findings(), ["Cargo.lock: goalcup-c is 0.5.0, the workspace is 0.6.0"]
        )

    def test_the_current_version_in_a_doc_is_flagged(self) -> None:
        self.write("docs/GUIDE.md", "Install goalcup 0.6.0 today.\n")
        found = self.findings()
        self.assertTrue(any(f.startswith("docs/GUIDE.md:1:") for f in found), found)

    def test_an_old_release_tag_in_a_doc_is_flagged_but_the_allowed_one_is_not(self) -> None:
        self.write("docs/GUIDE.md", "Use the v0.3.1 release.\n")
        self.write("docs/CONTRACTS.md", "The freeze took effect at the `v0.5.0` tag.\n")
        found = self.findings()
        self.assertEqual(len(found), 1, found)
        self.assertIn("docs/GUIDE.md:1", found[0])

    def test_a_version_field_with_a_literal_in_source_is_flagged(self) -> None:
        self.write("crates/c/src/lib.rs", 'let v = Req { tracer_version: "0.2.0" };\n')
        found = self.findings()
        self.assertTrue(any("tracer_version" in f for f in found), found)

    def test_a_proxy_version_field_or_the_product_name_with_the_version_is_flagged(self) -> None:
        self.write("crates/c/src/lib.rs", 'let v = Req { proxy_version: "0.2.0" };\n')
        self.write("docs/GUIDE.md", "Goal Cup Proxy 0.6.0 is out. The recording proxy since v0.3.1.\n")
        found = self.findings()
        self.assertTrue(any("proxy_version" in f for f in found), found)
        self.assertIn("docs/GUIDE.md:1: writes the current version 0.6.0", found)
        self.assertIn("docs/GUIDE.md:1: writes the release tag v0.3.1", found)

    def test_a_tarball_name_with_the_current_version_is_flagged(self) -> None:
        self.write("docs/GUIDE.md", "Download goalcup-0.6.0-aarch64-apple-darwin.tar.gz.\n")
        self.assertEqual(len(self.findings()), 1)

    def test_the_same_digits_outside_goalcups_context_are_not_flagged(self) -> None:
        self.write("docs/GUIDE.md", "The fake harness `matrix-fake-tui 0.6.0` and Node v22.18.0.\n")
        self.write("crates/c/src/lib.rs", '// /opt/homebrew/Cellar/goalcup/0.6.0/bin/goalcup\n')
        self.assertEqual(self.findings(), [])

    def test_tests_fixtures_and_marker_versions_are_not_scanned(self) -> None:
        self.write("crates/c/tests/it/a.rs", 'tracer_version: "0.1.0"; // 0.6.0\n')
        self.write("crates/c/src/mock.rs", 'daemon_version: "0.0.0-mock".to_string(),\n')
        self.write("docs/harnesses/x.md", "measured on 0.6.0\n")
        self.assertEqual(self.findings(), [])


if __name__ == "__main__":
    unittest.main()
