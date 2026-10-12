#!/usr/bin/env python3
"""Build an unsigned, unqualified Goal Cup staging directory.

The tool copies only named prebuilt binaries and the trusted connector tree.  It
does not build, execute, install, sign, archive, upload, or publish a release.
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import hashlib
import json
import os
import shutil
import stat
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import NamedTuple


# Local capture pilot: the capture CLI, daemon and verifier, and nothing else. The capture
# proxy runs in-process inside `goalcup start`; it is not a further executable. This is the
# only profile (D42): the V1 bundles required a `goalcup-publisher` no crate ever defined (so
# they could never ship) and bundled the V1 `goalcup-mcp`; neither is part of the v0.5 product.
M2_PILOT_BINARIES = (
    "goalcup",
    "goalcup-daemon",
    "goalcup-verify",
)
BINARIES = M2_PILOT_BINARIES

# The connector directories the bundle ships. Named, not "whichever exist": the two measured
# capture harnesses in the private pilot closure.
M2_PILOT_CONNECTORS = frozenset({"codex-cli", "claude-code-cli"})

# macOS only. Linux is best-effort until it is tested and Windows is planned (D42); neither has
# a bundle yet, and `goalcup-platform` refuses to compile on a non-unix host.
TARGETS = frozenset({"x86_64-apple-darwin", "aarch64-apple-darwin"})

PROFILES = ("m2-pilot",)

_ARCH_LABELS = {
    "x86_64-apple-darwin": "macos/x86_64",
    "aarch64-apple-darwin": "macos/aarch64",
}

_REPARSE_POINT = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
_READONLY_ATTRIBUTE = getattr(stat, "FILE_ATTRIBUTE_READONLY", 0x1)
_WINDOWS_HOST = os.name == "nt"
_COPY_BLOCK_SIZE = 1024 * 1024


class SourceEntry(NamedTuple):
    relative: str
    kind: str
    mode: int
    device: int
    inode: int
    size: int
    modified_ns: int
    changed_ns: int
    sha256: str | None


class TreeSnapshot(NamedTuple):
    root: SourceEntry
    entries: tuple[SourceEntry, ...]


def refuse(message: str) -> None:
    raise ValueError(message)


def _absolute_path(value: os.PathLike[str] | str) -> Path:
    """Normalize dot components without resolving filesystem links."""
    return Path(os.path.abspath(os.fspath(value)))


def _is_reparse(info: object) -> bool:
    return bool(getattr(info, "st_file_attributes", 0) & _REPARSE_POINT)


def _path_exists(path: Path) -> bool:
    """Return true for every directory entry, including a broken symlink."""
    return os.path.lexists(path)


def _validate_path_chain(path: Path, *, label: str) -> os.stat_result:
    """Reject links, reparse points and non-directories in a source path chain."""
    if not path.is_absolute():
        refuse(f"{label} must be absolute: {path}")

    current = Path(path.anchor)
    parts = path.parts[1:]
    if not parts:
        parts = ()
    last_info: os.stat_result | None = None
    for index, component in enumerate(parts):
        current /= component
        try:
            info = current.lstat()
        except FileNotFoundError:
            refuse(f"missing {label}: {current}")
        if stat.S_ISLNK(info.st_mode) or _is_reparse(info):
            refuse(f"refusing symlink/reparse source path for {label}: {current}")
        if index < len(parts) - 1 and not stat.S_ISDIR(info.st_mode):
            refuse(f"refusing non-directory source ancestor for {label}: {current}")
        last_info = info

    if last_info is None:
        last_info = current.lstat()
    return last_info


def _validate_directory(path: Path, *, label: str) -> os.stat_result:
    info = _validate_path_chain(path, label=label)
    if not stat.S_ISDIR(info.st_mode):
        refuse(f"refusing non-directory {label}: {path}")
    return info


def _validate_output_parent(output: Path) -> None:
    parent = output.parent
    info = _validate_path_chain(parent, label="output parent")
    if not stat.S_ISDIR(info.st_mode):
        refuse(f"refusing non-directory output parent: {parent}")


def _identity(
    relative: str,
    kind: str,
    info: os.stat_result,
    digest: str | None = None,
) -> SourceEntry:
    return SourceEntry(
        relative=relative,
        kind=kind,
        mode=stat.S_IMODE(info.st_mode),
        device=info.st_dev,
        inode=info.st_ino,
        size=info.st_size,
        modified_ns=info.st_mtime_ns,
        changed_ns=info.st_ctime_ns,
        sha256=digest,
    )


def _open_regular_source(path: Path) -> tuple[int, os.stat_result]:
    _validate_path_chain(path, label="input file")
    flags = os.O_RDONLY
    flags |= getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_NONBLOCK", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        if error.errno in (errno.ELOOP, errno.ENXIO):
            refuse(f"refusing symlink or special input file: {path}")
        raise
    info = os.fstat(descriptor)
    if _is_reparse(info) or not stat.S_ISREG(info.st_mode):
        os.close(descriptor)
        refuse(f"refusing non-regular or reparse input file: {path}")
    return descriptor, info


def _hash_regular_source(path: Path) -> tuple[str, os.stat_result]:
    descriptor, before = _open_regular_source(path)
    digest = hashlib.sha256()
    try:
        with os.fdopen(descriptor, "rb", closefd=True) as source:
            for block in iter(lambda: source.read(_COPY_BLOCK_SIZE), b""):
                digest.update(block)
            after = os.fstat(source.fileno())
    except Exception:
        # fdopen owns the descriptor after it succeeds; close only if it did not.
        try:
            os.close(descriptor)
        except OSError:
            pass
        raise
    if _identity("", "file", before) != _identity("", "file", after):
        refuse(f"source changed while reading: {path}")
    return digest.hexdigest(), after


def _read_regular_source(path: Path) -> bytes:
    descriptor, before = _open_regular_source(path)
    try:
        with os.fdopen(descriptor, "rb", closefd=True) as source:
            content = source.read()
            after = os.fstat(source.fileno())
    except Exception:
        try:
            os.close(descriptor)
        except OSError:
            pass
        raise
    if _identity("", "file", before) != _identity("", "file", after):
        refuse(f"source changed while reading: {path}")
    return content


def _snapshot_regular(path: Path, relative: str) -> SourceEntry:
    digest, info = _hash_regular_source(path)
    return _identity(relative, "file", info, digest)


def _snapshot_tree(
    root: Path, *, label: str, only: frozenset[str] | None = None
) -> TreeSnapshot:
    """Snapshot a source tree, optionally restricted to named top-level entries.

    `only`, when given, is a required set: every name in it must exist as a top-level
    directory or the snapshot refuses, and every top-level entry NOT in it is skipped
    entirely (never visited, never staged). Nested directories are unaffected by the
    filter — it only ever narrows which connector directories are considered, not what is
    inside one.
    """
    root_info = _validate_directory(root, label=label)
    entries: list[SourceEntry] = []
    seen_top_level: set[str] = set()

    def visit(directory: Path, relative_directory: Path) -> None:
        _validate_directory(directory, label=label)
        try:
            children = sorted(os.scandir(directory), key=lambda item: item.name)
        except OSError as error:
            raise ValueError(f"cannot scan {label}: {directory}: {error}") from error
        for child in children:
            if relative_directory == Path() and only is not None and child.name not in only:
                continue
            path = directory / child.name
            relative = (relative_directory / child.name).as_posix()
            info = child.stat(follow_symlinks=False)
            if stat.S_ISLNK(info.st_mode) or _is_reparse(info):
                refuse(f"refusing symlink/reparse in {label}: {path}")
            if relative_directory == Path():
                seen_top_level.add(child.name)
            if stat.S_ISDIR(info.st_mode):
                entries.append(_identity(relative, "directory", info))
                visit(path, relative_directory / child.name)
            elif stat.S_ISREG(info.st_mode):
                entries.append(_snapshot_regular(path, relative))
            else:
                refuse(f"refusing special or unsafe input in {label}: {path}")

    visit(root, Path())
    if only is not None:
        missing = sorted(only - seen_top_level)
        if missing:
            refuse(f"missing required connector: {missing[0]}")
    return TreeSnapshot(
        root=_identity("", "directory", root_info),
        entries=tuple(entries),
    )


def _snapshot_binary_sources(
    root: Path, extension: str, *, names: tuple[str, ...] = BINARIES
) -> tuple[SourceEntry, tuple[SourceEntry, ...]]:
    root_info = _validate_directory(root, label="binary source root")
    sources: list[SourceEntry] = []
    for name in names:
        filename = f"{name}{extension}"
        path = root / filename
        if not _path_exists(path):
            refuse(f"missing required binary: {filename}")
        sources.append(_snapshot_regular(path, filename))
    return _identity("", "directory", root_info), tuple(sources)


def _harness_ids(connectors: Path, snapshot: TreeSnapshot) -> dict[str, str]:
    by_path = {entry.relative: entry for entry in snapshot.entries}
    connector_directories = [
        entry
        for entry in snapshot.entries
        if "/" not in entry.relative and entry.kind == "directory"
    ]
    root_entries = [entry for entry in snapshot.entries if "/" not in entry.relative]
    if len(root_entries) != len(connector_directories):
        invalid = next(entry for entry in root_entries if entry.kind != "directory")
        refuse(f"invalid connector entry (ordinary directory required): {invalid.relative}")

    identifiers: dict[str, str] = {}
    for connector in connector_directories:
        harness_relative = f"{connector.relative}/harness.toml"
        harness = by_path.get(harness_relative)
        if harness is None or harness.kind != "file":
            refuse(f"missing connector harness: {harness_relative}")
        harness_path = connectors / harness_relative
        try:
            parsed = tomllib.loads(_read_regular_source(harness_path).decode("utf-8"))
        except (tomllib.TOMLDecodeError, UnicodeDecodeError) as error:
            raise ValueError(f"invalid connector harness {harness_relative}: {error}") from error
        identifier = parsed.get("id")
        # Match the protocol's ID syntax: it currently requires a nonempty string. Packaging
        # additionally binds that identity to the directory basename below, so metadata lookup
        # and installed paths cannot name two different connectors.
        if not isinstance(identifier, str) or not identifier:
            refuse(f"invalid connector id in {harness_relative}")
        if connector.relative != identifier:
            refuse(
                f"connector directory {connector.relative!r} declares id {identifier!r}; "
                "the directory basename and schema harness id must match"
            )
        if identifier in identifiers:
            refuse(f"duplicate connector id: {identifier}")
        identifiers[identifier] = connector.relative
    return identifiers


def _copy_regular_file(source: Path, destination: Path, mode: int) -> None:
    descriptor, before = _open_regular_source(source)
    try:
        with os.fdopen(descriptor, "rb", closefd=True) as input_file:
            with destination.open("xb") as output_file:
                shutil.copyfileobj(input_file, output_file, _COPY_BLOCK_SIZE)
            after = os.fstat(input_file.fileno())
    except Exception:
        try:
            os.close(descriptor)
        except OSError:
            pass
        raise
    if _identity("", "file", before) != _identity("", "file", after):
        refuse(f"source changed while copying: {source}")
    if _WINDOWS_HOST:
        # Windows chmod represents only the read-only attribute.  Keep the full
        # portable mode for bundle.json and apply the one bit this host can enforce.
        os.chmod(destination, stat.S_IWRITE if mode & stat.S_IWRITE else stat.S_IREAD)
    else:
        os.chmod(destination, mode)


def _copy_connector_tree(
    source: Path, destination: Path, snapshot: TreeSnapshot
) -> None:
    destination.mkdir(mode=0o700)
    directories = [entry for entry in snapshot.entries if entry.kind == "directory"]
    files = [entry for entry in snapshot.entries if entry.kind == "file"]

    for entry in directories:
        (destination / entry.relative).mkdir(mode=0o700)
    for entry in files:
        _copy_regular_file(source / entry.relative, destination / entry.relative, entry.mode)

    # Apply restrictive or read-only directory modes only after their children
    # have been constructed.
    for entry in sorted(directories, key=lambda item: item.relative.count("/"), reverse=True):
        os.chmod(destination / entry.relative, entry.mode)
    os.chmod(destination, snapshot.root.mode)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(_COPY_BLOCK_SIZE), b""):
            digest.update(block)
    return digest.hexdigest()


def _validate_staged_file(
    info: object,
    intended_mode: int,
    relative: str,
    *,
    windows_host: bool | None = None,
) -> None:
    """Validate the file kind and the permissions this host can represent."""
    if _is_reparse(info):
        refuse(f"staged output unexpectedly contains a reparse point: {relative}")
    mode = int(getattr(info, "st_mode"))
    if not stat.S_ISREG(mode):
        refuse(f"staged output unexpectedly contains a special file: {relative}")

    on_windows = _WINDOWS_HOST if windows_host is None else windows_host
    if on_windows:
        attributes = getattr(info, "st_file_attributes", None)
        actual_readonly = (
            bool(attributes & _READONLY_ATTRIBUTE)
            if attributes is not None
            else not bool(mode & stat.S_IWRITE)
        )
        intended_readonly = not bool(intended_mode & stat.S_IWRITE)
        if actual_readonly != intended_readonly:
            refuse(
                f"copied read-only/writable attribute does not match intended mode: {relative}"
            )
    elif stat.S_IMODE(mode) != intended_mode:
        refuse(f"copied mode does not match intended mode: {relative}")


def _expected_payload(
    connectors: TreeSnapshot,
    binaries: tuple[SourceEntry, ...],
    binary_mode: int,
) -> dict[str, tuple[str, int]]:
    expected: dict[str, tuple[str, int]] = {}
    for entry in connectors.entries:
        if entry.kind == "file":
            assert entry.sha256 is not None
            expected[f"connectors/{entry.relative}"] = (entry.sha256, entry.mode)
    for entry in binaries:
        assert entry.sha256 is not None
        expected[f"bin/{entry.relative}"] = (entry.sha256, binary_mode)
    return expected


def _build_inventory(
    root: Path, expected: dict[str, tuple[str, int]]
) -> list[dict[str, object]]:
    inventory: list[dict[str, object]] = []
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        if stat.S_ISLNK(info.st_mode) or _is_reparse(info):
            refuse(f"staged output unexpectedly contains a link/reparse point: {relative}")
        if stat.S_ISDIR(info.st_mode):
            continue
        if relative == "bundle.json":
            continue
        expected_file = expected.get(relative)
        if expected_file is None:
            refuse(f"staged inventory contains an unexpected payload file: {relative}")
        intended_mode = expected_file[1]
        _validate_staged_file(info, intended_mode, relative)
        inventory.append(
            {
                "path": relative,
                "type": "file",
                "sha256": _sha256(path),
                "size": info.st_size,
                "mode": f"{intended_mode:04o}",
            }
        )
    return inventory


def _validate_inventory(
    inventory: list[dict[str, object]],
    expected: dict[str, tuple[str, int]],
) -> None:
    actual_paths = {str(entry["path"]) for entry in inventory}
    if actual_paths != set(expected):
        refuse("staged inventory does not match the requested payload files")
    for item in inventory:
        expected_digest, expected_mode = expected[str(item["path"])]
        if item["type"] != "file":
            refuse(f"staged inventory has an invalid type: {item['path']}")
        if item["sha256"] != expected_digest:
            refuse(f"copied bytes do not match source: {item['path']}")
        if item["mode"] != f"{expected_mode:04o}":
            refuse(f"copied mode does not match intended mode: {item['path']}")


def _publish_no_replace(temporary: Path, output: Path) -> None:
    """Atomically rename a completed directory while refusing any destination."""
    if sys.platform == "darwin":
        library = ctypes.CDLL(None, use_errno=True)
        rename = getattr(library, "renamex_np", None)
        if rename is None:
            refuse("atomic no-replace directory publication is unsupported on this host")
        rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        rename.restype = ctypes.c_int
        result = rename(os.fsencode(temporary), os.fsencode(output), 0x00000004)
    elif sys.platform.startswith("linux"):
        library = ctypes.CDLL(None, use_errno=True)
        rename = getattr(library, "renameat2", None)
        if rename is None:
            refuse("atomic no-replace directory publication is unsupported on this host")
        rename.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        rename.restype = ctypes.c_int
        result = rename(-100, os.fsencode(temporary), -100, os.fsencode(output), 1)
    elif os.name == "nt":
        # Windows rename refuses an existing destination; there is no POSIX-style
        # replacement behavior to opt out of for this call.
        try:
            os.rename(temporary, output)
        except FileExistsError as error:
            raise ValueError(f"output already exists: {output}") from error
        return
    else:
        refuse("atomic no-replace directory publication is unsupported on this host")

    if result == 0:
        return
    error_number = ctypes.get_errno()
    if error_number in (errno.EEXIST, errno.ENOTEMPTY):
        refuse(f"output already exists: {output}")
    if error_number in (errno.ENOSYS, errno.ENOTSUP, errno.EINVAL):
        refuse("atomic no-replace directory publication is unsupported on this filesystem")
    raise OSError(error_number, os.strerror(error_number), os.fspath(output))


def _ensure_unchanged(
    repo_root: Path,
    connectors_path: Path,
    connector_snapshot: TreeSnapshot,
    binaries_root: Path,
    binary_root_snapshot: SourceEntry,
    binary_snapshot: tuple[SourceEntry, ...],
    extension: str,
    *,
    binary_names: tuple[str, ...] = BINARIES,
    connector_only: frozenset[str] | None = None,
) -> None:
    current_connectors = _snapshot_tree(
        connectors_path, label="connector source tree", only=connector_only
    )
    current_binary_root, current_binaries = _snapshot_binary_sources(
        binaries_root, extension, names=binary_names
    )
    _validate_directory(repo_root, label="repository source root")
    if current_connectors != connector_snapshot:
        refuse("source changed during staging: connector tree")
    if current_binary_root != binary_root_snapshot or current_binaries != binary_snapshot:
        refuse("source changed during staging: binary tree")


def stage(
    repo_root: os.PathLike[str] | str,
    binaries: os.PathLike[str] | str,
    output: os.PathLike[str] | str,
    target: str,
    revision: str,
    version: str,
    *,
    profile: str = "m2-pilot",
) -> Path:
    """Construct and atomically install one deterministic staged directory.

    `profile="m2-pilot"` (the default, and the only profile) is the private local-capture
    candidate: exactly three binaries, the two measured capture connectors and a macOS-only
    target. It records candidate, private, unsigned, unnotarized and unqualified labels, plus
    each schema harness's SHA-256. Every payload binary and connector file is
    SHA-256-bound in the inventory.
    No profile ever includes `goalcup-devserver`, `goalcup-git`, traces, keys, or `.git`:
    this tool never looks at any of them.
    """
    repo_path = _absolute_path(repo_root)
    binaries_path = _absolute_path(binaries)
    output_path = _absolute_path(output)

    for source in (repo_path, binaries_path):
        if (
            output_path == source
            or output_path.is_relative_to(source)
            or source.is_relative_to(output_path)
        ):
            refuse(f"output overlaps an input: {output_path} and {source}")
    if target not in TARGETS:
        refuse(f"unsupported target: {target}")
    if profile not in PROFILES:
        refuse(f"unsupported profile: {profile}")
    if not isinstance(revision, str) or not revision:
        refuse("revision must be a nonempty string")
    if not isinstance(version, str) or not version:
        refuse("version must be a nonempty string")
    if _path_exists(output_path):
        refuse(f"output already exists: {output_path}")
    _validate_output_parent(output_path)

    _validate_directory(repo_path, label="repository source root")
    extension = ""
    binary_names = M2_PILOT_BINARIES
    connector_only = M2_PILOT_CONNECTORS
    binary_root_snapshot, binary_snapshot = _snapshot_binary_sources(
        binaries_path, extension, names=binary_names
    )
    connectors_path = repo_path / "connectors"
    connector_snapshot = _snapshot_tree(
        connectors_path, label="connector source tree", only=connector_only
    )
    connector_paths = _harness_ids(connectors_path, connector_snapshot)

    binary_mode = 0o755
    with tempfile.TemporaryDirectory(
        prefix=f"{output_path.name}.tmp-", dir=output_path.parent
    ) as temporary_name:
        temporary = Path(temporary_name)
        (temporary / "bin").mkdir(mode=0o755)
        _copy_connector_tree(
            connectors_path, temporary / "connectors", connector_snapshot
        )
        for entry in binary_snapshot:
            _copy_regular_file(
                binaries_path / entry.relative,
                temporary / "bin" / entry.relative,
                binary_mode,
            )

        expected = _expected_payload(
            connector_snapshot, binary_snapshot, binary_mode
        )
        inventory = _build_inventory(temporary, expected)
        _validate_inventory(inventory, expected)
        _ensure_unchanged(
            repo_path,
            connectors_path,
            connector_snapshot,
            binaries_path,
            binary_root_snapshot,
            binary_snapshot,
            extension,
            binary_names=binary_names,
            connector_only=connector_only,
        )
        metadata = {
            "format": 1,
            "target": target,
            "revision": revision,
            "version": version,
            "profile": profile,
            "qualification": "unqualified",
            "signature": "unsigned",
            "files": inventory,
        }
        if profile == "m2-pilot":
            metadata.update(
                {
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
                            _ARCH_LABELS[target],
                        }
                    ),
                    "profile_sha256": {
                        connector: next(
                            item["sha256"]
                            for item in inventory
                            if item["path"]
                            == f"connectors/{directory}/harness.toml"
                        )
                        for connector, directory in sorted(connector_paths.items())
                    },
                }
            )
        bundle = temporary / "bundle.json"
        bundle.write_text(
            json.dumps(metadata, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
            + "\n",
            encoding="utf-8",
        )
        os.chmod(bundle, 0o644)
        _publish_no_replace(temporary, output_path)

    return output_path


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Create an unsigned, unqualified Goal Cup staging directory."
    )
    parser.add_argument(
        "--stage",
        action="store_true",
        help="required explicit staging mode; performs no build or publication",
    )
    parser.add_argument("--repo", required=True, help="trusted Goal Cup source root")
    parser.add_argument("--binaries", required=True, help="prebuilt binary directory")
    parser.add_argument("--output", required=True, help="fresh output directory")
    parser.add_argument("--target", required=True, help="planned native target triple")
    parser.add_argument("--revision", required=True, help="exact source revision label")
    parser.add_argument("--version", required=True, help="exact version label")
    parser.add_argument(
        "--profile",
        choices=PROFILES,
        default="m2-pilot",
        help="package profile: 'm2-pilot' (the default and only profile) is the three-binary "
        "private M2 macOS candidate",
    )
    return parser


def main() -> None:
    parser = _parser()
    arguments = parser.parse_args()
    if not arguments.stage:
        parser.error("--stage is required")
    try:
        stage(
            arguments.repo,
            arguments.binaries,
            arguments.output,
            arguments.target,
            arguments.revision,
            arguments.version,
            profile=arguments.profile,
        )
    except (OSError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
