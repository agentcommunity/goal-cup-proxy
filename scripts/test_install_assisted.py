#!/usr/bin/env python3
"""Fail-closed assisted-installer tests.

These tests use only disposable directories. They do not read or write a real harness home,
contact a model service, or install any service: the installer places files and nothing else.
"""

from __future__ import annotations

import hashlib
import importlib.util
import inspect
import json
import errno
import os
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "packaging" / "macos" / "install_assisted.py"
SHELL_PATH = ROOT / "packaging" / "macos" / "install-assisted.sh"
BINARIES = ("goalcup", "goalcup-daemon", "goalcup-verify")
CONNECTORS = ("codex-cli", "claude-code-cli")
# `.get`: on a machine name macOS never reports (Linux `aarch64`) the import still succeeds and
# the class below is skipped, instead of discovery failing with a KeyError.
ARCH = {"arm64": "macos/aarch64", "x86_64": "macos/x86_64"}.get(os.uname().machine, "")
TARGET = {"macos/aarch64": "aarch64-apple-darwin", "macos/x86_64": "x86_64-apple-darwin"}.get(
    ARCH, ""
)


def installer():
    if not MODULE_PATH.exists():
        raise AssertionError(f"installer module is missing: {MODULE_PATH}")
    spec = importlib.util.spec_from_file_location("goalcup_install_assisted", MODULE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_bundle(root: Path, *, extra_entries: list[dict[str, object]] | None = None) -> Path:
    bundle = root / "bundle"
    payload: dict[str, tuple[bytes, int]] = {}
    for name in BINARIES:
        payload[f"bin/{name}"] = (f"{name}\n".encode(), 0o755)
    for name in CONNECTORS:
        payload[f"connectors/{name}/harness.toml"] = (
            f'id = "{name}"\n'.encode(),
            0o644,
        )
        payload[f"connectors/{name}/rules.txt"] = (f"{name} rules\n".encode(), 0o640)

    files: list[dict[str, object]] = []
    for relative, (data, mode) in sorted(payload.items()):
        path = bundle / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(mode)
        files.append(
            {
                "path": relative,
                "type": "file",
                "sha256": sha256(data),
                "size": len(data),
                "mode": f"{mode:04o}",
            }
        )
    if extra_entries:
        files.extend(extra_entries)

    inventory = {entry["path"]: entry["sha256"] for entry in files}
    manifest = {
        "format": 1,
        "target": TARGET,
        "revision": "a" * 40,
        "version": "0.0.0-test",
        "profile": "m2-pilot",
        "qualification": "unqualified",
        "signature": "unsigned",
        "files": files,
        "candidate": True,
        "distribution": "private",
        "labels": sorted(
            {
                "m2-pilot",
                "candidate",
                "private",
                "unsigned",
                "unnotarized",
                "unqualified",
                ARCH,
            }
        ),
        "profile_sha256": {
            name: inventory[f"connectors/{name}/harness.toml"] for name in CONNECTORS
        },
    }
    (bundle / "bundle.json").write_text(
        json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    return bundle


def rewrite_manifest_path(bundle: Path, old: str, new: str) -> None:
    manifest_path = bundle / "bundle.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for entry in manifest["files"]:
        if entry["path"] == old:
            entry["path"] = new
            break
    manifest_path.write_text(
        json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )


def tree_fingerprint(path: Path) -> tuple[tuple[str, int, int, int, str], ...]:
    root = path.parent
    items = [path]
    if path.is_dir() and not path.is_symlink():
        items.extend(sorted(path.rglob("*")))
    result: list[tuple[str, int, int, int, str]] = []
    for item in items:
        info = item.lstat()
        digest = ""
        if stat.S_ISREG(info.st_mode):
            digest = hashlib.sha256(item.read_bytes()).hexdigest()
        result.append(
            (
                item.relative_to(root).as_posix(),
                info.st_ino,
                stat.S_IMODE(info.st_mode),
                info.st_size,
                digest,
            )
        )
    return tuple(result)


def wait_for_path(path: Path, timeout: float = 5.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return True
        time.sleep(0.01)
    return path.exists()


@unittest.skipUnless(sys.platform == "darwin", "macOS installer")
class AssistedInstallerTests(unittest.TestCase):
    def inspect(self, bundle: Path):
        module = installer()
        return module.inspect_bundle(bundle, "m2-pilot", BINARIES, CONNECTORS, ARCH)

    def shell_env(self, root: Path) -> dict[str, str]:
        home = root / "home"
        home.mkdir(mode=0o700, exist_ok=True)
        env = os.environ.copy()
        env.update({"HOME": os.fspath(home), "GOALCUP_INSTALL_TESTING": "1"})
        return env

    def prefix_for(self, root: Path) -> Path:
        prefix = root / "install" / "prefix"
        prefix.parent.mkdir(mode=0o700, exist_ok=True)
        return prefix

    def run_shell(
        self,
        root: Path,
        bundle: Path,
        *,
        fail_phase: str | None = None,
        caller_umask: int | None = None,
    ) -> tuple[subprocess.CompletedProcess[str], Path]:
        prefix = self.prefix_for(root)
        env = self.shell_env(root)
        if fail_phase:
            env["GOALCUP_INSTALL_TEST_FAIL"] = fail_phase
        result = subprocess.run(
            ["sh", os.fspath(SHELL_PATH), "--bundle", os.fspath(bundle), "--prefix", os.fspath(prefix)],
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
            preexec_fn=(lambda: os.umask(caller_umask)) if caller_umask is not None else None,
        )
        return result, prefix

    def start_at_barrier(
        self,
        root: Path,
        bundle: Path,
        phase: str,
        *,
        fail_phase: str | None = None,
    ) -> tuple[subprocess.Popen[str], Path, Path, Path]:
        """The installer, running until it reaches `phase` and waits for the release file."""
        prefix = self.prefix_for(root)
        ready_file, release_file = root / "barrier.ready", root / "barrier.release"
        env = self.shell_env(root)
        env.update(
            {
                "GOALCUP_INSTALL_TEST_BARRIER_PHASE": phase,
                "GOALCUP_INSTALL_TEST_BARRIER_READY": os.fspath(ready_file),
                "GOALCUP_INSTALL_TEST_BARRIER_RELEASE": os.fspath(release_file),
            }
        )
        if fail_phase:
            env["GOALCUP_INSTALL_TEST_FAIL"] = fail_phase
        process = subprocess.Popen(
            ["sh", os.fspath(SHELL_PATH), "--bundle", os.fspath(bundle), "--prefix", os.fspath(prefix)],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        return process, prefix, ready_file, release_file

    def run_barrier_mutation(
        self,
        root: Path,
        bundle: Path,
        *,
        phase: str,
        mutate,
    ) -> tuple[int, str, str, Path]:
        process, prefix, ready_file, release_file = self.start_at_barrier(root, bundle, phase)
        if not wait_for_path(ready_file):
            stdout, stderr = process.communicate(timeout=10)
            self.fail(
                f"missing {phase} barrier; rc={process.returncode}; stdout={stdout!r}; stderr={stderr!r}"
            )
        mutate(prefix)
        release_file.write_text("release\n", encoding="utf-8")
        stdout, stderr = process.communicate(timeout=10)
        return process.returncode, stdout, stderr, prefix

    def test_bundle_walk_refuses_directory_symlink_before_prefix_publication(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            bundle = write_bundle(root)
            shutil.rmtree(bundle / "connectors" / "codex-cli")
            (bundle / "connectors" / "codex-cli").symlink_to("claude-code-cli", target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink|link|directory"):
                self.inspect(bundle)

    def test_bundle_walk_refuses_fifo_socket_and_special_nodes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gc-", dir="/private/tmp") as temporary:
            root = Path(os.path.realpath(temporary))
            for kind in ("fifo", "socket"):
                with self.subTest(kind=kind):
                    case = root / kind
                    case.mkdir()
                    bundle = write_bundle(case)
                    unsafe = bundle / "connectors" / "codex-cli" / kind
                    if kind == "fifo":
                        os.mkfifo(unsafe)
                        closer = None
                    else:
                        closer = socket.socket(socket.AF_UNIX)
                        closer.bind(os.fspath(unsafe))
                    try:
                        with self.assertRaisesRegex(ValueError, "special|regular|unsafe"):
                            self.inspect(bundle)
                    finally:
                        if closer is not None:
                            closer.close()

    def test_copy_failure_leaves_no_visible_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            result, prefix = self.run_shell(root, write_bundle(root), fail_phase="copy")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(prefix.exists())
            self.assertEqual(result.stdout, "")

    def test_racing_destinations_are_preserved(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            destination = module.open_destination(root / "final")
            namespace = module.create_install_namespace(destination)
            try:
                os.mkdir("payload", mode=0o700, dir_fd=namespace.fd)
                racer = root / "final"
                racer.write_text("racer", encoding="utf-8")
                with self.assertRaises((FileExistsError, ValueError)):
                    module.publish_no_replace(namespace.fd, "payload", destination)
                self.assertEqual(racer.read_text(encoding="utf-8"), "racer")
            finally:
                namespace.close()
                destination.close()

    def test_cleanup_namespace_swap_preserves_replacement_and_original(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            destination = module.open_destination(root / "final")
            namespace = module.create_install_namespace(destination)
            held = root / "held-original"
            original_leaf = root / namespace.leaf_name
            original_leaf.rename(held)
            original_identity = held.stat().st_ino
            original_leaf.mkdir(mode=0o700)
            replacement_identity = original_leaf.stat().st_ino
            residual = module._cleanup_namespace(namespace)
            self.assertIsNotNone(residual)
            self.assertEqual(held.stat().st_ino, original_identity)
            self.assertEqual(original_leaf.stat().st_ino, replacement_identity)
            destination.close()

    def test_cleanup_never_removes_nonempty_staged_or_published_inode(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            destination = module.open_destination(root / "final")
            namespace = module.create_install_namespace(destination)
            os.mkdir("payload", mode=0o700, dir_fd=namespace.fd)
            payload = root / namespace.leaf_name / "payload"
            identity = payload.stat().st_ino
            residual = module._cleanup_namespace(namespace)
            self.assertIsNotNone(residual)
            self.assertEqual(payload.stat().st_ino, identity)
            namespace.close()
            destination.close()

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            plan = self.inspect(write_bundle(root))
            destination = module.open_destination(root / "final")
            namespace = module.create_install_namespace(destination)
            module.stage_install(plan, namespace, "payload", root / "final")
            published = module.publish_no_replace(namespace.fd, "payload", destination)
            published_path = root / "final"
            published_identity = published_path.stat().st_ino
            moved = root / namespace.leaf_name / "moved-published"
            published_path.rename(moved)
            residual = module._cleanup_namespace(namespace)
            self.assertIsNotNone(residual)
            self.assertEqual(moved.stat().st_ino, published_identity)
            self.assertEqual((published.st_dev, published.st_ino), (moved.stat().st_dev, moved.stat().st_ino))
            namespace.close()
            destination.close()
            plan.close()

    def test_staged_sources_are_identity_bound_through_publication(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            for replacement_kind in ("directory", "regular", "symlink"):
                    with self.subTest(replacement=replacement_kind):
                        case = root / f"prefix-{replacement_kind}"
                        case.mkdir()
                        bundle = write_bundle(case)
                        plan = self.inspect(bundle)
                        destination = module.open_destination(case / "published")
                        namespace = module.create_install_namespace(destination)
                        source_name = "payload"
                        try:
                            module.stage_install(plan, namespace, source_name, case / "published")
                            source = case / namespace.leaf_name / source_name
                            verified = case / namespace.leaf_name / "verified"
                            source.rename(verified)
                            if replacement_kind == "directory":
                                source.mkdir(mode=0o700)
                            elif replacement_kind == "regular":
                                source.write_bytes(b"replacement")
                            else:
                                source.symlink_to("verified")
                            replacement_identity = source.lstat().st_ino
                            with self.assertRaises((ValueError, OSError)):
                                module.publish_no_replace(namespace.fd, source_name, destination)
                            self.assertFalse(os.path.lexists(case / "published"))
                            self.assertEqual(source.lstat().st_ino, replacement_identity)
                        finally:
                            namespace.close()
                            destination.close()
                            plan.close()

    def test_pre_final_verification_source_swap_never_publishes_replacement(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            for source_kind in ("prefix",):
                with self.subTest(source=source_kind):
                    case = root / source_kind
                    case.mkdir()
                    plan = self.inspect(write_bundle(case))
                    destination = module.open_destination(case / "published")
                    namespace = module.create_install_namespace(destination)
                    ready_file, release_file = case / "ready", case / "release"
                    old_env = os.environ.copy()
                    try:
                        module.stage_install(plan, namespace, "payload", case / "published")
                        os.environ.update(
                            {
                                "GOALCUP_INSTALL_TESTING": "1",
                                "GOALCUP_INSTALL_TEST_BARRIER_PHASE": "before-prefix-rename",
                                "GOALCUP_INSTALL_TEST_BARRIER_READY": os.fspath(ready_file),
                                "GOALCUP_INSTALL_TEST_BARRIER_RELEASE": os.fspath(release_file),
                            }
                        )
                        outcome: list[BaseException | object] = []

                        def publish() -> None:
                            try:
                                outcome.append(module.publish_no_replace(namespace.fd, "payload", destination))
                            except BaseException as error:
                                outcome.append(error)

                        worker = threading.Thread(target=publish)
                        worker.start()
                        self.assertTrue(wait_for_path(ready_file), "pre-final-verification barrier missing")
                        source = case / namespace.leaf_name / "payload"
                        source.rename(case / namespace.leaf_name / "verified-original")
                        source.mkdir(mode=0o700)
                        (source / "UNVERIFIED").write_text("replacement", encoding="utf-8")
                        replacement = tree_fingerprint(source)
                        release_file.write_text("release\n", encoding="utf-8")
                        worker.join(5)
                        self.assertFalse(worker.is_alive())
                        self.assertTrue(outcome and isinstance(outcome[0], BaseException), outcome)
                        self.assertFalse(os.path.lexists(case / "published"))
                        self.assertEqual(tree_fingerprint(source), replacement)
                    finally:
                        os.environ.clear()
                        os.environ.update(old_env)
                        namespace.close()
                        destination.close()
                        plan.close()

    def test_staged_descendant_swap_is_refused_at_publication_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            process, prefix, ready_file, release_file = self.start_at_barrier(
                root, write_bundle(root), "before-prefix-rename"
            )
            self.assertTrue(wait_for_path(ready_file), "pre-final-verification barrier missing")
            matches = list(root.rglob(".goalcup-install-*/prefix/bin/goalcup-daemon"))
            self.assertEqual(len(matches), 1, matches)
            daemon = matches[0]
            staged_root_identity = daemon.parents[1].stat().st_ino
            daemon.rename(daemon.with_name("goalcup-daemon.verified-original"))
            daemon.write_bytes(b"#!/bin/sh\necho ATTACKER\n")
            daemon.chmod(0o755)
            replacement = tree_fingerprint(daemon)
            self.assertEqual(daemon.parents[1].stat().st_ino, staged_root_identity)
            release_file.write_text("release\n", encoding="utf-8")
            stdout, stderr = process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(stdout, "")
            self.assertFalse(prefix.exists())
            self.assertEqual(tree_fingerprint(daemon), replacement)
            self.assertIn("staged publication tree changed", stderr)
            self.assertIn(os.fspath(daemon.parents[2]), stderr)

    def test_staged_prefix_exact_mode_is_bound_at_publication(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))

            def change_mode(_prefix: Path) -> None:
                matches = list(root.rglob(".goalcup-install-*/prefix"))
                self.assertEqual(len(matches), 1, matches)
                matches[0].chmod(0o777)
                self.assertEqual(stat.S_IMODE(matches[0].stat().st_mode), 0o777)

            returncode, stdout, stderr, prefix = self.run_barrier_mutation(
                root, write_bundle(root), phase="before-prefix-rename", mutate=change_mode
            )
            self.assertNotEqual(returncode, 0)
            self.assertEqual(stdout, "")
            self.assertFalse(prefix.exists())
            self.assertIn("mode", stderr.lower())

    def test_validation_to_registration_descendant_swap_is_never_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            process, prefix, ready_file, release_file = self.start_at_barrier(
                root, write_bundle(root), "before-prefix-registration"
            )
            barrier_reached = wait_for_path(ready_file)
            if barrier_reached:
                matches = list(root.rglob(".goalcup-install-*/prefix/bin/goalcup-daemon"))
                self.assertEqual(len(matches), 1, matches)
                daemon = matches[0]
                daemon.rename(daemon.with_name("goalcup-daemon.validated-original"))
                daemon.write_bytes(b"#!/bin/sh\necho HANDOFF-ATTACKER\n")
                daemon.chmod(0o755)
                replacement = tree_fingerprint(daemon)
                release_file.write_text("release\n", encoding="utf-8")
            else:
                daemon = root / "barrier-not-reached"
                replacement = ()
            stdout, stderr = process.communicate(timeout=10)
            self.assertTrue(barrier_reached, f"missing validation-to-registration barrier: {stdout} {stderr}")
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(stdout, "")
            self.assertFalse(prefix.exists())
            self.assertEqual(tree_fingerprint(daemon), replacement)
            self.assertIn("validated staged tree changed before registration", stderr)

    def test_staged_tree_hashing_is_incremental_and_does_not_buffer_files(self) -> None:
        module = installer()
        self.assertTrue(hasattr(module, "_hash_fd_incrementally"), "missing streaming hash helper")
        snapshot_source = inspect.getsource(module._snapshot_staged_tree)
        hash_source = inspect.getsource(module._hash_fd_incrementally)
        self.assertNotIn("_read_fd(", snapshot_source)
        self.assertIn("_hash_fd_incrementally(", snapshot_source)
        self.assertIn("os.read(fd, _BLOCK)", hash_source)
        self.assertNotIn("chunks", hash_source)

    def test_post_final_verification_source_swap_is_ambiguous_and_reported(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            phase = "after-prefix-final-verification"
            process, prefix, ready_file, release_file = self.start_at_barrier(root, write_bundle(root), phase)
            self.assertTrue(wait_for_path(ready_file), f"missing {phase} barrier")
            matches = list(root.rglob(".goalcup-install-*/prefix"))
            self.assertEqual(len(matches), 1, matches)
            source = matches[0]
            retained_source = source.stat()
            source.rename(source.with_name("prefix.verified-original"))
            source.mkdir(mode=0o700)
            (source / "UNVERIFIED").write_bytes(b"replacement prefix")
            observed_destination = source.stat()
            replacement = tree_fingerprint(source)
            release_file.write_text("release\n", encoding="utf-8")
            stdout, stderr = process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(stdout, "")
            self.assertTrue(os.path.lexists(prefix))
            destination_fingerprint = tree_fingerprint(prefix)
            self.assertEqual(
                tuple(item[1:] for item in destination_fingerprint),
                tuple(item[1:] for item in replacement),
            )
            self.assertIn("publication outcome ambiguous", stderr)
            self.assertIn(os.fspath(prefix), stderr)
            self.assertIn(
                f"expected retained source device {retained_source.st_dev}, inode {retained_source.st_ino}",
                stderr,
            )
            self.assertIn(
                f"safely observed destination device {observed_destination.st_dev}, inode {observed_destination.st_ino}",
                stderr,
            )

    def test_post_final_verification_descendant_swap_is_ambiguous_and_reported(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            process, prefix, ready_file, release_file = self.start_at_barrier(
                root, write_bundle(root), "after-prefix-final-verification"
            )
            self.assertTrue(wait_for_path(ready_file), "missing after-prefix-final-verification barrier")
            matches = list(root.rglob(".goalcup-install-*/prefix"))
            self.assertEqual(len(matches), 1, matches)
            staged_prefix = matches[0]
            retained_root = staged_prefix.stat()
            daemon = staged_prefix / "bin" / "goalcup-daemon"
            replacement_bytes = b"#!/bin/sh\necho FINAL-SEAM-ATTACKER\n"
            replacement = daemon.with_name(".goalcup-daemon-replacement")
            replacement.write_bytes(replacement_bytes)
            replacement.chmod(0o755)
            os.replace(replacement, daemon)
            replacement_info = daemon.stat()
            self.assertEqual(staged_prefix.stat().st_ino, retained_root.st_ino)
            release_file.write_text("release\n", encoding="utf-8")
            stdout, stderr = process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(stdout, "")
            self.assertTrue(prefix.is_dir())
            self.assertEqual(prefix.stat().st_ino, retained_root.st_ino)
            self.assertEqual((prefix / "bin" / "goalcup-daemon").stat().st_ino, replacement_info.st_ino)
            self.assertEqual((prefix / "bin" / "goalcup-daemon").read_bytes(), replacement_bytes)
            self.assertIn("publication outcome ambiguous", stderr)
            self.assertIn("published paths preserved", stderr)
            self.assertIn(os.fspath(prefix), stderr)

    def test_private_install_namespaces_are_fresh_mode_0700_and_descriptor_held(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            destination = module.open_destination(Path(os.path.realpath(temporary)) / "final")
            first = module.create_install_namespace(destination)
            second = module.create_install_namespace(destination)
            try:
                self.assertNotEqual(first.leaf_name, second.leaf_name)
                for namespace in (first, second):
                    info = os.fstat(namespace.fd)
                    self.assertEqual(stat.S_IMODE(info.st_mode), 0o700)
                    self.assertEqual((info.st_dev, info.st_ino), (namespace.st_dev, namespace.st_ino))
                    self.assertTrue(stat.S_ISDIR(info.st_mode))
            finally:
                first.close()
                second.close()
                destination.close()

    def test_successful_rename_is_latched_before_injected_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            result, prefix = self.run_shell(root, write_bundle(root), fail_phase="after-prefix-rename")
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(prefix.is_dir())
            self.assertEqual(result.stdout, "")
            self.assertIn("failed after publication", result.stderr)
            self.assertIn(os.fspath(prefix), result.stderr)

    def test_ambiguous_rename_error_is_preserve_and_report(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            result, _prefix = self.run_shell(root, write_bundle(root), fail_phase="ambiguous-prefix-rename")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("publication outcome ambiguous", result.stderr)
            self.assertIn("inspection-first", result.stderr)

    def test_parent_swap_report_distinguishes_intended_and_current_path(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            parent = root / "parent"
            parent.mkdir()
            plan = self.inspect(write_bundle(root))
            destination = module.open_destination(parent / "final")
            namespace = module.create_install_namespace(destination)
            module.stage_install(plan, namespace, "payload", parent / "final")
            moved = root / "moved-parent"
            parent.rename(moved)
            parent.mkdir()
            published = module.publish_no_replace(namespace.fd, "payload", destination)
            report = module.report_manual_recovery((published,), "injected")
            self.assertIn(f"intended path (stale): {parent / 'final'}", report)
            self.assertIn(f"current descriptor-derived path: {moved / 'final'}", report)
            self.assertNotIn(f"published paths preserved:\n  - {parent / 'final'} ", report)
            namespace.close()
            destination.close()
            plan.close()

    def test_namespace_resource_failures_are_clean_or_reported(self) -> None:
        for phase in (
            "namespace-after-mkdir",
            "namespace-open-error",
            "namespace-fchmod-error",
        ):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as temporary:
                root = Path(os.path.realpath(temporary))
                result, prefix = self.run_shell(root, write_bundle(root), fail_phase=phase)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(prefix.exists())
                self.assertEqual(result.stdout, "")
                if phase == "namespace-open-error":
                    self.assertIn("[Errno 24] injected namespace os.open failure", result.stderr)
                residues = list(root.rglob(".goalcup-install-*"))
                if residues:
                    for residue in residues:
                        self.assertIn(os.fspath(residue), result.stderr)
                        self.assertEqual(stat.S_IMODE(residue.stat().st_mode), 0o700)

    def test_namespace_open_emfile_intercepts_actual_os_open_call(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            destination = module.open_destination(root / "final")
            original_open = module.os.open
            intercepted: list[tuple[str, int, int | None]] = []
            delegated: list[str | bytes] = []

            def intercept_namespace_open(
                path: str | bytes,
                flags: int,
                mode: int = 0o777,
                *,
                dir_fd: int | None = None,
            ) -> int:
                if (
                    isinstance(path, str)
                    and path.startswith(".goalcup-install-")
                    and dir_fd == destination.parent_fd
                    and flags & getattr(os, "O_DIRECTORY", 0)
                ):
                    intercepted.append((path, flags, dir_fd))
                    raise OSError(errno.EMFILE, "intercepted actual namespace os.open")
                delegated.append(path)
                return original_open(path, flags, mode, dir_fd=dir_fd)

            module.os.open = intercept_namespace_open
            try:
                probe_fd = module.os.open(".", module._flags(directory=True), dir_fd=destination.parent_fd)
                module.os.close(probe_fd)
                with self.assertRaises(module.NamespaceAcquisitionError) as caught:
                    module.create_install_namespace(destination)
            finally:
                module.os.open = original_open
                destination.close()

            self.assertEqual(len(intercepted), 1, intercepted)
            self.assertIn(".", delegated)
            residues = list(root.glob(".goalcup-install-*"))
            self.assertEqual(len(residues), 1, residues)
            residue = residues[0]
            self.assertEqual(intercepted[0][0], residue.name)
            self.assertEqual(stat.S_IMODE(residue.stat().st_mode), 0o700)
            self.assertIsNone(caught.exception.namespace)
            self.assertIn("[Errno 24] intercepted actual namespace os.open", str(caught.exception))
            self.assertEqual(
                caught.exception.residual,
                f"{residue}: created namespace could not be opened; preserved for inspection",
            )

    def test_final_cleanup_window_preserves_original_and_replacement(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            destination = module.open_destination(root / "final")
            namespace = module.create_install_namespace(destination)
            original = root / namespace.leaf_name
            held = root / "held-original"
            ready_file, release_file = root / "ready", root / "release"
            old_env = os.environ.copy()
            try:
                os.environ.update(
                    {
                        "GOALCUP_INSTALL_TESTING": "1",
                        "GOALCUP_INSTALL_TEST_BARRIER_PHASE": "before-namespace-cleanup-removal",
                        "GOALCUP_INSTALL_TEST_BARRIER_READY": os.fspath(ready_file),
                        "GOALCUP_INSTALL_TEST_BARRIER_RELEASE": os.fspath(release_file),
                    }
                )
                outcome: list[str | None] = []
                worker = threading.Thread(target=lambda: outcome.append(module._cleanup_namespace(namespace)))
                worker.start()
                self.assertTrue(wait_for_path(ready_file), "cleanup decision barrier missing")
                original.rename(held)
                original.mkdir(mode=0o700)
                original_inode, replacement_inode = held.stat().st_ino, original.stat().st_ino
                release_file.write_text("release\n", encoding="utf-8")
                worker.join(5)
                self.assertFalse(worker.is_alive())
                self.assertTrue(outcome and outcome[0])
                self.assertEqual(held.stat().st_ino, original_inode)
                self.assertEqual(original.stat().st_ino, replacement_inode)
            finally:
                os.environ.clear()
                os.environ.update(old_env)
                namespace.close()
                destination.close()

    def test_namespace_modes_ignore_caller_umask(self) -> None:
        module = installer()
        for caller_umask in (0o000, 0o077, 0o777):
            with self.subTest(umask=oct(caller_umask)), tempfile.TemporaryDirectory() as temporary:
                root = Path(os.path.realpath(temporary))
                destination = module.open_destination(root / "final")
                old_umask = os.umask(caller_umask)
                try:
                    namespace = module.create_install_namespace(destination)
                finally:
                    os.umask(old_umask)
                try:
                    self.assertEqual(stat.S_IMODE(os.fstat(namespace.fd).st_mode), 0o700)
                    self.assertEqual(stat.S_IMODE((root / namespace.leaf_name).stat().st_mode), 0o700)
                finally:
                    namespace.close()
                    destination.close()

    def test_signal_phase_matrix_is_controlled_and_preserves_phase_state(self) -> None:
        phases = {
            "before-publication": (False, "before publication"),
            "after-prefix-publication": (True, "after publication"),
        }
        for phase, (want_prefix, report_phase) in phases.items():
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as temporary:
                root = Path(os.path.realpath(temporary))
                process, prefix, ready_file, release_file = self.start_at_barrier(root, write_bundle(root), phase)
                barrier_reached = wait_for_path(ready_file)
                before_prefix = tree_fingerprint(prefix) if barrier_reached and want_prefix else ()
                if barrier_reached:
                    process.send_signal(signal.SIGTERM)
                    release_file.write_text("release\n", encoding="utf-8")
                stdout, stderr = process.communicate(timeout=10)
                self.assertTrue(barrier_reached, f"installer never reached {phase}: {stdout} {stderr}")
                self.assertNotEqual(process.returncode, 0)
                self.assertEqual(stdout, "")
                self.assertEqual(prefix.exists(), want_prefix)
                self.assertEqual(tree_fingerprint(prefix) if want_prefix else (), before_prefix)
                self.assertIn(report_phase, stderr)
                self.assertIn("controlled SIGTERM", stderr)

    def test_synchronized_destination_competitor_is_preserved(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            process, prefix, ready_file, release_file = self.start_at_barrier(
                root, write_bundle(root), "before-prefix-rename"
            )
            barrier_reached = wait_for_path(ready_file)
            if barrier_reached:
                prefix.write_bytes(b"competitor-content")
                competitor = tree_fingerprint(prefix)
                release_file.write_text("release\n", encoding="utf-8")
            else:
                competitor = ()
            stdout, stderr = process.communicate(timeout=10)
            self.assertTrue(barrier_reached, f"missing rename barrier: {stdout} {stderr}")
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(tree_fingerprint(prefix), competitor)
            self.assertEqual(prefix.read_bytes(), b"competitor-content")
            self.assertIn("destination already exists", stderr)

    def test_injected_post_publication_failure_preserves_the_exact_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            process, prefix, ready_file, release_file = self.start_at_barrier(
                root, write_bundle(root), "after-prefix-publication", fail_phase="after-prefix"
            )
            barrier_reached = wait_for_path(ready_file)
            before_prefix = tree_fingerprint(prefix) if barrier_reached else ()
            if barrier_reached:
                release_file.write_text("release\n", encoding="utf-8")
            stdout, stderr = process.communicate(timeout=10)
            self.assertTrue(barrier_reached, f"missing phase barrier: {stdout} {stderr}")
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(stdout, "")
            self.assertEqual(tree_fingerprint(prefix), before_prefix)
            self.assertIn("failed after publication", stderr)

    def test_mid_copy_failure_retains_and_reports_partial_private_namespace(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            result, prefix = self.run_shell(root, write_bundle(root), fail_phase="mid-copy")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(prefix.exists())
            residues = list(root.rglob(".goalcup-install-*"))
            self.assertTrue(residues)
            for residue in residues:
                self.assertIn(os.fspath(residue), result.stderr)
                self.assertEqual(stat.S_IMODE(residue.stat().st_mode), 0o700)

    def test_permission_matrix_covers_namespaces_payload_and_generated_files(self) -> None:
        for caller_umask in (0o000, 0o077, 0o777):
            with self.subTest(umask=oct(caller_umask)), tempfile.TemporaryDirectory() as temporary:
                root = Path(os.path.realpath(temporary))
                bundle = write_bundle(root)
                result, prefix = self.run_shell(root, bundle, caller_umask=caller_umask)
                self.assertEqual(result.returncode, 0, result.stderr)
                for directory in (
                    prefix,
                    prefix / "bin",
                    prefix / "share",
                    prefix / "share" / "goalcup" / "connectors",
                    prefix / "etc",
                    prefix / "var" / "artifacts",
                    prefix / "var" / "workspaces",
                ):
                    self.assertEqual(stat.S_IMODE(directory.stat().st_mode), 0o700, directory)
                manifest = json.loads((bundle / "bundle.json").read_text(encoding="utf-8"))
                for entry in manifest["files"]:
                    relative = Path(entry["path"])
                    installed = prefix / relative if relative.parts[0] == "bin" else prefix / "share" / "goalcup" / relative
                    self.assertEqual(stat.S_IMODE(installed.stat().st_mode), int(entry["mode"], 8))
                    self.assertEqual(hashlib.sha256(installed.read_bytes()).hexdigest(), entry["sha256"])
                self.assertEqual(stat.S_IMODE((prefix / "etc" / "daemon.toml").stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE((prefix / ".goalcup-install-manifest.json").stat().st_mode), 0o600)

    def test_success_publishes_the_complete_prefix_and_installs_no_service(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            result, prefix = self.run_shell(root, write_bundle(root))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((prefix / "bin" / "goalcup-daemon").is_file())
            self.assertTrue((prefix / "etc" / "daemon.toml").is_file())
            self.assertIn(f"installed under {prefix}", result.stdout)
            self.assertFalse((root / "home" / "Library").exists(), "nothing was written under HOME")
            manifest = json.loads((prefix / ".goalcup-install-manifest.json").read_text(encoding="utf-8"))
            self.assertNotIn("label", manifest)

    def test_leaf_and_ancestor_swap_is_detected_or_yields_one_self_consistent_stage(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            bundle = write_bundle(root)
            plan = self.inspect(bundle)
            original = root / "original-bundle"
            bundle.rename(original)
            replacement = write_bundle(root / "replacement-root")
            replacement.rename(bundle)
            (bundle / "bin" / "goalcup").write_bytes(b"malicious replacement")
            destination = module.open_destination(root / "installed")
            namespace = module.create_install_namespace(destination)
            try:
                module.stage_install(plan, namespace, "payload", root / "installed")
                staged_fd = os.open("payload", os.O_RDONLY | os.O_DIRECTORY, dir_fd=namespace.fd)
                try:
                    bin_fd = os.open("bin/goalcup", os.O_RDONLY, dir_fd=staged_fd)
                    try:
                        self.assertEqual(os.read(bin_fd, 1000), b"goalcup\n")
                    finally:
                        os.close(bin_fd)
                finally:
                    os.close(staged_fd)
            finally:
                namespace.close()
                destination.close()
                plan.close()

    def test_manifest_refuses_absolute_dotdot_dot_empty_and_duplicate_paths(self) -> None:
        invalid = (
            "/absolute",
            "connectors/../escape",
            "connectors/./file",
            "connectors//file",
            "connectors/file/",
            "connectors\\file",
            "connectors/file\x00tail",
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            for index, value in enumerate(invalid):
                with self.subTest(path=value):
                    case = root / str(index)
                    case.mkdir()
                    bundle = write_bundle(case)
                    rewrite_manifest_path(bundle, "bin/goalcup", value)
                    with self.assertRaises(ValueError):
                        self.inspect(bundle)
            duplicate_root = root / "duplicate"
            duplicate_root.mkdir()
            bundle = write_bundle(duplicate_root)
            manifest_path = bundle / "bundle.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["files"].append(dict(manifest["files"][0]))
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                self.inspect(bundle)

    def test_hostile_filesystem_input_matrix(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory(prefix="gc-hostile-", dir="/private/tmp") as temporary:
            root = Path(os.path.realpath(temporary))
            bundle = write_bundle(root / "leaf-case")
            binary = bundle / "bin" / "goalcup"
            binary.unlink()
            binary.symlink_to("goalcup-daemon")
            with self.assertRaises((ValueError, OSError)):
                self.inspect(bundle)

            real_parent = root / "real-parent"
            real_parent.mkdir()
            nested_bundle = write_bundle(real_parent)
            linked_parent = root / "linked-parent"
            linked_parent.symlink_to(real_parent, target_is_directory=True)
            with self.assertRaises((ValueError, OSError)):
                self.inspect(linked_parent / nested_bundle.name)

            destination_root = root / "destinations"
            destination_root.mkdir()
            real_destination_parent = destination_root / "real"
            real_destination_parent.mkdir()
            (destination_root / "linked").symlink_to(real_destination_parent, target_is_directory=True)
            with self.assertRaises((ValueError, OSError)):
                module.open_destination(destination_root / "linked" / "final")
            nondirectory = destination_root / "file"
            nondirectory.write_text("not a directory", encoding="utf-8")
            with self.assertRaises((ValueError, OSError)):
                module.open_destination(nondirectory / "final")

            plan = self.inspect(write_bundle(root / "racer-case"))
            destination = module.open_destination(real_destination_parent / "final")
            namespace = module.create_install_namespace(destination)
            try:
                module.stage_install(plan, namespace, "payload", root / "prefix")
                (real_destination_parent / "final").symlink_to("foreign")
                racer_identity = (real_destination_parent / "final").lstat().st_ino
                with self.assertRaises((FileExistsError, ValueError)):
                    module.publish_no_replace(namespace.fd, "payload", destination)
                self.assertEqual((real_destination_parent / "final").lstat().st_ino, racer_identity)
            finally:
                namespace.close()
                destination.close()
                plan.close()

            duplicate_key_root = root / "duplicate-key"
            duplicate_key_root.mkdir()
            bundle = write_bundle(duplicate_key_root)
            manifest_path = bundle / "bundle.json"
            original = manifest_path.read_text(encoding="utf-8")
            manifest_path.write_text('{"format":1,' + original[1:], encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                self.inspect(bundle)

    def test_device_node_rejection_or_named_capability_skip(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gc-device-", dir="/private/tmp") as temporary:
            root = Path(os.path.realpath(temporary))
            bundle = write_bundle(root)
            device = bundle / "connectors" / "codex-cli" / "device"
            try:
                os.mknod(device, stat.S_IFCHR | 0o600, os.makedev(1, 3))
            except PermissionError as error:
                if error.errno in (errno.EPERM, errno.EACCES):
                    self.skipTest(f"device-node capability denied by host: errno {error.errno}")
                raise
            except NotImplementedError:
                self.skipTest("device-node creation is not implemented on this host")
            with self.assertRaisesRegex(ValueError, "special|unsafe|regular"):
                self.inspect(bundle)

    def test_paths_with_quotes_and_newlines_round_trip_through_toml(self) -> None:
        module = installer()
        prefix = Path('/tmp/goal"cup\nline')
        toml_bytes = module.render_daemon_toml(prefix)
        parsed_toml = tomllib.loads(toml_bytes.decode("utf-8"))
        self.assertEqual(parsed_toml["artifact_root"], os.fspath(prefix / "var" / "artifacts"))
        self.assertEqual(parsed_toml["connectors_dir"], os.fspath(prefix / "share" / "goalcup" / "connectors"))
        self.assertEqual(parsed_toml["config_dir"], os.fspath(prefix / "etc"))
        self.assertEqual(parsed_toml["workspace_parent"], os.fspath(prefix / "var" / "workspaces"))

    def test_destination_parent_swap_cannot_redirect_publication(self) -> None:
        module = installer()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(os.path.realpath(temporary))
            parent = root / "parent"
            parent.mkdir()
            plan = self.inspect(write_bundle(root))
            destination = module.open_destination(parent / "final")
            namespace = module.create_install_namespace(destination)
            try:
                module.stage_install(plan, namespace, "payload", parent / "final")
                original = root / "original-parent"
                parent.rename(original)
                parent.mkdir()
                module.publish_no_replace(namespace.fd, "payload", destination)
                self.assertFalse((parent / "final").exists(), "replacement parent was redirected into")
                self.assertTrue((original / "final" / "bin" / "goalcup").is_file())
            finally:
                namespace.close()
                destination.close()
                plan.close()

if __name__ == "__main__":
    unittest.main()
