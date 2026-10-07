#!/usr/bin/env python3
"""Behavioral tests for the bounded release staging tool."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("package-release.py")
REPO_ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("package_release", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
package_release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package_release)

TARGET = "aarch64-apple-darwin"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(64 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def tree_snapshot(root: Path) -> dict[str, tuple[str, int, bytes | None]]:
    """Capture names, modes and regular-file bytes without following links."""
    result: dict[str, tuple[str, int, bytes | None]] = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        mode = stat.S_IMODE(info.st_mode)
        if stat.S_ISREG(info.st_mode):
            result[relative] = ("file", mode, path.read_bytes())
        elif stat.S_ISDIR(info.st_mode):
            result[relative] = ("directory", mode, None)
        elif stat.S_ISLNK(info.st_mode):
            result[relative] = ("symlink", mode, os.readlink(path).encode())
        else:
            result[relative] = ("special", mode, None)
    return result


class StagingFixture:
    def __init__(self, test: unittest.TestCase) -> None:
        # Resolve the platform temporary alias (for example macOS /var) so tests
        # can distinguish fixture-created symlinks from system path aliases.
        self.temp = Path(test.enterContext(tempfile.TemporaryDirectory())).resolve()
        self.repo = self.temp / "trusted repo"
        self.binaries = self.temp / "built binaries"
        self.repo.mkdir()
        self.binaries.mkdir()
        (self.repo / "connectors").mkdir()
        self.extension = ""
        self.target = TARGET

    def connector(
        self,
        directory_name: str,
        connector_id: str,
        *,
        files: dict[str, tuple[bytes, int]] | None = None,
    ) -> Path:
        connector = self.repo / "connectors" / directory_name
        connector.mkdir()
        harness = f"id = {json.dumps(connector_id, ensure_ascii=False)}\n".encode()
        (connector / "harness.toml").write_bytes(harness)
        for relative, (content, mode) in (files or {}).items():
            destination = connector / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(content)
            destination.chmod(mode)
        return connector

    def binaries_all(self, *, executable_scripts: bool = False) -> None:
        for index, name in enumerate(package_release.BINARIES):
            path = self.binaries / f"{name}{self.extension}"
            if executable_scripts:
                path.write_text(
                    f"#!/bin/sh\nprintf executed > {self.temp / ('ran-' + name)}\n",
                    encoding="utf-8",
                )
            else:
                path.write_bytes(b"binary\0" + bytes([index]) + name.encode())
            path.chmod(0o711)

    def minimal_complete(self, *, connector_count: int = 3) -> None:
        for index in range(connector_count):
            self.connector(f"connector-{index}", f"connector-{index}")
        self.binaries_all()

    def stage(
        self, output: Path, *, real_closure: bool = False, **overrides: object
    ) -> Path:
        """Stage with the pilot connector set widened to what the fixture created.

        The generic staging behaviours (modes, links, overlap, determinism) are asserted on
        throwaway connectors; `real_closure=True` keeps the shipped two-connector set.
        """
        arguments: dict[str, object] = {
            "repo_root": self.repo,
            "binaries": self.binaries,
            "output": output,
            "target": self.target,
            "revision": "deadbeef",
            "version": "1.2.3-test",
        }
        arguments.update(overrides)
        if real_closure:
            return package_release.stage(**arguments)
        repo = Path(arguments["repo_root"])
        present = frozenset(
            entry.name for entry in (repo / "connectors").iterdir()
        )
        with mock.patch.object(package_release, "M2_PILOT_CONNECTORS", present):
            return package_release.stage(**arguments)


class PackageReleaseTests(unittest.TestCase):
    def test_stages_exactly_the_three_pilot_binaries_without_devserver(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        (fixture.binaries / "goalcup-devserver").write_bytes(b"must stay out")
        (fixture.binaries / "goalcup-mcp").write_bytes(b"must stay out either")
        (fixture.binaries / "unrelated-tool").write_bytes(b"must stay out too")

        output = fixture.stage(fixture.temp / "stage")

        self.assertEqual(
            sorted(path.name for path in (output / "bin").iterdir()),
            sorted(package_release.BINARIES),
        )
        self.assertFalse((output / "bin" / "goalcup-devserver").exists())
        for name in package_release.BINARIES:
            self.assertEqual(
                stat.S_IMODE((output / "bin" / name).stat().st_mode), 0o755
            )
        manifest = json.loads((output / "bundle.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["format"], 1)
        self.assertEqual(manifest["target"], TARGET)
        self.assertEqual(manifest["revision"], "deadbeef")
        self.assertEqual(manifest["version"], "1.2.3-test")
        self.assertEqual(manifest["qualification"], "unqualified")
        self.assertEqual(manifest["signature"], "unsigned")

    def test_windows_host_native_attributes_are_checked(self) -> None:
        writable = types.SimpleNamespace(
            st_mode=stat.S_IFREG | 0o666,
            st_file_attributes=0,
        )
        readonly = types.SimpleNamespace(
            st_mode=stat.S_IFREG | 0o666,
            st_file_attributes=package_release._READONLY_ATTRIBUTE,
        )
        reparse = types.SimpleNamespace(
            st_mode=stat.S_IFREG | 0o666,
            st_file_attributes=package_release._REPARSE_POINT,
        )
        package_release._validate_staged_file(
            writable, 0o644, "bin/goalcup", windows_host=True
        )
        with self.assertRaisesRegex(ValueError, "read-only"):
            package_release._validate_staged_file(
                readonly, 0o644, "bin/goalcup", windows_host=True
            )
        with self.assertRaisesRegex(ValueError, "reparse"):
            package_release._validate_staged_file(
                reparse, 0o644, "bin/goalcup", windows_host=True
            )

    def test_a_v1_compat_daemon_is_refused_before_creating_output(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        (fixture.binaries / "goalcup-daemon").write_bytes(b"goalcup-daemon 0.1.0+v1-compat\0")
        output = fixture.temp / "stage"

        with self.assertRaisesRegex(ValueError, "v1-compat"):
            fixture.stage(output)

        self.assertFalse(output.exists())
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])

    def test_missing_binary_is_named_and_leaves_no_output(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        (fixture.binaries / "goalcup-verify").unlink()
        output = fixture.temp / "stage"

        with self.assertRaisesRegex(ValueError, "goalcup-verify"):
            fixture.stage(output)

        self.assertFalse(output.exists())
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])

    def test_discovers_a_fourth_connector_without_a_count_constant(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete(connector_count=4)

        output = fixture.stage(fixture.temp / "stage")

        self.assertEqual(
            sorted(path.name for path in (output / "connectors").iterdir()),
            [f"connector-{index}" for index in range(4)],
        )

    def test_real_connector_tree_stages_with_dummy_binaries(self) -> None:
        fixture = StagingFixture(self)
        fixture.binaries_all(executable_scripts=True)
        output = fixture.stage(
            fixture.temp / "real-connectors-stage", repo_root=REPO_ROOT
        )

        expected_connectors = sorted(
            path.name for path in (REPO_ROOT / "connectors").iterdir()
        )
        self.assertEqual(
            sorted(path.name for path in (output / "connectors").iterdir()),
            expected_connectors,
        )
        source_harness = REPO_ROOT / "connectors" / "codex-cli" / "harness.toml"
        staged_harness = output / "connectors" / "codex-cli" / "harness.toml"
        self.assertEqual(staged_harness.read_bytes(), source_harness.read_bytes())
        for name in package_release.BINARIES:
            self.assertFalse((fixture.temp / f"ran-{name}").exists())

    def test_connector_directory_must_match_declared_harness_id(self) -> None:
        fixture = StagingFixture(self)
        fixture.binaries_all()
        fixture.connector("directory-name", "declared-id")

        with self.assertRaisesRegex(
            ValueError, "connector directory.*directory-name.*declares.*declared-id"
        ):
            fixture.stage(fixture.temp / "stage")

    def test_ids_follow_protocol_nonempty_rule(self) -> None:
        fixture = StagingFixture(self)
        fixture.binaries_all()
        fixture.connector("directory with spaces ü", "directory with spaces ü")
        output = fixture.stage(fixture.temp / "valid-stage")
        self.assertTrue(
            (output / "connectors" / "directory with spaces ü" / "harness.toml").is_file()
        )

        invalid = StagingFixture(self)
        invalid.binaries_all()
        invalid.connector("empty", "")
        with self.assertRaisesRegex(ValueError, "invalid connector id"):
            invalid.stage(invalid.temp / "invalid-stage")

    def test_non_string_and_malformed_harness_ids_are_refused(self) -> None:
        for harness_bytes in (b"id = 42\n", b"id = [\n"):
            with self.subTest(harness=harness_bytes):
                fixture = StagingFixture(self)
                fixture.binaries_all()
                connector = fixture.repo / "connectors" / "bad"
                connector.mkdir()
                (connector / "harness.toml").write_bytes(harness_bytes)
                with self.assertRaisesRegex(ValueError, "harness|connector id"):
                    fixture.stage(fixture.temp / "stage")

    def test_preserves_bytes_modes_and_inventories_every_payload_file(self) -> None:
        fixture = StagingFixture(self)
        hook = b"#!/bin/sh\nprintf '\\000raw \\303\\274 bytes'\n"
        fixture.connector(
            "connector ü",
            "connector ü",
            files={
                "files/hook with spaces.sh": (hook, 0o751),
                "files/template \N{SNOWMAN}.txt": (b"template\0bytes\n", 0o640),
            },
        )
        (fixture.repo / "connectors/connector ü/files").chmod(0o750)
        fixture.binaries_all()

        output = fixture.stage(fixture.temp / "stage")
        staged_hook = output / "connectors/connector ü/files/hook with spaces.sh"
        self.assertEqual(staged_hook.read_bytes(), hook)
        self.assertEqual(stat.S_IMODE(staged_hook.stat().st_mode), 0o751)
        self.assertEqual(stat.S_IMODE(staged_hook.parent.stat().st_mode), 0o750)

        manifest = json.loads((output / "bundle.json").read_text(encoding="utf-8"))
        paths = [entry["path"] for entry in manifest["files"]]
        expected_paths = sorted(
            path.relative_to(output).as_posix()
            for path in output.rglob("*")
            if path.is_file() and path.name != "bundle.json"
        )
        self.assertEqual(paths, expected_paths)
        self.assertEqual(paths, sorted(paths))
        self.assertNotIn("bundle.json", paths)
        for entry in manifest["files"]:
            staged = output / entry["path"]
            self.assertEqual(entry["type"], "file")
            self.assertEqual(entry["size"], staged.stat().st_size)
            self.assertEqual(entry["sha256"], sha256(staged))
            self.assertRegex(entry["mode"], r"^0[0-7]{3}$")
            self.assertEqual(int(entry["mode"], 8), stat.S_IMODE(staged.stat().st_mode))

    def test_manifest_and_payload_are_deterministic(self) -> None:
        fixture = StagingFixture(self)
        fixture.connector(
            "z connector",
            "z connector",
            files={"files/z": (b"z", 0o600), "files/a": (b"a", 0o755)},
        )
        fixture.connector("a connector", "a connector")
        fixture.binaries_all()

        first = fixture.stage(fixture.temp / "first")
        second = fixture.stage(fixture.temp / "second")

        self.assertEqual(tree_snapshot(first), tree_snapshot(second))
        self.assertEqual(
            (first / "bundle.json").read_bytes(),
            (second / "bundle.json").read_bytes(),
        )

    def test_inputs_are_not_mutated_or_executed(self) -> None:
        fixture = StagingFixture(self)
        fixture.connector(
            "connector",
            "connector",
            files={"files/hook.sh": (b"#!/bin/sh\nexit 99\n", 0o711)},
        )
        fixture.binaries_all(executable_scripts=True)
        before_repo = tree_snapshot(fixture.repo)
        before_binaries = tree_snapshot(fixture.binaries)

        fixture.stage(fixture.temp / "stage")

        self.assertEqual(tree_snapshot(fixture.repo), before_repo)
        self.assertEqual(tree_snapshot(fixture.binaries), before_binaries)
        for name in package_release.BINARIES:
            self.assertFalse((fixture.temp / f"ran-{name}").exists())

    def test_unknown_target_refuses_before_creating_output(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        output = fixture.temp / "stage"
        with self.assertRaisesRegex(ValueError, "unsupported target"):
            fixture.stage(output, target="wasm32-unknown-unknown")
        self.assertFalse(output.exists())

    def test_cli_requires_explicit_stage_mode(self) -> None:
        fixture = StagingFixture(self)
        for name in package_release.M2_PILOT_CONNECTORS:
            fixture.connector(name, name)
        fixture.binaries_all()
        output = fixture.temp / "stage"
        command = [
            sys.executable,
            "-I",
            "-B",
            "-S",
            os.fspath(SCRIPT),
            "--repo",
            os.fspath(fixture.repo),
            "--binaries",
            os.fspath(fixture.binaries),
            "--output",
            os.fspath(output),
            "--target",
            fixture.target,
            "--revision",
            "deadbeef",
            "--version",
            "1.2.3-test",
        ]

        result = subprocess.run(command, capture_output=True, text=True, check=False)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--stage is required", result.stderr)
        self.assertFalse(output.exists())

        staged = subprocess.run(
            [*command, "--stage"], capture_output=True, text=True, check=False
        )
        self.assertEqual(staged.returncode, 0, staged.stderr)
        self.assertTrue((output / "bundle.json").is_file())

    def test_existing_output_is_preserved(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        output = fixture.temp / "existing stage"
        output.mkdir()
        sentinel = output / "preserve me"
        sentinel.write_bytes(b"previous evidence")

        with self.assertRaisesRegex(ValueError, "output already exists"):
            fixture.stage(output)

        self.assertEqual(sentinel.read_bytes(), b"previous evidence")
        self.assertEqual(list(output.iterdir()), [sentinel])

    def test_output_overlap_is_refused_in_every_direction(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        before_repo = tree_snapshot(fixture.repo)
        before_binaries = tree_snapshot(fixture.binaries)
        cases = {
            "equal repo": fixture.repo,
            "equal binaries": fixture.binaries,
            "inside repo": fixture.repo / "new output",
            "inside binaries": fixture.binaries / "new output",
            "ancestor of both": fixture.temp,
        }
        for label, output in cases.items():
            with self.subTest(label=label):
                with self.assertRaisesRegex(ValueError, "output overlaps"):
                    fixture.stage(output)
        self.assertEqual(tree_snapshot(fixture.repo), before_repo)
        self.assertEqual(tree_snapshot(fixture.binaries), before_binaries)

    def test_output_ancestor_checks_each_input_independently(self) -> None:
        for input_name in ("repo", "binaries"):
            with self.subTest(input=input_name):
                fixture = StagingFixture(self)
                fixture.minimal_complete()
                ancestor = fixture.temp / f"{input_name}-parent"
                ancestor.mkdir()
                if input_name == "repo":
                    moved = ancestor / "repo"
                    fixture.repo.rename(moved)
                    fixture.repo = moved
                else:
                    moved = ancestor / "binaries"
                    fixture.binaries.rename(moved)
                    fixture.binaries = moved

                with self.assertRaisesRegex(ValueError, "output overlaps"):
                    fixture.stage(ancestor)

    def test_connector_entries_must_be_directories_with_harnesses(self) -> None:
        for case in ("root file", "missing harness"):
            with self.subTest(case=case):
                fixture = StagingFixture(self)
                fixture.binaries_all()
                if case == "root file":
                    (fixture.repo / "connectors/stray").write_bytes(b"not a connector")
                else:
                    (fixture.repo / "connectors/no-harness").mkdir()

                with self.assertRaisesRegex(
                    ValueError, "ordinary directory|required|missing connector harness"
                ):
                    fixture.stage(fixture.temp / "stage")

    def test_symlinks_at_source_roots_ancestors_and_leaves_are_refused(self) -> None:
        cases = (
            "repo root",
            "repo ancestor",
            "binary root",
            "connectors root",
            "connector directory",
            "nested directory",
            "binary leaf",
            "connector file",
        )
        for case in cases:
            with self.subTest(case=case):
                fixture = StagingFixture(self)
                fixture.minimal_complete()
                repo = fixture.repo
                binaries = fixture.binaries
                if case == "repo root":
                    alias = fixture.temp / "repo-link"
                    alias.symlink_to(repo, target_is_directory=True)
                    repo = alias
                elif case == "repo ancestor":
                    real_parent = fixture.temp / "real-parent"
                    real_parent.mkdir()
                    repo.rename(real_parent / "repo")
                    alias_parent = fixture.temp / "alias-parent"
                    alias_parent.symlink_to(real_parent, target_is_directory=True)
                    repo = alias_parent / "repo"
                elif case == "binary root":
                    alias = fixture.temp / "binaries-link"
                    alias.symlink_to(binaries, target_is_directory=True)
                    binaries = alias
                elif case == "connectors root":
                    connectors = repo / "connectors"
                    real = repo / "real-connectors"
                    connectors.rename(real)
                    connectors.symlink_to(real, target_is_directory=True)
                elif case == "connector directory":
                    connector = repo / "connectors/connector-0"
                    real = fixture.temp / "outside-connector"
                    connector.rename(real)
                    connector.symlink_to(real, target_is_directory=True)
                elif case == "nested directory":
                    connector = repo / "connectors/connector-0"
                    outside = fixture.temp / "outside-directory"
                    outside.mkdir()
                    (outside / "secret").write_bytes(b"outside")
                    (connector / "files").symlink_to(outside, target_is_directory=True)
                elif case == "binary leaf":
                    binary = binaries / "goalcup"
                    outside = fixture.temp / "outside-binary"
                    outside.write_bytes(b"outside")
                    binary.unlink()
                    binary.symlink_to(outside)
                elif case == "connector file":
                    connector = repo / "connectors/connector-0"
                    outside = fixture.temp / "outside-file"
                    outside.write_bytes(b"outside")
                    (connector / "files").mkdir()
                    (connector / "files/secret").symlink_to(outside)

                with self.assertRaisesRegex(ValueError, "symlink|unsafe|source path"):
                    fixture.stage(
                        fixture.temp / f"stage-{case.replace(' ', '-')}",
                        repo_root=repo,
                        binaries=binaries,
                    )

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO creation is unavailable")
    def test_special_files_in_binaries_and_connectors_are_refused(self) -> None:
        for location in ("binary", "connector"):
            with self.subTest(location=location):
                fixture = StagingFixture(self)
                fixture.minimal_complete()
                if location == "binary":
                    path = fixture.binaries / "goalcup"
                    path.unlink()
                else:
                    path = fixture.repo / "connectors/connector-0/fifo"
                os.mkfifo(path)

                with self.assertRaisesRegex(ValueError, "non-regular|special|unsafe"):
                    fixture.stage(fixture.temp / f"stage-{location}")

    def test_windows_reparse_attribute_is_treated_as_unsafe(self) -> None:
        reparse = types.SimpleNamespace(st_file_attributes=0x400)
        regular = types.SimpleNamespace(st_file_attributes=0)
        self.assertTrue(package_release._is_reparse(reparse))
        self.assertFalse(package_release._is_reparse(regular))

    def test_injected_copy_failure_leaves_no_final_or_temporary_bundle(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        output = fixture.temp / "stage"
        unrelated = fixture.temp / "stage.tmp-keep"
        unrelated.mkdir()
        (unrelated / "evidence").write_bytes(b"unrelated")
        original = package_release._copy_regular_file
        calls = 0

        def fail_during_copy(source: Path, destination: Path, mode: int) -> None:
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError("injected copy failure")
            original(source, destination, mode)

        with mock.patch.object(
            package_release, "_copy_regular_file", side_effect=fail_during_copy
        ):
            with self.assertRaisesRegex(OSError, "injected copy failure"):
                fixture.stage(output)

        self.assertGreaterEqual(calls, 3)
        self.assertFalse(output.exists())
        self.assertEqual((unrelated / "evidence").read_bytes(), b"unrelated")
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [unrelated])

    def test_atomic_publish_refuses_a_racing_destination_and_preserves_it(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()
        output = fixture.temp / "stage"
        publish = package_release._publish_no_replace
        racing_identity: tuple[int, int] | None = None

        def race(temporary: Path, destination: Path) -> None:
            nonlocal racing_identity
            destination.mkdir()
            info = destination.stat()
            racing_identity = (info.st_dev, info.st_ino)
            publish(temporary, destination)

        with mock.patch.object(package_release, "_publish_no_replace", side_effect=race):
            with self.assertRaisesRegex(ValueError, "output already exists"):
                fixture.stage(output)

        info = output.stat()
        self.assertEqual((info.st_dev, info.st_ino), racing_identity)
        self.assertEqual(list(output.iterdir()), [])
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])

    # ------------------------------------------------------------------ profiles

    def test_m2_pilot_is_the_default_and_only_profile(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()

        explicit = fixture.stage(fixture.temp / "explicit", profile="m2-pilot")
        implicit = fixture.stage(fixture.temp / "implicit")

        self.assertEqual(tree_snapshot(explicit), tree_snapshot(implicit))
        manifest = json.loads((explicit / "bundle.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["profile"], "m2-pilot")
        self.assertEqual(package_release.PROFILES, ("m2-pilot",))

    def test_retired_and_unknown_profiles_are_refused(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()

        for profile in ("release", "foundation", "staging"):
            output = fixture.temp / f"stage-{profile}"
            with self.assertRaisesRegex(ValueError, "unsupported profile"):
                fixture.stage(output, profile=profile)
            self.assertFalse(output.exists())

    def test_non_macos_targets_are_refused_before_creating_output(self) -> None:
        fixture = StagingFixture(self)
        fixture.minimal_complete()

        for target in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"):
            output = fixture.temp / f"stage-{target}"
            with self.assertRaisesRegex(ValueError, "unsupported target"):
                fixture.stage(output, target=target)
            self.assertFalse(output.exists())

    def test_m2_pilot_missing_codex_connector_is_named_and_leaves_no_output(self) -> None:
        fixture = StagingFixture(self)
        fixture.connector("claude-code-cli", "claude-code-cli")
        fixture.connector("unshipped-harness", "unshipped-harness")
        fixture.binaries_all()
        output = fixture.temp / "stage"

        with self.assertRaisesRegex(ValueError, "missing required connector: codex-cli"):
            fixture.stage(output, real_closure=True)

        self.assertFalse(output.exists())
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])

    def test_m2_pilot_missing_required_peer_is_named_and_leaves_no_output(self) -> None:
        # The pilot allowlist is one release closure. Refuse an incomplete copy at staging time
        # rather than silently changing which measured harnesses the named profile ships.
        fixture = StagingFixture(self)
        fixture.connector("codex-cli", "codex-cli")
        fixture.binaries_all()
        output = fixture.temp / "stage"

        with self.assertRaisesRegex(
            ValueError, "missing required connector: claude-code-cli"
        ):
            fixture.stage(output, real_closure=True)

        self.assertFalse(output.exists())
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])

    def test_m2_pilot_mismatched_binary_is_refused_not_silently_staged(self) -> None:
        # A binary that changes identity mid-copy must abort rather than produce a bundle
        # around stale bytes.
        fixture = self._m2_pilot_fixture()
        output = fixture.temp / "stage"
        changed_binary = fixture.binaries / "goalcup-daemon"
        original = package_release._copy_regular_file
        mutated = False

        def mutate_after_copy(source, destination, mode) -> None:
            nonlocal mutated
            original(source, destination, mode)
            if source == changed_binary and not mutated:
                source.write_bytes(b"replaced binary bytes")
                mutated = True

        with mock.patch.object(
            package_release, "_copy_regular_file", side_effect=mutate_after_copy
        ):
            with self.assertRaisesRegex(ValueError, "source changed during staging"):
                fixture.stage(output, real_closure=True)

        self.assertTrue(mutated)
        self.assertFalse(output.exists())
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])

    def test_m2_pilot_staging_is_deterministic_across_two_runs(self) -> None:
        fixture = StagingFixture(self)
        fixture.connector(
            "codex-cli",
            "codex-cli",
            files={"files/helper.sh": (b"#!/bin/sh\nexit 0\n", 0o755)},
        )
        fixture.connector("claude-code-cli", "claude-code-cli")
        fixture.binaries_all()

        first = fixture.stage(fixture.temp / "first", real_closure=True)
        second = fixture.stage(fixture.temp / "second", real_closure=True)

        self.assertEqual(tree_snapshot(first), tree_snapshot(second))
        self.assertEqual(
            (first / "bundle.json").read_bytes(),
            (second / "bundle.json").read_bytes(),
        )

    def test_m2_pilot_stages_the_real_connector_closure_only(self) -> None:
        fixture = StagingFixture(self)
        fixture.binaries_all(executable_scripts=True)

        output = fixture.stage(
            fixture.temp / "real-stage", real_closure=True, repo_root=REPO_ROOT
        )

        self.assertEqual(
            sorted(path.name for path in (output / "connectors").iterdir()),
            sorted(package_release.M2_PILOT_CONNECTORS),
        )
        self.assertFalse((output / "connectors" / "unshipped-harness").exists())
        for name in package_release.M2_PILOT_CONNECTORS:
            source_harness = REPO_ROOT / "connectors" / name / "harness.toml"
            staged_harness = output / "connectors" / name / "harness.toml"
            self.assertEqual(staged_harness.read_bytes(), source_harness.read_bytes())
        for name in package_release.M2_PILOT_BINARIES:
            self.assertFalse((fixture.temp / f"ran-{name}").exists())

    # ------------------------------------------------------------------ M2 pilot profile

    def _m2_pilot_fixture(self) -> StagingFixture:
        fixture = StagingFixture(self)
        for name in package_release.M2_PILOT_CONNECTORS:
            fixture.connector(name, name)
        fixture.binaries_all()
        return fixture

    def test_m2_pilot_exact_three_binary_allowlist(self) -> None:
        fixture = self._m2_pilot_fixture()

        output = fixture.stage(fixture.temp / "m2-pilot-stage", profile="m2-pilot", real_closure=True)

        self.assertEqual(
            sorted(path.name for path in (output / "bin").iterdir()),
            sorted(package_release.M2_PILOT_BINARIES),
        )
        self.assertEqual(
            sorted(path.name for path in (output / "connectors").iterdir()),
            sorted(package_release.M2_PILOT_CONNECTORS),
        )

    def test_m2_pilot_excludes_private_artifacts(self) -> None:
        fixture = self._m2_pilot_fixture()
        for private_name in ("harnesses", "homes", "keys", "traces", "reports", ".cache"):
            (fixture.repo / private_name).mkdir()
            (fixture.repo / private_name / "private.txt").write_text("private")
        (fixture.binaries / "goalcup-mcp").write_bytes(b"MCP must stay out")
        (fixture.binaries / "goalcup-devserver").write_bytes(b"devserver must stay out")

        output = fixture.stage(fixture.temp / "m2-pilot-stage", profile="m2-pilot", real_closure=True)

        payload_paths = {
            path.relative_to(output).as_posix() for path in output.rglob("*")
        }
        private_roots = ("harnesses/", "homes/", "keys/", "traces/", "reports/", ".cache/")
        self.assertFalse(any(path.startswith(private_roots) for path in payload_paths))
        self.assertFalse((output / "bin" / "goalcup-mcp").exists())
        self.assertFalse((output / "bin" / "goalcup-devserver").exists())

    def test_m2_pilot_local_config_needs_no_service(self) -> None:
        fixture = self._m2_pilot_fixture()

        output = fixture.stage(fixture.temp / "m2-pilot-stage", profile="m2-pilot", real_closure=True)

        manifest = json.loads((output / "bundle.json").read_text(encoding="utf-8"))
        self.assertNotIn("upload_url", manifest)
        self.assertNotIn("server_public_key", manifest)
        self.assertNotIn("receipt_service", manifest)
        paths = " ".join(item["path"] for item in manifest["files"])
        self.assertNotIn("devserver", paths)

    def test_m2_pilot_is_private_unsigned_candidate_bound_to_target_revision_and_profiles(
        self,
    ) -> None:
        fixture = self._m2_pilot_fixture()
        output = fixture.stage(
            fixture.temp / "m2-pilot-stage",
            profile="m2-pilot",
            revision="0123456789abcdef0123456789abcdef01234567",
        )

        manifest = json.loads((output / "bundle.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["profile"], "m2-pilot")
        self.assertEqual(manifest["target"], fixture.target)
        self.assertEqual(manifest["revision"], "0123456789abcdef0123456789abcdef01234567")
        self.assertEqual(manifest["version"], "1.2.3-test")
        self.assertTrue(manifest["candidate"])
        self.assertEqual(manifest["distribution"], "private")
        self.assertEqual(manifest["qualification"], "unqualified")
        self.assertEqual(manifest["signature"], "unsigned")
        self.assertEqual(
            manifest["labels"],
            [
                "candidate",
                "m2-pilot",
                "macos/aarch64",
                "private",
                "unnotarized",
                "unqualified",
                "unsigned",
            ],
        )
        self.assertEqual(
            set(manifest["profile_sha256"]), set(package_release.M2_PILOT_CONNECTORS)
        )
        self.assertNotIn("harness_sha256", manifest)
        inventory = {item["path"]: item["sha256"] for item in manifest["files"]}
        for connector in package_release.M2_PILOT_CONNECTORS:
            self.assertEqual(
                manifest["profile_sha256"][connector],
                inventory[f"connectors/{connector}/harness.toml"],
            )

    def test_source_change_during_copy_prevents_publication(self) -> None:
        fixture = StagingFixture(self)
        connector = fixture.connector(
            "connector", "connector", files={"files/hook": (b"before", 0o700)}
        )
        fixture.binaries_all()
        output = fixture.temp / "stage"
        source_to_change = connector / "files/hook"
        original = package_release._copy_regular_file
        changed = False

        def mutate_after_copy(source: Path, destination: Path, mode: int) -> None:
            nonlocal changed
            original(source, destination, mode)
            if source == source_to_change and not changed:
                source.write_bytes(b"changed by simulated concurrent writer")
                changed = True

        with mock.patch.object(
            package_release, "_copy_regular_file", side_effect=mutate_after_copy
        ):
            with self.assertRaisesRegex(ValueError, "source changed during staging"):
                fixture.stage(output)

        self.assertTrue(changed)
        self.assertFalse(output.exists())
        self.assertEqual(list(fixture.temp.glob("stage.tmp-*")), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
