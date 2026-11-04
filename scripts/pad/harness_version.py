"""One harness version parser, the twin of crates/goalcup/src/connector/version.rs (grammar pinned in
docs/superpowers/plans/2026-11-04-v1-fixes/F-version-core.md): `parse` reads a `--version` line, `matches` checks a
parsed version against a connector's version spec."""
from __future__ import annotations

import re

WORD = re.compile(r"([0-9]+(?:\.[0-9]+)+)(?:[-+][0-9A-Za-z.-]+)?")


def _read(line: str) -> tuple[str, list[int]] | None:
    for word in line.split():
        word = word.lstrip("(").rstrip(".,;:)")
        word = word[1:] if word.startswith("v") else word
        m = WORD.fullmatch(word)
        nums = m and _numbers(m.group(1))
        if nums:
            return word, nums
    return None


def _numbers(s: str) -> list[int] | None:
    """Dotted ASCII numbers, each fitting u64; None for anything else."""
    parts = s.split(".")
    if not all(p and p.isascii() and p.isdigit() and int(p) < 2**64 for p in parts):
        return None
    return [int(p) for p in parts]


def parse(line: str) -> str | None:
    """The first word of `line` shaped like `1.2.3`, with an optional `-suffix` or `+suffix`."""
    got = _read(line)
    return got and got[0]


def matches(text: str, spec: str) -> bool:
    """Whether version `text` holds every `,`-separated part of `spec`: `>=1.2`, `<2`, `=1.0.105`, or a bare prefix
    such as `1.`, `1.x` or `0.22.6`. An empty spec or an unreadable part never holds."""
    got = _read(text)
    if not got:
        return False
    nums = got[1]
    for part in spec.split(","):
        op = next((o for o in (">=", "<=", ">", "<", "=") if part.startswith(o)), "")
        value = part[len(op):]
        value = (value[:-1] if value.endswith("x") else value).rstrip(".")
        want = _numbers(value)
        if want is None:
            return False
        n = max(len(nums), len(want))
        a, b = nums + [0] * (n - len(nums)), want + [0] * (n - len(want))
        held = {">=": a >= b, "<=": a <= b, ">": a > b, "<": a < b, "=": a == b}.get(op, nums[:len(want)] == want)
        if not held:
            return False
    return True
