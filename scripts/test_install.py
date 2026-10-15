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
import stat
import subprocess
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


def make_release(root: Path, connectors: dict[str, str], version: str = VERSION) -> Path:
    """Write <root>/release/{SHA256SUMS,goalcup-<version>-<target>.tar.gz}; return the directory."""
    target = this_target()
    name = f"goalcup-{version}-{target}"
    stage = root / "stage" / name
    for binary in ("goalcup", "goalcup-daemon", "goalcup-verify"):
        path = stage / "bin" / binary
        path.parent.mkdir(parents=True, exist_ok=True)
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

    def test_unsupported_system_is_refused_before_any_download(self) -> None:
        # A fake `uname` reports FreeBSD; GOALCUP_RELEASE_BASE points at nothing.
        fake = self.root / "bin"
        fake.mkdir()
        uname = fake / "uname"
        uname.write_text('#!/bin/sh\n[ "$1" = -s ] && echo FreeBSD || echo amd64\n')
        uname.chmod(0o755)
        result = self.install(
            self.root / "missing", PATH=f"{fake}{os.pathsep}{os.environ['PATH']}"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no build for 'FreeBSD'", result.stderr)


if __name__ == "__main__":
    unittest.main()
