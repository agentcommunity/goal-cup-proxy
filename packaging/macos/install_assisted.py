#!/usr/bin/env python3
"""Descriptor-rooted assisted installation for the private macOS M2 candidate.

It places the three binaries, the shipped connectors and a daemon configuration under one
fresh prefix, and nothing else: there is no service. `goalcup start` starts the daemon for a
run (with `GOALCUP_CONFIG_DIR=<prefix>/etc` it finds `<prefix>/etc/daemon.toml`), and the
daemon exits once nothing is unsealed.
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import hashlib
import json
import os
import re
import secrets
import signal
import stat
import sys
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from contextlib import contextmanager
from typing import Iterator, NoReturn

_BLOCK = 1024 * 1024
_HEX = re.compile(r"\A[0-9a-f]{64}\Z")
_REVISION = re.compile(r"\A(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")
_MODE = re.compile(r"\A[0-7]{4}\Z")
_MANIFEST_KEYS = {"format", "target", "revision", "version", "profile", "qualification", "signature", "files", "candidate", "distribution", "labels", "profile_sha256"}
_FILE_KEYS = {"path", "type", "sha256", "size", "mode"}
_BROAD_PREFIXES = {"/", "/Users", "/System", "/Library", "/Applications", "/usr", "/bin", "/sbin", "/etc", "/var", "/private", "/opt"}


def _close(fd: int) -> None:
    if fd >= 0:
        os.close(fd)


@dataclass(frozen=True)
class BundleFile:
    relative_path: PurePosixPath
    size: int
    mode: int
    sha256: str


@dataclass
class BundlePlan:
    root_fd: int
    profile: str
    source_revision: str
    files: tuple[BundleFile, ...]
    binaries: tuple[str, ...]
    connectors: tuple[str, ...]

    def close(self) -> None:
        if self.root_fd >= 0:
            os.close(self.root_fd)
            self.root_fd = -1


@dataclass
class Destination:
    parent_fd: int
    parent_path: Path
    leaf_name: str
    st_dev: int
    st_ino: int
    published: PublishedPath | None = None
    publication_ambiguous: PublishedPath | None = None

    def close(self) -> None:
        if self.parent_fd >= 0:
            os.close(self.parent_fd)
            self.parent_fd = -1


@dataclass
class InstallNamespace:
    parent: Destination
    leaf_name: str
    fd: int
    st_dev: int
    st_ino: int

    def close(self) -> None:
        for key in [key for key in _STAGED_LEAVES if key[0] == self.fd]:
            _STAGED_LEAVES.pop(key).close()
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1


@dataclass(frozen=True)
class PublishedPath:
    destination: Destination
    st_dev: int
    st_ino: int
    observed_st_dev: int | None = None
    observed_st_ino: int | None = None


@dataclass(frozen=True)
class _TreeEntry:
    relative_path: str
    st_dev: int
    st_ino: int
    st_mode: int
    st_size: int
    digest: str | None


@dataclass(frozen=True)
class _ExpectedTreeEntry:
    node_type: int
    mode: int
    size: int | None
    digest: str | None


@dataclass
class _StagedLeaf:
    fd: int
    st_dev: int
    st_ino: int
    st_mode: int
    st_size: int
    digest: str | None
    tree: tuple[_TreeEntry, ...] | None

    def close(self) -> None:
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1


class PublicationAmbiguous(RuntimeError):
    def __init__(self, message: str, candidate: PublishedPath):
        super().__init__(message)
        self.candidate = candidate


class ControlledSignal(RuntimeError):
    pass


class NamespaceAcquisitionError(RuntimeError):
    def __init__(self, message: str, residual: str, namespace: InstallNamespace | None = None):
        super().__init__(message)
        self.residual = residual
        self.namespace = namespace


_STAGED_LEAVES: dict[tuple[int, str], _StagedLeaf] = {}
_CONTROLLED_SIGNALS = {signal.SIGINT, signal.SIGTERM}


def _value_error(message: str) -> NoReturn:
    raise ValueError(message)


def _testing_failure(name: str) -> bool:
    return os.environ.get("GOALCUP_INSTALL_TESTING") == "1" and os.environ.get("GOALCUP_INSTALL_TEST_FAIL") == name


@contextmanager
def _blocked_control_signals() -> Iterator[None]:
    old_mask: set[signal.Signals] | None = None
    if hasattr(signal, "pthread_sigmask"):
        old_mask = signal.pthread_sigmask(signal.SIG_BLOCK, _CONTROLLED_SIGNALS)
    try:
        yield
    finally:
        if old_mask is not None:
            signal.pthread_sigmask(signal.SIG_SETMASK, old_mask)


def _mkdir_exact(parent_fd: int, name: str) -> None:
    with _blocked_control_signals():
        previous = os.umask(0o077)
        try:
            os.mkdir(name, 0o700, dir_fd=parent_fd)
        finally:
            os.umask(previous)
    child = os.open(name, _flags(directory=True), dir_fd=parent_fd)
    try:
        os.fchmod(child, 0o700)
    finally:
        os.close(child)


def _test_barrier(phase: str) -> None:
    if os.environ.get("GOALCUP_INSTALL_TESTING") != "1" or os.environ.get("GOALCUP_INSTALL_TEST_BARRIER_PHASE") != phase:
        return
    ready = os.environ.get("GOALCUP_INSTALL_TEST_BARRIER_READY")
    release = os.environ.get("GOALCUP_INSTALL_TEST_BARRIER_RELEASE")
    if not ready or not release:
        _value_error("test barrier paths are required")
    Path(ready).write_text(phase + "\n", encoding="utf-8")
    while not Path(release).exists():
        time.sleep(0.01)


def _hash_fd_incrementally(fd: int) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    os.lseek(fd, 0, os.SEEK_SET)
    while block := os.read(fd, _BLOCK):
        digest.update(block)
        size += len(block)
    os.lseek(fd, 0, os.SEEK_SET)
    return size, digest.hexdigest()


def _snapshot_staged_tree(
    root_fd: int,
    expected: dict[str, _ExpectedTreeEntry] | None = None,
) -> tuple[_TreeEntry, ...]:
    entries: list[_TreeEntry] = []
    root_before = os.fstat(root_fd)

    def validate_expected(relative: PurePosixPath, info: os.stat_result, digest: str | None) -> None:
        if expected is None:
            return
        path = relative.as_posix()
        wanted = expected.get(path)
        if wanted is None:
            _value_error(f"unexpected path in staged publication tree: {path}")
        if stat.S_IFMT(info.st_mode) != wanted.node_type:
            _value_error(f"staged path type differs from inspected manifest: {path}")
        if stat.S_IMODE(info.st_mode) != wanted.mode:
            _value_error(f"staged path mode differs from inspected manifest: {path}")
        if wanted.size is not None and info.st_size != wanted.size:
            _value_error(f"staged path size differs from inspected manifest: {path}")
        if wanted.digest is not None and digest != wanted.digest:
            _value_error(f"staged path digest differs from inspected manifest: {path}")

    def visit(directory_fd: int, prefix: PurePosixPath) -> None:
        with os.scandir(directory_fd) as children:
            names = sorted(child.name for child in children)
        for name in names:
            relative = prefix / name
            before = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
            if stat.S_ISDIR(before.st_mode):
                child_fd = os.open(name, _flags(directory=True), dir_fd=directory_fd)
                try:
                    opened = os.fstat(child_fd)
                    if not stat.S_ISDIR(opened.st_mode) or (
                        before.st_dev,
                        before.st_ino,
                    ) != (opened.st_dev, opened.st_ino):
                        _value_error(f"staged directory changed during snapshot: {relative}")
                    validate_expected(relative, opened, None)
                    visit(child_fd, relative)
                    after = os.fstat(child_fd)
                    if not _same_node(opened, after):
                        _value_error(f"staged directory changed during snapshot: {relative}")
                    entries.append(
                        _TreeEntry(
                            relative.as_posix(),
                            opened.st_dev,
                            opened.st_ino,
                            opened.st_mode,
                            opened.st_size,
                            None,
                        )
                    )
                finally:
                    os.close(child_fd)
            elif stat.S_ISREG(before.st_mode):
                child_fd = os.open(name, _flags(directory=False), dir_fd=directory_fd)
                try:
                    opened = os.fstat(child_fd)
                    size, digest = _hash_fd_incrementally(child_fd)
                    after = os.fstat(child_fd)
                    if (
                        not stat.S_ISREG(opened.st_mode)
                        or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino)
                        or not _same_node(opened, after)
                        or size != opened.st_size
                    ):
                        _value_error(f"staged file changed during snapshot: {relative}")
                    validate_expected(relative, opened, digest)
                    entries.append(
                        _TreeEntry(
                            relative.as_posix(),
                            opened.st_dev,
                            opened.st_ino,
                            opened.st_mode,
                            opened.st_size,
                            digest,
                        )
                    )
                finally:
                    os.close(child_fd)
            else:
                _value_error(f"refusing special node in staged publication tree: {relative}")

    visit(root_fd, PurePosixPath())
    root_after = os.fstat(root_fd)
    if not _same_node(root_before, root_after):
        _value_error("staged publication root changed during snapshot")
    if expected is not None and {entry.relative_path for entry in entries} != set(expected):
        _value_error("staged publication tree is missing an inspected path")
    return tuple(sorted(entries, key=lambda entry: entry.relative_path))


def _verify_retained_staged_leaf(staged: _StagedLeaf) -> None:
    retained = os.fstat(staged.fd)
    if (retained.st_dev, retained.st_ino, stat.S_IFMT(retained.st_mode)) != (
        staged.st_dev,
        staged.st_ino,
        stat.S_IFMT(staged.st_mode),
    ):
        _value_error("verified publication source identity changed")
    if stat.S_IMODE(retained.st_mode) != stat.S_IMODE(staged.st_mode):
        _value_error("verified publication source mode changed")
    if retained.st_size != staged.st_size:
        _value_error("verified publication source size changed")
    if staged.digest is not None:
        size, digest = _hash_fd_incrementally(staged.fd)
        if size != staged.st_size or digest != staged.digest:
            _value_error("verified publication source content changed")
    if staged.tree is not None and _snapshot_staged_tree(staged.fd) != staged.tree:
        _value_error("validated staged tree changed; staged publication tree changed")


def _register_staged_leaf(
    namespace_fd: int,
    name: str,
    fd: int,
    *,
    expected_mode: int,
    expected_size: int | None,
    expected_digest: str | None,
    expected_tree: dict[str, _ExpectedTreeEntry] | None,
) -> None:
    info = os.fstat(fd)
    digest: str | None = None
    tree: tuple[_TreeEntry, ...] | None = None
    if stat.S_IMODE(info.st_mode) != expected_mode:
        _value_error("validated staged publication root mode changed before registration")
    if stat.S_ISREG(info.st_mode):
        if expected_tree is not None or expected_size is None or expected_digest is None:
            _value_error("regular staged publication authority is incomplete")
        size, digest = _hash_fd_incrementally(fd)
        after = os.fstat(fd)
        if not _same_node(info, after) or size != expected_size or digest != expected_digest:
            _value_error("validated staged file changed before registration")
    elif stat.S_ISDIR(info.st_mode):
        if expected_tree is None or expected_size is not None or expected_digest is not None:
            _value_error("directory staged publication authority is incomplete")
        try:
            tree = _snapshot_staged_tree(fd, expected_tree)
        except ValueError as error:
            raise ValueError(f"validated staged tree changed before registration: {error}") from error
        after = os.fstat(fd)
        if not _same_node(info, after):
            _value_error("validated staged tree changed before registration")
    else:
        _value_error("staged publication leaf must be a regular file or directory")
    key = (namespace_fd, name)
    previous = _STAGED_LEAVES.pop(key, None)
    if previous is not None:
        previous.close()
    _STAGED_LEAVES[key] = _StagedLeaf(fd, info.st_dev, info.st_ino, info.st_mode, info.st_size, digest, tree)


def _verify_staged_leaf(namespace_fd: int, name: str) -> _StagedLeaf:
    staged = _STAGED_LEAVES.get((namespace_fd, name))
    if staged is None:
        _value_error("publication source was not verified and descriptor-bound")
    _verify_retained_staged_leaf(staged)
    try:
        named = os.stat(name, dir_fd=namespace_fd, follow_symlinks=False)
        opened = os.open(name, _flags(directory=stat.S_ISDIR(staged.st_mode)), dir_fd=namespace_fd)
    except OSError as error:
        raise ValueError("verified publication source was replaced") from error
    try:
        current = os.fstat(opened)
    finally:
        os.close(opened)
    expected = (
        staged.st_dev,
        staged.st_ino,
        staged.st_mode,
        staged.st_size,
    )
    if (
        (named.st_dev, named.st_ino, named.st_mode, named.st_size) != expected
        or (current.st_dev, current.st_ino, current.st_mode, current.st_size) != expected
    ):
        _value_error("verified publication source identity, mode, or size changed")
    return staged


def _verify_published_leaf(staged: _StagedLeaf, destination: Destination) -> PublishedPath:
    candidate = PublishedPath(destination, staged.st_dev, staged.st_ino)
    try:
        _verify_retained_staged_leaf(staged)
    except (OSError, ValueError) as error:
        raise PublicationAmbiguous(
            f"publication outcome ambiguous: retained staged content or mode changed: {error}",
            candidate,
        ) from error
    try:
        named = os.stat(destination.leaf_name, dir_fd=destination.parent_fd, follow_symlinks=False)
        opened = os.open(
            destination.leaf_name,
            _flags(directory=stat.S_ISDIR(staged.st_mode)),
            dir_fd=destination.parent_fd,
        )
    except OSError as error:
        raise PublicationAmbiguous(
            "publication outcome ambiguous: published leaf could not be identity-checked",
            candidate,
        ) from error
    try:
        current = os.fstat(opened)
    finally:
        os.close(opened)
    if (named.st_dev, named.st_ino, stat.S_IFMT(named.st_mode)) == (
        current.st_dev,
        current.st_ino,
        stat.S_IFMT(current.st_mode),
    ):
        candidate = PublishedPath(
            destination,
            staged.st_dev,
            staged.st_ino,
            current.st_dev,
            current.st_ino,
        )
    expected = (staged.st_dev, staged.st_ino, staged.st_mode, staged.st_size)
    if (
        (named.st_dev, named.st_ino, named.st_mode, named.st_size) != expected
        or (current.st_dev, current.st_ino, current.st_mode, current.st_size) != expected
    ):
        raise PublicationAmbiguous(
            "publication outcome ambiguous: destination identity, mode, or size differs from the retained verified source",
            candidate,
        )
    return candidate


def _flags(*, directory: bool) -> int:
    value = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    if directory:
        value |= getattr(os, "O_DIRECTORY", 0)
    else:
        value |= getattr(os, "O_NONBLOCK", 0)
    return value


def _same_node(first: os.stat_result, second: os.stat_result) -> bool:
    return (first.st_dev, first.st_ino, first.st_mode, first.st_size, first.st_mtime_ns, first.st_ctime_ns) == (second.st_dev, second.st_ino, second.st_mode, second.st_size, second.st_mtime_ns, second.st_ctime_ns)


def _open_absolute_directory(path: Path) -> int:
    absolute = Path(os.path.abspath(os.fspath(path)))
    current = os.open("/", _flags(directory=True))
    try:
        for component in absolute.parts[1:]:
            if component in ("", ".", "..") or "/" in component or "\x00" in component:
                _value_error(f"invalid directory component: {component!r}")
            child = os.open(component, _flags(directory=True), dir_fd=current)
            info = os.fstat(child)
            if not stat.S_ISDIR(info.st_mode):
                os.close(child)
                _value_error(f"refusing non-directory path component: {absolute}")
            if info.st_uid not in (0, os.getuid()) and not (info.st_mode & stat.S_ISVTX):
                os.close(child)
                _value_error(f"refusing directory not owned by the current user or root: {absolute}")
            os.close(current)
            current = child
        return current
    except Exception:
        os.close(current)
        raise


def _open_relative(root_fd: int, relative: PurePosixPath, *, directory: bool) -> int:
    parts = relative.parts
    if not parts:
        return os.dup(root_fd)
    current = os.dup(root_fd)
    try:
        for component in parts[:-1]:
            child = os.open(component, _flags(directory=True), dir_fd=current)
            info = os.fstat(child)
            if not stat.S_ISDIR(info.st_mode):
                os.close(child)
                _value_error(f"bundle path component is not a directory: {relative}")
            os.close(current)
            current = child
        result = os.open(parts[-1], _flags(directory=directory), dir_fd=current)
        os.close(current)
        return result
    except Exception:
        os.close(current)
        raise


def _read_fd(fd: int, *, limit: int | None = None) -> bytes:
    chunks: list[bytes] = []
    total = 0
    while True:
        block = os.read(fd, _BLOCK)
        if not block:
            return b"".join(chunks)
        chunks.append(block)
        total += len(block)
        if limit is not None and total > limit:
            _value_error("input file exceeds the installer limit")


def _strict_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            _value_error(f"duplicate JSON key in bundle.json: {key}")
        result[key] = value
    return result


def _canonical_inventory_path(raw: object) -> PurePosixPath:
    if not isinstance(raw, str) or not raw:
        _value_error("bundle inventory path must be a nonempty string")
    if "\x00" in raw or "\\" in raw:
        _value_error(f"bundle inventory path has an unsafe character: {raw!r}")
    if raw.startswith("/"):
        _value_error(f"bundle inventory path must be relative: {raw!r}")
    components = raw.split("/")
    if any(component in ("", ".", "..") for component in components):
        _value_error(f"bundle inventory path is not canonical: {raw!r}")
    canonical = PurePosixPath(*components)
    if canonical.as_posix() != raw:
        _value_error(f"bundle inventory path is not canonical: {raw!r}")
    return canonical


def _open_regular(root_fd: int, relative: PurePosixPath) -> tuple[int, os.stat_result]:
    try:
        fd = _open_relative(root_fd, relative, directory=False)
    except OSError as error:
        if error.errno in (errno.ELOOP, errno.ENXIO, getattr(errno, "EFTYPE", -1)):
            raise ValueError(f"refusing symlink or special bundle node: {relative}") from error
        raise
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode):
        os.close(fd)
        _value_error(f"bundle node is not a regular file: {relative}")
    return fd, info


def _hash_open_regular(root_fd: int, entry: BundleFile) -> None:
    fd, before = _open_regular(root_fd, entry.relative_path)
    digest = hashlib.sha256()
    size = 0
    try:
        while block := os.read(fd, _BLOCK):
            digest.update(block)
            size += len(block)
        after = os.fstat(fd)
    finally:
        os.close(fd)
    if not _same_node(before, after):
        _value_error(f"bundle source changed while reading: {entry.relative_path}")
    if size != entry.size or before.st_size != entry.size:
        _value_error(f"bundle size mismatch: {entry.relative_path}")
    if stat.S_IMODE(before.st_mode) != entry.mode:
        _value_error(f"bundle mode mismatch: {entry.relative_path}")
    if digest.hexdigest() != entry.sha256:
        _value_error(f"bundle sha256 mismatch: {entry.relative_path}")


def _walk_bundle(root_fd: int) -> tuple[set[str], set[str]]:
    files: set[str] = set()
    directories: set[str] = set()

    def visit(directory_fd: int, prefix: PurePosixPath) -> None:
        with os.scandir(directory_fd) as children:
            for child in children:
                relative = prefix / child.name
                raw = relative.as_posix()
                before = child.stat(follow_symlinks=False)
                if stat.S_ISLNK(before.st_mode):
                    _value_error(f"refusing symlink in staged bundle: {raw}")
                if stat.S_ISDIR(before.st_mode):
                    try:
                        child_fd = os.open(child.name, _flags(directory=True), dir_fd=directory_fd)
                    except OSError as error:
                        raise ValueError(f"refusing unsafe bundle directory: {raw}") from error
                    try:
                        after = os.fstat(child_fd)
                        if not stat.S_ISDIR(after.st_mode) or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                            _value_error(f"bundle directory changed during traversal: {raw}")
                        directories.add(raw)
                        visit(child_fd, relative)
                    finally:
                        os.close(child_fd)
                elif stat.S_ISREG(before.st_mode):
                    fd = os.open(child.name, _flags(directory=False), dir_fd=directory_fd)
                    after = os.fstat(fd)
                    os.close(fd)
                    if not stat.S_ISREG(after.st_mode) or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                        _value_error(f"bundle file changed during traversal: {raw}")
                    files.add(raw)
                else:
                    _value_error(f"refusing special or unsafe bundle node: {raw}")

    visit(root_fd, PurePosixPath())
    return files, directories


def inspect_bundle(bundle: Path, requested_profile: str, expected_binaries: tuple[str, ...], expected_connectors: tuple[str, ...], arch_label: str) -> BundlePlan:
    root_fd = _open_absolute_directory(bundle)
    try:
        manifest_fd, manifest_info = _open_regular(root_fd, PurePosixPath("bundle.json"))
        try:
            if manifest_info.st_size > 16 * 1024 * 1024:
                _value_error("bundle.json exceeds the installer limit")
            manifest = json.loads(
                _read_fd(manifest_fd, limit=16 * 1024 * 1024),
                object_pairs_hook=_strict_object,
            )
        finally:
            os.close(manifest_fd)
        if not isinstance(manifest, dict) or set(manifest) != _MANIFEST_KEYS:
            _value_error("bundle.json must contain exactly the supported manifest keys")
        if manifest["format"] != 1 or isinstance(manifest["format"], bool):
            _value_error("unsupported bundle manifest format")
        for key in ("target", "revision", "version", "profile", "qualification", "signature", "distribution"):
            if not isinstance(manifest[key], str):
                _value_error(f"bundle manifest {key} must be a string")
        if manifest["profile"] != requested_profile or requested_profile != "m2-pilot":
            _value_error("bundle profile does not match the requested m2-pilot profile")
        if manifest["candidate"] is not True or manifest["distribution"] != "private":
            _value_error("bundle is not a private candidate")
        if manifest["qualification"] != "unqualified" or manifest["signature"] != "unsigned":
            _value_error("bundle must remain unqualified and unsigned")
        if not _REVISION.fullmatch(manifest["revision"]):
            _value_error("bundle revision must be an exact lowercase Git SHA")
        target = {"macos/aarch64": "aarch64-apple-darwin", "macos/x86_64": "x86_64-apple-darwin"}.get(arch_label)
        if target is None or manifest["target"] != target:
            _value_error("bundle target does not match this macOS architecture")
        required_labels = sorted({"m2-pilot", "candidate", "private", "unsigned", "unnotarized", "unqualified", arch_label})
        labels = manifest["labels"]
        if not isinstance(labels, list) or any(not isinstance(item, str) for item in labels) or labels != required_labels:
            _value_error("bundle labels must be the exact private candidate label set")
        raw_files = manifest["files"]
        if not isinstance(raw_files, list) or not raw_files:
            _value_error("bundle files must be a nonempty array")
        files: list[BundleFile] = []
        seen: set[PurePosixPath] = set()
        for raw_entry in raw_files:
            if not isinstance(raw_entry, dict) or set(raw_entry) != _FILE_KEYS:
                _value_error("bundle file entry must contain exactly path, type, sha256, size and mode")
            relative = _canonical_inventory_path(raw_entry["path"])
            if relative in seen:
                _value_error(f"duplicate bundle inventory path: {relative}")
            seen.add(relative)
            if raw_entry["type"] != "file":
                _value_error(f"bundle inventory type is not file: {relative}")
            size, mode_text, digest = raw_entry["size"], raw_entry["mode"], raw_entry["sha256"]
            if not isinstance(size, int) or isinstance(size, bool) or size < 0:
                _value_error(f"bundle size is invalid: {relative}")
            if not isinstance(mode_text, str) or not _MODE.fullmatch(mode_text):
                _value_error(f"bundle mode is invalid: {relative}")
            if not isinstance(digest, str) or not _HEX.fullmatch(digest):
                _value_error(f"bundle sha256 is invalid: {relative}")
            files.append(BundleFile(relative, size, int(mode_text, 8), digest))
        file_names, directory_names = _walk_bundle(root_fd)
        if file_names != {item.relative_path.as_posix() for item in files} | {"bundle.json"}:
            _value_error("bundle tree does not exactly match the manifest inventory")
        needed_directories = {PurePosixPath(*path.parts[:index]).as_posix() for path in seen for index in range(1, len(path.parts))}
        if directory_names != needed_directories:
            _value_error("bundle contains an undeclared or missing directory")
        for item in files:
            _hash_open_regular(root_fd, item)
        binary_names = {path.parts[1] for path in seen if len(path.parts) == 2 and path.parts[0] == "bin"}
        if binary_names != set(expected_binaries) or any(path.parts[0] == "bin" and len(path.parts) != 2 for path in seen):
            _value_error("bundle binary set does not match the profile allowlist")
        connector_names = {path.parts[1] for path in seen if len(path.parts) >= 3 and path.parts[0] == "connectors"}
        if connector_names != set(expected_connectors):
            _value_error("bundle connector set does not match the profile allowlist")
        profile_hashes = manifest["profile_sha256"]
        if not isinstance(profile_hashes, dict) or set(profile_hashes) != set(expected_connectors):
            _value_error("bundle connector profile hashes do not match the allowlist")
        by_path = {item.relative_path.as_posix(): item.sha256 for item in files}
        for connector in expected_connectors:
            value = profile_hashes[connector]
            if not isinstance(value, str) or not _HEX.fullmatch(value) or by_path.get(f"connectors/{connector}/harness.toml") != value:
                _value_error(f"connector profile hash mismatch: {connector}")
        return BundlePlan(root_fd, requested_profile, manifest["revision"], tuple(files), tuple(expected_binaries), tuple(expected_connectors))
    except Exception:
        os.close(root_fd)
        raise


def open_destination(path: Path) -> Destination:
    absolute = Path(os.path.abspath(os.fspath(path)))
    if not absolute.is_absolute() or absolute.name in ("", ".", "..") or "/" in absolute.name or "\x00" in absolute.name:
        _value_error(f"destination must have a safe absolute leaf path: {path}")
    parent_fd = _open_absolute_directory(absolute.parent)
    info = os.fstat(parent_fd)
    return Destination(parent_fd, absolute.parent, absolute.name, info.st_dev, info.st_ino)


def _open_install_namespace(parent_fd: int, leaf_name: str) -> int:
    if _testing_failure("namespace-open-error"):
        raise OSError(errno.EMFILE, "injected namespace os.open failure")
    return os.open(leaf_name, _flags(directory=True), dir_fd=parent_fd)


def create_install_namespace(destination: Destination) -> InstallNamespace:
    info = os.fstat(destination.parent_fd)
    if (info.st_dev, info.st_ino) != (destination.st_dev, destination.st_ino):
        _value_error("destination parent descriptor identity changed")
    for _ in range(32):
        leaf_name = f".goalcup-install-{secrets.token_hex(16)}"
        namespace: InstallNamespace | None = None
        try:
            with _blocked_control_signals():
                previous = os.umask(0o077)
                try:
                    os.mkdir(leaf_name, 0o700, dir_fd=destination.parent_fd)
                finally:
                    os.umask(previous)
                if _testing_failure("namespace-after-mkdir"):
                    raise NamespaceAcquisitionError(
                        "private namespace open failed after mkdir",
                        f"{destination.parent_path / leaf_name}: created namespace could not be opened; preserved for inspection",
                    )
                try:
                    fd = _open_install_namespace(destination.parent_fd, leaf_name)
                except OSError as error:
                    raise NamespaceAcquisitionError(
                        f"private namespace open failed after mkdir: {error}",
                        f"{destination.parent_path / leaf_name}: created namespace could not be opened; preserved for inspection",
                    ) from error
                child = os.fstat(fd)
                namespace = InstallNamespace(destination, leaf_name, fd, child.st_dev, child.st_ino)
        except FileExistsError:
            continue
        except ControlledSignal as error:
            raise NamespaceAcquisitionError(
                str(error),
                f"{destination.parent_path / leaf_name}: retained namespace after controlled signal",
                namespace,
            ) from error
        assert namespace is not None
        try:
            if _testing_failure("namespace-fchmod-error"):
                raise OSError(errno.EMFILE, "injected namespace fchmod failure")
            os.fchmod(fd, 0o700)
            child = os.fstat(fd)
            if not stat.S_ISDIR(child.st_mode) or child.st_uid != os.getuid() or stat.S_IMODE(child.st_mode) != 0o700:
                _value_error("private install namespace has an unsafe owner, type or mode")
            return namespace
        except Exception as error:
            raise NamespaceAcquisitionError(
                f"private namespace finishing failed after authority was retained: {error}",
                f"{destination.parent_path / leaf_name}: retained namespace after finishing failure",
                namespace,
            ) from error
    _value_error("could not allocate a fresh private install namespace")


def render_daemon_toml(prefix: Path) -> bytes:
    values = {"artifact_root": prefix / "var" / "artifacts", "connectors_dir": prefix / "share" / "goalcup" / "connectors", "config_dir": prefix / "etc", "workspace_parent": prefix / "var" / "workspaces"}
    text = "# Written by install-assisted.py. Owner-only; never edited by a run.\n"
    text += "".join(f"{key} = {json.dumps(os.fspath(value), ensure_ascii=False)}\n" for key, value in values.items())
    text += 'control_addr = "127.0.0.1:0"\n'
    encoded = text.encode("utf-8")
    parsed = tomllib.loads(encoded.decode("utf-8"))
    for key, value in values.items():
        if parsed.get(key) != os.fspath(value):
            _value_error(f"rendered daemon.toml did not round-trip {key}")
    return encoded


def _mkdirs_at(root_fd: int, parts: tuple[str, ...]) -> int:
    current = os.dup(root_fd)
    try:
        for part in parts:
            try:
                _mkdir_exact(current, part)
            except FileExistsError:
                pass
            child = os.open(part, _flags(directory=True), dir_fd=current)
            info = os.fstat(child)
            if not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o700:
                os.close(child)
                _value_error(f"staged directory is not mode 0700: {part}")
            os.close(current)
            current = child
        return current
    except Exception:
        os.close(current)
        raise


def _write_new(parent_fd: int, name: str, data: bytes, mode: int) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(name, flags, mode, dir_fd=parent_fd)
    try:
        offset = 0
        while offset < len(data):
            offset += os.write(fd, data[offset:])
        os.fchmod(fd, mode)
    finally:
        os.close(fd)


def _mapped_path(relative: PurePosixPath) -> PurePosixPath:
    if relative.parts[0] == "bin":
        return relative
    if relative.parts[0] == "connectors":
        return PurePosixPath("share", "goalcup", "connectors", *relative.parts[1:])
    _value_error(f"unsupported bundle inventory root: {relative}")


def _copy_bundle_file(plan: BundlePlan, source: BundleFile, staged_root_fd: int, destination: PurePosixPath) -> None:
    source_fd, before = _open_regular(plan.root_fd, source.relative_path)
    parent_fd = _mkdirs_at(staged_root_fd, tuple(destination.parts[:-1]))
    output_fd = -1
    digest, size = hashlib.sha256(), 0
    try:
        output_fd = os.open(destination.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0), source.mode, dir_fd=parent_fd)
        while block := os.read(source_fd, _BLOCK):
            digest.update(block)
            size += len(block)
            written = 0
            while written < len(block):
                written += os.write(output_fd, block[written:])
            if _testing_failure("mid-copy"):
                _value_error("injected mid-copy failure")
        os.fchmod(output_fd, source.mode)
        after = os.fstat(source_fd)
    finally:
        _close(output_fd)
        os.close(parent_fd)
        os.close(source_fd)
    if not _same_node(before, after) or size != source.size or digest.hexdigest() != source.sha256 or stat.S_IMODE(before.st_mode) != source.mode:
        _value_error(f"bundle source failed validation while copying: {source.relative_path}")


def stage_install(plan: BundlePlan, namespace: InstallNamespace, temporary_name: str, prefix: Path) -> None:
    if not temporary_name or "/" in temporary_name or temporary_name in (".", "..") or "\x00" in temporary_name:
        _value_error("invalid staging leaf name")
    _mkdir_exact(namespace.fd, temporary_name)
    staged_fd = os.open(temporary_name, _flags(directory=True), dir_fd=namespace.fd)
    keep_fd = False
    try:
        for path in (("bin",), ("share",), ("share", "goalcup"), ("share", "goalcup", "connectors"), ("etc",), ("var",), ("var", "artifacts"), ("var", "workspaces")):
            fd = _mkdirs_at(staged_fd, path)
            os.close(fd)
        for item in plan.files:
            _copy_bundle_file(plan, item, staged_fd, _mapped_path(item.relative_path))
        daemon_toml = render_daemon_toml(prefix)
        etc_fd = _open_relative(staged_fd, PurePosixPath("etc"), directory=True)
        try:
            _write_new(etc_fd, "daemon.toml", daemon_toml, 0o600)
        finally:
            os.close(etc_fd)
        manifest = {"format": 2, "source_revision": plan.source_revision, "config_file": os.fspath(prefix / "etc" / "daemon.toml"), "installed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
        manifest_content = (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode()
        _write_new(staged_fd, ".goalcup-install-manifest.json", manifest_content, 0o600)
        mapped = {_mapped_path(item.relative_path): item for item in plan.files}
        expected_directories = {
            PurePosixPath(*path.parts[:index]).as_posix()
            for path in mapped
            for index in range(1, len(path.parts))
        } | {
            "bin",
            "share",
            "share/goalcup",
            "share/goalcup/connectors",
            "etc",
            "var",
            "var/artifacts",
            "var/workspaces",
        }
        expected_tree = {
            path: _ExpectedTreeEntry(stat.S_IFDIR, 0o700, None, None)
            for path in expected_directories
        }
        expected_tree.update(
            {
                path.as_posix(): _ExpectedTreeEntry(
                    stat.S_IFREG,
                    item.mode,
                    item.size,
                    item.sha256,
                )
                for path, item in mapped.items()
            }
        )
        expected_tree["etc/daemon.toml"] = _ExpectedTreeEntry(
            stat.S_IFREG,
            0o600,
            len(daemon_toml),
            hashlib.sha256(daemon_toml).hexdigest(),
        )
        expected_tree[".goalcup-install-manifest.json"] = _ExpectedTreeEntry(
            stat.S_IFREG,
            0o600,
            len(manifest_content),
            hashlib.sha256(manifest_content).hexdigest(),
        )
        _test_barrier("before-prefix-registration")
        _register_staged_leaf(
            namespace.fd,
            temporary_name,
            staged_fd,
            expected_mode=0o700,
            expected_size=None,
            expected_digest=None,
            expected_tree=expected_tree,
        )
        keep_fd = True
    finally:
        if not keep_fd:
            os.close(staged_fd)


def publish_no_replace(
    namespace_fd: int,
    temporary_name: str,
    destination: Destination,
    *,
    barrier_phase: str = "before-prefix-rename",
    post_verification_barrier_phase: str | None = None,
) -> PublishedPath:
    if not temporary_name or "/" in temporary_name or temporary_name in (".", ".."):
        _value_error("invalid publication source leaf")
    parent = os.fstat(destination.parent_fd)
    if (parent.st_dev, parent.st_ino) != (destination.st_dev, destination.st_ino):
        _value_error("destination parent descriptor identity changed")
    _test_barrier(barrier_phase)
    library = ctypes.CDLL(None, use_errno=True)
    rename = getattr(library, "renameatx_np", None)
    if rename is None:
        _value_error("Darwin renameatx_np(RENAME_EXCL) is unavailable")
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    with _blocked_control_signals():
        staged = _verify_staged_leaf(namespace_fd, temporary_name)
        if post_verification_barrier_phase is not None:
            _test_barrier(post_verification_barrier_phase)
        candidate = PublishedPath(destination, staged.st_dev, staged.st_ino)
        if _testing_failure("ambiguous-prefix-rename"):
            destination.publication_ambiguous = candidate
            raise PublicationAmbiguous("publication outcome ambiguous: injected rename error", candidate)
        result = rename(namespace_fd, os.fsencode(temporary_name), destination.parent_fd, os.fsencode(destination.leaf_name), 0x00000004)
        if result == 0:
            # This assignment is the first instruction after the syscall: it cannot fail and
            # is visible to recovery even if a later hook, barrier, or append raises.
            destination.published = candidate
            try:
                candidate = _verify_published_leaf(staged, destination)
                destination.published = candidate
            except PublicationAmbiguous as error:
                destination.published = error.candidate
                destination.publication_ambiguous = error.candidate
                raise
    if result != 0:
        number = ctypes.get_errno()
        if number in (errno.EEXIST, errno.ENOTEMPTY):
            raise FileExistsError(number, "destination already exists", destination.leaf_name)
        if number in (errno.ENOSYS, errno.ENOTSUP, errno.EINVAL):
            _value_error("exclusive no-replace publication is unsupported on this filesystem")
        destination.publication_ambiguous = candidate
        raise PublicationAmbiguous(f"publication outcome ambiguous: {os.strerror(number)}", candidate)
    if _testing_failure("after-prefix-rename"):
        _value_error("injected failure immediately after successful prefix rename")
    return candidate


def _descriptor_path(fd: int) -> Path | None:
    if sys.platform == "darwin":
        try:
            import fcntl

            raw = fcntl.fcntl(fd, 50, b"\0" * 1024)
            value = raw.split(b"\0", 1)[0]
            return Path(os.fsdecode(value)) if value else None
        except (OSError, ValueError):
            return None
    try:
        return Path(os.readlink(f"/proc/self/fd/{fd}"))
    except OSError:
        return None


def _reported_path(item: PublishedPath) -> str:
    intended = item.destination.parent_path / item.destination.leaf_name
    current_parent = _descriptor_path(item.destination.parent_fd)
    identity = f"expected retained source device {item.st_dev}, inode {item.st_ino}"
    if item.observed_st_dev is None or item.observed_st_ino is None:
        identity += "; safely observed destination identity unavailable"
    else:
        identity += (
            f"; safely observed destination device {item.observed_st_dev}, "
            f"inode {item.observed_st_ino}"
        )
    if current_parent is None:
        return f"  - intended path at validation: {intended}; current descriptor-derived path: unavailable ({identity})"
    current = current_parent / item.destination.leaf_name
    if current != intended:
        return f"  - intended path (stale): {intended}; current descriptor-derived path: {current} ({identity})"
    return f"  - intended path at validation: {intended}; current descriptor-derived path: {current} ({identity})"


def report_manual_recovery(published: tuple[PublishedPath, ...], reason: str) -> str:
    paths = "\n".join(_reported_path(item) for item in published)
    return (f"assisted installation failed after publication: {reason}\npublished paths preserved:\n{paths}\nmanual recovery is inspection-first: verify that each path and its install manifest still belong to this install before deliberately running the documented uninstall step. A same-UID process may have replaced a path; the installer cannot distinguish or delete that replacement safely.")


def _cleanup_namespace(namespace: InstallNamespace) -> str | None:
    if namespace.fd < 0:
        return None
    parent_fd, leaf = namespace.parent.parent_fd, namespace.leaf_name
    intended = namespace.parent.parent_path / leaf
    try:
        retained = os.fstat(namespace.fd)
        named = os.stat(leaf, dir_fd=parent_fd, follow_symlinks=False)
        if not stat.S_ISDIR(named.st_mode) or (named.st_dev, named.st_ino) != (namespace.st_dev, namespace.st_ino):
            return f"{intended}: namespace path no longer names retained descriptor; preserved for inspection"
        if (retained.st_dev, retained.st_ino) != (namespace.st_dev, namespace.st_ino):
            return f"{intended}: retained namespace descriptor identity changed; preserved for inspection"
        with os.scandir(namespace.fd) as children:
            if next(children, None) is not None:
                current = _descriptor_path(namespace.fd)
                return f"{current or intended}: retained nonempty private namespace for inspection"
        _test_barrier("before-namespace-cleanup-removal")
        current = _descriptor_path(namespace.fd)
        return f"{current or intended}: retained empty private namespace because inode-conditional removal is unavailable"
    except FileNotFoundError:
        current = _descriptor_path(namespace.fd)
        if current is None or str(current).endswith(" (deleted)"):
            return None
        return f"{current}: private namespace no longer has its intended name; preserved for inspection"
    except Exception as error:
        return f"{intended}: {error}"


def _parse_arguments(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Install the private M2 pilot bundle on macOS.")
    parser.add_argument("--bundle", required=True, type=Path)
    parser.add_argument("--prefix", required=True, type=Path)
    parser.add_argument("--profile", default="m2-pilot")
    return parser.parse_args(argv)


def _host_arch() -> str:
    machine = os.uname().machine
    if machine == "arm64":
        return "macos/aarch64"
    if machine == "x86_64":
        return "macos/x86_64"
    _value_error(f"unsupported host architecture: {machine}")


def _validate_prefix(prefix: Path, home: Path) -> Path:
    if not prefix.is_absolute():
        _value_error(f"--prefix must be absolute: {prefix}")
    absolute = Path(os.path.abspath(os.fspath(prefix)))
    text, home_text = os.fspath(absolute), os.fspath(home)
    if text in _BROAD_PREFIXES or text.startswith("/System/") or text.startswith("/usr/"):
        _value_error(f"refusing a broad install root: {absolute}")
    if text in {home_text, f"{home_text}/Library", f"{home_text}/Desktop", f"{home_text}/Documents", f"{home_text}/Downloads", f"{home_text}/Applications", f"{home_text}/Pictures", f"{home_text}/Movies", f"{home_text}/Music"}:
        _value_error(f"refusing a broad install root: {absolute}")
    return absolute


def install(arguments: argparse.Namespace) -> None:
    if sys.platform != "darwin":
        _value_error("this installer only supports macOS")
    home_value = os.environ.get("HOME")
    if not home_value:
        _value_error("HOME is required")
    home = Path(os.path.abspath(home_value))
    prefix = _validate_prefix(arguments.prefix, home)
    plan = prefix_destination = prefix_namespace = None
    published: list[PublishedPath] = []
    private_residuals: list[str] = []
    old_handlers: dict[signal.Signals, object] = {}

    def controlled_handler(number: int, _frame: object) -> NoReturn:
        raise ControlledSignal(f"controlled {signal.Signals(number).name}")

    for number in _CONTROLLED_SIGNALS:
        old_handlers[number] = signal.signal(number, controlled_handler)

    def remember_residual(value: str | None) -> None:
        if value and value not in private_residuals:
            private_residuals.append(value)

    def recover_publication_state() -> None:
        if prefix_destination is None:
            return
        item = prefix_destination.publication_ambiguous or prefix_destination.published
        if item is not None and item not in published:
            published.append(item)

    try:
        plan = inspect_bundle(arguments.bundle, arguments.profile, ("goalcup", "goalcup-daemon", "goalcup-verify"), ("codex-cli", "claude-code-cli"), _host_arch())
        prefix_destination = open_destination(prefix)
        try:
            prefix_namespace = create_install_namespace(prefix_destination)
        except NamespaceAcquisitionError as error:
            prefix_namespace = error.namespace
            remember_residual(error.residual)
            raise
        stage_install(plan, prefix_namespace, "prefix", prefix)
        if _testing_failure("copy"):
            _value_error("injected copy failure")
        _test_barrier("before-publication")
        published.append(
            publish_no_replace(
                prefix_namespace.fd,
                "prefix",
                prefix_destination,
                barrier_phase="before-prefix-rename",
                post_verification_barrier_phase="after-prefix-final-verification",
            )
        )
        _test_barrier("after-prefix-publication")
        if _testing_failure("after-prefix"):
            _value_error("injected post-prefix publication failure")
        remember_residual(_cleanup_namespace(prefix_namespace))
        if private_residuals:
            print(
                "install-assisted.sh: retained private namespace(s) for inspection: "
                + "; ".join(private_residuals),
                file=sys.stderr,
            )
        print(f"install-assisted.sh: installed under {prefix}")
        print(f"install-assisted.sh: daemon config {prefix / 'etc' / 'daemon.toml'}")
        print(f"install-assisted.sh: artifact root {prefix / 'var' / 'artifacts'}")
        print(
            "install-assisted.sh: no service was installed; run "
            f"GOALCUP_CONFIG_DIR={prefix / 'etc'} {prefix / 'bin' / 'goalcup'} start ..., "
            "which starts the daemon for the run"
        )
    except (Exception, KeyboardInterrupt) as error:
        recover_publication_state()
        if prefix_namespace is not None:
            remember_residual(_cleanup_namespace(prefix_namespace))
        if published:
            reason = str(error) + (("; retained private namespace: " + "; ".join(private_residuals)) if private_residuals else "")
            print(report_manual_recovery(tuple(published), reason), file=sys.stderr)
        else:
            message = f"assisted installation refused before publication: {error}"
            if private_residuals:
                message += "; retained private namespace: " + "; ".join(private_residuals)
            print(message, file=sys.stderr)
        raise SystemExit(1) from error
    finally:
        for number, handler in old_handlers.items():
            signal.signal(number, handler)
        if plan is not None:
            plan.close()
        if prefix_namespace is not None:
            prefix_namespace.close()
        if prefix_destination is not None:
            prefix_destination.close()


def main(argv: list[str] | None = None) -> None:
    install(_parse_arguments(sys.argv[1:] if argv is None else argv))


if __name__ == "__main__":
    main()
