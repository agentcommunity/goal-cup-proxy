#!/usr/bin/env python3
"""Derive the sanitized T21 Anthropic Messages fixture from the E01 Claude Code chain.

Usage: derive.py CHAIN OUT

CHAIN is the git-ignored E01 trace
  spikes/m2-first-release/out/early/live/claude-20260928T032648Z/trace/chain.jsonl
(worktree m2-e01-live), read only; its sha256 must equal SOURCE_SHA256. OUT receives the
`anthropic-messages` streams only (their Open, Segment and End capture records, in chain
order) as CaptureRecord JSON lines, except that a Segment's `data_b64` is written as `text`
(the same bytes as UTF-8) so the committed fixture can be reviewed by eye. The test loader
turns `text` back into `data_b64`.

Transformations, all recorded in provenance.json:
- producer, connection and clock IDs -> fixed 32-hex placeholders;
- provider IDs (msg_, toolu_, req_) -> same-length placeholders, consistently across
  streams, so a continuation's `thread.previous_message_id` and `tool_use_id`s still name
  the earlier response's IDs;
- SSE bodies: only the non-empty thinking `signature` values change, to a same-length filler,
  so every response segment keeps its real length, offset and boundary;
- request bodies are re-serialized compactly after eliding: every `system` text, the
  `metadata.user_id` (account, device and session IDs), each tool's `description` and
  `input_schema`, and each `<system-reminder>` text block longer than 128 bytes that carries
  no T04 redaction marker (environment, model identity, deferred tools, agent types, skills);
  elided values become `<elided: N bytes>` (tool schemas `{}`);
- the first request, originally two segments with two T04 `email` audit spans in the first,
  is split again into two parts at its `"system"` member, and its audit spans are
  recomputed from the retained `<redacted:email>` markers and the original source span
  lengths, so source offsets stay consistent with the retained bytes.

No other byte changes. Observation times are copied as recorded.
"""
import base64
import hashlib
import json
import re
import sys

SOURCE_SHA256 = "bb20b7c6ddeba4cc3a7e5215eee769ccd83719044b7df25b407ed9d7fc4bdacf"
HINT = "anthropic-messages"
PRODUCER = "a0000000000000000000000000000001"
CONNECTION = "c0000000000000000000000000000001"
ID_PATTERN = re.compile(rb"\b(msg|toolu|req)_[A-Za-z0-9]{10,}")
SIGNATURE = re.compile(rb'"signature":"([^"]+)"')
MARKER = re.compile(r"<redacted:([a-z_]+)>")
FILLER = b"elided-signature-"


def records(path):
    for line in open(path, encoding="utf-8"):
        body = json.loads(line)["body"]
        if body.get("kind") == "capture":
            yield body["content"]["record"]


def placeholder(original, prefix, number):
    head = prefix + b"_PLACEHOLDER"
    digits = len(original) - len(head)
    if digits < 2:
        sys.exit("provider ID too short for a same-length placeholder")
    return head + str(number).zfill(digits).encode()


def elided(value):
    return "<elided: %d bytes>" % len(value.encode("utf-8"))


