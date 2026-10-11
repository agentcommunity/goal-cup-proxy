#!/usr/bin/env python3
"""Behavioral tests for scripts/check-connector.sh, with a fake cargo and uname on PATH.

    python3 -m unittest discover -s scripts -p 'test_check_connector.py'

Each test copies the script into a throwaway tree that holds a stub pin script, one connector
and its kit case, so no real cargo, build or pin regeneration runs.
"""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-connector.sh")
SKIP_LINE = "SKIPPED: harness kit is macOS-only; a maintainer runs it"
PINS = "crates/goalcup-protocol/pins/connector-dirs.txt"
HEX = "a" * 64

FAKE_CARGO = """#!/bin/sh
echo "$*" >>"$FAKE_CARGO_LOG"
if [ -n "${FAKE_CARGO_FAIL:-}" ]; then
  case "$*" in *"$FAKE_CARGO_FAIL"*)
    echo "loader-script.json differs from what the code now produces; first difference"
    exit 101 ;;
  esac
fi
exit 0
"""
FAKE_UNAME = '#!/bin/sh\necho "$FAKE_UNAME"\n'
# Appends FAKE_PIN_LINE (if set) to the directory pin table, as a regeneration would.
STUB_PINS = """#!/bin/sh
cd "$(dirname "$0")/.."
if [ -n "${FAKE_PIN_LINE:-}" ]; then
  printf '%s\\n' "$FAKE_PIN_LINE" >>crates/goalcup-protocol/pins/connector-dirs.txt
fi
"""

GIT_ENV = {
    **{k: v for k, v in os.environ.items() if not k.startswith("GIT_")},
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.invalid",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.invalid",
}


