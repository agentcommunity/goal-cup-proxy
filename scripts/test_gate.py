#!/usr/bin/env python3
"""Behavioral tests for the test gate, scripts/gate.sh.

gate.sh runs against a stub `cargo` (and `python3`) placed first on PATH. The stub appends its
argv to a log, so each test reads the commands gate.sh would have run.
"""

from __future__ import annotations

import os
import re
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


GATE = Path(__file__).with_name("gate.sh")

# The stub logs "<tool> <argv>" per call. `cargo nextest --version` fails when STUB_NO_NEXTEST is
# set, and any call whose argv starts with $STUB_FAIL_ON fails.
STUB = """#!/bin/sh
echo "$(basename "$0") $*" >> "$STUB_LOG"
if [ "$1" = nextest ] && [ "$2" = --version ] && [ -n "${STUB_NO_NEXTEST:-}" ]; then
    exit 1
fi
if [ -n "${STUB_FAIL_ON:-}" ]; then
    case "$*" in
    "$STUB_FAIL_ON"*) exit 1 ;;
    esac
fi
exit 0
"""


class GateTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.log = root / "calls.log"
        self.log.touch()
        bindir = root / "bin"
        bindir.mkdir()
        for name in ("cargo", "python3"):
            stub = bindir / name
            stub.write_text(STUB)
            stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
        self.env = {
            **os.environ,
            "PATH": f"{bindir}{os.pathsep}{os.environ['PATH']}",
            "STUB_LOG": str(self.log),
        }
        self.env.pop("STUB_NO_NEXTEST", None)
        self.env.pop("STUB_FAIL_ON", None)

    def run_gate(self, tier: str, **extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", str(GATE), tier],
            env={**self.env, **extra},
            capture_output=True,
            text=True,
        )

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines()

    def nextest_runs(self) -> list[str]:
        return [c for c in self.calls() if c.startswith("cargo nextest run")]

    def test_fast_builds_workspace_before_tests(self) -> None:
        result = self.run_gate("fast")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls()[0], "cargo build --offline --workspace")

    def test_fast_excludes_full(self) -> None:
        self.run_gate("fast")
        runs = self.nextest_runs()
        self.assertEqual(len(runs), 1, runs)
        self.assertIn("-E not (binary(full) | binary(m2_cli_lifecycle))", runs[0])

    def test_fast_runs_everything_while_no_full_binary_exists(self) -> None:
        # nextest refuses a `binary(...)` filter when no binary has that name. The fallback is
        # announced on stderr, so a renamed binary does not silently slow the fast tier.
        result = self.run_gate("fast", STUB_FAIL_ON="nextest list")
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = self.nextest_runs()
        self.assertEqual(len(runs), 1, runs)
        self.assertNotIn("binary(full)", runs[0])
        self.assertIn("running EVERY test", result.stderr)
        self.assertNotIn("running EVERY test", result.stdout)

    def test_full_runs_everything(self) -> None:
        result = self.run_gate("full")
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = self.nextest_runs()
        self.assertEqual(len(runs), 1, runs)
        self.assertNotIn("binary(full)", runs[0])
        self.assertNotIn("-E", runs[0])
        self.assertIn("python3 -m unittest discover -s scripts", self.calls())
        self.assertIn("python3 scripts/check-links.py --root .", self.calls())

    def test_doctests_run_beside_nextest_but_not_twice_in_the_fallback(self) -> None:
        # nextest does not run doctests; `cargo test` (the fallback) already does.
        self.run_gate("fast")
        self.assertIn("cargo test --offline --workspace --doc", self.calls())
        self.log.write_text("")
        self.run_gate("fast", STUB_NO_NEXTEST="1")
        self.assertNotIn("cargo test --offline --workspace --doc", self.calls())

    def test_fast_skips_script_checks(self) -> None:
        self.run_gate("fast")
        self.assertEqual([c for c in self.calls() if c.startswith("python3")], [])

    def test_fast_runs_fmt_and_clippy(self) -> None:
        self.run_gate("fast")
        self.assertIn("cargo fmt --all --check", self.calls())
        self.assertIn(
            "cargo clippy --offline --workspace --all-targets -- -D warnings", self.calls()
        )

    def test_falls_back_without_nextest(self) -> None:
        result = self.run_gate("fast", STUB_NO_NEXTEST="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "cargo-nextest not found; falling back to cargo test (runs every tier, slower)",
            result.stdout.splitlines(),
        )
        self.assertIn("cargo test --offline --workspace", self.calls())
        self.assertEqual(self.nextest_runs(), [])

    def test_reports_pass_line(self) -> None:
        result = self.run_gate("fast")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout.splitlines()[-1], r"^gate fast: PASS in [0-9]+s$")
        full = self.run_gate("full")
        self.assertRegex(full.stdout.splitlines()[-1], r"^gate full: PASS in [0-9]+s$")

    def test_failure_stops_and_reports_fail_line(self) -> None:
        result = self.run_gate("fast", STUB_FAIL_ON="clippy")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout.splitlines()[-1], "gate fast: FAIL")
        self.assertEqual(self.nextest_runs(), [])

    def test_failing_tests_report_fail_line(self) -> None:
        result = self.run_gate("full", STUB_FAIL_ON="nextest run")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout.splitlines()[-1], "gate full: FAIL")
        self.assertEqual([c for c in self.calls() if c.startswith("python3")], [])

    def test_unknown_tier_is_a_usage_error(self) -> None:
        result = self.run_gate("slow")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.calls(), [])
        self.assertIsNone(re.search(r"^gate ", result.stdout, re.M))


if __name__ == "__main__":
    unittest.main()
