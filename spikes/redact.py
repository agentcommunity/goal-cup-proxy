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
rule merges the same claims into the block). An END line must start within 15,872 characters of
the BEGIN line; without one, the block is those characters. (The daemon counts the bound in
bytes; for a body with non-ASCII text this script's block is longer, never shorter.) A block is
then cut at every unescaped `"` and only the parts inside JSON strings are redacted, starting
inside: a key streamed as many JSON string deltas loses every delta's text, and the JSON
between the strings stays. A block with no END ends at the first later BEGIN inside it, where
the next block starts, and is cut once 8,192 characters after its BEGIN (past any backslashes
there and the character they escape, so no escape is split). A rule whose span is a capture
group (SPAN_GROUP: the JSON form of generic_secret_assignment) replaces that group alone, so the
record stays JSON, and a match whose group overlaps an earlier rule's marker is dropped, as the
daemon's first-wins selection drops a claim over claimed bytes. And the last rule, masked_key,
is soft: after that pass, any byte of a masked-key shape (matched on the
ORIGINAL text) that is still raw is replaced too, as its own "residual" marker. The daemon
(crates/goalcup-daemon/src/redact.rs) reaches the same result by a different route; the shared
rules, and the few places the two differ, are pinned by crates/goalcup-daemon/tests/it/redaction.rs.

Bytes that are not valid UTF-8 match as the character "0", one per byte, as in the daemon: so an
invalid byte inside a key body does not end the key. They are written back unchanged where no
rule fires.

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

# The JSON form of generic_secret_assignment, the daemon's spelling: a quoted key whose `_`/`-`
# segment is the key word, or a camelCase prefix before it, then a quoted value of sixteen or more
# units with no quote, and a backslash only as an escape of one non-quote unit (`\/`). Each quote
# may follow up to seven backslashes (a secret inside an escaped string).
# Only the value, group 1, is replaced (SPAN_GROUP below): the key, the colon and the quotes stay.
GENERIC_JSON = re.compile(r'(?i)\\{0,7}"[A-Za-z0-9_\-]{0,1000}(?:api[_-]?key|secret|token|password)(?:[_\-][A-Za-z0-9_\-]{0,999})?\\{0,7}"\s*:\s*\\{0,7}"((?:[^"\\]|\\{1,7}[^"\\]){16,})\\{0,7}"')

PATTERNS = [
    ("anthropic_key", re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}")),
    ("openai_key", re.compile(r"sk-(?:proj-)?[A-Za-z0-9_\-]{20,}")),
    ("github_token", re.compile(r"github_pat_[A-Za-z0-9_]{22,}")),
    ("github_token", re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}")),
    ("aws_access_key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("bearer", re.compile(r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}")),
    ("jwt", re.compile(r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}")),
    ("private_key", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?:[\s\S]{0,15872}?-----END [A-Z ]*PRIVATE KEY-----|[\s\S]{0,15872})")),
    ("slack_token", re.compile(r"xox[baprs]-[A-Za-z0-9\-]{10,}")),
    # The JSON form first, then the shell/TOML form, as the daemon lists them.
    ("generic_secret_assignment", GENERIC_JSON),
    # The shell/TOML form. Its unspaced value is form-encoded (`refresh_token=1%2F%2F...`, or a
    # dotted `0.AXoA...`) and must be sixteen or more units with a `%XX` escape or the dotted
    # shape, so a path after `--token=` is not a secret (the daemon's SHARED literal).
    ("generic_secret_assignment", re.compile(r"(?i)(api[_-]?key|secret|token|password)(?:\s*[=:]\s*(?:\\{0,7}['\"])?([A-Za-z0-9_\-]{16,})|=(%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){15,}|[A-Za-z0-9_.~+/=\-]{1}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){14,}|[A-Za-z0-9_.~+/=\-]{2}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){13,}|[A-Za-z0-9_.~+/=\-]{3}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){12,}|[A-Za-z0-9_.~+/=\-]{4}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){11,}|[A-Za-z0-9_.~+/=\-]{5}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){10,}|[A-Za-z0-9_.~+/=\-]{6}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){9,}|[A-Za-z0-9_.~+/=\-]{7}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){8,}|[A-Za-z0-9_.~+/=\-]{8}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){7,}|[A-Za-z0-9_.~+/=\-]{9}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){6,}|[A-Za-z0-9_.~+/=\-]{10}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){5,}|[A-Za-z0-9_.~+/=\-]{11}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){4,}|[A-Za-z0-9_.~+/=\-]{12}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){3,}|[A-Za-z0-9_.~+/=\-]{13}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){2,}|[A-Za-z0-9_.~+/=\-]{14}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2}){1,}|[A-Za-z0-9_.~+/=\-]{15,}%[0-9A-Fa-f]{2}(?:[A-Za-z0-9_.~+/=\-]|%[0-9A-Fa-f]{2})*|[0-9]\.[A-Za-z0-9_.~+/=\-]{14,}))")),
    ("email", re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")),
    # The tracer's own proxy credential (H8), as the daemon spells it: the userinfo of the
    # capture launch's HTTPS_PROXY=http://goalcup:<32 hex>@127.0.0.1:<port>, the same
    # percent-encoded once, and base64("goalcup:<secret>"), the Basic token (its "==" stays).
    ("proxy_credential", re.compile(r"goalcup:[a-f0-9]{32}")),
    ("proxy_credential", re.compile(r"goalcup%3[Aa][a-f0-9]{32}")),
    ("proxy_credential", re.compile(r"Z29hbGN1cD[op][A-Za-z0-9+/]{43}")),
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

# The rules whose span is a capture group, not the whole match (the daemon's
# `Pattern::span_group`). The shell/TOML form's groups capture too, but its span is the whole
# match, as the committed fixtures carry it.
SPAN_GROUP = {GENERIC_JSON: 1}


def span_of(rx, m):
    """The span a match of rx replaces: its group in SPAN_GROUP, or the whole match."""
    return m.span(SPAN_GROUP.get(rx, 0))


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


def string_pieces(text, start, end):
    """The parts of text[start:end] a block redacts: cut at every unescaped quote (one after an
    even run of backslashes), starting inside a string, every piece inside a string, and a piece
    outside one when it holds a key-shaped run (holds_a_base64_run): raw text whose quotes do
    not pair as JSON. A piece loses the backslashes it ends on, and an empty piece is dropped.
    The daemon's `string_pieces` in redact.rs."""
    pieces, inside, frm, slashes = [], True, start, 0

    def push(a, b):
        while b > a and text[b - 1] == "\\":
            b -= 1
        if b > a:
            pieces.append([a, b])

    for at in range(start, end):
        c = text[at]
        if c == '"' and slashes % 2 == 0:
            if inside or holds_a_base64_run(text[frm:at]):
                push(frm, at)
            frm = at + 1
            inside = not inside
        slashes = slashes + 1 if c == "\\" else 0
    if inside or holds_a_base64_run(text[frm:end]):
        push(frm, end)
    return pieces


BASE64_RUN = re.compile(r"[A-Za-z0-9+/=]{16,}")


def holds_a_base64_run(text):
    """A run of sixteen or more base64 characters with a letter in it other than e or E, so a
    number-shaped run (an exponent float) does not count (the daemon's `holds_a_base64_run`)."""
    return any(re.search(r"[A-DF-Za-df-z]", m.group(0)) for m in BASE64_RUN.finditer(text))


BEGIN_LINE = re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")
END_AT_END = re.compile(r"-----END [A-Z ]*PRIVATE KEY-----\Z")
PEM_SLICE = 8192


def pem_matches(rx, text):
    """Each private_key match as (start, end, bounded). A match with no END line (bounded, a lone
    BEGIN's window) ends at the first later BEGIN inside it, and the search goes on from there.
    The daemon's `pem_matches` in redact.rs."""
    out, at = [], 0
    while True:
        m = rx.search(text, at)
        if not m:
            return out
        start, end = m.span()
        bounded = not END_AT_END.search(m.group(0))
        nxt = end
        if bounded:
            b = BEGIN_LINE.search(text, start + 1)
            if b and b.start() < end:
                end = nxt = b.start()
        out.append((start, end, bounded))
        at = max(nxt, start + 1)


def block_pieces(text, start, end, bounded):
    """string_pieces, and for a bounded block the piece that holds start + PEM_SLICE in two
    there. A cut right after a backslash first moves past the run of backslashes and the
    character they escape, so no escape is split (the daemon's `block_pieces`)."""
    pieces = string_pieces(text, start, end)
    cut = start + PEM_SLICE
    if bounded:
        while cut < end and text[cut - 1] == "\\":
            cut += 1
        for i, (a, b) in enumerate(pieces):
            if a < cut < b:
                pieces[i:i + 1] = [[a, cut], [cut, b]]
                break
    return pieces


def block_spans(text):
    """The block rule's spans: the string pieces of each private_key match, each grown over
    every other (non-soft) rule's match that overlaps it, and over every match overlapping
    those, until nothing more joins. Spans that then overlap are one."""
    blocks = [
        piece
        for kind, rx in PATTERNS
        if kind == BLOCK_KIND
        for start, end, bounded in pem_matches(rx, text)
        for piece in block_pieces(text, start, end, bounded)
    ]
    if not blocks:
        return []
    others = sorted(
        span_of(rx, m)
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
    # An invalid byte is one lone surrogate here, and "0" in the text the rules match on.
    source = open(src, encoding="utf-8", errors="surrogateescape").read()
    original = re.sub("[\udc80-\udcff]", "0", source)
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
            found = [span_of(rx, m) for m in rx.finditer(joined)]
            if rx in SPAN_GROUP:
                # A group span over an earlier rule's marker is dropped, as the daemon drops a
                # claim that overlaps a claimed span. (A whole-match rule never matches into a
                # marker: no shared value class takes `<`, `:` or `>`.)
                found = [(a, b) for a, b in found if all(p[3] for p in segments.slice(a, b))]
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
        elif raw:
            # The original text, invalid bytes and all.
            final.append(source[oa:ob])
        else:
            final.append(text)
    open(dst, "w", encoding="utf-8", errors="surrogateescape").write("".join(final))
    print(f"{count} redactions -> {dst}", file=sys.stderr)


if __name__ == "__main__":
    main()
