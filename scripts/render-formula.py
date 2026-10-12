"""Render packaging/homebrew/goalcup.rb for one release and print it.

    python3 scripts/render-formula.py --version 0.5.0 --sums dist/SHA256SUMS \\
        --base-url https://github.com/agentcommunity/goal-cup-tracer/releases/download/v0.5.0

--sums is the release's SHA256SUMS (`sha256sum` format). Every target the formula names must
have a tarball in it, and every @TOKEN@ must be filled, or nothing is printed.
"""

import argparse
import re
import sys
from pathlib import Path

TEMPLATE = Path(__file__).resolve().parent.parent / "packaging" / "homebrew" / "goalcup.rb"
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin",
           "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--sums", required=True, type=Path)
    args = parser.parse_args()

    sums = {}
    for line in args.sums.read_text().splitlines():
        digest, _, name = line.partition("  ")
        sums[name.lstrip("*")] = digest
    tokens = {"VERSION": args.version, "BASE_URL": args.base_url.rstrip("/")}
    for target in TARGETS:
        digest = sums.get(f"goalcup-{args.version}-{target}.tar.gz", "")
        if not re.fullmatch(r"[0-9a-f]{64}", digest):
            sys.exit(f"render-formula: no SHA-256 for goalcup-{args.version}-{target}.tar.gz "
                     f"in {args.sums}")
        tokens["SHA256_" + target.upper().replace("-", "_")] = digest

    # The template's own header comment is about rendering; the formula starts at its class.
    text = TEMPLATE.read_text()
    text = text[text.index("class Goalcup"):]
    for name, value in tokens.items():
        text = text.replace(f"@{name}@", value)
    left = re.findall(r"@[A-Z0-9_]+@", text)
    if left:
        sys.exit(f"render-formula: unfilled tokens {sorted(set(left))}")
    sys.stdout.write(text)


if __name__ == "__main__":
    main()
