#!/usr/bin/env python3
"""Behavioral tests for scripts/linux-matrix.sh, run against a stub `docker` on PATH.

The stub logs its argv and succeeds (and answers `inner-home-ready` with `no`), so the script
walks its host half; each test reads the logged docker lines. The Codex home config is tested by
running the container half on a temporary configuration directory.
"""

from __future__ import annotations

import json
import os
import signal
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("linux-matrix.sh")
SENTINEL = "SENTINEL-KEY-VALUE-never-printed-0123456789"
STUB = """#!/bin/sh
echo "$*" >> "$STUB_LOG"
case "$*" in *inner-home-ready*) echo "${STUB_HOME_READY:-no}" ;; esac
case "$*" in "ps -q "*) [ -z "${STUB_BUSY:-}" ] || echo "$STUB_BUSY" ;; esac
if [ -n "${STUB_SLEEP:-}" ]; then case "$*" in *"$STUB_SLEEP"*) sleep 15 ;; esac; fi
if [ -n "${STUB_FAIL:-}" ]; then case "$*" in *"$STUB_FAIL"*) exit 1 ;; esac; fi
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
        for name in ("GOALCUP_TEST_APPROVAL_KEY", "LINUX_MATRIX_SERVER",
                     "GOALCUP_TEST_MEMBER_ID", "GOALCUP_HEAVY_LOCK_HELD", "LINUX_MATRIX_LOCK_WAIT"):
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

    def assert_container_removed_last(self, pid: int) -> None:
        name = f"goalcup-linux-matrix-codex-cli-{pid}"
        [docker_run] = self.docker_lines("run")
        self.assertIn(f"--name {name} ", docker_run)
        self.assertEqual(self.log.read_text().splitlines()[-1], f"rm -f {name}")
        self.assertFalse(self.lock.exists(), "the heavy lock was left held")

    def popen(self, *args: str, **env: str) -> subprocess.Popen:
        return subprocess.Popen(["sh", str(SCRIPT), *args], env={**self.env, **env},
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                stdin=subprocess.DEVNULL)

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

    def test_matrix_selects_the_container_build_over_mounted_host_binaries(self) -> None:
        run = self.run_script("run", "codex-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn("-e MATRIX_BIN_DIR=/target/debug", docker_run)

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

    def test_login_of_hermes_stops_at_the_owner_step_as_codex_does(self) -> None:
        # Hermes runs in chat mode from its own home (D61): its sign-in is kept like Codex's, and
        # no provider key file is asked for.
        run = self.run_script("login", "hermes-cli")
        self.assertEqual(run.returncode, 3, run.stdout + run.stderr)
        self.assertIn("linux-matrix: OWNER STEP", run.stdout)
        self.assertIn("sh scripts/linux-matrix.sh login hermes-cli", run.stdout)
        self.assertTrue(any("inner-login hermes-cli" in x for x in self.docker_lines("exec")))

    def test_a_caller_holding_the_lock_is_refused_at_once(self) -> None:
        self.lock.mkdir()
        start = time.monotonic()
        run = self.run_script("run", "codex-cli", GOALCUP_HEAVY_LOCK_HELD="1")
        self.assertLess(time.monotonic() - start, 10)
        self.assertEqual(run.returncode, 1)
        self.assertIn("you hold the heavy-run lock", run.stdout)
        self.assertTrue(self.lock.is_dir(), "the script removed a lock it did not take")
        self.assertEqual(self.docker_lines("run"), [])

    def test_a_lock_held_too_long_fails_instead_of_waiting_for_ever(self) -> None:
        self.lock.mkdir()
        run = self.run_script("run", "codex-cli", LINUX_MATRIX_LOCK_WAIT="0")
        self.assertEqual(run.returncode, 1)
        self.assertIn("is still held after 0s", run.stdout)
        self.assertTrue(self.lock.is_dir(), "the script removed a lock it did not take")
        self.assertEqual(self.docker_lines("run"), [])

    def test_a_running_container_of_the_connector_is_refused_not_removed(self) -> None:
        run = self.run_script("run", "codex-cli", STUB_BUSY="abc123")
        self.assertEqual(run.returncode, 1)
        self.assertIn("another linux-matrix container of codex-cli is running", run.stdout)
        self.assertNotIn("rm -f abc123", self.log.read_text())
        self.assertEqual(self.docker_lines("run"), [])
        self.assertFalse(self.lock.exists())

    def test_the_container_is_removed_after_a_run(self) -> None:
        proc = self.popen("run", "codex-cli")
        out, err = proc.communicate(timeout=60)
        self.assertEqual(proc.returncode, 0, out + err)
        self.assert_container_removed_last(proc.pid)

    def test_the_container_is_removed_after_a_failed_step(self) -> None:
        proc = self.popen("run", "codex-cli", STUB_FAIL="cargo build")
        out, err = proc.communicate(timeout=60)
        self.assertEqual(proc.returncode, 1, out + err)
        self.assertIn("linux-matrix: FAIL build", out)
        self.assert_container_removed_last(proc.pid)

    def test_the_container_is_removed_at_the_owner_step(self) -> None:
        proc = self.popen("login", "codex-cli")
        out, err = proc.communicate(timeout=60)
        self.assertEqual(proc.returncode, 3, out + err)
        self.assert_container_removed_last(proc.pid)

    def test_the_container_is_removed_on_a_signal(self) -> None:
        proc = self.popen("run", "codex-cli", STUB_SLEEP="cargo build")
        deadline = time.monotonic() + 30
        while "cargo build" not in self.log.read_text() and time.monotonic() < deadline:
            time.sleep(0.1)
        proc.send_signal(signal.SIGTERM)
        # Only the exit is waited for: the stub's sleep, orphaned, still holds the output pipes.
        self.assertEqual(proc.wait(timeout=20), 130)
        proc.stdout.close()
        proc.stderr.close()
        self.assert_container_removed_last(proc.pid)

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


class CodexModelCheckTests(unittest.TestCase):
    """The container half's check of the run's Codex turns, on a temporary volume."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name).resolve()
        self.sessions = self.base / "vol/cfg/capture-homes/codex-cli/sessions/2026/10/16"
        self.sessions.mkdir(parents=True)
        self.script = self.base / "linux-matrix.sh"
        self.script.write_text(SCRIPT.read_text().replace("VOL=/vol\n", f"VOL={self.base / 'vol'}\n", 1))
        self.marker = self.base / "marker"
        self.marker.touch()
        past = time.time() - 3600
        os.utime(self.marker, (past, past))

    def rollout(self, name: str, *turns: tuple[str, str], old: bool = False) -> None:
        lines = [json.dumps({"type": "session_meta", "payload": {"id": name}})]
        lines += [json.dumps({"type": "turn_context", "payload": {"model": m, "effort": e}})
                  for m, e in turns]
        path = self.sessions / f"rollout-{name}.jsonl"
        path.write_text("\n".join(lines) + "\n")
        if old:
            past = time.time() - 7200
            os.utime(path, (past, past))

    def check(self) -> subprocess.CompletedProcess:
        return subprocess.run(["sh", str(self.script), "inner-model-check", "codex-cli",
                               str(self.marker)], capture_output=True, text=True)

    def test_luna_at_low_passes(self) -> None:
        self.rollout("a", ("gpt-5.6-luna", "low"), ("gpt-5.6-luna", "low"))
        run = self.check()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn("gpt-5.6-luna at effort low", run.stdout)

    def test_astra_fails(self) -> None:
        self.rollout("a", ("gpt-5.6-luna", "low"), ("gpt-6-astra", "low"))
        self.assertEqual(self.check().returncode, 1)

    def test_another_effort_fails(self) -> None:
        self.rollout("a", ("gpt-5.6-luna", "medium"))
        self.assertEqual(self.check().returncode, 1)

    def test_no_turn_found_fails(self) -> None:
        self.rollout("a")
        run = self.check()
        self.assertEqual(run.returncode, 1)
        self.assertIn("could not be confirmed", run.stdout)

    def test_sessions_from_before_the_run_are_not_counted(self) -> None:
        self.rollout("old", ("gpt-6-astra", "high"), old=True)
        self.rollout("new", ("gpt-5.6-luna", "low"))
        self.assertEqual(self.check().returncode, 0)


if __name__ == "__main__":
    unittest.main()