def write(path: Path, text: str, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


class CheckConnectorTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.bin = root / "bin"
        self.log = root / "cargo.log"
        write(self.repo / "scripts" / SCRIPT.name, SCRIPT.read_text(), 0o755)
        write(self.repo / "scripts" / "pin-manifests.sh", STUB_PINS, 0o755)
        write(self.repo / "connectors" / "toy-cli" / "harness.toml", 'id = "toy-cli"\n')
        kit = self.repo / "crates/goalcup-cli/tests/fixtures/harness-kit/toy-cli"
        for name in ("harness.toml", "stand-in.py", "expect.toml"):
            write(kit / name, "# stub\n")
        write(self.repo / PINS, f"# header\ncodex-cli {HEX}\n")
        write(self.bin / "cargo", FAKE_CARGO, 0o755)
        write(self.bin / "uname", FAKE_UNAME, 0o755)
        for args in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "base"]):
            subprocess.run(["git", *args], cwd=self.repo, env=GIT_ENV, check=True)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_script(self, *args: str, uname: str = "Linux", **env: str):
        full = {
            **GIT_ENV,
            "PATH": f"{self.bin}{os.pathsep}{os.environ.get('PATH', '/usr/bin:/bin')}",
            "FAKE_CARGO_LOG": str(self.log),
            "FAKE_UNAME": uname,
            "CARGO_TARGET_DIR": "",
            **env,
        }
        return subprocess.run(
            ["sh", str(self.repo / "scripts" / SCRIPT.name), *args],
            env=full,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def cargo_calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    # Argument handling.

    def test_no_argument_is_a_usage_error(self) -> None:
        result = self.run_script()
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)
        self.assertEqual(self.cargo_calls(), [])

    def test_two_ids_are_a_usage_error(self) -> None:
        result = self.run_script("toy-cli", "codex-cli")
        self.assertEqual(result.returncode, 2)
        self.assertIn("one connector id only", result.stderr)

    def test_unknown_option_is_a_usage_error(self) -> None:
        result = self.run_script("--regen", "toy-cli")
        self.assertEqual(result.returncode, 2)
        self.assertIn("unknown option", result.stderr)

    def test_ids_outside_the_schema_grammar_are_refused(self) -> None:
        for bad in ("Toy-cli", "1toy", "t", "toy_cli", "toy/cli", "a" * 49):
            with self.subTest(bad=bad):
                result = self.run_script(bad)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("is not a connector id", result.stderr)
        self.assertEqual(self.cargo_calls(), [])

    def test_a_missing_connector_stops_before_cargo(self) -> None:
        result = self.run_script("nope-cli")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no connectors/nope-cli/harness.toml", result.stderr)
        self.assertIn("toy-cli", result.stderr)
        self.assertEqual(self.cargo_calls(), [])

    def test_help_prints_the_header_and_exits_zero(self) -> None:
        result = self.run_script("--help")
        self.assertEqual(result.returncode, 0)
        self.assertIn("--allow-no-kit", result.stdout)
        self.assertNotIn("set -u", result.stdout)

    # The non-macOS kit skip.

    def test_linux_skips_the_kit_loudly_and_fails_without_the_flag(self) -> None:
        result = self.run_script("toy-cli")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn(SKIP_LINE, result.stdout)
        self.assertIn("FAIL kit skipped without --allow-no-kit", result.stdout)
        calls = self.cargo_calls()
        self.assertFalse(any("harness_contract_kit" in c for c in calls), calls)
        self.assertNotIn("build", calls)

    def test_linux_with_the_flag_passes_but_still_says_skipped(self) -> None:
        result = self.run_script("--allow-no-kit", "toy-cli")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn(SKIP_LINE, result.stdout)
        self.assertIn("toy-cli: 5 passed, 0 failed, 1 skipped", result.stdout)
        calls = self.cargo_calls()
        self.assertIn(
            "test -p goalcup-protocol --test harness_conformance --test connector_pins", calls
        )
        self.assertIn("test -p goalcup-cli --test harness_contract_golden", calls)
        self.assertIn("test -p goalcup-daemon --test harness_contract_golden", calls)

    def test_macos_builds_then_runs_the_kit(self) -> None:
        result = self.run_script("toy-cli", uname="Darwin")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("SKIPPED", result.stdout)
        calls = self.cargo_calls()
        kit = "test -p goalcup-cli --test harness_contract_kit"
        self.assertIn(kit, calls)
        self.assertLess(calls.index("build"), calls.index(kit))

    # Failures are reported and do not stop later steps.

    def test_a_golden_failure_prints_the_regeneration_hint(self) -> None:
        result = self.run_script(
            "--allow-no-kit", "toy-cli", FAKE_CARGO_FAIL="goalcup-daemon --test harness"
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn(
            "GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-daemon --test "
            "harness_contract_golden",
            result.stdout,
        )
        self.assertIn("PASS goldens (goalcup-cli)", result.stdout)
        self.assertIn("FAIL goldens (goalcup-daemon): loader-script.json", result.stdout)
        self.assertIn("does not mention 'toy-cli'", result.stdout)

    def test_a_target_dir_outside_the_checkout_is_warned_about(self) -> None:
        result = self.run_script(
            "--allow-no-kit", "toy-cli", CARGO_TARGET_DIR=str(Path(self.tmp.name) / "shared")
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("WARNING: CARGO_TARGET_DIR=", result.stdout)
        inside = self.run_script(
            "--allow-no-kit", "toy-cli", CARGO_TARGET_DIR=str(self.repo / "target")
        )
        self.assertNotIn("WARNING", inside.stdout)

    def test_a_missing_kit_case_fails_but_the_rest_still_runs(self) -> None:
        shutil.rmtree(self.repo / "crates/goalcup-cli/tests/fixtures/harness-kit/toy-cli")
        result = self.run_script("--allow-no-kit", "toy-cli")
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL files", result.stdout)
        self.assertTrue(any("harness_conformance" in c for c in self.cargo_calls()))

    # The pin diff.

    def test_a_pin_change_for_this_connector_passes_and_is_shown(self) -> None:
        result = self.run_script("--allow-no-kit", "toy-cli", FAKE_PIN_LINE=f"toy-cli {HEX}")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn(f"+toy-cli {HEX}", result.stdout)
        self.assertIn("commit them with the connector", result.stdout)

    def test_a_pin_change_for_another_connector_fails(self) -> None:
        result = self.run_script("--allow-no-kit", "toy-cli", FAKE_PIN_LINE=f"hermes-cli {HEX}")
        self.assertEqual(result.returncode, 1)
        self.assertIn("other connectors changed: hermes-cli", result.stdout)


class CiScopeTest(unittest.TestCase):
    """The script and the templates are not docs-only, so CI runs the Rust smoke for them."""

    def test_scripts_and_templates_require_rust(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "ci_scope", Path(__file__).with_name("ci-scope.py")
        )
        assert spec is not None and spec.loader is not None
        scope = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(scope)
        for path in (
            "scripts/check-connector.sh",
            "scripts/test_check_connector.py",
            "skills/adding-a-capture-harness/templates/connector-harness.toml",
            "skills/adding-a-capture-harness/SKILL.md",
        ):
            with self.subTest(path=path):
                self.assertTrue(scope.requires_rust([path]))


if __name__ == "__main__":
    unittest.main()
