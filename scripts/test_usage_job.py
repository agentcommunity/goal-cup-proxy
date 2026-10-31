#!/usr/bin/env python3
"""Behavioral tests for scripts/usage-job.sh, run.json after a `continue`.

    python3 scripts/test_usage_job.py

No database, no R2, no verifier: a copy of the job sits next to a fake fetch-run.sh and a fake
goalcup-verify, the index is a JSON file (`--index`) and a directory stands in for the usage bucket
(`--usage-store`). This covers the job's own rollup and put rules, not a real staging run.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

JOB = Path(__file__).with_name("usage-job.sh")
RUN = "r0123456789abcdef"
E1 = "e000000000000001"
E2 = "e000000000000002"
EVENT = "ev1"

FAKE_FETCH = """#!/bin/sh
# fake fetch-run.sh: last two arguments are the entry and the outdir
for a; do last2=${last1:-}; last1=$a; done
mkdir -p "$last1/$last2"
seg=1; [ "$last2" = "%s" ] && seg=2
printf '{"segment": %%s}' "$seg" >"$last1/$last2/fetch.json"
echo "$last1/$last2"
"""

FAKE_VERIFY = """#!/bin/sh
if [ "$1" = "--version" ]; then echo "goalcup-verify 1.0.0"; exit 0; fi
in=$2; out=$5; entry=$(basename "$in"); mkdir -p "$out"
cat >"$out/$entry.json" <<JSON
{"reader":"1.0.0","threads":[{"id":"t1","wire_calls":1}],"tokens":{"merged":{"input":10},"calls_without_usage":0}}
JSON
: >"$out/$entry.calls.jsonl"
"""


class UsageJob(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        scripts = self.tmp / "scripts"
        scripts.mkdir()
        shutil.copy(JOB, scripts / "usage-job.sh")
        (scripts / "fetch-run.sh").write_text(FAKE_FETCH % E2)
        verify = self.tmp / "verify"
        verify.write_text(FAKE_VERIFY)
        verify.chmod(0o755)
        self.verify = verify
        self.store = self.tmp / "usage"
        self.index = self.tmp / "index.json"

    def run_job(self, entries: list[str]) -> str:
        rows = [{"entry_id": e, "run_id": RUN, "event": EVENT, "sealed_at": "2026-10-31T00:00:00Z"} for e in entries]
        self.index.write_text(json.dumps(rows))
        env = {"PATH": os.environ["PATH"], "GOALCUP_VERIFY": str(self.verify)}
        done = subprocess.run(
            ["sh", str(self.tmp / "scripts" / "usage-job.sh"), "--index", str(self.index), "--usage-store", str(self.store)],
            env=env, capture_output=True, text=True, cwd=self.tmp,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        return done.stdout

    def run_json(self) -> dict:
        return json.loads((self.store / "usage" / EVENT / RUN / "run.json").read_text())

    def test_run_json_includes_an_entry_sealed_later(self) -> None:
        self.run_job([E1])
        self.assertEqual([x["entry"] for x in self.run_json()["entries"]], [E1])
        self.assertEqual(self.run_json()["threads"][0]["wire_calls"], 1)
        out = self.run_job([E1, E2])
        self.assertIn(f"put usage/{EVENT}/{RUN}/run.json", out)
        doc = self.run_json()
        self.assertEqual([x["entry"] for x in doc["entries"]], [E1, E2])
        self.assertEqual(doc["threads"][0]["wire_calls"], 2)
        self.assertEqual(doc["tokens"]["merged"]["input"], 20)

    def test_a_rerun_puts_nothing(self) -> None:
        self.run_job([E1, E2])
        self.assertNotIn("put ", self.run_job([E1, E2]))


if __name__ == "__main__":
    unittest.main()
