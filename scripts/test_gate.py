"""gate.sh parts, nextest partition and --list: the parts together are exactly `fast`."""
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

    def test_full_refuses_a_part(self):
        self.assertEqual(gate("full", "--part", "lint", "--list").returncode, 2)


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
