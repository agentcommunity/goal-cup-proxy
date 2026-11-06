"""A verified tier may carry a stated note (plan Task 2). No network. Run: python3.11 -m unittest discover -s scripts/pad -p 'test_table_note.py'"""
import importlib.machinery
import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


table = load("padtable_note", "table")
NOTE = "subagents shown by their task logs, not tied to individual calls"
ROWS = [{"connector": "hermes-cli", "case": c, "mode": "full", "variant": [], "result": "PASS",
         "pad_trusted_at": "c" * 40, "harness_version": "0.22.5", "os": "darwin-arm64", "started": "2026-11-03T19:0%dZ" % i,
         "checks": {k: "PASS" for k in table.CHECKS}} for i, c in enumerate(table.CASES)]


class TierNote(unittest.TestCase):
    def test_verified_carries_the_note(self):
        with mock.patch.object(table, "harness_lines", return_value=['tier_note = "%s"' % NOTE, "[gateway]"]):
            self.assertEqual(table.tier_for("hermes-cli", ROWS), ("verified", "0.22.5", "darwin-arm64", NOTE))
            md = table.results_md(ROWS, {cid: table.tier_for(cid, ROWS) for cid in table.CONNECTOR_IDS})
        self.assertIn("verified with a note: " + NOTE, md)

    def test_note_never_hides_a_failure(self):
        rows = ROWS[:2] + [dict(ROWS[2], result="FAIL", checks=dict(ROWS[2]["checks"], C3="FAIL"), why=["C3 known hit"])]
        with mock.patch.object(table, "harness_lines", return_value=['tier_note = "%s"' % NOTE, "[gateway]"]):
            t = table.tier_for("hermes-cli", rows)
        self.assertEqual(t[0], "not verified")
        self.assertNotIn(NOTE, t[3])

    def test_no_note_is_plain_verified(self):
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            self.assertEqual(table.tier_for("hermes-cli", ROWS), ("verified", "0.22.5", "darwin-arm64", ""))


class ConnectorIds(unittest.TestCase):
    def test_ids_come_from_connectors_dir(self):
        with tempfile.TemporaryDirectory() as d:
            c = Path(d) / "connectors"
            c.mkdir()
            for n in ("a-cli.toml", "model-hosts.toml", "tiers.toml", ".x.toml", "notes.md"):
                (c / n).write_text("")
            (c / "dir.toml").mkdir()
            self.assertEqual(table.connector_ids(Path(d)), ["a-cli"])

    def test_ids_match_repo(self):
        want = sorted(p.stem for p in (table.ROOT / "connectors").glob("*.toml")
                      if p.stem not in ("model-hosts", "tiers") and not p.stem.startswith("."))
        self.assertEqual(table.CONNECTOR_IDS, want)
        self.assertEqual(len(want), 15)

    def test_opencode_has_one_line(self):
        rows = [dict(ROWS[0], connector="opencode-cli")]
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            md = table.results_md(rows, {cid: table.tier_for(cid, rows) for cid in table.CONNECTOR_IDS})
        lines = md.splitlines()
        self.assertEqual(sum(ln.startswith("| opencode-cli |") for ln in lines), 1)
        self.assertFalse(any(ln.startswith("| opencode |") for ln in lines))
        self.assertIn("## opencode-cli, runs", md)
        self.assertNotIn("## opencode, ", md)


if __name__ == "__main__":
    unittest.main()
