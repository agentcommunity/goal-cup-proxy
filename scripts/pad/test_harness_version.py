from __future__ import annotations

import os
import unittest

from harness_version import matches, parse

FIXTURE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates", "goalcup", "tests", "fixtures",
                       "version-lines.tsv")


class TestFixture(unittest.TestCase):
    def test_fixture_lines_parse(self):
        with open(FIXTURE, encoding="utf-8") as f:
            rows = [line.rstrip("\n").split("\t") for line in f if line.strip() and not line.startswith("#")]
        for _, raw, exp in rows:
            if exp == "PLACEHOLDER":
                continue
            with self.subTest(raw=raw):
                self.assertEqual(parse(raw), None if exp == "-" else exp)


class TestMatches(unittest.TestCase):
    def test_specs_match(self):
        self.assertTrue(matches("2.2.2", "2."))
        self.assertFalse(matches("10.0.0", "1."))
        self.assertFalse(matches("1.19.5", "2."))
        self.assertTrue(matches("1.19.5", "1.x"))
        self.assertTrue(matches("0.22.6", "0.22.6"))
        self.assertFalse(matches("0.22.60", "0.22.6"))
        self.assertTrue(matches("0.22.6", "0.22"))
        self.assertTrue(matches("0.22.5", ">=0.22.5,<0.23"))
        self.assertFalse(matches("0.23.0", ">=0.22.5,<0.23"))
        self.assertTrue(matches("1.0.105", "=1.0.105"))
        self.assertFalse(matches("1.0.0", ""))
        self.assertFalse(matches("1.0.0", ">=x"))


if __name__ == "__main__":
    unittest.main()
