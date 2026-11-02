"""Scan files for secret values held in the environment, or scrub them out.

Usage: python3 keyscan.py [--scrub] PATH...

A hit is any run of 12 or more characters of a secret value found in a file.
Every regular file outside .git/ is scanned, dotfiles and .py files included.
Names and counts are printed, never a value. Python 3.9 compatible.

Exit codes: 0 clean, 1 hits, 2 usage, 3 no secret values in the environment
(nothing was proven).
"""
import os
import sys
from urllib.parse import unquote, urlsplit

NAMES = (
    "VERCEL_TOKEN", "VERCEL_TEAM_ID", "VERCEL_PROJECT_ID",
    "CLAUDE_API_KEY", "OPENAI_API_KEY", "DEEPSEEK_API_KEY", "GEMINI_API_KEY",
    "AI_GATEWAY_API_KEY", "DATABASE_URL", "CLOUDFLARE_ACCOUNT_ID",
    "CLOUDFLARE_API_TOKEN", "GOALCUP_TEST_APPROVAL_KEY",
    "GOALCUP_TEST_MEMBER_ID", "GH_TOKEN",
)
N = 12
REDACTED = "[KEY-FRAGMENT-REDACTED]"


def _pieces(name, value):
    if name != "DATABASE_URL":
        return [value]
    try:
        u = urlsplit(value)
        out = [u.password or "", u.hostname or ""]
    except ValueError:
        out = [value]
    return out + [unquote(p) for p in out]


def values(env):
    """List of (name, value) with at least N characters; DATABASE_URL gives password and host."""
    out = []
    for name in NAMES:
        v = env.get(name, "")
        if not v:
            continue
        seen = set()
        for p in _pieces(name, v):
            if len(p) >= N and p not in seen:
                seen.add(p)
                out.append((name, p))
    return out


def _files(paths):
    for p in paths:
        if os.path.isfile(p) and not os.path.islink(p):
            yield p
            continue
        for root, dirs, names in os.walk(p):
            dirs[:] = [d for d in dirs if d != ".git"]
            for n in names:
                f = os.path.join(root, n)
                if os.path.isfile(f) and not os.path.islink(f):
                    yield f


def _read(f):
    with open(f, "rb") as fh:
        return fh.read().decode("utf-8", "surrogateescape")


def scan(paths, vals):
    hits = []
    for f in _files(paths):
        t = _read(f)
        for name, v in vals:
            if any(v[i:i + N] in t for i in range(len(v) - N + 1)):
                hits.append((name, f))
    return hits


def scrub(paths, vals):
    total = 0
    for f in _files(paths):
        if f.endswith(".tar"):
            continue
        t = _read(f)
        orig = t
        hits = 0
        for _name, k in vals:
            grams = {k[i:i + N] for i in range(len(k) - N + 1)}
            i = 0
            while i <= len(t) - N:
                if t[i:i + N] in grams:
                    j = i + N
                    while j < len(t) and t[i:j + 1] in k:
                        j += 1
                    t = t[:i] + REDACTED + t[j:]
                    hits += 1
                    i += len(REDACTED)
                else:
                    i += 1
        if t != orig:
            with open(f, "wb") as fh:
                fh.write(t.encode("utf-8", "surrogateescape"))
            print("scrubbed %d fragment(s) in %s" % (hits, f))
            total += hits
    return total


def main(argv):
    args = argv[1:]
    do_scrub = False
    if args and args[0] == "--scrub":
        do_scrub = True
        args = args[1:]
    if not args or any(a.startswith("--") for a in args):
        print("usage: keyscan.py [--scrub] PATH...", file=sys.stderr)
        return 2
    vals = values(os.environ)
    if not vals:
        print("keyscan: no secret values in env; nothing proven")
        return 3
    if do_scrub:
        scrub(args, vals)
        return 0
    hits = scan(args, vals)
    for name, f in hits:
        print("HIT %s %s" % (name, f))
    have = {n for n, _ in vals}
    unset = [n for n in NAMES if n not in have]
    print("keyscan: %d hit(s), %d keys checked, unset: %s"
          % (len(hits), len(have), ", ".join(unset) if unset else "none"))
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
