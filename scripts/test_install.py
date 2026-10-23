#!/usr/bin/env python3
"""Behavioral tests for packaging/install.sh, run against a fake release on disk.

The fake release is a SHA256SUMS file and one tarball for this machine's target, in the layout
scripts/package-release.sh writes. install.sh reads it through GOALCUP_RELEASE_BASE=file://...,
so nothing touches the network.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from pathlib import Path

INSTALL = Path(__file__).resolve().parent.parent / "packaging" / "install.sh"
VERSION = "9.8.7"
ROOT = Path(__file__).resolve().parent.parent
STATUS_FIXTURES = ROOT / "crates" / "goalcup-protocol" / "tests" / "fixtures" / "m2-status"

# The copy install.sh prints (docs/plans/2026-10-23-release-is-the-list.md, global constraints).
INSTALL_BLOCKED = (
    "A Goal Cup run is recording on this computer. Update after it ends, or run the installer "
    "again with: curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/"
    "download/install.sh | sh -s -- --force"
)
INSTALL_UNSURE = (
    "Could not tell whether a Goal Cup run is recording. If one is, stop this update and run it "
    "after the run ends."
)
# What `goalcup status` says on a computer that never started a run (participant::no_run_yet,
# printed on stderr with the program's prefix, exit 1, nothing on stdout).
NO_RUN_YET = (
    "goalcup: Run goalcup prep, then goalcup start --harness <id> --practice: no run has been "
    "started from this machine yet. Nothing to show."
)


def status_json(fixture: str, health: str | None = None, run: str | None = None,
                compact: bool = False) -> str:
    """`goalcup status --json` as the CLI prints it: {payload, public_key, signature, cached_at,
    local}. The payload is a real fixture of the protocol crate (the daemon's StatusPayload, whose
    `seconds_remaining` is null for every current run), with `health` and `capture.run` set."""
    payload = json.loads((STATUS_FIXTURES / f"{fixture}.json").read_text())
    if health is not None:
        payload["health"] = health
    if run is not None:
        payload["capture"]["run"] = run
    document = {
        "payload": payload,
        "public_key": "ab" * 32,
        "signature": "cd" * 64,
        "cached_at": "2026-10-23T10:00:00Z",
        "local": {"disk_free_bytes": 123456789, "agent_paused": None},
    }
    if compact:
        return json.dumps(document, separators=(",", ":")) + "\n"
    return json.dumps(document, indent=2) + "\n"


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


def make_release(
    root: Path,
    connectors: dict[str, str],
    version: str = VERSION,
    broken: str | None = None,
) -> Path:
    """Write <root>/release/{SHA256SUMS,goalcup-<version>-<target>.tar.gz}; return the directory.

    `broken` names a binary that fails to run, as a build for a newer glibc does."""
    target = this_target()
    name = f"goalcup-{version}-{target}"
    stage = root / "stage" / name
    for binary in ("goalcup", "goalcup-daemon", "goalcup-verify"):
        path = stage / "bin" / binary
        path.parent.mkdir(parents=True, exist_ok=True)
        if binary == broken:
            path.write_text("#!/bin/sh\necho 'GLIBC_2.34 not found' >&2\nexit 1\n")
        elif binary == "goalcup":
            path.write_text(
                "#!/bin/sh\n"
                f'if [ "$1" = status ]; then echo {NO_RUN_YET!r} >&2; exit 1; fi\n'
                f'echo "{binary} {version}"\n'
            )
        else:
            path.write_text(f'#!/bin/sh\necho "{binary} {version}"\n')
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    for connector, content in connectors.items():
        folder = stage / "share" / "goalcup" / "connectors" / connector
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "harness.toml").write_text(content)
    for text in ("LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.md"):
        (stage / text).write_text(text)
    release = root / "release"
    release.mkdir(exist_ok=True)
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
        self.pathbin.mkdir()
        self.fake_goalcup(self.pathbin / "goalcup", stderr=NO_RUN_YET, rc=1)
        self.base_path = f"{self.pathbin}{os.pathsep}{os.environ['PATH']}"

    def fake_goalcup(self, path: Path, *, document: str | None = None, stderr: str = "",
                     rc: int = 0, hang: bool = False, no_json: bool = False) -> Path:
        """A goalcup that logs its arguments to the returned file and answers `status --json`.

        Its own files live outside the install, which tests compare before and after."""
        path.parent.mkdir(parents=True, exist_ok=True)
        aux = Path(tempfile.mkdtemp(dir=self.root))
        answer, log, errfile = aux / "answer", aux / "log", aux / "err"
        answer.write_text(document or "")
        errfile.write_text(stderr)
        script = f'#!/bin/sh\necho "$@" >> "{log}"\n'
        if no_json:
            script += 'echo "error: unexpected argument \'--json\' found" >&2\nexit 2\n'
        else:
            script += 'if [ "$1 $2" != "status --json" ]; then exit 2; fi\n'
            if hang:
                script += "exec sleep 60\n"
            else:
                script += f'cat "{answer}"\ncat "{errfile}" >&2\nexit {rc}\n'
        path.write_text(script)
        path.chmod(0o755)
        return log

    def install(self, release: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", str(INSTALL), *args],
            env={
                "PATH": self.base_path,
                "HOME": str(self.home),
                "GOALCUP_RELEASE_BASE": release.as_uri(),
                **env,
            },
            capture_output=True,
            text=True,
        )

    def test_installs_binaries_and_connectors_under_the_home(self) -> None:
        release = make_release(self.root, {"codex-cli": "a", "hermes-cli": "b"})
        result = self.install(release)
        self.assertEqual(result.returncode, 0, result.stderr)
        run = subprocess.run(
            [str(self.home / ".local/bin/goalcup"), "--version"], capture_output=True, text=True
        )
        self.assertEqual(run.stdout.strip(), f"goalcup {VERSION}")
        connectors = self.home / ".local/share/goalcup/connectors"
        self.assertEqual(sorted(p.name for p in connectors.iterdir()), ["codex-cli", "hermes-cli"])
        # Nothing else is written in the home directory.
        self.assertEqual([p.name for p in self.home.iterdir()], [".local"])
        # The last line is the next step for a new participant, and the folder reads as typed.
        self.assertIn("~/.local/bin", result.stdout)
        self.assertEqual(result.stdout.strip().splitlines()[-1], "Next: goalcup login")
        # The PATH note names the folder as the guide does, and the export line is the full path.
        self.assertIn("~/.local/bin is not on your PATH", result.stdout)
        self.assertIn(f'export PATH="{self.home}/.local/bin:$PATH"', result.stdout)

    def test_wrong_checksum_installs_nothing(self) -> None:
        release = make_release(self.root, {"codex-cli": "a"})
        sums = release / "SHA256SUMS"
        sums.write_text("0" * 64 + sums.read_text()[64:])
        result = self.install(release)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(list(self.home.iterdir()), [])

    def test_upgrade_replaces_the_connectors_instead_of_merging(self) -> None:
        self.install(make_release(self.root, {"codex-cli": "old", "retired-cli": "x"}))
        second = Path(tempfile.mkdtemp(dir=self.root))
        result = self.install(make_release(second, {"codex-cli": "new"}, version="9.8.8"))
        self.assertEqual(result.returncode, 0, result.stderr)
        connectors = self.home / ".local/share/goalcup/connectors"
        self.assertEqual([p.name for p in connectors.iterdir()], ["codex-cli"])
        self.assertEqual((connectors / "codex-cli/harness.toml").read_text(), "new")
        out = subprocess.run(
            [str(self.home / ".local/bin/goalcup"), "--version"], capture_output=True, text=True
        ).stdout
        self.assertEqual(out.strip(), "goalcup 9.8.8")
        leftovers = [p.name for p in (self.home / ".local/share").iterdir()]
        self.assertEqual(leftovers, ["goalcup"])

    def test_a_release_without_this_target_is_refused(self) -> None:
        release = make_release(self.root, {"codex-cli": "a"})
        (release / "SHA256SUMS").write_text("0" * 64 + "  goalcup-9.8.7-riscv64-unknown-linux-gnu.tar.gz\n")
        result = self.install(release)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no " + this_target() + " build", result.stderr)

    def test_prefix_option_and_path_hint(self) -> None:
        release = make_release(self.root, {"codex-cli": "a"})
        prefix = self.root / "elsewhere"
        result = self.install(release, "--prefix", str(prefix))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((prefix / "bin/goalcup").is_file())
        self.assertIn(f"{prefix}/bin is not on your PATH", result.stdout)
        self.assertEqual(list(self.home.iterdir()), [])

    def fake_tools(self, uname_os: str, uname_arch: str, **tools: str) -> str:
        """A PATH whose `uname` (and any other tool named in `tools`) answers as given."""
        fake = self.root / "fake-bin"
        fake.mkdir(exist_ok=True)
        uname = fake / "uname"
        uname.write_text(f'#!/bin/sh\n[ "$1" = -s ] && echo {uname_os} || echo {uname_arch}\n')
        uname.chmod(0o755)
        for name, output in tools.items():
            tool = fake / name
            tool.write_text(f"#!/bin/sh\necho '{output}'\n")
            tool.chmod(0o755)
        return f"{fake}{os.pathsep}{self.base_path}"

    def test_unsupported_system_is_refused_before_any_download(self) -> None:
        result = self.install(self.root / "missing", PATH=self.fake_tools("FreeBSD", "amd64"))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(self.home.iterdir()), [])

    def test_musl_linux_is_refused_before_any_download(self) -> None:
        path = self.fake_tools("Linux", "x86_64", ldd="musl libc (x86_64)")
        result = self.install(self.root / "missing", PATH=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("musl", result.stderr)

    def test_an_intel_shell_on_apple_silicon_gets_the_arm_build(self) -> None:
        # Under Rosetta uname says x86_64 but sysctl says the CPU is arm64.
        path = self.fake_tools("Darwin", "x86_64", sysctl="1")
        release = make_release(self.root, {"codex-cli": "a"})
        (release / "SHA256SUMS").write_text("0" * 64 + "  goalcup-9.8.7-x86_64-apple-darwin.tar.gz\n")
        result = self.install(release, PATH=path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aarch64-apple-darwin", result.stderr)

    def test_a_binary_that_does_not_run_installs_nothing(self) -> None:
        release = make_release(self.root, {"codex-cli": "a"}, broken="goalcup")
        result = self.install(release)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Installed", result.stdout)
        self.assertEqual(list(self.home.iterdir()), [])

    def test_a_broken_upgrade_leaves_the_old_install_as_it_was(self) -> None:
        self.install(make_release(self.root, {"codex-cli": "old"}))
        second = Path(tempfile.mkdtemp(dir=self.root))
        broken = make_release(second, {"codex-cli": "new"}, version="9.8.8", broken="goalcup-daemon")
        result = self.install(broken)
        self.assertNotEqual(result.returncode, 0)
        for binary in ("goalcup", "goalcup-daemon", "goalcup-verify"):
            out = subprocess.run(
                [str(self.home / ".local/bin" / binary), "--version"], capture_output=True, text=True
            ).stdout
            self.assertEqual(out.strip(), f"{binary} {VERSION}")
        connectors = self.home / ".local/share/goalcup/connectors/codex-cli/harness.toml"
        self.assertEqual(connectors.read_text(), "old")

    def tree(self, root: Path) -> dict[str, bytes | None]:
        """The observable installed files and directories, including unexpected leftovers."""
        return {str(path.relative_to(root)): None if path.is_dir() else path.read_bytes()
                for path in root.rglob("*")}

    def fault_path(self, tool: str, destination: Path, *, signal_after: bool = False) -> str:
        """Fail one filesystem operation, or TERM the installer just after it completes.

        All other operations use the real tool, including the rollback. This replaces only
        an otherwise difficult-to-reproduce filesystem failure, not the installer behavior.
        """
        fake = Path(tempfile.mkdtemp(dir=self.root))
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

    def assert_old_install(self, prefix: Path, before: dict[str, bytes | None]) -> None:
        self.assertEqual(self.tree(prefix), before)
        for binary in ("goalcup", "goalcup-daemon", "goalcup-verify"):
            result = subprocess.run([str(prefix / "bin" / binary), "--version"],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), f"{binary} {VERSION}")

    def test_each_binary_copy_failure_preserves_the_complete_old_install(self) -> None:
        old = make_release(self.root, {"codex-cli": "old"})
        prefix = self.root / "prefix with spaces"
        self.assertEqual(self.install(old, "--prefix", str(prefix)).returncode, 0)
        before = self.tree(prefix)
        newer = make_release(Path(tempfile.mkdtemp(dir=self.root)), {"codex-cli": "new"}, "9.8.8")
        for binary in ("goalcup", "goalcup-daemon", "goalcup-verify"):
            with self.subTest(binary=binary):
                result = self.install(newer, "--prefix", str(prefix),
                                      PATH=self.fault_path("cp", prefix / "bin" / binary))
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Installed", result.stdout)
                self.assert_old_install(prefix, before)

    def test_each_failed_promotion_restores_the_complete_old_install(self) -> None:
        old = make_release(self.root, {"codex-cli": "old"})
        prefix = self.root / "prefix with spaces"
        self.assertEqual(self.install(old, "--prefix", str(prefix)).returncode, 0)
        before = self.tree(prefix)
        newer = make_release(Path(tempfile.mkdtemp(dir=self.root)), {"codex-cli": "new"}, "9.8.8")
        for relative in ("share/goalcup", "bin/goalcup", "bin/goalcup-daemon", "bin/goalcup-verify"):
            with self.subTest(destination=relative):
                result = self.install(newer, "--prefix", str(prefix),
                                      PATH=self.fault_path("mv", prefix / relative))
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Installed", result.stdout)
                self.assert_old_install(prefix, before)

    def test_term_after_each_promotion_restores_the_complete_old_install(self) -> None:
        old = make_release(self.root, {"codex-cli": "old"})
        prefix = self.root / "prefix with spaces"
        self.assertEqual(self.install(old, "--prefix", str(prefix)).returncode, 0)
        before = self.tree(prefix)
        newer = make_release(Path(tempfile.mkdtemp(dir=self.root)), {"codex-cli": "new"}, "9.8.8")
        for relative in ("share/goalcup", "bin/goalcup", "bin/goalcup-daemon", "bin/goalcup-verify"):
            with self.subTest(destination=relative):
                result = self.install(newer, "--prefix", str(prefix),
                                      PATH=self.fault_path("mv", prefix / relative, signal_after=True))
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Installed", result.stdout)
                self.assert_old_install(prefix, before)

    def test_helper_destination_directories_are_preserved_and_refused(self) -> None:
        release = make_release(self.root, {"codex-cli": "new"})
        for binary in ("goalcup-daemon", "goalcup-verify"):
            with self.subTest(binary=binary):
                prefix = Path(tempfile.mkdtemp(dir=self.root))
                conflict = prefix / "bin" / binary
                conflict.mkdir(parents=True)
                (conflict / "keep").write_text("participant data")
                before = self.tree(prefix)
                result = self.install(release, "--prefix", str(prefix))
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Installed", result.stdout)
                self.assertEqual(self.tree(prefix), before)

    def test_space_containing_prefix_installs_and_upgrades_all_three_binaries(self) -> None:
        prefix = self.root / "prefix with spaces"
        for version in (VERSION, "9.8.8"):
            release = make_release(Path(tempfile.mkdtemp(dir=self.root)), {"codex-cli": version}, version)
            result = self.install(release, "--prefix", str(prefix))
            self.assertEqual(result.returncode, 0, result.stderr)
            for binary in ("goalcup", "goalcup-daemon", "goalcup-verify"):
                out = subprocess.run([str(prefix / "bin" / binary), "--version"],
                                     capture_output=True, text=True)
                self.assertEqual(out.returncode, 0, out.stderr)
                self.assertEqual(out.stdout.strip(), f"{binary} {version}")
            self.assertEqual((prefix / "share/goalcup/connectors/codex-cli/harness.toml").read_text(), version)


    # --- never update over a recording run ---------------------------------------------------

    def old_install(self, prefix: Path, goalcup_kwargs: dict) -> tuple[Path, dict[str, bytes | None]]:
        """An existing install under `prefix` whose goalcup answers as `goalcup_kwargs` says."""
        self.assertEqual(self.install(make_release(self.root, {"codex-cli": "old"}),
                                      "--prefix", str(prefix)).returncode, 0)
        log = self.fake_goalcup(prefix / "bin" / "goalcup", **goalcup_kwargs)
        return log, self.tree(prefix)

    def newer(self) -> Path:
        return make_release(Path(tempfile.mkdtemp(dir=self.root)), {"codex-cli": "new"}, "9.8.8")

    def assert_upgraded(self, result: subprocess.CompletedProcess[str], prefix: Path) -> None:
        self.assertEqual(result.returncode, 0, result.stderr)
        out = subprocess.run([str(prefix / "bin/goalcup"), "--version"], capture_output=True, text=True)
        self.assertEqual(out.stdout.strip(), "goalcup 9.8.8")

    def test_a_recording_run_blocks_the_install(self) -> None:
        # health online and the capture run starting, running or exited: the daemon of this
        # release is still needed (an exited run that is not sealed can be continued).
        cases = [("starting", "starting", False), ("recording-pending", "running", False),
                 ("exited", "exited", False), ("recording-pending", "running", True)]
        for fixture, run, compact in cases:
            with self.subTest(run=run, compact=compact):
                prefix = Path(tempfile.mkdtemp(dir=self.root)) / "prefix"
                doc = status_json(fixture, health="online", run=run, compact=compact)
                log, before = self.old_install(prefix, {"document": doc})
                result = self.install(self.newer(), "--prefix", str(prefix))
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertEqual(result.stderr.strip(), INSTALL_BLOCKED)
                self.assertNotIn("Installed", result.stdout)
                self.assertEqual(log.read_text().strip(), "status --json")
                self.assertEqual(self.tree(prefix), before)

    def test_a_run_that_is_not_recording_does_not_block(self) -> None:
        cases = [("sealed run", status_json("sealed-with-gap")),
                 ("recovery needed", status_json("recording-pending", health="sealing_recovery_needed")),
                 ("unknown health", status_json("recording-pending", health="unknown"))]
        for label, doc in cases:
            with self.subTest(label):
                prefix = Path(tempfile.mkdtemp(dir=self.root)) / "prefix"
                self.old_install(prefix, {"document": doc})
                result = self.install(self.newer(), "--prefix", str(prefix))
                self.assert_upgraded(result, prefix)
                self.assertNotIn("INSTALL_", result.stderr)
                self.assertNotIn("recording", result.stderr)
                self.assertNotIn("Could not tell", result.stderr)

    def test_no_run_on_record_goes_on_quietly(self) -> None:
        prefix = self.root / "prefix"
        self.old_install(prefix, {"stderr": NO_RUN_YET, "rc": 1})
        result = self.install(self.newer(), "--prefix", str(prefix))
        self.assert_upgraded(result, prefix)
        self.assertEqual(result.stderr, "")

    def test_a_goalcup_without_status_json_gives_the_unsure_line_and_goes_on(self) -> None:
        prefix = self.root / "prefix"
        self.old_install(prefix, {"no_json": True})
        result = self.install(self.newer(), "--prefix", str(prefix))
        self.assert_upgraded(result, prefix)
        self.assertEqual(result.stderr.strip(), INSTALL_UNSURE)

    def test_output_that_cannot_be_read_gives_the_unsure_line_and_goes_on(self) -> None:
        online_without_capture = json.loads(status_json("legacy-capture-none"))
        self.assertNotIn("capture", online_without_capture["payload"])
        cases = {"not json": ("goalcup 9.8.7\n", "", 0),
                 "empty": ("", "", 0),
                 "other error": ("", "goalcup: the control port is busy", 1),
                 "online without a capture run": (status_json("legacy-capture-none"), "", 0)}
        for label, (document, stderr, rc) in cases.items():
            with self.subTest(label):
                prefix = Path(tempfile.mkdtemp(dir=self.root)) / "prefix"
                self.old_install(prefix, {"document": document, "stderr": stderr, "rc": rc})
                result = self.install(self.newer(), "--prefix", str(prefix))
                self.assert_upgraded(result, prefix)
                self.assertEqual(result.stderr.strip(), INSTALL_UNSURE)

    def test_a_hanging_status_is_stopped_after_10_seconds_and_gives_the_unsure_line(self) -> None:
        prefix = self.root / "prefix"
        self.old_install(prefix, {"hang": True})
        started = time.monotonic()
        result = self.install(self.newer(), "--prefix", str(prefix))
        took = time.monotonic() - started
        self.assert_upgraded(result, prefix)
        self.assertEqual(result.stderr.strip(), INSTALL_UNSURE)
        self.assertGreaterEqual(took, 9.5)
        self.assertLess(took, 25)

    def test_the_prefix_copy_is_asked_before_the_one_on_path(self) -> None:
        recording = status_json("recording-pending", health="online", run="running")
        idle = status_json("sealed-with-gap")
        # Prefix copy recording, PATH copy not: blocked, and the PATH copy is never run.
        prefix = self.root / "prefix"
        prefix_log, before = self.old_install(prefix, {"document": recording})
        path_log = self.fake_goalcup(self.pathbin / "goalcup", document=idle)
        result = self.install(self.newer(), "--prefix", str(prefix))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr.strip(), INSTALL_BLOCKED)
        self.assertEqual(self.tree(prefix), before)
        self.assertTrue(prefix_log.exists())
        self.assertFalse(path_log.exists())
        # No goalcup in the prefix (a first install there): the one on PATH is asked.
        fresh = self.root / "fresh"
        path_log = self.fake_goalcup(self.pathbin / "goalcup", document=recording)
        result = self.install(self.newer(), "--prefix", str(fresh))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr.strip(), INSTALL_BLOCKED)
        self.assertFalse(fresh.exists())
        self.assertEqual(path_log.read_text().strip(), "status --json")

    def test_force_skips_the_guard(self) -> None:
        recording = status_json("recording-pending", health="online", run="running")
        for label, args, env in (("flag", ("--force",), {}), ("environment", (), {"GOALCUP_FORCE": "1"})):
            with self.subTest(label):
                prefix = Path(tempfile.mkdtemp(dir=self.root)) / "prefix"
                log, _ = self.old_install(prefix, {"document": recording})
                result = self.install(self.newer(), "--prefix", str(prefix), *args, **env)
                self.assert_upgraded(result, prefix)
                self.assertEqual(result.stderr, "")
                self.assertFalse(log.exists())

    def test_the_installer_names_the_renamed_repository(self) -> None:
        text = INSTALL.read_text()
        self.assertIn('repo="agentcommunity/goal-cup-proxy"', text)
        self.assertIn("github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh\n", text)
        self.assertNotIn("goal-cup-tracer", text)


if __name__ == "__main__":
    unittest.main()
