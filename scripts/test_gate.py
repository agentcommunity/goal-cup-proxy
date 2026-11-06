"""gate.sh parts, nextest partition and --list: the parts together are exactly `fast`."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

GATE = str(Path(__file__).resolve().parent / "gate.sh")


def gate(*args):
    return subprocess.run(["sh", GATE, *args], capture_output=True, text=True)


def listed(*args):
    r = gate(*args, "--list")
    assert r.returncode == 0, r.stderr
    return r.stdout.splitlines()


class GateParts(unittest.TestCase):
    def test_parts_cover_fast_exactly(self):
        parts = []
        for p in ("lint", "rust", "rest"):
            parts += listed("fast", "--part", p)
        self.assertEqual(sorted(parts), sorted(listed("fast")))

    def test_partition_reaches_nextest(self):
        lines = listed("fast", "--part", "rust", "--partition", "2/3")
        self.assertIn("cargo nextest run --locked --workspace --partition hash:2/3", lines)

    def test_partition_needs_rust_part(self):
        self.assertEqual(gate("fast", "--partition", "1/3", "--list").returncode, 2)
        self.assertEqual(gate("fast", "--part", "lint", "--partition", "1/3", "--list").returncode, 2)

    def test_full_list_exits_0_with_no_release_build(self):
        self.assertIn("cargo build --release --locked -p goalcup -p goalcup-verify", listed("full"))

    def test_full_refuses_a_part(self):
        self.assertEqual(gate("full", "--part", "lint", "--list").returncode, 2)


class Prerequisites(unittest.TestCase):
    """The real gate on a PATH without cargo, npm or python: it stops at the `missing:` line, no step runs."""

    def run_gate(self, *args):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        bin_dir = Path(tmp.name, "bin")
        bin_dir.mkdir()
        for tool in ("dirname", "date", "id"):
            bin_dir.joinpath(tool).symlink_to(shutil.which(tool))
        pg_bin = Path(tmp.name, "pg")
        pg_bin.mkdir()
        env = dict(os.environ, PATH=str(bin_dir), PG_BIN=str(pg_bin), GOALCUP_GATE_ALLOW_ROOT="1")
        return subprocess.run([shutil.which("sh"), GATE, *args], capture_output=True, text=True, env=env)

    def test_full_needs_initdb(self):
        r = self.run_gate("full")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("missing:", r.stdout)
        self.assertIn("initdb(install PostgreSQL", r.stdout)

    def test_fast_rest_skips_sql_without_initdb(self):
        r = self.run_gate("fast", "--part", "rest")
        self.assertIn("missing:", r.stdout)
        self.assertNotIn("initdb", r.stdout)

    def test_check_links_tests_listed(self):
        self.assertTrue(any("scripts/test_check_links.py" in ln for ln in listed("fast", "--part", "rest")))


class PathGuard(unittest.TestCase):
    """The personal-path step as --list prints it, run in a throwaway repo."""

    def guard(self, text):
        cmd = [ln for ln in listed("fast", "--part", "rest") if "git grep" in ln][0]
        self.assertTrue(cmd.startswith("sh -c "))
        with tempfile.TemporaryDirectory() as tmp:
            run = lambda *a: subprocess.run(a, cwd=tmp, capture_output=True, text=True)
            run("git", "init", "-q")
            if text is not None:
                Path(tmp, "f.txt").write_text(text)
                run("git", "add", "f.txt")
            else:  # not a repo: git grep itself errors
                Path(tmp, ".git").rename(Path(tmp, "gone"))
            return run("sh", "-c", cmd[len("sh -c "):]).returncode

    def test_stand_in_passes(self):
        self.assertEqual(self.guard("see /Users/placeholder/x\n"), 0)

    def test_no_match_passes(self):
        self.assertEqual(self.guard("nothing here\n"), 0)

    def test_personal_path_beside_stand_in_fails(self):
        self.assertNotEqual(self.guard("/Users/placeholder/x and /Users/" + "alice/y\n"), 0)

    def test_git_grep_error_fails(self):
        self.assertNotEqual(self.guard(None), 0)


if __name__ == "__main__":
    unittest.main()
