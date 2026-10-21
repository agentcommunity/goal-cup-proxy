#!/usr/bin/env python3
"""Behavioral tests for scripts/linux-matrix.sh, run against a stub `docker` on PATH.

The stub logs its argv and succeeds (and answers `inner-home-ready` with `no`), so the script
walks its host half; each test reads the logged docker lines. The Codex home config is tested by
running the container half on a temporary configuration directory.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("linux-matrix.sh")
SENTINEL = "SENTINEL-KEY-VALUE-never-printed-0123456789"
TOKEN = "FAKE-GITHUB-TOKEN-for-tests-only-0123456789"
PRACTICE = "https://github.com/agentcommunity/goalcup-matrix-empty"
STUB = """#!/bin/sh
echo "$*" >> "$STUB_LOG"
case "$*" in "build "*) cat >> "$STUB_LOG.dockerfiles" ;; esac
case "$*" in *inner-home-ready*) echo "${STUB_HOME_READY:-no}" ;; esac
case "$*" in "ps -q "*) [ -z "${STUB_BUSY:-}" ] || echo "$STUB_BUSY" ;; esac
if [ -n "${STUB_SLEEP:-}" ]; then case "$*" in *"$STUB_SLEEP"*) sleep "${STUB_SLEEP_S:-15}" ;; esac; fi
case "$*" in *" pgrep "*) [ -z "${STUB_PGREP_NONE:-}" ] || exit 1 ;; esac
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
                     "GOALCUP_TEST_MEMBER_ID", "GOALCUP_HEAVY_LOCK_HELD", "LINUX_MATRIX_LOCK_WAIT",
                     "LINUX_MATRIX_CELL", "LINUX_MATRIX_PREBUILT", "LINUX_MATRIX_CODEX_EFFORT",
                     "MATRIX_PROMPT", "MATRIX_PROMPT_FILE", "MATRIX_PROMPT_DELAY",
                     "MATRIX_SUBMIT_DELAY", "MATRIX_PRE_PROMPT_KEYS", "MATRIX_PRE_PROMPT_DELAY",
                     "MATRIX_REPO_URL", "MATRIX_GITHUB_TOKEN_FILE"):
            self.env.pop(name, None)
        self.token = self.base / "github-token"
        self.token.write_text(TOKEN + "\n")
        self.token.chmod(0o600)

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
        for server in ("https://ingest.agentcommunity.org", "https://INGEST.AgentCommunity.ORG/",
                       "https://x.Ingest.agentcommunity.org"):
            with self.subTest(server=server):
                run = self.run_script("run", "codex-cli", LINUX_MATRIX_SERVER=server)
                self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
                self.assertIn("production", run.stdout)
                self.assertEqual(self.log.read_text(), "")

    def test_a_bad_command_line_names_the_problem_in_one_line(self) -> None:
        for args in ((), ("frobnicate",), ("run",), ("login", "codex-cli", "extra")):
            with self.subTest(args=args):
                run = self.run_script(*args)
                self.assertEqual(run.returncode, 2)
                self.assertEqual(len(run.stderr.strip().splitlines()), 1, run.stderr)
                self.assertIn("see --help", run.stderr)
        self.assertEqual(self.log.read_text(), "")

    def test_a_signal_during_a_run_lets_matrix_seal_before_the_container_goes(self) -> None:
        proc = self.popen("run", "codex-cli", STUB_SLEEP="inner-run", STUB_SLEEP_S="3",
                          STUB_PGREP_NONE="1")
        deadline = time.monotonic() + 30
        while "inner-run" not in self.log.read_text() and time.monotonic() < deadline:
            time.sleep(0.1)
        time.sleep(0.3)
        proc.send_signal(signal.SIGTERM)
        self.assertEqual(proc.wait(timeout=60), 130)
        proc.stdout.close()
        proc.stderr.close()
        lines = self.log.read_text().splitlines()
        name = f"goalcup-linux-matrix-codex-cli-{proc.pid}"
        stop = f"exec -u dev {name} pkill -TERM -f scripts/matrix.sh"
        self.assertIn(stop, lines)
        self.assertEqual(lines[-1], f"rm -f {name}")
        self.assertLess(lines.index(stop), len(lines) - 1)

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
        # The container half reads its fixed paths; point them at the temporary volume.
        script = container_copy(self.base)
        run = subprocess.run(["sh", str(script), "inner-home-config", "codex-cli"],
                             capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        text = (home / "config.toml").read_text()
        lines = text.splitlines()
        self.assertEqual(lines[:2], ['model = "gpt-5.6-luna"', 'model_reasoning_effort = "low"'])
        self.assertNotIn("gpt-6-astra", text)
        self.assertIn('[projects."/w"]\ntrust_level = "trusted"\nmodel = "kept-in-table"', text)
        self.assertEqual(stat.S_IMODE((home / "config.toml").stat().st_mode), 0o600)

    # ---- build once, run prebuilt, cells, options

    def test_build_makes_the_images_and_binaries_once_under_the_lock(self) -> None:
        run = self.run_script("build", "codex-cli", "claude-code-cli", "amp-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        builds = self.docker_lines("build")
        self.assertEqual(len(builds), 3)
        versions = {c: version_of(c) for c in ("codex-cli", "claude-code-cli", "amp-cli")}
        for c, v in versions.items():
            self.assertTrue(any(f"goalcup-linux-matrix-{c}:{v}" in b for b in builds), (c, v))
        dockerfiles = Path(str(self.log) + ".dockerfiles").read_text()
        self.assertIn(f"npm install -g @anthropic-ai/claude-code@{versions['claude-code-cli']}", dockerfiles)
        self.assertIn(f"npm install --prefix /opt/amp @ampcode/cli@{versions['amp-cli']}", dockerfiles)
        self.assertIn("-g24a9fe", versions["amp-cli"])
        cargo = [x for x in self.docker_lines("exec") if "cargo build" in x]
        self.assertEqual(len(cargo), 1)
        [docker_run] = self.docker_lines("run")
        self.assertNotIn(":/vol", docker_run, "the build touched a connector's volume")
        self.assertIn(":/target ", docker_run + " ")
        self.assertTrue(run.stdout.rstrip().splitlines()[-1].startswith("linux-matrix: built"))
        self.assertFalse(self.lock.exists(), "the heavy lock was left held")

    def test_build_with_no_connector_builds_every_one_with_a_recipe(self) -> None:
        run = self.run_script("build")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        tags = " ".join(self.docker_lines("build"))
        for c in ("codex-cli", "claude-code-cli", "amp-cli", "hermes-cli"):
            self.assertIn(f"goalcup-linux-matrix-{c}:", tags)

    def test_a_prebuilt_run_never_builds_and_takes_no_heavy_lock(self) -> None:
        self.lock.mkdir()  # someone else holds it: a prebuilt run does not wait for it
        run = self.run_script("run", "codex-cli", LINUX_MATRIX_PREBUILT="1", LINUX_MATRIX_LOCK_WAIT="0")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(self.docker_lines("build"), [])
        self.assertFalse(any(" cargo " in f" {x} " for x in self.docker_lines("exec")))
        self.assertTrue(any(x.startswith("image inspect goalcup-linux-matrix-codex-cli:")
                            for x in self.log.read_text().splitlines()))
        [docker_run] = self.docker_lines("run")
        self.assertRegex(docker_run, r"-v goalcup-linux-matrix-target-\d+:/target:ro ")
        [chown] = [x for x in self.docker_lines("exec") if "chown" in x]
        self.assertNotIn("/target", chown)
        self.assertTrue(self.lock.is_dir(), "the script removed a lock it did not take")

    def test_a_prebuilt_run_without_the_image_fails_and_says_build(self) -> None:
        run = self.run_script("run", "codex-cli", LINUX_MATRIX_PREBUILT="1", STUB_FAIL="image inspect")
        self.assertEqual(run.returncode, 1)
        self.assertIn("run scripts/linux-matrix.sh build codex-cli first", run.stdout)
        self.assertEqual(self.docker_lines("run"), [])

    def test_a_cell_has_its_own_fixed_name_volume_and_label(self) -> None:
        run = self.run_script("run", "codex-cli", LINUX_MATRIX_CELL="lx-a")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn("--name goalcup-linux-matrix-codex-cli-lx-a ", docker_run)
        self.assertIn("--label goalcup-linux-matrix=codex-cli/lx-a ", docker_run)
        self.assertIn("-v goalcup-linux-matrix-codex-cli-lx-a:/vol ", docker_run)
        self.assertIn("-e LINUX_MATRIX_CELL=lx-a ", docker_run)
        self.assertTrue(any("label=goalcup-linux-matrix=codex-cli/lx-a" in x
                            for x in self.docker_lines("ps")))
        self.assertEqual(self.log.read_text().splitlines()[-1], "rm -f goalcup-linux-matrix-codex-cli-lx-a")

    def test_a_busy_cell_is_refused_and_its_container_left_alone(self) -> None:
        run = self.run_script("run", "codex-cli", LINUX_MATRIX_CELL="lx-a", STUB_BUSY="abc123")
        self.assertEqual(run.returncode, 1)
        self.assertIn("(cell lx-a) is running", run.stdout)
        self.assertNotIn("rm -f goalcup-linux-matrix-codex-cli-lx-a", self.log.read_text())
        self.assertNotIn("rm -f abc123", self.log.read_text())

    def test_a_bad_cell_name_is_refused_before_docker(self) -> None:
        for bad in ("Lx", "a_b", "-a", "a b", "x" * 41):
            with self.subTest(bad=bad):
                run = self.run_script("run", "codex-cli", LINUX_MATRIX_CELL=bad)
                self.assertEqual(run.returncode, 1)
                self.assertIn("LINUX_MATRIX_CELL", run.stdout)
        self.assertEqual(self.log.read_text(), "")

    def test_matrix_options_after_the_separator_reach_the_matrix(self) -> None:
        run = self.run_script("run", "codex-cli", "--", "--test", "--duration", "7200", "--keep",
                              MATRIX_GITHUB_TOKEN_FILE=str(self.token))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [inner] = [x for x in self.docker_lines("exec") if "inner-run" in x]
        self.assertTrue(inner.endswith("inner-run codex-cli --test --duration 7200 --keep"), inner)
        self.assertNotIn(SENTINEL, self.log.read_text() + run.stdout + run.stderr)

    def test_the_token_file_is_mounted_read_only_and_never_passes_its_value(self) -> None:
        run = self.run_script("run", "codex-cli", "--", "--test", "--duration", "60",
                              MATRIX_GITHUB_TOKEN_FILE=str(self.token))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn(f"-v {self.token}:/run/goalcup/github-token:ro", docker_run)
        [inner] = [x for x in self.docker_lines("exec") if "inner-run" in x]
        self.assertIn("-e MATRIX_GITHUB_TOKEN_FILE=/run/goalcup/github-token", inner)
        self.assertNotIn(TOKEN, self.log.read_text() + run.stdout + run.stderr)

    def test_a_test_run_without_the_token_file_is_refused_before_docker(self) -> None:
        run = self.run_script("run", "codex-cli", "--", "--test", "--duration", "60")
        self.assertEqual(run.returncode, 1)
        self.assertIn("MATRIX_GITHUB_TOKEN_FILE is needed for --test", run.stdout)
        self.assertEqual(self.docker_lines("run"), [])

    def test_login_settles_with_the_token_file(self) -> None:
        run = self.run_script("login", "codex-cli", STUB_HOME_READY="yes",
                              MATRIX_GITHUB_TOKEN_FILE=str(self.token))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [settle] = [x for x in self.docker_lines("exec") if "inner-settle" in x]
        self.assertIn("-e MATRIX_GITHUB_TOKEN_FILE=/run/goalcup/github-token", settle)

    def test_other_matrix_options_are_refused(self) -> None:
        for extra in (["--allow-production"], ["--log", "/x"], ["--duration", "2h"],
                      ["--check-copy", "/x"], ["--duration"]):
            with self.subTest(extra=extra):
                run = self.run_script("run", "codex-cli", "--", *extra)
                self.assertEqual(run.returncode, 1, run.stdout)
        self.assertEqual(self.docker_lines("run"), [])
        login = self.run_script("login", "codex-cli", "--", "--test")
        self.assertEqual(login.returncode, 2)

    def test_a_prompt_file_is_mounted_read_only_and_named_to_the_matrix(self) -> None:
        prompt = self.base / "prompt.txt"
        prompt.write_text("Build it.\nThen test it.\n")
        run = self.run_script("run", "codex-cli", MATRIX_PROMPT_FILE=str(prompt))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn(f"-v {prompt}:/run/goalcup/prompt:ro", docker_run)
        [inner] = [x for x in self.docker_lines("exec") if "inner-run" in x]
        self.assertIn("-e MATRIX_PROMPT_FILE=/run/goalcup/prompt", inner)
        self.assertNotIn("Build it.", self.log.read_text())

    def test_a_relative_or_missing_prompt_file_is_refused(self) -> None:
        for path in ("prompt.txt", str(self.base / "missing.txt")):
            run = self.run_script("run", "codex-cli", MATRIX_PROMPT_FILE=path)
            self.assertEqual(run.returncode, 1, run.stdout)
        self.assertEqual(self.docker_lines("run"), [])

    def test_matrix_settings_that_are_set_are_passed_on(self) -> None:
        run = self.run_script("run", "claude-code-cli", MATRIX_SUBMIT_DELAY="5",
                              MATRIX_PRE_PROMPT_KEYS="")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn("-e MATRIX_SUBMIT_DELAY=5 ", docker_run)
        # Set but empty still travels: it overrides the Claude trust-dialog default.
        self.assertIn("-e MATRIX_PRE_PROMPT_KEYS= ", docker_run)
        self.assertNotIn("MATRIX_PROMPT_DELAY", docker_run)

    def test_codex_effort_is_low_or_minimal_and_reaches_the_container(self) -> None:
        run = self.run_script("run", "codex-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        [docker_run] = self.docker_lines("run")
        self.assertIn("-e LINUX_MATRIX_CODEX_EFFORT=low ", docker_run)
        bad = self.run_script("run", "codex-cli", LINUX_MATRIX_CODEX_EFFORT="high")
        self.assertEqual(bad.returncode, 1)
        self.assertIn("low or minimal", bad.stdout)
        self.assertEqual(len(self.docker_lines("run")), 1)

    def test_the_claude_login_records_file_names_in_a_login_log(self) -> None:
        run = self.run_script("login", "claude-code-cli", STUB_HOME_READY="yes", LINUX_MATRIX_CELL="lx-c")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertTrue(any("inner-home-files claude-code-cli" in x for x in self.docker_lines("exec")))
        self.assertTrue(list((self.base / "logs").glob("login-claude-code-cli-lx-c-*.log")))
        self.assertNotIn(SENTINEL, self.log.read_text() + run.stdout + run.stderr)


def container_copy(base: Path) -> Path:
    """The script with its volume at base/vol, and the settle helper beside it."""
    script = base / "linux-matrix.sh"
    script.write_text(SCRIPT.read_text().replace("VOL=/vol\n", f"VOL={base / 'vol'}\n", 1))
    shutil.copy(SCRIPT.with_name("kit_settle.py"), base / "kit_settle.py")
    return script


def version_of(connector: str) -> str:
    first = (SCRIPT.parent.parent / f"connectors/{connector}/harness.toml").read_text().split("\n")[0]
    return next(w.rstrip(",") for w in first.split() if w[:1].isdigit())


class ContainerHalfTests(unittest.TestCase):
    """The container half's home config and file listing, on a temporary volume."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name).resolve()
        self.script = container_copy(self.base)
        self.homes = self.base / "vol/cfg/capture-homes"
        self.bin = self.base / "bin"
        self.bin.mkdir()

    def inner(self, *args: str, **env: str) -> subprocess.CompletedProcess:
        environment = {k: v for k, v in os.environ.items()
                       if k not in ("LINUX_MATRIX_CODEX_EFFORT", "MATRIX_PRE_PROMPT_KEYS")}
        environment["PATH"] = f"{self.bin}:{os.environ['PATH']}"
        environment.update(env)
        return subprocess.run(["sh", str(self.script), *args], env=environment,
                              capture_output=True, text=True)

    def codex_stub(self, efforts: list[str]) -> None:
        doc = {"models": [
            {"slug": "gpt-6-astra", "supported_reasoning_levels": [{"effort": "minimal"}]},
            {"slug": "gpt-5.6-luna", "supported_reasoning_levels": [{"effort": e} for e in efforts]},
        ]}
        stub = self.bin / "codex"
        stub.write_text("#!/bin/sh\n[ \"$*\" = \"debug models --bundled\" ] || exit 2\n"
                        f"cat <<'EOF'\n{json.dumps(doc)}\nEOF\n")
        stub.chmod(0o755)

    def test_claude_settings_are_merged_keeping_other_keys(self) -> None:
        home = self.homes / "claude-code-cli"
        home.mkdir(parents=True)
        (home / "settings.json").write_text(json.dumps({
            "model": "opus", "alwaysThinkingEnabled": True, "theme": "dark",
            "permissions": {"allow": ["Bash(ls)"]}}))
        run = self.inner("inner-home-config", "claude-code-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        doc = json.loads((home / "settings.json").read_text())
        self.assertEqual(doc["model"], "haiku")
        self.assertIs(doc["alwaysThinkingEnabled"], False)
        self.assertEqual(doc["theme"], "dark")
        self.assertEqual(doc["permissions"], {"allow": ["Bash(ls)"]})
        self.assertEqual(stat.S_IMODE((home / "settings.json").stat().st_mode), 0o600)

    def test_claude_settings_are_written_when_there_are_none(self) -> None:
        run = self.inner("inner-home-config", "claude-code-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        doc = json.loads((self.homes / "claude-code-cli/settings.json").read_text())
        self.assertEqual(doc, {"model": "haiku", "alwaysThinkingEnabled": False})

    def test_claude_settings_that_are_not_json_are_left_alone_and_fail(self) -> None:
        home = self.homes / "claude-code-cli"
        home.mkdir(parents=True)
        (home / "settings.json").write_text("{not json")
        run = self.inner("inner-home-config", "claude-code-cli")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("FAIL", run.stdout)
        self.assertEqual((home / "settings.json").read_text(), "{not json")

    def test_codex_minimal_effort_is_set_when_codex_lists_it(self) -> None:
        self.codex_stub(["minimal", "low", "medium"])
        run = self.inner("inner-home-config", "codex-cli", LINUX_MATRIX_CODEX_EFFORT="minimal")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        lines = (self.homes / "codex-cli/config.toml").read_text().splitlines()
        self.assertEqual(lines[:2], ['model = "gpt-5.6-luna"', 'model_reasoning_effort = "minimal"'])

    def test_codex_minimal_effort_fails_when_codex_does_not_list_it_for_luna(self) -> None:
        self.codex_stub(["low", "medium"])  # astra lists minimal; luna does not
        run = self.inner("inner-home-config", "codex-cli", LINUX_MATRIX_CODEX_EFFORT="minimal")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("does not list minimal", run.stdout)
        self.assertFalse((self.homes / "codex-cli/config.toml").exists())

    def inner_run_world(self) -> Path:
        """A stand-in /src: the connector's manifest and a matrix.sh that records what it got
        and leaves one haiku transcript, on a volume where prep recorded the sign-in."""
        src = self.base / "src"
        (src / "connectors/claude-code-cli").mkdir(parents=True)
        (src / "connectors/claude-code-cli/harness.toml").write_text(
            (SCRIPT.parent.parent / "connectors/claude-code-cli/harness.toml").read_text())
        (src / "scripts").mkdir()
        got = self.base / "matrix-got.json"
        (src / "scripts/matrix.sh").write_text(f"""#!/bin/sh
python3 -c 'import json, os, sys; json.dump({{"argv": sys.argv[1:], "keys": os.environ.get("MATRIX_PRE_PROMPT_KEYS"), "cfg": os.environ.get("MATRIX_CONFIG_DIR")}}, open("{got}", "w"))' "$@"
d="$MATRIX_CONFIG_DIR/capture-homes/claude-code-cli/projects/-w"
mkdir -p "$d"
echo '{{"type":"assistant","message":{{"model":"claude-haiku-4-5"}}}}' > "$d/s.jsonl"
echo "claude-code-cli linux-x86_64 2.1.283 PASS 2026-10-20"
""")
        cfg = self.base / "vol/cfg"
        cfg.mkdir(parents=True)
        (cfg / "prep.json").write_text(json.dumps({"harness": "claude-code-cli", "home_ready": True}))
        self.script.write_text(self.script.read_text().replace("/src/", f"{src}/"))
        return got

    def test_inner_run_passes_matrix_options_and_the_claude_trust_keys(self) -> None:
        got = self.inner_run_world()
        run = self.inner("inner-run", "claude-code-cli", "--test", "--duration", "7200",
                         TMPDIR=str(self.base / "tmp"), LINUX_MATRIX_SERVER="https://s.example")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        seen = json.loads(got.read_text())
        self.assertEqual(seen["argv"][0:2], ["claude-code-cli", "https://s.example"])
        self.assertEqual(seen["argv"][4:], ["--test", "--duration", "7200"])
        self.assertEqual(seen["keys"], "\\033[B\\r")
        self.assertEqual(seen["cfg"], str(self.base / "vol/cfg"))
        self.assertTrue(run.stdout.rstrip().splitlines()[-1].endswith("PASS 2026-10-20"))
        settings = json.loads((self.homes / "claude-code-cli/settings.json").read_text())
        self.assertEqual(settings["model"], "haiku")

    def test_inner_run_keeps_trust_keys_that_were_set_even_empty(self) -> None:
        got = self.inner_run_world()
        run = self.inner("inner-run", "claude-code-cli", MATRIX_PRE_PROMPT_KEYS="",
                         TMPDIR=str(self.base / "tmp"), LINUX_MATRIX_SERVER="https://s.example")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(json.loads(got.read_text())["keys"], "")

    def settle(self, **env: str) -> subprocess.CompletedProcess:
        return self.inner("inner-settle", TMPDIR=str(self.base / "tmp"), **env)

    def test_settle_writes_the_token_into_github_json_0600(self) -> None:
        token = self.base / "github-token"
        token.write_text(TOKEN + "\n")
        cfg = self.base / "vol/cfg"
        cfg.mkdir(parents=True)
        (cfg / "prep.json").write_text(json.dumps({"harness": "codex-cli", "home_ready": True}))
        (cfg / "github.json").write_text(json.dumps({"repo_url": PRACTICE,
                                                     "token": "ghp_matrixfaketoken00000000000000000000"}))
        run = self.settle(MATRIX_GITHUB_TOKEN_FILE=str(token))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(json.loads((cfg / "github.json").read_text()),
                         {"repo_url": PRACTICE, "token": TOKEN})
        prep = json.loads((cfg / "prep.json").read_text())
        self.assertEqual(prep, {"harness": "codex-cli", "home_ready": True, "repo_url": PRACTICE,
                                "work_root": str(self.base / "vol/work")})
        for name in ("github.json", "prep.json"):
            self.assertEqual(stat.S_IMODE((cfg / name).stat().st_mode), 0o600, name)
        self.assertEqual(sorted(p.name for p in cfg.iterdir()), ["github.json", "prep.json"])
        self.assertNotIn(TOKEN, run.stdout + run.stderr)

    def test_settle_keeps_a_prepared_full_flow_repository(self) -> None:
        token = self.base / "github-token"
        token.write_text(TOKEN + "\n")
        cfg = self.base / "vol/cfg"
        cfg.mkdir(parents=True)
        prep = json.dumps({"harness": "codex-cli", "repo_url": "https://github.com/someone/entry",
                           "work_root": "/vol/work", "home_ready": True})
        gh = json.dumps({"repo_url": "https://github.com/someone/entry",
                         "token": "FAKE-participant-token-for-tests-0000"})
        (cfg / "prep.json").write_text(prep)
        (cfg / "github.json").write_text(gh)
        run = self.settle(MATRIX_GITHUB_TOKEN_FILE=str(token))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual((cfg / "prep.json").read_text(), prep)
        self.assertEqual((cfg / "github.json").read_text(), gh)

    def test_settle_refuses_a_token_file_without_a_token_and_never_prints_it(self) -> None:
        token = self.base / "github-token"
        token.write_text("not a token " + SENTINEL + "\n")
        cfg = self.base / "vol/cfg"
        cfg.mkdir(parents=True)
        (cfg / "prep.json").write_text(json.dumps({"harness": "codex-cli"}))
        run = self.settle(MATRIX_GITHUB_TOKEN_FILE=str(token))
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("FAIL settle", run.stdout)
        self.assertNotIn(SENTINEL, run.stdout + run.stderr)
        self.assertFalse((cfg / "github.json").exists())

    def test_home_files_lists_names_never_contents(self) -> None:
        home = self.homes / "claude-code-cli"
        (home / "projects/p").mkdir(parents=True)
        (home / ".credentials.json").write_text(SENTINEL)
        (home / "projects/p/s.jsonl").write_text(SENTINEL)
        run = self.inner("inner-home-files", "claude-code-cli")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn(".credentials.json", run.stdout)
        self.assertIn("projects/p/s.jsonl", run.stdout)
        self.assertNotIn(SENTINEL, run.stdout + run.stderr)


class CodexModelCheckTests(unittest.TestCase):
    """The container half's check of the run's Codex turns, on a temporary volume."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name).resolve()
        self.sessions = self.base / "vol/cfg/capture-homes/codex-cli/sessions/2026/10/16"
        self.sessions.mkdir(parents=True)
        self.script = container_copy(self.base)
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

    def check(self, effort: str | None = None) -> subprocess.CompletedProcess:
        env = {k: v for k, v in os.environ.items() if k != "LINUX_MATRIX_CODEX_EFFORT"}
        if effort:
            env["LINUX_MATRIX_CODEX_EFFORT"] = effort
        return subprocess.run(["sh", str(self.script), "inner-model-check", "codex-cli",
                               str(self.marker)], capture_output=True, text=True, env=env)

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

    def test_the_configured_effort_is_the_one_required(self) -> None:
        self.rollout("a", ("gpt-5.6-luna", "minimal"))
        self.assertEqual(self.check("minimal").returncode, 0)
        self.assertEqual(self.check("low").returncode, 1)


class ClaudeModelCheckTests(unittest.TestCase):
    """The container half's check of the run's Claude transcripts, on a temporary volume."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name).resolve()
        self.projects = self.base / "vol/cfg/capture-homes/claude-code-cli/projects/-vol-work-x"
        self.projects.mkdir(parents=True)
        self.script = container_copy(self.base)
        self.marker = self.base / "marker"
        self.marker.touch()
        past = time.time() - 3600
        os.utime(self.marker, (past, past))

    def transcript(self, name: str, *models: str, old: bool = False) -> None:
        lines = [json.dumps({"type": "user", "message": {"role": "user", "content": "hi"}})]
        lines += [json.dumps({"type": "assistant", "message": {"role": "assistant", "model": m}})
                  for m in models]
        path = self.projects / f"{name}.jsonl"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines) + "\n")
        if old:
            past = time.time() - 7200
            os.utime(path, (past, past))

    def check(self) -> subprocess.CompletedProcess:
        return subprocess.run(["sh", str(self.script), "inner-model-check", "claude-code-cli",
                               str(self.marker)], capture_output=True, text=True)

    def test_haiku_passes(self) -> None:
        self.transcript("a", "claude-haiku-4-5-20251001", "claude-haiku-4-5-20251001")
        run = self.check()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn("claude-haiku-4-5-20251001 x2", run.stdout)

    def test_another_model_fails_even_in_a_subagent(self) -> None:
        self.transcript("a", "claude-haiku-4-5-20251001")
        self.transcript("a/subagents/agent-1", "claude-sonnet-4-5")
        self.assertEqual(self.check().returncode, 1)

    def test_no_assistant_message_fails(self) -> None:
        self.transcript("a")
        run = self.check()
        self.assertEqual(run.returncode, 1)
        self.assertIn("could not be confirmed", run.stdout)

    def test_synthetic_messages_are_counted_apart(self) -> None:
        self.transcript("a", "claude-haiku-4-5-20251001", "<synthetic>")
        run = self.check()
        self.assertEqual(run.returncode, 0, run.stdout)
        self.assertIn("1 <synthetic>", run.stdout)
        self.transcript("b", "<synthetic>")
        self.transcript("a", "<synthetic>")
        self.assertEqual(self.check().returncode, 1)

    def test_transcripts_from_before_the_run_are_not_counted(self) -> None:
        self.transcript("old", "claude-opus-4-1", old=True)
        self.transcript("new", "claude-haiku-4-5-20251001")
        self.assertEqual(self.check().returncode, 0)


if __name__ == "__main__":
    unittest.main()
