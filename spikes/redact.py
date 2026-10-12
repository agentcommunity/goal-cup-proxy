#!/usr/bin/env python3
"""Conservative secret redaction for spike fixtures. Text in, text out, plus a redaction log.

Usage: redact.py IN OUT   (writes OUT and OUT.redactions.jsonl)

Replaces each match with <redacted:KIND:first8ofsha256>. Review the output by eye before committing;
this catches common shapes, not everything.

How the rules combine. They are applied one after another over the text the previous rule left,
in PATTERNS order, so an earlier rule's marker hides its bytes from every later rule. Two
exceptions. private_key is the block rule: it runs first, on the original text, and its span
grows to take in every other rule's match that overlaps it (and every match that overlaps
those), so a PEM block is one redaction whatever an earlier rule would have matched inside it
or across its edge, and no raw remainder of such a match is left behind (the daemon's block
rule merges the same claims into the block). A block with no END line runs to the end of the
text. And the last rule, masked_key, is soft: after that pass, any byte of a masked-key shape (matched on the
ORIGINAL text) that is still raw is replaced too, as its own "residual" marker. The daemon
(crates/goalcup-daemon/src/redact.rs) reaches the same result by a different route; the shared
rules, and the few places the two differ, are pinned by crates/goalcup-daemon/tests/it/redaction.rs.

Two offset frames in OUT.redactions.jsonl, both counted in characters, not bytes:
  - an entry from the sequential pass (no "residual" key) has start/end in the text as the
    EARLIER rules left it, with their markers already in place. Equal to original offsets only
    when no earlier rule fired before it in the text. (private_key runs first, so its entries
    are always in original offsets.)
  - an entry with "residual": true has start/end in the ORIGINAL text.
The daemon's audit spans are always original byte offsets. Do not compare the two logs
offset for offset; compare the redacted output, or the kinds.
"""
import hashlib, json, re, sys

