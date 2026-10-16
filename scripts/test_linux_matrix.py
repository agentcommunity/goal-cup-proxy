#!/usr/bin/env python3
"""Behavioral tests for scripts/linux-matrix.sh, run against a stub `docker` on PATH.

The stub logs its argv and succeeds (and answers `inner-home-ready` with `no`), so the script
walks its host half; each test reads the logged docker lines. The Codex home config is tested by
running the container half on a temporary configuration directory.
"""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("linux-matrix.sh")
SENTINEL = "SENTINEL-KEY-VALUE-never-printed-0123456789"
STUB = """#!/bin/sh
echo "$*" >> "$STUB_LOG"
case "$*" in *inner-home-ready*) echo "${STUB_HOME_READY:-no}" ;; esac
exit 0
"""


class LinuxMatrixTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name).resolve()
        self.log = self.base / "docker.log"
        self.log.touch()
        bindir = self.base / "bin"
        bindir.mkdir()
        stub = bindir / "docker"
        stub.write_text(STUB)
        stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
        (self.base / "cargo/registry").mkdir(parents=True)
        self.key = self.base / "approval-key"
        self.key.write_text(SENTINEL + "\n")
        self.key.chmod(0o600)
        self.lock = self.base / "heavy.lock"
        self.env = {
            **os.environ,
            "PATH": f"{bindir}:{os.environ['PATH']}",
            "STUB_LOG": str(self.log),
            "CARGO_HOME": str(self.base / "cargo"),
            "GOALCUP_HEAVY_LOCK": str(self.lock),
            "LINUX_MATRIX_LOG_DIR": str(self.base / "logs"),
            "GOALCUP_TEST_APPROVAL_KEY_FILE": str(self.key),
        }
        for name in ("GOALCUP_TEST_APPROVAL_KEY", "MATRIX_KEY_ENV_FILE", "LINUX_MATRIX_SERVER",
                     "GOALCUP_TEST_MEMBER_ID"):
            self.env.pop(name, None)

    def run_script(self, *args: str, **env: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["sh", str(SCRIPT), *args],
            env={**self.env, **env},
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
        )

    def docker_lines(self, verb: str) -> list[str]:
        return [x for x in self.log.read_text().splitlines() if x.startswith(verb + " ")]

    def test_run_mounts_the_key_file_read_only_and_never_passes_its_value(self) -> None:
        run = self.run_script("run", "codex-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn(f"-v {self.key}:/run/goalcup/approval-key:ro", docker_run)
        self.assertIn("--cpus=", docker_run)
        self.assertIn("-v goalcup-linux-matrix-codex-cli:/vol", docker_run)
        self.assertIn("-e GOALCUP_TEST_MEMBER_ID=00000000-0000-4000-8000-0000000c0de1", docker_run)
        self.assertIn("-e LINUX_MATRIX_SERVER=https://goalcup-ingest-staging.", docker_run)
        self.assertNotIn("--privileged", docker_run)
        # The key's path reaches the container only on the exec that runs the matrix.
        self.assertNotIn("GOALCUP_TEST_APPROVAL_KEY_FILE", docker_run)
        [inner] = [x for x in self.docker_lines("exec") if "inner-run codex-cli" in x]
        self.assertIn("-e GOALCUP_TEST_APPROVAL_KEY_FILE=/run/goalcup/approval-key", inner)
        everything = self.log.read_text() + run.stdout + run.stderr
        self.assertNotIn(SENTINEL, everything)
        self.assertFalse(self.lock.exists(), "the heavy lock was left held")

    def test_the_build_installs_the_connectors_codex_version_from_npm(self) -> None:
        run = self.run_script("run", "codex-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        first = (SCRIPT.parent.parent / "connectors/codex-cli/harness.toml").read_text().split("\n")[0]
        version = first.split()[3].rstrip(",")
        [build] = self.docker_lines("build")
        self.assertIn(f"goalcup-linux-matrix-codex-cli:{version}", build)
        self.assertTrue(any("cargo build --offline --locked --workspace" in x
                            for x in self.docker_lines("exec")))

    def test_a_provider_key_file_is_mounted_and_passed_to_matrix_by_path(self) -> None:
        keyenv = self.base / "key.env"
        keyenv.write_text("DEEPSEEK_API_KEY=" + SENTINEL + "\n")
        keyenv.chmod(0o600)
        run = self.run_script("run", "hermes-cli", MATRIX_KEY_ENV_FILE=str(keyenv))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn(f"-v {keyenv}:/run/goalcup/key.env:ro", docker_run)
        [inner] = [x for x in self.docker_lines("exec") if "inner-run hermes-cli" in x]
        self.assertIn("-e MATRIX_KEY_ENV_FILE=/run/goalcup/key.env", inner)
        self.assertNotIn(SENTINEL, self.log.read_text() + run.stdout + run.stderr)

    def test_a_key_in_the_environment_is_refused_before_docker_runs(self) -> None:
        run = self.run_script("run", "codex-cli", GOALCUP_TEST_APPROVAL_KEY=SENTINEL)
        self.assertNotEqual(run.returncode, 0)
        self.assertEqual(self.log.read_text(), "")
        self.assertNotIn(SENTINEL, run.stdout + run.stderr)

    def test_production_is_refused(self) -> None:
        run = self.run_script("run", "codex-cli",
                              LINUX_MATRIX_SERVER="https://ingest.agentcommunity.org")
        self.assertNotEqual(run.returncode, 0)
        self.assertEqual(self.log.read_text(), "")

    def test_login_without_a_terminal_stops_at_the_owner_step(self) -> None:
        run = self.run_script("login", "codex-cli")
        self.assertEqual(run.returncode, 3, run.stdout + run.stderr)
        self.assertIn("linux-matrix: OWNER STEP", run.stdout)
        self.assertIn("sh scripts/linux-matrix.sh login codex-cli", run.stdout)
        execs = self.docker_lines("exec")
        self.assertTrue(any("inner-login codex-cli" in x for x in execs))
        self.assertFalse(any(" -it " in f" {x} " for x in execs), "prep opened without a terminal")
        self.assertFalse(self.lock.exists(), "the lock is held through the owner's step")

    def test_login_once_signed_in_does_not_open_the_harness_again(self) -> None:
        run = self.run_script("login", "codex-cli", STUB_HOME_READY="yes")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertTrue(run.stdout.rstrip().splitlines()[-1].startswith("linux-matrix: signed in"))
        self.assertFalse(any("goalcup prep" in x for x in self.docker_lines("exec")))

    def test_login_of_a_headless_connector_is_refused(self) -> None:
        run = self.run_script("login", "hermes-cli")
        self.assertEqual(run.returncode, 1)
        self.assertIn("headless", run.stdout)
        self.assertEqual(self.log.read_text(), "")

    def test_codex_home_config_sets_luna_low_and_drops_another_model(self) -> None:
        cfg = self.base / "vol/cfg"
        home = cfg / "capture-homes/codex-cli"
        home.mkdir(parents=True)
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_reasoning_effort = "high"\n\n'
            '[projects."/w"]\ntrust_level = "trusted"\nmodel = "kept-in-table"\n'
        )
        script = self.base / "linux-matrix.sh"
        # The container half reads its fixed paths; point them at the temporary volume.
        script.write_text(SCRIPT.read_text().replace("VOL=/vol\n", f"VOL={self.base / 'vol'}\n", 1))
        run = subprocess.run(["sh", str(script), "inner-home-config", "codex-cli"],
                             capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        text = (home / "config.toml").read_text()
        lines = text.splitlines()
        self.assertEqual(lines[:2], ['model = "gpt-5.6-luna"', 'model_reasoning_effort = "low"'])
        self.assertNotIn("gpt-6-astra", text)
        self.assertIn('[projects."/w"]\ntrust_level = "trusted"\nmodel = "kept-in-table"', text)
        self.assertEqual(stat.S_IMODE((home / "config.toml").stat().st_mode), 0o600)


if __name__ == "__main__":
    unittest.main()
