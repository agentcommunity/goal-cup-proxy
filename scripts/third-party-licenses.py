"""Print the third-party notices a binary release carries (NOTICE): every crates.io package the
three shipped binaries link for one target, with its licence expression and the text of the
licence files it ships.

    python3 scripts/third-party-licenses.py --target aarch64-apple-darwin > THIRD-PARTY-LICENSES.md

Only normal dependencies are followed: build scripts' and tests' dependencies are not in the
binaries. Run it after the build, so every package's sources are already in the cargo registry.

Every package's SPDX expression must be allowed by ALLOWED (an `A OR B` is allowed when one side
is, an `A AND B` when both are); otherwise the script names the packages and exits 1, and nothing
is shipped. A package that ships no licence file gets the standard text of its licence (MIT with
its authors, Apache-2.0 from this repository's LICENSE); with none known, the script fails too.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SHIPPED = {"goalcup-cli", "goalcup-daemon", "goalcup-verifier"}
LICENCE_FILE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense")
# Permissive licences a release may carry (SPDX ids). MPL-2.0 is file-level copyleft and allowed.
ALLOWED = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0",
           "CDLA-Permissive-2.0", "Zlib", "MPL-2.0"}

MIT_TEXT = """MIT License

Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""


class LicenceError(Exception):
    pass


def licence_allowed(expr, allowed=ALLOWED) -> bool:
    """Whether an SPDX expression is satisfied by the allowed ids. `/` is the old spelling of OR;
    `X WITH exception` counts as X (an exception only adds permissions). Malformed raises."""
    if not expr or not expr.strip():
        raise LicenceError("no licence expression")
    tokens = re.findall(r"\(|\)|[^\s()/]+|/", expr.replace("/", " / "))
    pos = 0

    def peek():
        return tokens[pos] if pos < len(tokens) else None

    def take():
        nonlocal pos
        pos += 1
        return tokens[pos - 1]

    def atom():
        tok = take() if peek() is not None else None
        if tok == "(":
            value = either()
            if peek() != ")":
                raise LicenceError(f"unbalanced parentheses in {expr!r}")
            take()
            return value
        if tok is None or tok in (")", "/") or tok.upper() in ("AND", "OR", "WITH"):
            raise LicenceError(f"malformed licence expression {expr!r}")
        value = tok.rstrip("+") in allowed
        if peek() is not None and peek().upper() == "WITH":
            take()
            if peek() is None:
                raise LicenceError(f"malformed licence expression {expr!r}")
            take()
        return value

    def both():
        value = atom()
        while peek() is not None and peek().upper() == "AND":
            take()
            value = atom() and value
        return value

    def either():
        value = both()
        while peek() is not None and (peek().upper() == "OR" or peek() == "/"):
            take()
            value = both() or value
        return value

    result = either()
    if pos != len(tokens):
        raise LicenceError(f"malformed licence expression {expr!r}")
    return result


def licence_ids(expr) -> list:
    """The licence ids an expression names, in order, without operators or exceptions."""
    words = re.findall(r"[^\s()/]+", expr or "")
    out, skip = [], False
    for w in words:
        if skip:
            skip = False
            continue
        if w.upper() == "WITH":
            skip = True
            continue
        if w.upper() in ("AND", "OR"):
            continue
        if w not in out:
            out.append(w)
    return out


def standard_texts(package: dict) -> list:
    """(name, text) of the standard licence texts known for the package's licence ids."""
    out = []
    for lid in licence_ids(package.get("license")):
        lid = lid.rstrip("+")
        if lid == "MIT":
            authors = [re.sub(r"\s*<[^>]*>", "", a).strip() for a in package.get("authors") or []]
            holders = ", ".join(a for a in authors if a) or f"the {package['name']} authors"
            out.append(("MIT (standard text)", MIT_TEXT.format(holders=holders)))
        elif lid == "Apache-2.0" and (REPO / "LICENSE").is_file():
            out.append(("Apache-2.0 (standard text)", (REPO / "LICENSE").read_text().strip()))
    return out


def linked_packages(meta: dict) -> list:
    packages = {package["id"]: package for package in meta["packages"]}
    nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
    todo = [pid for pid, package in packages.items()
            if package["name"] in SHIPPED and package["source"] is None]
    seen = set()
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    third = [packages[pid] for pid in seen if packages[pid]["source"] is not None]
    return sorted(third, key=lambda package: (package["name"], package["version"]))


def licence_files(package: dict) -> list:
    root = Path(package["manifest_path"]).parent
    files = {path for path in root.iterdir()
             if path.is_file() and path.name.lower().startswith(LICENCE_FILE_PREFIXES)}
    if package.get("license_file"):
        named = root / package["license_file"]
        if named.is_file():
            files.add(named)
    return sorted(files)


def refusals(packages: list) -> list:
    """One line per package whose licence is not allowed or whose text cannot be carried."""
    out = []
    for package in packages:
        name = f"{package['name']} {package['version']}"
        expr = package.get("license")
        try:
            if not licence_allowed(expr):
                out.append(f"{name}: licence {expr} is not in the allowlist")
                continue
        except LicenceError as e:
            out.append(f"{name}: {e}")
            continue
        if not licence_files(package) and not standard_texts(package):
            out.append(f"{name}: ships no licence file and no standard text is known for {expr}")
    return out


def render(packages: list, target: str) -> str:
    lines = [f"# Third-party licences ({target})\n",
             "The goalcup, goalcup-daemon and goalcup-verify binaries link these crates.io "
             f"packages ({len(packages)}), each under its own licence. Generated from Cargo.lock "
             "by scripts/third-party-licenses.py.\n"]
    for package in packages:
        lines.append(f"## {package['name']} {package['version']}\n")
        lines.append(f"Licence: {package['license']}\n")
        files = licence_files(package)
        if not files:
            lines.append("The package ships no licence file; the standard text of its licence "
                         "follows.\n")
            for name, text in standard_texts(package):
                lines.append(f"### {name}\n\n```text\n{text}\n```\n")
        for path in files:
            text = path.read_text(encoding="utf-8", errors="replace").strip()
            lines.append(f"### {path.name}\n\n```text\n{text}\n```\n")
    return "\n".join(lines) + "\n"


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--target", required=True, help="the target triple the release is for")
    args = parser.parse_args(argv)
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--offline", "--format-version", "1", "--locked",
         "--filter-platform", args.target],
        check=True, capture_output=True, text=True).stdout)
    packages = linked_packages(meta)
    bad = refusals(packages)
    if bad:
        for line in bad:
            print(f"third-party-licenses: {line}", file=sys.stderr)
        print(f"third-party-licenses: {len(bad)} package(s) refused; allowed: "
              f"{', '.join(sorted(ALLOWED))}", file=sys.stderr)
        return 1
    sys.stdout.write(render(packages, args.target))
    return 0


if __name__ == "__main__":
    sys.exit(main())