PATTERNS = [
    ("anthropic_key", re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}")),
    ("openai_key", re.compile(r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}")),
    ("github_token", re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}")),
    ("aws_access_key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("bearer", re.compile(r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}")),
    ("jwt", re.compile(r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}")),
    ("private_key", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|\Z)")),
    ("slack_token", re.compile(r"xox[baprs]-[A-Za-z0-9\-]{10,}")),
    ("generic_secret_assignment", re.compile(r"(?i)(api[_-]?key|secret|token|password)\s*[=:]\s*['\"]?([A-Za-z0-9_\-]{16,})")),
    ("email", re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")),
    # A provider key echoed back masked (D41): OpenAI's `sk-R7aMV` + 39 stars + `oKYo`, Hermes'
    # `sk-0...cdef` and `key[:8]...key[-4:]`. The daemon treats this rule as SOFT: it never
    # shadows another rule, and it keeps the part of a mask no other rule covered. This script
    # applies its rules one after another, so listing the rule LAST keeps it from shadowing
    # anything, and `main` adds the second half: after the pass, any byte of a mask (found in
    # the ORIGINAL text) that is still raw is redacted as `masked_key`.
    ("masked_key", re.compile(r"(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}|…|\\u2026|\*{3,64})[A-Za-z0-9]{3,4}")),
]


SOFT_KIND = "masked_key"
BLOCK_KIND = "private_key"


class Segments:
    """The text as pieces that remember where they came from.

    Each piece is [text, orig_start, orig_end, raw]. A raw piece is original text, so its
    length is orig_end - orig_start and it can be cut anywhere. A marker piece stands for the
    original range it replaced, and when it is cut every part of it keeps that whole range.
    """

    def __init__(self, text):
        self.pieces = [[text, 0, len(text), True]] if text else []

    def joined(self):
        return "".join(p[0] for p in self.pieces)

    def slice(self, a, b):
        """The pieces that make up joined()[a:b]."""
        out, at = [], 0
        for text, oa, ob, raw in self.pieces:
            lo, hi = max(a, at), min(b, at + len(text))
            if lo < hi:
                if raw:
                    out.append([text[lo - at:hi - at], oa + (lo - at), oa + (hi - at), True])
                else:
                    out.append([text[lo - at:hi - at], oa, ob, False])
            at += len(text)
        return out


def union(spans):
    out = []
    for a, b in sorted(spans):
        if out and a <= out[-1][1]:
            out[-1][1] = max(out[-1][1], b)
        else:
            out.append([a, b])
    return out


def block_spans(text):
    """The block rule's spans: each private_key match, grown over every other (non-soft)
    rule's match that overlaps it, and over every match overlapping those, until nothing more
    joins. Spans that then overlap are one."""
    blocks = [list(m.span()) for kind, rx in PATTERNS if kind == BLOCK_KIND for m in rx.finditer(text)]
    if not blocks:
        return []
    others = sorted(
        m.span()
        for kind, rx in PATTERNS
        if kind not in (BLOCK_KIND, SOFT_KIND)
        for m in rx.finditer(text)
    )
    spans = blocks
    while True:
        grown = False
        for a, b in others:
            for g in spans:
                if a < g[1] and b > g[0] and (a < g[0] or b > g[1]):
                    g[0], g[1] = min(g[0], a), max(g[1], b)
                    grown = True
        merged = []
        for g in sorted(spans):
            if merged and g[0] < merged[-1][1]:
                merged[-1][1] = max(merged[-1][1], g[1])
                grown = True
            else:
                merged.append(g)
        spans = merged
        if not grown:
            return spans


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, dst = sys.argv[1], sys.argv[2]
    original = open(src, encoding="utf-8", errors="replace").read()
    log = open(dst + ".redactions.jsonl", "w")
    count = 0
    segments = Segments(original)
    soft = dict(PATTERNS)[SOFT_KIND]
    soft_spans = union([m.span() for m in soft.finditer(original)])

    def marker(kind, s):
        h = hashlib.sha256(s.encode()).hexdigest()[:8]
        return h, f"<redacted:{kind}:{h}>"

    # The block rule first, then the rest in PATTERNS order.
    ordered = [p for p in PATTERNS if p[0] == BLOCK_KIND] + [p for p in PATTERNS if p[0] != BLOCK_KIND]
    for kind, rx in ordered:
        joined = segments.joined()
        pieces, cursor = [], 0
        if kind == BLOCK_KIND:
            found = [(a, b) for a, b in block_spans(joined)]
        else:
            found = [m.span() for m in rx.finditer(joined)]
        for start, end in found:
            s = joined[start:end]
            h, replacement = marker(kind, s)
            # Offsets in `joined`: the text as the earlier rules left it, not the original.
            log.write(json.dumps({"kind": kind, "start": start, "end": end, "sha256_8": h}) + "\n")
            count += 1
            pieces.extend(segments.slice(cursor, start))
            covered = segments.slice(start, end)
            pieces.append([replacement, min(c[1] for c in covered), max(c[2] for c in covered), False])
            cursor = end
        pieces.extend(segments.slice(cursor, len(joined)))
        segments.pieces = pieces

    # The soft half: what is left raw of a mask that another rule only partly covered.
    pieces, run = [], None
    for text, oa, ob, raw in segments.pieces:
        parts = []
        if raw:
            at = oa
            for a, b in soft_spans:
                lo, hi = max(a, oa), min(b, ob)
                if lo < hi:
                    if lo > at:
                        parts.append([text[at - oa:lo - oa], at, lo, True])
                    parts.append([text[lo - oa:hi - oa], lo, hi, "residual"])
                    at = hi
            if at < ob:
                parts.append([text[at - oa:], at, ob, True])
        else:
            parts.append([text, oa, ob, False])
        for part in parts:
            if part[3] == "residual":
                if run is not None and run[2] == part[1]:
                    run[0] += part[0]
                    run[2] = part[2]
                else:
                    if run is not None:
                        pieces.append(run)
                    run = [part[0], part[1], part[2], "residual"]
            else:
                if run is not None:
                    pieces.append(run)
                    run = None
                pieces.append(part)
    if run is not None:
        pieces.append(run)
    final = []
    for text, oa, ob, raw in pieces:
        if raw == "residual":
            h, replacement = marker(SOFT_KIND, text)
            # Original offsets (oa, ob), unlike the sequential pass above.
            log.write(json.dumps({"kind": SOFT_KIND, "start": oa, "end": ob, "sha256_8": h, "residual": True}) + "\n")
            count += 1
            final.append(replacement)
        else:
            final.append(text)
    open(dst, "w", encoding="utf-8").write("".join(final))
    print(f"{count} redactions -> {dst}", file=sys.stderr)


if __name__ == "__main__":
    main()
