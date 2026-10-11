#!/usr/bin/env python3
"""Decide whether a CI run needs the Rust smoke checks.

    python3 scripts/ci-scope.py --base <SHA> --head <SHA>

Prints exactly `rust=true` or `rust=false` on stdout and exits 0, so the workflow can append the
line to `$GITHUB_OUTPUT`. A change needs no Rust checks only when every changed path is
documentation: Markdown under `docs/`, Markdown at the repository root, or
`.github/PULL_REQUEST_TEMPLATE.md`. Every other path, including ones added later, runs them.

The comparison is `git diff --no-renames --name-only -z BASE HEAD --` in the current directory.
`--no-renames` lists both sides of a rename, so moving code into `docs/` still runs Rust. Any
comparison that cannot be trusted runs Rust: an empty, missing or all-zero SHA (a new branch or a
manual start), a Git failure, or output that is not NUL-terminated UTF-8 paths. An empty
successful diff needs no Rust checks.

Stdlib only. No network, and nothing is written to the repository.
"""

from __future__ import annotations

import argparse
import subprocess
import sys

DOCS_ONLY_FILES = frozenset({".github/PULL_REQUEST_TEMPLATE.md"})


def is_docs_only(path: str) -> bool:
    if path in DOCS_ONLY_FILES:
        return True
    if not path.endswith(".md"):
        return False
    return "/" not in path or path.startswith("docs/")


def requires_rust(paths: list[str]) -> bool:
    return not all(is_docs_only(path) for path in paths)


def usable_sha(value: str) -> bool:
    return bool(value) and not value.startswith("-") and value.strip("0") != ""


def changed_paths(base: str, head: str) -> list[str] | None:
    """The paths changed between two commits, or None when the comparison cannot be trusted."""
    if not (usable_sha(base) and usable_sha(head)):
        return None
    try:
        raw = subprocess.check_output(
            ["git", "diff", "--no-renames", "--name-only", "-z", base, head, "--"],
            stderr=subprocess.DEVNULL,
            timeout=120,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if not isinstance(raw, bytes):
        return None
    if raw == b"":
        return []
    if not raw.endswith(b"\0"):
        return None
    paths = []
    for item in raw[:-1].split(b"\0"):
        if not item:
            return None
        try:
            paths.append(item.decode("utf-8"))
        except UnicodeDecodeError:
            return None
    return paths


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="", help="comparison base commit")
    parser.add_argument("--head", default="", help="comparison head commit")
    args = parser.parse_args(argv)
    paths = changed_paths(args.base, args.head)
    if paths is None:
        print("ci-scope: comparison unavailable; running Rust checks", file=sys.stderr)
        rust = True
    else:
        rust = requires_rust(paths)
    print(f"rust={'true' if rust else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
