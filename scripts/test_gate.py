"""gate.sh parts, nextest partition and --list: the parts together are exactly `fast`."""
import subprocess
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


if __name__ == "__main__":
    unittest.main()