def sanitize_request(body):
    request = json.loads(body)
    for block in request.get("system", []):
        block["text"] = elided(block["text"])
    metadata = request.get("metadata") or {}
    if "user_id" in metadata:
        metadata["user_id"] = elided(metadata["user_id"])
    for tool in request.get("tools", []):
        tool["description"] = elided(tool["description"])
        tool["input_schema"] = {}
    for message in request["messages"]:
        content = message["content"]
        if not isinstance(content, list):
            continue
        for block in content:
            text = block.get("text")
            if (
                block.get("type") == "text"
                and text.startswith("<system-reminder>")
                and len(text.encode("utf-8")) > 128
                and not MARKER.search(text)
            ):
                block["text"] = elided(text)
    return json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    chain, out_path = sys.argv[1], sys.argv[2]
    digest = hashlib.sha256(open(chain, "rb").read()).hexdigest()
    if digest != SOURCE_SHA256:
        sys.exit("unexpected source chain sha256 " + digest)

    all_records = list(records(chain))
    opens = {}
    for record in all_records:
        if record["kind"] == "open" and record["metadata"]["protocol_hint"] == HINT:
            opens[record["stream"]["stream"]] = record
    producers = {record["stream"]["producer"] for record in opens.values()}
    connections = {record["metadata"]["wire"]["connection"] for record in opens.values()}
    if len(producers) != 1 or len(connections) != 1:
        sys.exit("expected one producer and one connection")
    (producer,) = producers

    def selected(record):
        stream = record.get("stream") or {}
        return stream.get("producer") == producer and stream.get("stream") in opens

    kept = [r for r in all_records if r["kind"] in ("open", "segment", "end") and selected(r)]
    segments = {}
    for record in kept:
        if record["kind"] == "segment":
            segments.setdefault(record["stream"]["stream"], []).append(record)
    bodies = {
        stream: b"".join(base64.b64decode(s["data_b64"]) for s in parts)
        for stream, parts in segments.items()
    }

    ids = {}
    for stream in sorted(bodies):
        for match in ID_PATTERN.finditer(bodies[stream]):
            original = match.group(0)
            if original not in ids:
                prefix = match.group(1)
                count = sum(1 for k in ids if k.startswith(prefix + b"_")) + 1
                ids[original] = placeholder(original, prefix, count)

    def replace_ids(data):
        return ID_PATTERN.sub(lambda m: ids[m.group(0)], data)

    new_parts = {}
    for stream, body in bodies.items():
        source = opens[stream]["source"]
        parts = segments[stream]
        if source == "wire_response":
            filled = SIGNATURE.sub(
                lambda m: b'"signature":"'
                + (FILLER * (len(m.group(1)) // len(FILLER) + 1))[: len(m.group(1))]
                + b'"',
                body,
            )
            filled = replace_ids(filled)
            if len(filled) != len(body):
                sys.exit("response length changed")
            cut, pieces = 0, []
            for part in parts:
                size = len(base64.b64decode(part["data_b64"]))
                pieces.append((part, filled[cut : cut + size], part["source_offset"], part["source_len"], part["audit"]))
                cut += size
            new_parts[stream] = pieces
            continue
        reduced = replace_ids(sanitize_request(body))
        if len(parts) == 1:
            if parts[0]["audit"]["spans"]:
                sys.exit("unexpected audit on a one-part request")
            new_parts[stream] = [(parts[0], reduced, 0, len(reduced), {"spans": []})]
            continue
        if len(parts) != 2 or parts[1]["audit"]["spans"]:
            sys.exit("expected a two-part request audited in part 0 only")
        cut = reduced.find(b',"system":[')
        if cut < 0:
            sys.exit("no system member to split at")
        head, tail = reduced[:cut], reduced[cut:]
        original_spans = parts[0]["audit"]["spans"]
        markers = list(MARKER.finditer(head.decode("utf-8")))
        if len(markers) != len(original_spans) or MARKER.search(tail.decode("utf-8")):
            sys.exit("redaction markers do not match the audit")
        spans, shift = [], 0
        for marker, span in zip(markers, original_spans):
            if marker.group(1) != span["kind"]:
                sys.exit("marker kind differs from its audit span")
            at = len(head.decode("utf-8")[: marker.start()].encode("utf-8"))
            length = span["end"] - span["start"]
            spans.append({"kind": span["kind"], "start": at + shift, "end": at + shift + length})
            shift += length - len(marker.group(0).encode("utf-8"))
        new_parts[stream] = [
            (parts[0], head, 0, len(head) + shift, {"spans": spans}),
            (parts[1], tail, len(head) + shift, len(tail), {"spans": []}),
        ]

    def placed(time):
        return dict(time, clock_id=PRODUCER)

    def key(record):
        return {"producer": PRODUCER, "stream": record["stream"]["stream"]}

    lines = []
    for record in kept:
        stream = record["stream"]["stream"]
        if record["kind"] == "open":
            metadata = dict(record["metadata"])
            metadata["wire"] = dict(metadata["wire"], connection=CONNECTION)
            lines.append(dict(record, stream=key(record), observed=placed(record["observed"]), metadata=metadata))
        elif record["kind"] == "end":
            lines.append(dict(record, stream=key(record), observed=placed(record["observed"])))
        else:
            pieces = [p for p in new_parts[stream] if p[0] is record]
            if not pieces:
                sys.exit("segment without a derived piece")
            _, data, offset, length, audit = pieces[0]
            lines.append(
                {
                    "kind": "segment",
                    "stream": key(record),
                    "part": record["part"],
                    "source_offset": offset,
                    "source_len": length,
                    "first": placed(record["first"]),
                    "last": placed(record["last"]),
                    "text": data.decode("utf-8"),
                    "audit": audit,
                }
            )

    with open(out_path, "w", encoding="utf-8") as out:
        for line in lines:
            out.write(json.dumps(line, ensure_ascii=False, separators=(",", ":")) + "\n")
    written = hashlib.sha256(open(out_path, "rb").read()).hexdigest()
    print("%d records, %d provider IDs replaced, sha256 %s" % (len(lines), len(ids), written))


if __name__ == "__main__":
    main()
