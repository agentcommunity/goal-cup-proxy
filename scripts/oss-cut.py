#!/usr/bin/env python3
"""Build a clean public tree from an explicit allowlist (D42: the public repo is a fresh copy).

    python3 scripts/oss-cut.py --out /path/to/new-dir [--gate smoke|full] [--cargo-arg=--offline]

The source must be a clean repository root with a commit. Ignored and untracked files never travel;
a committed regular file is copied only when an ALLOW rule matches it and no DENY rule does. The
result is then scanned and a report is printed:

  * configured home paths and private names;
  * email addresses outside a short allowlist of safe ones;
  * secret-shaped strings (counts only; the repository's own redaction tests and fixtures hold
    synthetic ones, so a nonzero count is a prompt to look, not proof of a leak);
  * relative Markdown links and anchors, checked by the copied canonical checker.

Exit status table (earlier non-link findings keep their established precedence):

  * 0: the cut and every requested check passed;
  * 1: a cut-tree usage/refusal check or requested build/test gate failed;
  * 2: a strict home path, unclassified email, or canonical link check failed;
  * 3: a required file is missing (including the canonical link checker).

Home paths inside crates tests, spikes fixtures and docs/qualification evidence are reported but
accepted: much of that is hash-bound (see docs/DECISIONS.md D42 and the OS-02 decision).

`--gate smoke` runs `cargo build` and `cargo test -p goalcup-protocol` in an isolated sibling
validation copy; `--gate full` runs the four-command gate from README.md there. Pass
`--cargo-arg=FLAG` (repeatable, with the equals sign) to add
flags such as `--offline`. Set CARGO_BUILD_JOBS in the environment to bound parallelism.

The script writes only `--out` plus same-parent transactional staging, gate-target and recovery
directories, and never touches the network itself. A non-empty `--out` is replaced only when its
marker, exact file/directory inventory, modes, and file digests prove it is an unchanged cut of a
commit in the same source repository. Replacement validates a sibling candidate before
publication, verifies the published identity and contents before reporting success, and retains
one previous owned cut as a recovery backup; the next replacement removes that backup before
touching the live cut. These checks narrow same-UID races but do not create an indivisible
filesystem snapshot across separate syscalls.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

MARKER = ".oss-cut-marker"
CUT_DIRECTORY_MODE = 0o755
MARKER_MODE = 0o644
EXPECTED_GATE_RESIDUE = {
    "crates/goalcup-platform/target": 0o700,
    "crates/goalcup-platform/target/readcap-unix-tests": 0o700,
    "crates/goalcup-platform/target/tracecap-unit-tests": 0o700,
    "crates/goalcup-platform/target/unix-tracecap-native-tests": 0o700,
}
# `git -C <source>` does not override repository routing inherited through the environment. Strip
# every ambient Git control, including variables introduced by future Git releases, before the
# first subprocess. The one fixed variable below is the cutter's own replace-object policy.
def git_environment() -> dict[str, str]:
    return {
        name: value
        for name, value in os.environ.items()
        if not name.startswith("GIT_")
    }


GIT_ENV = git_environment()
# Replacement refs (including `git replace --graft`) can otherwise make a named commit expose a
# different tree. Legacy info/grafts and shallow metadata change ancestry only, not a commit's own
# tree; missing objects still make the captured tree/blob operations below fail closed.
GIT_ENV["GIT_NO_REPLACE_OBJECTS"] = "1"

# ---- the allowlist -------------------------------------------------------------------------
# (kind, value). kinds: exact path, prefix (directory), top (a directory's direct children with
# one suffix), regex (against the posix path).
ALLOW = [
    *[("exact", f) for f in (
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".gitignore", ".gitattributes",
        "LICENSE", "NOTICE", "README.md", "SECURITY.md", "CONTRIBUTING.md",
        "CODE_OF_CONDUCT.md", "AGENTS.md", "STATUS.md", "LEDGER.md",
        "harnesses/install.sh", "spikes/redact.py", "spikes/codex-app/make_fixtures.py",
        "spikes/m2-first-release/e02/early-load.py",
    )],
    ("prefix", ".github/"),
    ("prefix", ".claude/skills/"),
    ("prefix", "crates/"),
    ("prefix", "connectors/"),
    ("prefix", "skills/"),
    ("prefix", "scripts/"),
    ("prefix", "packaging/"),
    ("top", ("docs/", ".md")),
    ("prefix", "docs/qualification/"),
    ("prefix", "docs/research/"),
    ("prefix", "docs/harness-research/"),
    # Tests read these (daemon redaction walks every spikes/*/fixtures/*.jsonl; the verifier reads
    # the claude-code-cli, claude-desktop and codex-app fixtures).
    ("regex", r"^spikes/[^/]+/fixtures/"),
]

# Internal by D42: plans, handoffs, reviews, dogfood logs, spike reports; build output; the one
# non-workspace crate; local scratch.
DENY = [
    ("prefix", ".superpowers/"),
    ("prefix", "docs/superpowers/"),
    ("prefix", "docs/archive/"),
    ("prefix", "docs/dogfood/"),
    # Local compatibility alias for the canonical, already-included skills/ directory.
    ("exact", ".claude/skills/adding-a-capture-harness"),
    ("prefix", "crates/goalcup-git/"),
    ("prefix", "docs/qualification/2026-09-16-review-evidence/"),
    ("exact", "docs/qualification/2026-09-16-review.md"),
    ("exact", "docs/qualification/2026-09-17-journey-dogfood.md"),
    ("regex", r"(^|/)out/"),
    ("regex", r"(^|/)target/"),
    ("regex", r"(^|/)\.DS_Store$"),
    ("regex", r"(^|/)__pycache__/"),
]

REQUIRED = [
    ".gitattributes", "LICENSE", "NOTICE", "README.md", "SECURITY.md", "CONTRIBUTING.md",
    "CODE_OF_CONDUCT.md",
    "AGENTS.md", ".github/CODEOWNERS", ".github/PULL_REQUEST_TEMPLATE.md",
    ".github/workflows/ci.yml", "Cargo.toml", "Cargo.lock", "docs/CONTRACTS.md",
    "scripts/check-links.py",
]

# Areas where a home path is a defect rather than an accepted, hash-bound residual.
STRICT_PREFIXES = (
    ".github/", "skills/", "scripts/", "packaging/", "connectors/", ".claude/",
)
STRICT_FILES = {
    "README.md", "AGENTS.md", "CONTRIBUTING.md", "SECURITY.md", "CODE_OF_CONDUCT.md",
    "NOTICE", "STATUS.md", "LEDGER.md", "LICENSE", "Cargo.toml", ".gitattributes",
}


def matches(rule: tuple, path: str) -> bool:
    kind, val = rule
    if kind == "exact":
        return path == val
    if kind == "prefix":
        return path.startswith(val)
    if kind == "top":
        directory, suffix = val
        rest = path[len(directory):] if path.startswith(directory) else None
        return rest is not None and "/" not in rest and rest.endswith(suffix)
    if kind == "regex":
        return re.search(val, path) is not None
    raise ValueError(kind)


def is_strict(path: str) -> bool:
    if path in STRICT_FILES or path.startswith(STRICT_PREFIXES):
        return True
    # current top-level docs
    return path.startswith("docs/") and path.count("/") == 1 and path.endswith(".md")


def area(path: str) -> str:
    parts = path.split("/")
    if parts[0] in ("crates", "spikes", "docs") and len(parts) > 2:
        return "/".join(parts[:2]) if parts[0] != "crates" else "crates (tests/src)"
    return parts[0] if len(parts) > 1 else "(root)"


# ---- scan patterns -------------------------------------------------------------------------
USER_HOME_NAME = "/" + "Users/"
USER_HOME_BYTES = b"/" + b"Users/"
CLAUDE_TMP_NAME = "/private/tmp/" + "claude"
CLAUDE_TMP_BYTES = b"/private/tmp/" + b"claude"
HOME_PATTERNS = {
    # Generic account names and placeholders are excluded; any other account name is a person.
    USER_HOME_NAME: re.compile(USER_HOME_BYTES + rb"(?!you\b|Shared\b|username\b|name\b|<)"),
    CLAUDE_TMP_NAME: re.compile(CLAUDE_TMP_BYTES),
    "private names": re.compile(rb"[Ww]ork" + rb"trunk|coworker_" + rb"agent"),
}
EMAIL = re.compile(rb"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}")
# Deliberately small. Test placeholders (x@y.com, a@b.co, ...) are listed so that what remains
# is worth reading: every unmatched address is printed.
SAFE_EMAIL_DOMAINS = (
    "example.com", "example.org", "example.net", "example.co.uk", "example", "invalid", "test",
    "localhost", "users.noreply.github.com",
    "y.com", "y.co", "a.com", "b.co", "ex.com", "ex-ample.com", "y.bearer", "a.glpat",
    "example-model.org",
)
SAFE_EMAILS = {
    "noreply@anthropic.com", "noreply@github.com", "goalhack@agentcommunity.org",
    "goalcup@agentcommunity.org",
}
SECRET_PATTERNS = {
    "sk- key": re.compile(rb"\bsk-[A-Za-z0-9_-]{20,}"),
    "AWS key id": re.compile(rb"\bAKIA[0-9A-Z]{16}\b"),
    "GitHub token": re.compile(rb"\bgh[pousr]_[A-Za-z0-9]{30,}"),
    "Slack token": re.compile(rb"\bxox[baprs]-[A-Za-z0-9-]{10,}"),
    "private key block": re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    "JWT": re.compile(rb"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
    "Bearer token": re.compile(rb"Bearer [A-Za-z0-9._~+/-]{24,}"),
    "key=value secret": re.compile(
        rb"(?i)(?:api[_-]?key|secret|token|password)[\"']?\s*[:=]\s*[\"'][A-Za-z0-9/+_-]{20,}[\"']"
    ),
}


def git_tree(source: Path, revision: str) -> dict[str, tuple[str, str, str]]:
    raw = subprocess.run(
        ["git", "-C", str(source), "ls-tree", "-rz", "--full-tree", revision],
        check=True,
        capture_output=True,
        env=GIT_ENV,
    ).stdout
    entries: dict[str, tuple[str, str, str]] = {}
    for item in raw.split(b"\0"):
        if not item:
            continue
        metadata, encoded_path = item.split(b"\t", 1)
        mode, kind, object_id = metadata.decode("ascii").split()
        entries[encoded_path.decode("utf-8")] = (mode, kind, object_id)
    return entries


def is_commit(source: Path, object_id: str) -> bool:
    try:
        result = subprocess.run(
            ["git", "-C", str(source), "cat-file", "-t", object_id],
            capture_output=True,
            text=True,
            env=GIT_ENV,
        )
    except OSError:
        return False
    return result.returncode == 0 and result.stdout.strip() == "commit"


def source_snapshot(source: Path) -> tuple[str, dict[str, tuple[str, str, str]]] | None:
    try:
        top = subprocess.run(
            ["git", "-C", str(source), "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
            env=GIT_ENV,
        ).stdout.strip()
        head = subprocess.run(
            ["git", "-C", str(source), "rev-parse", "--verify", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
            env=GIT_ENV,
        ).stdout.strip()
        dirty = subprocess.run(
            ["git", "-C", str(source), "status", "--porcelain=v1", "--untracked-files=no"],
            check=True,
            capture_output=True,
            text=True,
            env=GIT_ENV,
        ).stdout
        entries = git_tree(source, head)
    except (OSError, subprocess.CalledProcessError, UnicodeError, ValueError) as error:
        print(f"refusing: cannot establish committed source identity: {error}", file=sys.stderr)
        return None
    if Path(top).resolve() != source:
        print("refusing: --source must name the repository root", file=sys.stderr)
        return None
    if dirty:
        print("refusing: source has staged or unstaged tracked changes", file=sys.stderr)
        return None
    return head, entries


def select(files: list[str]) -> tuple[list[str], collections.Counter]:
    kept: list[str] = []
    left_out: collections.Counter = collections.Counter()
    for f in files:
        if any(matches(r, f) for r in ALLOW) and not any(matches(r, f) for r in DENY):
            kept.append(f)
        else:
            left_out[f.split("/")[0] if "/" in f else "(root)"] += 1
    return kept, left_out


def regular_sources(
    source: Path, kept: list[str], entries: dict[str, tuple[str, str, str]]
) -> bool:
    for f in kept:
        src = source / f
        mode, kind, _ = entries[f]
        try:
            source_mode = os.lstat(src).st_mode
        except OSError as error:
            print(f"refusing: cannot inspect retained source {f}: {error}", file=sys.stderr)
            return False
        if kind != "blob" or mode not in ("100644", "100755") \
                or not stat.S_ISREG(source_mode):
            print(f"refusing: retained source is not a regular file: {f}", file=sys.stderr)
            return False
    return True


def copy_tree(
    source: Path,
    out: Path,
    kept: list[str],
    entries: dict[str, tuple[str, str, str]],
) -> None:
    for f in kept:
        dst = out / f
        dst.parent.mkdir(parents=True, exist_ok=True)
        mode, _, object_id = entries[f]
        with dst.open("xb") as destination:
            result = subprocess.run(
                ["git", "-C", str(source), "cat-file", "blob", object_id],
                stdout=destination,
                env=GIT_ENV,
            )
        if result.returncode != 0:
            raise OSError(f"git cat-file failed for {f} with status {result.returncode}")
        dst.chmod(0o755 if mode == "100755" else 0o644)


def set_directory_modes(out: Path, files: list[str]) -> None:
    for relative in cut_directories([*files, MARKER]):
        directory = out if relative == "." else out / relative
        directory.chmod(CUT_DIRECTORY_MODE)


def file_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def blob_digest(source: Path, object_id: str) -> str | None:
    digest = hashlib.sha256()
    process: subprocess.Popen | None = None
    try:
        process = subprocess.Popen(
            ["git", "-C", str(source), "cat-file", "blob", object_id],
            stdout=subprocess.PIPE,
            env=GIT_ENV,
        )
        if process.stdout is None:
            return None
        with process.stdout:
            while chunk := process.stdout.read(1024 * 1024):
                digest.update(chunk)
        if process.wait() != 0:
            return None
    except OSError:
        return None
    finally:
        if process is not None:
            if process.stdout is not None and not process.stdout.closed:
                process.stdout.close()
            if process.poll() is None:
                try:
                    process.kill()
                except OSError:
                    pass
                try:
                    process.wait()
                except OSError:
                    pass
    return digest.hexdigest()


def write_marker(
    out: Path,
    head: str,
    kept: list[str],
    entries: dict[str, tuple[str, str, str]],
) -> None:
    observed = cut_inventory(out)
    expected_files = set(kept)
    expected_directories = cut_directories([*kept, MARKER])
    if observed is None or set(observed) != expected_files | expected_directories:
        raise OSError("generated cut inventory is not closed")
    inventory: dict[str, dict[str, str | int]] = {}
    for relative in sorted(expected_directories):
        kind, mode = observed[relative]
        if kind != "directory" or mode != CUT_DIRECTORY_MODE:
            raise OSError(f"generated directory has wrong type or mode: {relative}")
        inventory[relative] = {"kind": kind}
    for relative in sorted(expected_files):
        kind, mode = observed[relative]
        if kind != "file":
            raise OSError(f"generated file has wrong type: {relative}")
        inventory[relative] = {
            "kind": kind,
            "mode": mode,
            "git_mode": entries[relative][0],
            "digest": file_digest(out / relative),
        }
    marker = {
        "format": 4,
        "source_head": head,
        "inventory": inventory,
    }
    marker_path = out / MARKER
    marker_path.write_text(json.dumps(marker, sort_keys=True) + "\n", encoding="utf-8")
    marker_path.chmod(MARKER_MODE)
    complete = cut_inventory(out)
    if complete is None or set(complete) != expected_files | expected_directories | {MARKER}:
        raise OSError("generated cut inventory is not closed after marker creation")
    if complete[MARKER] != ("file", MARKER_MODE):
        raise OSError("generated marker has wrong type or mode")


def cut_directories(files: list[str]) -> set[str]:
    directories = {"."}
    for relative in files:
        parts = relative.split("/")[:-1]
        directories.update("/".join(parts[:end]) for end in range(1, len(parts) + 1))
    return directories


def cut_inventory(out: Path) -> dict[str, tuple[str, int]] | None:
    found: dict[str, tuple[str, int]] = {}

    def visit(path: Path, relative: str) -> bool:
        mode = os.lstat(path).st_mode
        if not stat.S_ISDIR(mode):
            return False
        found[relative] = ("directory", stat.S_IMODE(mode))
        with os.scandir(path) as iterator:
            children = list(iterator)
        for child in children:
            child_path = path / child.name
            child_relative = child.name if relative == "." else f"{relative}/{child.name}"
            child_mode = os.lstat(child_path).st_mode
            if stat.S_ISDIR(child_mode):
                if not visit(child_path, child_relative):
                    return False
            elif stat.S_ISREG(child_mode):
                found[child_relative] = ("file", stat.S_IMODE(child_mode))
            else:
                return False
        return True

    try:
        if not visit(out, "."):
            return None
    except OSError:
        return None
    return found


def directory_identity(path: Path) -> tuple[int, int] | None:
    try:
        metadata = os.lstat(path)
    except OSError:
        return None
    if not stat.S_ISDIR(metadata.st_mode):
        return None
    return metadata.st_dev, metadata.st_ino


def open_directory_bound(path: Path, expected_identity: tuple[int, int]) -> int | None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_CLOEXEC", 0)
    descriptor: int | None = None
    try:
        descriptor = os.open(path, flags)
        metadata = os.fstat(descriptor)
    except OSError:
        if descriptor is not None:
            os.close(descriptor)
        return None
    if not stat.S_ISDIR(metadata.st_mode) \
            or (metadata.st_dev, metadata.st_ino) != expected_identity:
        os.close(descriptor)
        return None
    return descriptor


def remove_directory_contents(descriptor: int) -> None:
    with os.scandir(descriptor) as iterator:
        children = list(iterator)
    directory_flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) \
        | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    for child in children:
        child_metadata = os.stat(child.name, dir_fd=descriptor, follow_symlinks=False)
        if stat.S_ISDIR(child_metadata.st_mode):
            child_descriptor = os.open(child.name, directory_flags, dir_fd=descriptor)
            try:
                opened_metadata = os.fstat(child_descriptor)
                if (opened_metadata.st_dev, opened_metadata.st_ino) != (
                    child_metadata.st_dev,
                    child_metadata.st_ino,
                ):
                    raise OSError(f"directory identity changed during cleanup: {child.name}")
                remove_directory_contents(child_descriptor)
            finally:
                os.close(child_descriptor)
            os.rmdir(child.name, dir_fd=descriptor)
        else:
            os.unlink(child.name, dir_fd=descriptor)


def remove_tree_if_identity(
    path: Path, expected_identity: tuple[int, int], description: str
) -> bool:
    if directory_identity(path) != expected_identity:
        print(
            f"{description} cleanup refused: directory identity changed; preserved {path}",
            file=sys.stderr,
        )
        return False
    descriptor = open_directory_bound(path, expected_identity)
    if descriptor is None:
        print(
            f"{description} cleanup refused: directory could not be opened with its bound identity; "
            f"preserved {path}",
            file=sys.stderr,
        )
        return False
    try:
        remove_directory_contents(descriptor)
    except OSError as error:
        print(f"{description} cleanup failed; preserved {path}: {error}", file=sys.stderr)
        return False
    finally:
        os.close(descriptor)
    if directory_identity(path) != expected_identity:
        print(
            f"{description} cleanup could not remove the bound root; preserved residue {path}",
            file=sys.stderr,
        )
        return False
    try:
        path.rmdir()
    except OSError as error:
        print(f"{description} cleanup failed; preserved {path}: {error}", file=sys.stderr)
        return False
    return True


def cleanup_staging(
    candidate: Path,
    candidate_identity: tuple[int, int],
    staging_parent: Path,
    staging_parent_identity: tuple[int, int],
) -> bool:
    if os.path.lexists(candidate):
        if not remove_tree_if_identity(candidate, candidate_identity, "candidate"):
            print(f"staging parent preserved {staging_parent}", file=sys.stderr)
            return False
    if directory_identity(staging_parent) != staging_parent_identity:
        print(
            f"staging cleanup refused: parent identity changed; preserved {staging_parent}",
            file=sys.stderr,
        )
        return False
    try:
        staging_parent.rmdir()
    except OSError as error:
        print(f"staging cleanup failed; preserved {staging_parent}: {error}", file=sys.stderr)
        return False
    return True


def is_owned_cut(
    out: Path,
    source: Path,
    allowed_extra_directories: dict[str, int] | None = None,
) -> bool:
    marker_path = out / MARKER
    try:
        marker_mode = os.lstat(marker_path).st_mode
        if not stat.S_ISREG(marker_mode) or stat.S_IMODE(marker_mode) != MARKER_MODE:
            return False
        marker_bytes = marker_path.read_bytes()
        marker = json.loads(marker_bytes)
    except (OSError, UnicodeError, json.JSONDecodeError):
        return False
    if marker_bytes != (json.dumps(marker, sort_keys=True) + "\n").encode("utf-8"):
        return False
    if not isinstance(marker, dict) or set(marker) != {"format", "source_head", "inventory"}:
        return False
    head = marker["source_head"]
    recorded = marker["inventory"]
    if marker["format"] != 4:
        return False
    if not isinstance(head, str) or re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", head) is None:
        return False
    if not isinstance(recorded, dict):
        return False
    if not is_commit(source, head):
        return False
    try:
        entries = git_tree(source, head)
    except (OSError, subprocess.CalledProcessError, UnicodeError, ValueError):
        return False
    expected, _ = select(sorted(entries))
    expected_files = set(expected)
    expected_directories = cut_directories([*expected, MARKER])
    observed = cut_inventory(out)
    if set(recorded) != expected_files | expected_directories:
        return False
    if observed is None:
        if allowed_extra_directories is not None:
            print("gate test residue is not exact", file=sys.stderr)
        return False
    expected_base = expected_files | expected_directories | {MARKER}
    extras = set(observed) - expected_base
    allowed_extras = allowed_extra_directories or {}
    if set(observed) - extras != expected_base:
        return False
    if extras and extras != set(allowed_extras):
        if allowed_extra_directories is not None:
            print("gate test residue is not exact", file=sys.stderr)
        return False
    for relative in extras:
        if observed[relative] != ("directory", allowed_extras[relative]):
            if allowed_extra_directories is not None:
                print("gate test residue is not exact", file=sys.stderr)
            return False
    if observed[MARKER] != ("file", MARKER_MODE):
        return False
    for relative in expected_directories:
        record = recorded.get(relative)
        if not isinstance(record, dict) or set(record) != {"kind"}:
            return False
        if record["kind"] != "directory" \
                or observed[relative] != ("directory", CUT_DIRECTORY_MODE):
            return False
    for relative in expected:
        record = recorded.get(relative)
        mode, kind, object_id = entries[relative]
        if kind != "blob" or mode not in ("100644", "100755"):
            return False
        if not isinstance(record, dict) \
                or set(record) != {"kind", "mode", "git_mode", "digest"}:
            return False
        digest = record["digest"]
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            return False
        if record["kind"] != "file" or record["git_mode"] != mode:
            return False
        output_mode = observed[relative][1]
        if record["mode"] != output_mode \
                or output_mode != (0o755 if mode == "100755" else 0o644):
            return False
        if file_digest(out / relative) != digest or blob_digest(source, object_id) != digest:
            return False
    return True


def checker_is_authoritative(out: Path) -> bool:
    trusted = Path(__file__).resolve().with_name("check-links.py")
    candidate = out / "scripts/check-links.py"
    try:
        if not stat.S_ISREG(os.lstat(trusted).st_mode):
            return False
        if not stat.S_ISREG(os.lstat(candidate).st_mode):
            return False
        return file_digest(candidate) == file_digest(trusted)
    except OSError:
        return False


def is_text(data: bytes) -> bool:
    return b"\0" not in data[:8192]


def scan(out: Path, kept: list[str]) -> dict:
    home = {k: collections.Counter() for k in HOME_PATTERNS}
    home_files: dict[str, set] = {k: set() for k in HOME_PATTERNS}
    strict_hits: list[tuple[str, str, int]] = []
    emails: collections.Counter = collections.Counter()
    secrets = {k: [0, set()] for k in SECRET_PATTERNS}
    for f in kept:
        p = out / f
        if p.is_symlink() or not p.is_file():
            continue
        data = p.read_bytes()
        for name, pat in HOME_PATTERNS.items():
            n = len(pat.findall(data))
            if n:
                home[name][area(f)] += n
                home_files[name].add(f)
                if is_strict(f) and name != "private names":
                    strict_hits.append((name, f, n))
        if not is_text(data):
            continue
        for m in EMAIL.findall(data):
            addr = m.decode("utf-8", "replace")
            lower = addr.lower()
            domain = lower.rsplit("@", 1)[1]
            user = lower.rsplit("@", 1)[0]
            if lower in SAFE_EMAILS or user == "git" or "-----" in user:
                continue  # `-----` is the tail of a PEM header glued to a test string
            if any(domain == d or domain.endswith("." + d) for d in SAFE_EMAIL_DOMAINS):
                continue
            emails[addr] += 1
        for name, pat in SECRET_PATTERNS.items():
            n = len(pat.findall(data))
            if n:
                secrets[name][0] += n
                secrets[name][1].add(f)
    return dict(home=home, home_files=home_files, strict_hits=strict_hits, emails=emails,
                secrets=secrets)


def run_link_checker(out: Path) -> int:
    try:
        return subprocess.run(
            [
                sys.executable,
                "-I",
                "-S",
                str(out / "scripts/check-links.py"),
                "--root",
                str(out),
            ],
            cwd=out,
        ).returncode
    except OSError as error:
        print(f"link checker execution failed: {error}", file=sys.stderr)
        return 1


def run_gate(out: Path, mode: str, cargo_args: list[str], source: Path) -> int:
    ca = cargo_args
    if mode == "smoke":
        steps = [["cargo", "build", *ca], ["cargo", "test", *ca, "-p", "goalcup-protocol"]]
    else:
        steps = [
            ["cargo", "build", *ca],
            ["cargo", "fmt", "--all", "--check"],
            ["cargo", "clippy", "--workspace", "--all-targets", *ca, "--", "-D", "warnings"],
            ["cargo", "test", "--workspace", *ca],
        ]
    candidate_identity = directory_identity(out)
    if candidate_identity is None or not owned_cut_is_bound(
        out, candidate_identity, source, "candidate", "before gate"
    ):
        return 1
    gate_parent = Path(
        tempfile.mkdtemp(prefix=f".{out.name}.oss-cut-gate-", dir=out.parent)
    )
    gate_parent_identity = directory_identity(gate_parent)
    if gate_parent_identity is None:
        print(f"gate workspace identity could not be bound: {gate_parent}", file=sys.stderr)
        return 1
    gate_tree = gate_parent / "tree"
    cargo_target = gate_parent / "target"
    gate_tree_identity: tuple[int, int] | None = None
    cargo_target_identity: tuple[int, int] | None = None
    gate_tree_descriptor: int | None = None
    previous_cwd_descriptor: int | None = None
    cwd_restored = True
    gate_status = 0
    try:
        gate_tree.mkdir(mode=0o700)
        gate_tree_identity = directory_identity(gate_tree)
        if gate_tree_identity is None:
            print(f"gate copy identity could not be bound: {gate_tree}", file=sys.stderr)
            gate_status = 1
            return gate_status
        gate_tree_descriptor = open_directory_bound(gate_tree, gate_tree_identity)
        if gate_tree_descriptor is None:
            print(f"gate copy could not be opened with its bound identity: {gate_tree}", file=sys.stderr)
            gate_status = 1
            return gate_status
        cwd_flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_CLOEXEC", 0)
        previous_cwd_descriptor = os.open(".", cwd_flags)
        os.fchdir(gate_tree_descriptor)
        cwd_restored = False
        # The cutter is single-threaded. Keeping its cwd on the opened directory makes both the
        # copy and gate children use the bound inode even if its original pathname is replaced.
        shutil.copytree(out, Path("."), dirs_exist_ok=True)
        if directory_identity(gate_tree) != gate_tree_identity:
            print("gate copy identity changed during copy", file=sys.stderr)
            gate_status = 1
            return gate_status
        if not owned_cut_is_bound(
            gate_tree, gate_tree_identity, source, "gate copy", "before gate"
        ):
            print("gate copy is not an exact owned cut", file=sys.stderr)
            gate_status = 1
            return gate_status
        cargo_target.mkdir(mode=0o700)
        cargo_target_identity = directory_identity(cargo_target)
        if cargo_target_identity is None:
            print(f"gate target identity could not be bound: {cargo_target}", file=sys.stderr)
            gate_status = 1
            return gate_status
        gate_env = GIT_ENV.copy()
        gate_env["GIT_CEILING_DIRECTORIES"] = str(gate_parent)
        gate_env["CARGO_TARGET_DIR"] = str(cargo_target)
        for step in steps:
            print(f"\n$ {' '.join(step)}", flush=True)
            rc = subprocess.run(step, env=gate_env, umask=0o077).returncode
            print(f"exit {rc}", flush=True)
            if rc != 0:
                gate_status = rc
                break
    except OSError as error:
        print(f"gate execution failed: {error}", file=sys.stderr)
        gate_status = 1
    finally:
        if previous_cwd_descriptor is not None:
            try:
                os.fchdir(previous_cwd_descriptor)
                cwd_restored = True
            except OSError as error:
                print(f"gate cwd restoration failed; workspace preserved: {error}", file=sys.stderr)
                gate_status = gate_status or 1
            finally:
                os.close(previous_cwd_descriptor)
        if gate_tree_descriptor is not None:
            os.close(gate_tree_descriptor)
        if not cwd_restored:
            gate_status = gate_status or 1
        else:
            gate_copy_verified = gate_tree_identity is not None and owned_cut_is_bound(
                gate_tree,
                gate_tree_identity,
                source,
                "gate copy",
                "after gate",
                allowed_extra_directories=EXPECTED_GATE_RESIDUE,
            )
            if not gate_copy_verified:
                gate_status = gate_status or 1
                print(
                    f"gate workspace preserved after failed verification: {gate_parent}",
                    file=sys.stderr,
                )
            else:
                if os.path.lexists(cargo_target):
                    if cargo_target_identity is None or not remove_tree_if_identity(
                        cargo_target, cargo_target_identity, "gate target"
                    ):
                        gate_status = gate_status or 1
                if os.path.lexists(gate_tree):
                    if not remove_tree_if_identity(gate_tree, gate_tree_identity, "gate copy"):
                        gate_status = gate_status or 1
                if directory_identity(gate_parent) != gate_parent_identity:
                    print(
                        f"gate workspace cleanup refused: parent identity changed; preserved {gate_parent}",
                        file=sys.stderr,
                    )
                    gate_status = gate_status or 1
                else:
                    try:
                        gate_parent.rmdir()
                    except OSError as error:
                        print(
                            f"gate workspace cleanup failed; preserved {gate_parent}: {error}",
                            file=sys.stderr,
                        )
                        gate_status = gate_status or 1
        if not owned_cut_is_bound(
            out, candidate_identity, source, "candidate", "after gate"
        ):
            gate_status = gate_status or 1
    return gate_status


def validate_candidate(
    out: Path,
    display_out: Path,
    source: Path,
    files: list[str],
    kept: list[str],
    left_out: collections.Counter,
    gate: str,
    cargo_args: list[str],
) -> int:
    print(f"source   {source}")
    print(f"cut tree {display_out}")
    print(f"tracked files {len(files)}; copied {len(kept)}; left out {len(files) - len(kept)}")
    print(
        "left out by top-level path: "
        + ", ".join(f"{k} {v}" for k, v in sorted(left_out.items()))
    )
    by_top: collections.Counter = collections.Counter(
        (f.split("/")[0] if "/" in f else "(root)") for f in kept
    )
    print(
        "copied by top-level path:   "
        + ", ".join(f"{k} {v}" for k, v in sorted(by_top.items()))
    )

    status = 0
    missing = [required for required in REQUIRED if not (out / required).exists()]
    print(f"\nrequired files missing: {len(missing)}" + (f" {missing}" if missing else ""))
    if missing:
        status = 3

    res = scan(out, kept)
    print("\n== home paths and private names (count of matches; files)")
    for name in HOME_PATTERNS:
        total = sum(res["home"][name].values())
        print(f"  {name}: {total} matches in {len(res['home_files'][name])} files")
        for area_name, count in res["home"][name].most_common(8):
            print(f"      {area_name}: {count}")
    print(f"  STRICT areas with a home-path hit: {len(res['strict_hits'])}")
    for name, relative, count in res["strict_hits"][:20]:
        print(f"      {name}  {relative} ({count})")
    if res["strict_hits"]:
        status = status or 2

    print("\n== emails outside the safe list")
    print(f"  distinct: {len(res['emails'])}, occurrences: {sum(res['emails'].values())}")
    for address, count in res["emails"].most_common(15):
        print(f"      {address}  x{count}")
    if res["emails"]:
        status = status or 2

    print("\n== secret-shaped strings (counts only; known synthetic in redaction tests and fixtures)")
    total_secret_files: set = set()
    for name, (count, matching_files) in res["secrets"].items():
        total_secret_files |= matching_files
        print(f"  {name}: {count} matches in {len(matching_files)} files")
    print(f"  files with any hit: {len(total_secret_files)}")

    if checker_is_authoritative(out):
        link_status = run_link_checker(out)
        if not checker_is_authoritative(out):
            print("canonical link checker changed during execution", file=sys.stderr)
            link_status = link_status or 1
    else:
        print("canonical link checker is missing or not authoritative", file=sys.stderr)
        link_status = 1
    print(f"\nlink checker: {'PASS' if link_status == 0 else 'FAIL'}")
    if link_status != 0:
        status = status or 2

    if gate != "none":
        rc = run_gate(out, gate, cargo_args, source)
        print(f"\ngate ({gate}): {'PASS' if rc == 0 else 'FAIL'}")
        if rc != 0:
            status = status or 1

    if not is_owned_cut(out, source):
        print("candidate changed or became unverifiable during validation", file=sys.stderr)
        status = status or 1
    return status


def owned_cut_is_bound(
    path: Path,
    expected_identity: tuple[int, int],
    source: Path,
    description: str,
    phase: str,
    allowed_extra_directories: dict[str, int] | None = None,
) -> bool:
    if directory_identity(path) != expected_identity:
        print(f"{description} identity changed {phase}", file=sys.stderr)
        return False
    if not is_owned_cut(path, source, allowed_extra_directories):
        print(f"{description} changed or became unverifiable {phase}", file=sys.stderr)
        return False
    if directory_identity(path) != expected_identity:
        print(f"{description} identity changed during {phase} verification", file=sys.stderr)
        return False
    return True


def candidate_is_bound(
    candidate: Path, expected_identity: tuple[int, int], source: Path
) -> bool:
    return owned_cut_is_bound(
        candidate, expected_identity, source, "candidate", "before publication"
    )


def quarantine_published_candidate(
    out: Path, candidate: Path, candidate_identity: tuple[int, int]
) -> bool:
    quarantine = candidate.parent / "invalid-published"
    if os.path.lexists(quarantine):
        print(
            f"invalid publication quarantine already exists; preserved live path {out}",
            file=sys.stderr,
        )
        return False
    if directory_identity(out) != candidate_identity:
        print(
            f"invalid publication identity changed; preserved live path {out}",
            file=sys.stderr,
        )
        return False
    try:
        os.rename(out, quarantine)
    except OSError as error:
        print(f"invalid publication could not be quarantined: {error}", file=sys.stderr)
        return False
    if directory_identity(quarantine) != candidate_identity:
        print(
            f"invalid publication quarantine identity changed; preserved {quarantine}",
            file=sys.stderr,
        )
        return False
    print(f"invalid publication preserved for inspection {quarantine}", file=sys.stderr)
    return True


def restore_backup(
    backup: Path,
    backup_identity: tuple[int, int],
    out: Path,
    source: Path,
    failure: str,
) -> bool:
    if not owned_cut_is_bound(
        backup, backup_identity, source, "recovery backup", "before rollback"
    ):
        print(f"{failure}; recovery backup preserved {backup}", file=sys.stderr)
        return False
    try:
        os.rename(backup, out)
    except OSError as rollback_error:
        print(f"{failure}; rollback failed: {rollback_error}", file=sys.stderr)
        return False
    if not owned_cut_is_bound(
        out, backup_identity, source, "restored cut", "after rollback"
    ):
        print(f"{failure}; rollback result is unverifiable at {out}", file=sys.stderr)
        return False
    print(f"{failure}; old cut restored", file=sys.stderr)
    return True


def publish_candidate(
    candidate: Path,
    candidate_identity: tuple[int, int],
    out: Path,
    replacing: bool,
    source: Path,
) -> bool:
    if not candidate_is_bound(candidate, candidate_identity, source):
        return False
    if not replacing:
        try:
            if out.exists():
                out.rmdir()
            if not candidate_is_bound(candidate, candidate_identity, source):
                return False
            os.rename(candidate, out)
            if not owned_cut_is_bound(
                out, candidate_identity, source, "published cut", "after publication"
            ):
                quarantine_published_candidate(out, candidate, candidate_identity)
                return False
            return True
        except OSError as error:
            print(f"cut publication failed: {error}", file=sys.stderr)
            return False

    backup = out.parent / f".{out.name}.oss-cut-backup"
    if os.path.lexists(backup):
        backup_identity = directory_identity(backup)
        if backup_identity is None or not is_owned_cut(backup, source) \
                or directory_identity(backup) != backup_identity:
            print(f"refusing: prior recovery backup is not an unchanged owned cut: {backup}", file=sys.stderr)
            return False
        if not remove_tree_if_identity(backup, backup_identity, "prior recovery backup"):
            return False

    old_identity = directory_identity(out)
    if old_identity is None or not owned_cut_is_bound(
        out, old_identity, source, "live cut", "before backup"
    ):
        return False
    if not candidate_is_bound(candidate, candidate_identity, source):
        return False
    try:
        os.rename(out, backup)
    except OSError as error:
        print(f"cut publication could not retain the old cut: {error}", file=sys.stderr)
        return False
    if not owned_cut_is_bound(
        backup, old_identity, source, "recovery backup", "after backup rename"
    ):
        print(f"old cut backup became unverifiable; preserved {backup}", file=sys.stderr)
        return False
    if not candidate_is_bound(candidate, candidate_identity, source):
        restore_backup(
            backup, old_identity, out, source, "candidate changed before publication"
        )
        return False
    try:
        os.rename(candidate, out)
    except OSError as error:
        restore_backup(
            backup, old_identity, out, source, f"cut publication failed: {error}"
        )
        return False
    if not owned_cut_is_bound(
        out, candidate_identity, source, "published cut", "after publication"
    ):
        if quarantine_published_candidate(out, candidate, candidate_identity):
            restore_backup(
                backup,
                old_identity,
                out,
                source,
                "published cut failed post-publication verification",
            )
        else:
            print(f"old cut remains available as recovery backup {backup}", file=sys.stderr)
        return False
    if not owned_cut_is_bound(
        backup, old_identity, source, "recovery backup", "before success"
    ):
        print(f"published cut retained but recovery backup is unverifiable {backup}", file=sys.stderr)
        return False
    print(f"recovery backup retained {backup}")
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--out", required=True, help="directory to create (must not exist, or be empty)")
    ap.add_argument("--source", default=None, help="source checkout (default: this script's repo)")
    ap.add_argument("--gate", choices=("none", "smoke", "full"), default="none")
    ap.add_argument("--cargo-arg", action="append", default=[], help="extra cargo flag, repeatable")
    ap.add_argument("--replace", action="store_true",
                    help="re-cut into an --out that carries this script's marker file")
    args = ap.parse_args()

    source = Path(args.source).resolve() if args.source else Path(__file__).resolve().parent.parent
    out = Path(args.out).resolve()
    if out == source or source in out.parents or out in source.parents:
        print("refusing: --out and --source must be separate trees", file=sys.stderr)
        return 1
    snapshot = source_snapshot(source)
    if snapshot is None:
        return 1
    head, entries = snapshot
    files = sorted(entries)
    kept, left_out = select(files)
    if not regular_sources(source, kept, entries):
        return 1
    replacing = False
    if out.exists() and any(out.iterdir()):
        if args.replace and is_owned_cut(out, source):
            replacing = True
        else:
            print(
                f"refusing: {out} is not empty or is not an unchanged owned cut",
                file=sys.stderr,
            )
            return 1
    out.parent.mkdir(parents=True, exist_ok=True)
    staging_parent = Path(
        tempfile.mkdtemp(prefix=f".{out.name}.oss-cut-stage-", dir=out.parent)
    )
    staging_parent_identity = directory_identity(staging_parent)
    if staging_parent_identity is None:
        print(f"cannot bind staging parent identity: {staging_parent}", file=sys.stderr)
        return 1
    candidate = staging_parent / "candidate"
    try:
        candidate.mkdir(mode=0o700)
    except OSError as error:
        print(f"cannot create private candidate directory: {error}", file=sys.stderr)
        if directory_identity(staging_parent) == staging_parent_identity:
            try:
                staging_parent.rmdir()
            except OSError as cleanup_error:
                print(
                    f"staging parent preserved {staging_parent}: {cleanup_error}",
                    file=sys.stderr,
                )
        return 1
    candidate_identity = directory_identity(candidate)
    if candidate_identity is None:
        print(f"cannot bind candidate identity: {candidate}", file=sys.stderr)
        cleanup_staging(candidate, (-1, -1), staging_parent, staging_parent_identity)
        return 1
    try:
        copy_tree(source, candidate, kept, entries)
        set_directory_modes(candidate, kept)
        write_marker(candidate, head, kept, entries)
    except OSError as error:
        print(f"cut copy failed: {error}", file=sys.stderr)
        cleanup_staging(
            candidate, candidate_identity, staging_parent, staging_parent_identity
        )
        return 1
    status = validate_candidate(
        candidate,
        out,
        source,
        files,
        kept,
        left_out,
        args.gate,
        args.cargo_arg,
    )
    if status != 0:
        cleanup_staging(
            candidate, candidate_identity, staging_parent, staging_parent_identity
        )
        print(f"\nresult status {status}")
        return status
    if not publish_candidate(candidate, candidate_identity, out, replacing, source):
        cleanup_staging(
            candidate, candidate_identity, staging_parent, staging_parent_identity
        )
        print("\nresult status 1")
        return 1
    cleanup_staging(candidate, candidate_identity, staging_parent, staging_parent_identity)
    print("\nresult status 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
