"""Print the third-party notices a binary release carries (NOTICE): every crates.io package the
three shipped binaries link for one target, with its licence expression and the text of the
licence files it ships.

    python3 scripts/third-party-licenses.py --target aarch64-apple-darwin > THIRD-PARTY-LICENSES.md

Only normal dependencies are followed: build scripts' and tests' dependencies are not in the
binaries. Run it after the build, so every package's sources are already in the cargo registry.
"""

import argparse
import json
import subprocess
from pathlib import Path

SHIPPED = {"goalcup-cli", "goalcup-daemon", "goalcup-verifier"}
LICENCE_FILE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense")


def linked_packages(meta: dict) -> list[dict]:
    packages = {package["id"]: package for package in meta["packages"]}
    nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
    todo = [pid for pid, package in packages.items()
            if package["name"] in SHIPPED and package["source"] is None]
    seen: set[str] = set()
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


def licence_files(package: dict) -> list[Path]:
    root = Path(package["manifest_path"]).parent
    files = {path for path in root.iterdir()
             if path.is_file() and path.name.lower().startswith(LICENCE_FILE_PREFIXES)}
    if package.get("license_file"):
        named = root / package["license_file"]
        if named.is_file():
            files.add(named)
    return sorted(files)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--target", required=True, help="the target triple the release is for")
    args = parser.parse_args()
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked",
         "--filter-platform", args.target],
        check=True, capture_output=True, text=True).stdout)

    packages = linked_packages(meta)
    print(f"# Third-party licences ({args.target})\n")
    print("The goalcup, goalcup-daemon and goalcup-verify binaries link these crates.io "
          f"packages ({len(packages)}), each under its own licence. Generated from Cargo.lock by "
          "scripts/third-party-licenses.py.\n")
    for package in packages:
        print(f"## {package['name']} {package['version']}\n")
        print(f"Licence: {package.get('license') or 'see the files below'}\n")
        files = licence_files(package)
        if not files:
            print("The package ships no licence file.\n")
        for path in files:
            text = path.read_text(encoding="utf-8", errors="replace").strip()
            print(f"### {path.name}\n\n```text\n{text}\n```\n")


if __name__ == "__main__":
    main()
