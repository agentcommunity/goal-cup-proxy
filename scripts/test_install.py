#!/usr/bin/env python3
"""Behavioral tests for packaging/install.sh, run against a fake release on disk.

The fake release is a SHA256SUMS file and one tarball for this machine's target, in the layout
scripts/package-release.sh writes. install.sh reads it through GOALCUP_RELEASE_BASE=file://...,
so nothing touches the network. The run guard cases put a fake `goalcup` in the prefix that
prints a chosen `status --json` line and exits with a chosen code.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from pathlib import Path

INSTALL = Path(__file__).resolve().parent.parent / "packaging" / "install.sh"
VERSION = "9.8.7"
BINARIES = ("goalcup", "goalcup-verify")
# Short fragments of the two guard lines.
BLOCKED = "A Goal Cup run is recording"
UNSURE = "Could not tell whether"
PROFILES = (".zshrc", ".bashrc", ".profile", ".bash_profile")


def status_line(mode: object) -> str:
    """One `goalcup status --json` line as 1.0 prints it: {run, status, upload}, keys sorted, so
    the `run` object (which has its own `mode`) comes before `status`. A note after `mode` holds
    another `mode` key inside the status object, one level down."""
    document = {
        "run": {"v": 1, "run": "r0123456789abcdef", "mode": "practice", "deadline": None},
        "status": {
            "v": 1, "updated": "2026-10-30T10:00:00Z", "pid": 4242, "harness": None,
            "losing": {"streams_in_gap": 0, "dropped_bytes": 0}, "mode": mode,
            "notes": [{"what": "x", "mode": "sealed", "text": "a \"quoted\" } { word"}],
            "off": {"spans": 0, "ms": 0, "since": None},
        },
        "upload": None,
    }
    return json.dumps(document, separators=(",", ":")) + "\n"


def this_target() -> str:
    arch = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64", "AMD64": "x86_64"}[
        platform.machine()
    ]
    if platform.system() == "Darwin":
        # install.sh installs the native build under Rosetta; the test host is the same machine.
        if arch == "x86_64" and subprocess.run(
            ["sysctl", "-n", "hw.optional.arm64"], capture_output=True, text=True
        ).stdout.strip() == "1":
            arch = "aarch64"
        return f"{arch}-apple-darwin"
    return f"{arch}-unknown-linux-gnu"


def make_release(root: Path, version: str = VERSION, broken: str | None = None) -> Path:
    """Write <root>/release/{SHA256SUMS,goalcup-<version>-<target>.tar.gz}; return the directory.

    `broken` names a binary that fails to run, as a build for a newer glibc does."""
    name = f"goalcup-{version}-{this_target()}"
    stage = root / "stage" / name
    (stage / "bin").mkdir(parents=True)
    for binary in BINARIES:
        path = stage / "bin" / binary
        if binary == broken:
            path.write_text("#!/bin/sh\necho 'GLIBC_2.34 not found' >&2\nexit 1\n")
        else:
            # A new goalcup with no run on this computer: exit 1, nothing on stdout.
            path.write_text("#!/bin/sh\n"
                            'if [ "$1" = status ]; then echo "goalcup: no run" >&2; exit 1; fi\n'
                            f'echo "{binary} {version}"\n')
        path.chmod(0o755)
    for text in ("LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.md"):
        (stage / text).write_text(text)
    release = root / "release"
    release.mkdir()
    tarball = release / f"{name}.tar.gz"
    with tarfile.open(tarball, "w:gz") as tar:
        tar.add(stage, arcname=name)
    digest = hashlib.sha256(tarball.read_bytes()).hexdigest()
    (release / "SHA256SUMS").write_text(f"{digest}  {tarball.name}\n")
    return release


class InstallTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.home = self.root / "home"
        self.home.mkdir()
        # A goalcup first on PATH that has no run, so a real goalcup on the machine running the
        # tests (possibly recording) is never asked.
        self.pathbin = self.root / "pathbin"
        self.fake_goalcup(self.pathbin / "goalcup", stderr="goalcup: no run", rc=1)
        self.base_path = f"{self.pathbin}{os.pathsep}{os.environ['PATH']}"

    def scratch(self) -> Path:
        return Path(tempfile.mkdtemp(dir=self.root))

    def fake_goalcup(self, path: Path, *, stdout: str = "", stderr: str = "", rc: int = 0,
                     hang: bool = False) -> Path:
        """A goalcup that logs its arguments to the returned file and answers `status --json`
        with `stdout`, `stderr` and exit code `rc`. Its own files live outside the install."""
        path.parent.mkdir(parents=True, exist_ok=True)
        aux = self.scratch()
        log, out, err = aux / "log", aux / "out", aux / "err"
        out.write_text(stdout)
        err.write_text(stderr)
        script = f'#!/bin/sh\necho "$@" >> "{log}"\n'
        script += "exec sleep 60\n" if hang else f'cat "{out}"\ncat "{err}" >&2\nexit {rc}\n'
        path.write_text(script)
        path.chmod(0o755)
        return log

    def install(self, release: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", str(INSTALL), *args],
            env={"PATH": self.base_path, "HOME": str(self.home),
                 "GOALCUP_RELEASE_BASE": release.as_uri(), **env},
            capture_output=True,
            text=True,
        )

    def tree(self, root: Path) -> dict[str, bytes | None]:
        """The observable installed files and directories, including unexpected leftovers."""
        return {str(path.relative_to(root)): None if path.is_dir() else path.read_bytes()
                for path in root.rglob("*")}

    def versions(self, prefix: Path) -> list[str]:
        return [subprocess.run([str(prefix / "bin" / b), "--version"], capture_output=True,
                               text=True).stdout.strip() for b in BINARIES]

    # --- install, checksum, target ------------------------------------------------------------

    def test_installs_under_the_home_and_writes_no_profile(self) -> None:
        for shell in ("/bin/zsh", "/bin/bash", "/bin/sh"):
            with self.subTest(shell=shell):
                shutil.rmtree(self.home)
                self.home.mkdir()
                result = self.install(make_release(self.scratch()), SHELL=shell)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.versions(self.home / ".local"),
                                 [f"{b} {VERSION}" for b in BINARIES])
                for profile in PROFILES:
                    self.assertFalse((self.home / profile).exists(), profile)
                self.assertEqual([p.name for p in self.home.iterdir()], [".local"])

    def test_wrong_checksum_installs_nothing(self) -> None:
        release = make_release(self.root)
        sums = release / "SHA256SUMS"
        sums.write_text("0" * 64 + sums.read_text()[64:])
        result = self.install(release)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(list(self.home.iterdir()), [])

    def test_a_release_without_this_target_is_refused(self) -> None:
        release = make_release(self.root)
        (release / "SHA256SUMS").write_text(
            "0" * 64 + "  goalcup-9.8.7-riscv64-unknown-linux-gnu.tar.gz\n")
        result = self.install(release)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(this_target(), result.stderr)
        self.assertEqual(list(self.home.iterdir()), [])

    # --- rollback -----------------------------------------------------------------------------

    def old_install(self, prefix: Path) -> dict[str, bytes | None]:
        self.assertEqual(self.install(make_release(self.scratch()), "--prefix", str(prefix))
                         .returncode, 0)
        return self.tree(prefix)

    def newer(self, broken: str | None = None) -> Path:
        return make_release(self.scratch(), "9.8.8", broken)

    def assert_old_install(self, result: subprocess.CompletedProcess[str], prefix: Path,
                           before: dict[str, bytes | None]) -> None:
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Installed", result.stdout)
        self.assertEqual(self.tree(prefix), before)
        self.assertEqual(self.versions(prefix), [f"{b} {VERSION}" for b in BINARIES])

    def test_a_binary_that_does_not_run_leaves_the_old_install_as_it_was(self) -> None:
        for binary in BINARIES:
            with self.subTest(binary=binary):
                prefix = self.scratch() / "prefix"
                before = self.old_install(prefix)
                result = self.install(self.newer(broken=binary), "--prefix", str(prefix))
                self.assert_old_install(result, prefix, before)

    def fault_path(self, tool: str, destination: Path, *, signal_after: bool = False) -> str:
        """Fail one filesystem operation, or TERM the installer just after it completes.

        All other operations use the real tool, including the rollback. This replaces only
        an otherwise difficult-to-reproduce filesystem failure, not the installer behavior.
        """
        fake = self.scratch()
        wrapper = fake / tool
        wrapper.write_text(
            f"#!{sys.executable}\n"
            "import os, pathlib, signal, subprocess, sys\n"
            f"real = {shutil.which(tool)!r}\n"
            f"destination = pathlib.Path({str(destination)!r})\n"
            f"marker = pathlib.Path({str(fake / 'triggered')!r})\n"
            "args = sys.argv[1:]\n"
            "match = pathlib.Path(args[-1]) == destination\n"
            # Staging can change, but the binary being copied and its prefix remain observable.
            "if pathlib.Path(sys.argv[0]).name == 'cp':\n"
            "    match = (pathlib.Path(args[-2]).name == destination.name\n"
            "             and destination.parent in pathlib.Path(args[-1]).parents)\n"
            "if match and not marker.exists():\n"
            "    marker.touch()\n"
            f"    if {signal_after!r}:\n"
            "        result = subprocess.run([real, *args])\n"
            "        if result.returncode == 0:\n"
            "            os.kill(os.getppid(), signal.SIGTERM)\n"
            "        sys.exit(result.returncode)\n"
            "    sys.exit(1)\n"
            "os.execv(real, [real, *args])\n"
        )
        wrapper.chmod(0o755)
        return f"{fake}{os.pathsep}{self.base_path}"

    def test_each_failed_copy_or_promotion_restores_the_complete_old_install(self) -> None:
        prefix = self.root / "prefix with spaces"
        before = self.old_install(prefix)
        newer = self.newer()
        for binary in BINARIES:
            destination = prefix / "bin" / binary
            for label, path in (("cp fails", self.fault_path("cp", destination)),
                                ("mv fails", self.fault_path("mv", destination)),
                                ("TERM after mv", self.fault_path("mv", destination,
                                                                  signal_after=True))):
                with self.subTest(binary=binary, fault=label):
                    result = self.install(newer, "--prefix", str(prefix), PATH=path)
                    self.assert_old_install(result, prefix, before)

    # --- never update over a recording run ----------------------------------------------------

    def guarded(self, **answer: object) -> tuple[Path, Path, dict[str, bytes | None]]:
        """An install whose goalcup answers `status --json` as `answer` says."""
        prefix = self.scratch() / "prefix"
        self.old_install(prefix)
        log = self.fake_goalcup(prefix / "bin" / "goalcup", **answer)  # type: ignore[arg-type]
        return prefix, log, self.tree(prefix)

    def assert_upgraded(self, result: subprocess.CompletedProcess[str], prefix: Path) -> None:
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.versions(prefix), [f"{b} 9.8.8" for b in BINARIES])
        self.assertNotIn(BLOCKED, result.stderr)

    def test_each_live_mode_blocks_the_install(self) -> None:
        for mode in ("recording", "losing", "writer_slow", "null", None):
            with self.subTest(mode=mode):
                prefix, log, before = self.guarded(stdout=status_line(mode))
                result = self.install(self.newer(), "--prefix", str(prefix))
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(BLOCKED, result.stderr)
                self.assertNotIn("Installed", result.stdout)
                self.assertEqual(log.read_text().strip(), "status --json")
                self.assertEqual(self.tree(prefix), before)

    def test_a_mode_that_is_not_recording_goes_on(self) -> None:
        for mode in ("sealed", "practice"):
            with self.subTest(mode=mode):
                prefix, _, _ = self.guarded(stdout=status_line(mode))
                result = self.install(self.newer(), "--prefix", str(prefix))
                self.assert_upgraded(result, prefix)
                self.assertNotIn(UNSURE, result.stderr)

    def test_force_skips_the_guard(self) -> None:
        for label, args, env in (("flag", ("--force",), {}), ("env", (), {"GOALCUP_FORCE": "1"})):
            with self.subTest(label):
                prefix, log, _ = self.guarded(stdout=status_line("recording"))
                result = self.install(self.newer(), "--prefix", str(prefix), *args, **env)
                self.assert_upgraded(result, prefix)
                self.assertFalse(log.exists())

    def test_no_run_yet_goes_on_quietly(self) -> None:
        prefix, _, _ = self.guarded(stderr="goalcup: There is no run on this computer yet.", rc=1)
        result = self.install(self.newer(), "--prefix", str(prefix))
        self.assert_upgraded(result, prefix)
        self.assertEqual(result.stderr, "")

    def test_an_answer_that_cannot_be_read_gives_the_unsure_line_and_goes_on(self) -> None:
        signed = json.dumps({"payload": {"health": "online", "capture": {"run": "running"}},
                             "public_key": "ab" * 32, "signature": "cd" * 64}, indent=2) + "\n"
        cases = {"not json": {"stdout": "goalcup 9.8.7\n"},
                 "empty": {"stdout": ""},
                 "exit 3": {"stderr": "goalcup: could not read status.json", "rc": 3},
                 "a goalcup from before 1.0": {"stdout": signed},
                 "an object for a mode": {"stdout": status_line({"mode": "recording"})}}
        for label, answer in cases.items():
            with self.subTest(label):
                prefix, _, _ = self.guarded(**answer)
                result = self.install(self.newer(), "--prefix", str(prefix))
                self.assert_upgraded(result, prefix)
                self.assertIn(UNSURE, result.stderr)

    def test_a_hanging_status_is_stopped_after_10_seconds_and_gives_the_unsure_line(self) -> None:
        prefix, _, _ = self.guarded(hang=True)
        started = time.monotonic()
        result = self.install(self.newer(), "--prefix", str(prefix))
        took = time.monotonic() - started
        self.assert_upgraded(result, prefix)
        self.assertIn(UNSURE, result.stderr)
        self.assertGreaterEqual(took, 9.5)
        self.assertLess(took, 25)


if __name__ == "__main__":
    unittest.main()
