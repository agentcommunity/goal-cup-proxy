#!/usr/bin/env python3
"""Fail when the tracer's version is written anywhere but the workspace Cargo.toml.

    python3 scripts/check-version.py [--root DIR]

The workspace `version` in Cargo.toml is the only place a release version is written (see
scripts/release.sh). Every crate inherits it, the binaries and the report fields (`tracer_version`,
`daemon_version`, `verifier_version`) read it at build time, and docs say "the latest release".
This check reads the tracked files of a git checkout and reports:

  1. the workspace version is not a plain x.y.z;
  2. a crate manifest does not say `version.workspace = true`, or carries its own version;
  3. Cargo.lock holds a goalcup-* package at another version than the workspace's;
  4. the current version is written, as a whole token, in a doc, script, workflow, packaging file
     or non-test source;
  5. a release tag such as v1.2.3 is written in a doc, script, workflow or packaging file (any
     version, not only the current one), except the entries of ALLOWED below;
  6. a doc or source gives `tracer_version`, `daemon_version` or `verifier_version`, or the output
     of `goalcup --version`, a literal version.

Test code and fixtures are not scanned: they send made-up versions to fake servers. Measurement
records (docs/harnesses/, docs/research/, connectors/, spikes/) carry the versions of the
harnesses, not of the tracer, and are not scanned either.

Exit status 0 when clean, 1 with one line per finding otherwise.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

# The only tag-shaped versions that may stay written: the frozen v5 wire format took effect at
# the existing v0.5.0 tag, and these two documents say so. Keep this list tiny and explicit; it
# maps a path to the exact text allowed there.
ALLOWED: dict[str, tuple[str, ...]] = {
    "docs/CONTRACTS.md": ("v0.5.0",),
    "docs/DECISIONS.md": ("v0.5.0",),
}

# Scanned for rules 4 to 6: documentation, packaging, automation and non-test source.
SCANNED_PREFIXES = (
    "docs/",
    "skills/",
    "packaging/",
    "scripts/",
    ".github/",
    "crates/",
    "server/worker/src/",
)
SCANNED_FILES = {
    "README.md",
    "AGENTS.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "server/worker/README.md",
    "server/worker/wrangler.toml",
}
# Source files may name the v0.5.0 tag in a comment or a message (the old daemon that finishes a V1
# entry); only the current version (rule 4) and version fields (rule 6) are checked there.
SOURCE_PREFIXES = ("crates/", "server/worker/src/")
# Excluded from the scan even under a scanned prefix.
EXCLUDED_PREFIXES = ("docs/harnesses/", "docs/research/")
EXCLUDED_PARTS = ("/tests/", "/fixtures/")
# This script and its test must name versions to look for them.
EXCLUDED_FILES = {"scripts/check-version.py", "scripts/test_check_version.py"}
TEXT_SUFFIXES = {".md", ".rs", ".py", ".sh", ".toml", ".yml", ".yaml", ".ts", ".rb", ".txt"}

SEMVER = r"\d+\.\d+\.\d+"
# A made-up marker such as 0.0.0-mock is not a release version.
END = r"(?![0-9A-Za-z\-])"
TAG = re.compile(rf"(?<![A-Za-z0-9_.\-]){'v'}{SEMVER}(?![0-9]|\.[0-9])")
# `tracer_version": "1.2.3"`, `tracer_version: "1.2.3"`, `daemon_version = 1.2.3`.
VERSION_FIELD = re.compile(rf"\b(?:tracer|daemon|verifier)_version\W{{1,6}}{SEMVER}{END}")
# `goalcup 1.2.3`, `goalcup-daemon 1.2.3`, `goalcup-verify 1.2.3` (the `--version` output).
BINARY_VERSION = re.compile(rf"\bgoalcup(?:-daemon|-verify)? {SEMVER}{END}")
# A GitHub Action pinned by digest carries its own version in a comment.
ACTION_PIN = re.compile(r"uses: [\w./-]+@[0-9a-f]{40}")


def tracked(root: Path) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True
    ).stdout
    return [name for name in out.decode().split("\0") if name]


def workspace_version(root: Path) -> str | None:
    text = (root / "Cargo.toml").read_text()
    section = re.search(r"^\[workspace\.package\]\n(.*?)(?=^\[|\Z)", text, re.S | re.M)
    if not section:
        return None
    found = re.search(r'^version = "([^"]*)"$', section.group(1), re.M)
    return found.group(1) if found else None


def is_scanned(name: str) -> bool:
    if name in EXCLUDED_FILES or name.startswith(EXCLUDED_PREFIXES):
        return False
    if any(part in "/" + name for part in EXCLUDED_PARTS):
        return False
    if Path(name).suffix not in TEXT_SUFFIXES:
        return False
    return name in SCANNED_FILES or name.startswith(SCANNED_PREFIXES)


def check(root: Path) -> list[str]:
    findings: list[str] = []
    version = workspace_version(root)
    if version is None or not re.fullmatch(SEMVER, version):
        findings.append(f"Cargo.toml: [workspace.package] version {version!r} is not x.y.z")
        return findings
    files = tracked(root)

    for name in files:
        if re.fullmatch(r"crates/[^/]+/Cargo\.toml", name):
            lines = (root / name).read_text().splitlines()
            if "version.workspace = true" not in lines:
                findings.append(f"{name}: must say `version.workspace = true`")
            if any(line.startswith("version = ") for line in lines):
                findings.append(f"{name}: carries a version of its own")

    lock = root / "Cargo.lock"
    if lock.is_file():
        for package, found in re.findall(
            r'name = "(goalcup-[^"]+)"\nversion = "([^"]+)"', lock.read_text()
        ):
            if found != version:
                findings.append(f"Cargo.lock: {package} is {found}, the workspace is {version}")

    current = re.compile(rf"(?<![0-9.\-]){re.escape(version)}(?![0-9]|\.[0-9])")
    for name in files:
        if not is_scanned(name) or not (root / name).is_file():
            continue
        allowed = ALLOWED.get(name, ())
        for number, line in enumerate((root / name).read_text(errors="replace").splitlines(), 1):
            for text in allowed:
                line = line.replace(text, "")
            if ACTION_PIN.search(line):
                continue
            where = f"{name}:{number}"
            if current.search(line):
                findings.append(f"{where}: writes the current version {version}")
            for tag in [] if name.startswith(SOURCE_PREFIXES) else TAG.findall(line):
                findings.append(f"{where}: writes the release tag {tag}")
            for hit in VERSION_FIELD.findall(line) + BINARY_VERSION.findall(line):
                findings.append(f"{where}: writes a tracer version ({hit})")
    return sorted(set(findings))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent))
    args = parser.parse_args()
    findings = check(Path(args.root))
    for finding in findings:
        print(f"check-version: {finding}", file=sys.stderr)
    if findings:
        print(
            "check-version: the version lives in the workspace Cargo.toml only; docs say "
            '"the latest release" (scripts/check-version.py lists the rules)',
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
