#!/usr/bin/env python3
"""Behavioral tests for packaging/install.sh, run against a fake release on disk.

The fake release is a SHA256SUMS file and one tarball for this machine's target, in the layout
scripts/package-release.sh writes. install.sh reads it through GOALCUP_RELEASE_BASE=file://...,
so nothing touches the network.
"""

from __future__ import annotations

import hashlib
import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

INSTALL = Path(__file__).resolve().parent.parent / "packaging" / "install.sh"
VERSION = "9.8.7"


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

    def install(self, release: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", str(INSTALL), *args],
            env={
                "PATH": os.environ["PATH"],
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
        return f"{fake}{os.pathsep}{os.environ['PATH']}"

    def logging_curl(self, release: Path) -> tuple[str, Path]:
        """A PATH whose `curl` logs each URL and serves the file of the same name from `release`."""
        fake = Path(tempfile.mkdtemp(dir=self.root))
        log = fake / "urls"
        curl = fake / "curl"
        curl.write_text(
            "#!/bin/sh\n"
            'out=""; url=""\n'
            'while [ $# -gt 0 ]; do\n'
            '    case "$1" in -o) out=$2; shift 2 ;; *) url=$1; shift ;; esac\n'
            "done\n"
            f'echo "$url" >> "{log}"\n'
            f'cp "{release}/${{url##*/}}" "$out"\n'
        )
        curl.chmod(0o755)
        return f"{fake}{os.pathsep}{os.environ['PATH']}", log

    def test_downloads_come_from_the_goal_cup_proxy_releases(self) -> None:
        # The repository was renamed from goal-cup-tracer. GitHub redirects the old URL, but the
        # installer asks for the new one; GOALCUP_RELEASE_BASE still replaces it.
        release = make_release(self.root, {"codex-cli": "a"})
        github = "https://github.com/agentcommunity/goal-cup-proxy/releases"
        cases = [
            ([], {}, f"{github}/latest/download"),
            (["--version", "v1.2.3"], {}, f"{github}/download/v1.2.3"),
            ([], {"GOALCUP_VERSION": "1.2.4"}, f"{github}/download/v1.2.4"),
            ([], {"GOALCUP_RELEASE_BASE": "https://mirror.example/r/"}, "https://mirror.example/r"),
        ]
        for args, env, base in cases:
            with self.subTest(base=base):
                path, log = self.logging_curl(release)
                prefix = Path(tempfile.mkdtemp(dir=self.root))
                result = subprocess.run(
                    ["sh", str(INSTALL), "--prefix", str(prefix), *args],
                    env={"PATH": path, "HOME": str(self.home), **env},
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                tarball = f"goalcup-{VERSION}-{this_target()}.tar.gz"
                self.assertEqual(log.read_text().split(), [f"{base}/SHA256SUMS", f"{base}/{tarball}"])

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
        return f"{fake}{os.pathsep}{os.environ['PATH']}"

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


if __name__ == "__main__":
    unittest.main()
