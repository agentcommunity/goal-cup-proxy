#!/usr/bin/env python3
"""Derive the sanitized T20 OpenAI Responses fixture from one raw E01 Codex chain.

Usage: derive.py CHAIN_JSONL OUT_JSONL

Reads the git-ignored E01 chain read-only (orchestrator ruling R12), keeps only the streams
whose Open carries protocol_hint "openai-responses", and writes one JSON line per stream in
Open order. Each line holds the recorder metadata of that stream (every stream is on host
chatgpt.com, path /backend-api/codex/responses, so those two are not repeated), the first/last
observation times and count of its retained segments, and its retained bytes as sanitized
UTF-8 text.

Sanitization is structural and value-level; the JSON shape of every provider message is kept:
  * provider IDs (response/item/call/turn IDs, cache keys) -> consistent placeholders, so
    equal originals stay equal and previous_response_id chains are preserved;
  * client_metadata (installation/window/session IDs, turn metadata) is dropped;
  * safety_identifier, encrypted_content, obfuscation padding, access_programs values ->
    fixed placeholders;
  * tool schemas/descriptions in tools/additional_tools -> names and types only;
  * developer/user context text (base instructions, skills, plugins, environment context)
    -> a placeholder naming its original length; the E01 goal text is public and kept;
  * the vendor events codex.rate_limits, codex.response.metadata and
    responsesapi.websocket_timing (account plan, rate limits, headers, telemetry) keep only
    their "type" member;
  * usage.attribution item keys -> the same ID placeholders.
Recorder producer/connection IDs are not written; the test supplies documented placeholders.

It then scans the output: the needle values it replaced, '@', UUID shapes, long mixed
tokens, secret shapes and home-directory paths must all be absent. The run path
/Users/Shared/goalcup-m2-e01/<run> inside the tool inputs is shared, not personal, and is kept
(the same judgement E01 recorded for its committed excerpts). Run spikes/redact.py on the
output and review it by eye before committing.
"""
import base64
import json
import re
import sys

GOAL = (
    "Create a file named result.txt in the current working directory whose entire content is "
    "exactly the 13 bytes m2-capture-ok with no trailing newline. Then read result.txt back "
    "using a tool and check that its content is exactly m2-capture-ok. Then reply with the "
    "single word done. Do not ask questions and do not wait for further input."
)
HINT = "openai-responses"
HOST = "chatgpt.com"
PATH = "/backend-api/codex/responses"
ID_KEYS = {
    "id", "call_id", "item_id", "previous_response_id", "response_id",
    "comparison_response_id", "turn_id", "root_turn_id", "prompt_cache_key",
}
PREFIXES = {"resp", "msg", "rs", "ctc", "ctco", "call", "at", "fc", "fco"}
VENDOR_EVENTS = {"codex.rate_limits", "codex.response.metadata", "responsesapi.websocket_timing"}
UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
MIXED = re.compile(r"(?=[A-Za-z0-9_\-]*[0-9])(?=[A-Za-z0-9_\-]*[A-Za-z])[A-Za-z0-9_\-]{20,}")
SECRETS = re.compile(r"sk-[A-Za-z0-9]|[Bb]earer |eyJ[A-Za-z0-9_\-]{10,}|refresh_token|goalcup:[0-9a-f]")
RUN_PATH = "/Users/Shared/goalcup-m2-e01/"

ids = {}
needles = set()


def ph_id(value):
    if value not in ids:
        head = value.split("_", 1)[0] if "_" in value else ""
        prefix = head if head in PREFIXES else "ph"
        ids[value] = f"{prefix}_ph{len(ids) + 1:03d}"
    needles.add(value)
    return ids[value]


def fixed(value, placeholder):
    if isinstance(value, str) and len(value) >= 6:
        needles.add(value)
    return placeholder


def slim_tools(tools):
    out = []
    for tool in tools:
        slim = {k: tool[k] for k in ("type", "name") if k in tool}
        if "description" in tool:
            slim["description"] = f"<omitted: {len(tool['description'] or '')} chars>"
        if isinstance(tool.get("tools"), list):
            slim["tools"] = slim_tools(tool["tools"])
        out.append(slim)
    return out


def context_text(text):
    if text == GOAL:
        return text
    return f"<omitted: context text, {len(text)} chars>"


def walk(value, path):
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            here = path + (key,)
            if key == "client_metadata":
                for v in item.values():
                    fixed(v, None)
                continue
            if key in ID_KEYS and isinstance(item, str):
                out[key] = ph_id(item)
            elif key == "safety_identifier":
                out[key] = fixed(item, "ph-safety-identifier")
            elif key == "encrypted_content":
                out[key] = fixed(item, "ph-encrypted-content")
            elif key == "obfuscation":
                out[key] = fixed(item, "ph")
            elif key == "access_programs" and isinstance(item, dict):
                out[key] = {k: fixed(v, "ph") for k, v in item.items()}
            elif key == "tools" and isinstance(item, list):
                out[key] = slim_tools(item)
            elif key == "attribution" and isinstance(item, dict):
                items = item.get("items", {})
                out[key] = {"items": {ph_id(k): walk(v, here) for k, v in items.items()}}
            else:
                out[key] = walk(item, here)
        return out
    if isinstance(value, list):
        return [walk(v, path) for v in value]
    return value


def collect_values(value, skip):
    if isinstance(value, dict):
        for item in value.values():
            collect_values(item, skip)
    elif isinstance(value, list):
        for item in value:
            collect_values(item, skip)
    elif isinstance(value, str) and len(value) >= 6 and value != skip:
        # Plain lower-case enumeration words ("complete", "response") are not identifiers.
        if not re.fullmatch(r"[a-z_]+", value):
            needles.add(value)


def sanitize(message):
    kind = message.get("type")
    if kind in VENDOR_EVENTS:
        collect_values(message, skip=kind)
        return {"type": kind}
    out = walk(message, ())
    if kind == "response.create":
        for item in out.get("input", []):
            if item.get("type") == "message":
                for part in item.get("content", []):
                    if isinstance(part.get("text"), str):
                        original = part["text"]
                        part["text"] = context_text(original)
                        if part["text"] != original:
                            needles.update(t for t in re.split(r"\s+", original) if len(t) >= 24)
    return out


def main():
    chain, out_path = sys.argv[1], sys.argv[2]
    opens, parts, order = {}, {}, []
    for line in open(chain, encoding="utf-8"):
        body = json.loads(line)["body"]
        if body["kind"] != "capture":
            continue
        record = body["content"]["record"]
        kind = record["kind"]
        if kind == "open" and record["metadata"].get("protocol_hint") == HINT:
            key = (record["stream"]["producer"], record["stream"]["stream"])
            opens[key] = record
            parts[key] = []
            order.append(key)
        elif kind == "segment":
            key = (record["stream"]["producer"], record["stream"]["stream"])
            if key in parts:
                if record["audit"]["spans"]:
                    sys.exit(f"stream {key} carries an audit span; not expected in this run")
                parts[key].append(record)
    lines = []
    for number, key in enumerate(order, start=1):
        record = opens[key]
        meta = record["metadata"]
        segs = parts[key]
        raw = b"".join(base64.b64decode(s["data_b64"]) for s in segs)
        text = ""
        if raw:
            text = json.dumps(sanitize(json.loads(raw)), ensure_ascii=False, separators=(",", ":"))
        if (meta["host"], meta["path"]) != (HOST, PATH):
            sys.exit(f"stream {key} is not {HOST}{PATH}")
        wire = meta["wire"] or {}
        line = {"stream": number, "source": record["source"], "exchange": wire.get("exchange")}
        for name, value in (
            ("message_index", wire.get("message_index")),
            ("method", meta["method"]),
            ("status", meta["status"]),
            ("content_type", meta["content_type"]),
        ):
            if value is not None:
                line[name] = value
        line["label"] = meta["label"]
        if segs:
            line["retained_segments"] = len(segs)
            line["first"] = {"wall": segs[0]["first"]["wall"], "mono_ns": segs[0]["first"]["mono_ns"]}
            line["last"] = {"wall": segs[-1]["last"]["wall"], "mono_ns": segs[-1]["last"]["mono_ns"]}
        line["text"] = text
        lines.append(line)
    with open(out_path, "w", encoding="utf-8") as out:
        for line in lines:
            out.write(json.dumps(line, ensure_ascii=False, separators=(",", ":")) + "\n")
    written = open(out_path, encoding="utf-8").read()
    problems = []
    for needle in sorted(needles):
        if needle and needle in written:
            problems.append(("needle", needle[:12] + "..."))
    for name, rx in (("uuid", UUID), ("secret", SECRETS)):
        problems += [(name, m.group(0)[:12]) for m in rx.finditer(written)]
    problems += [("@", written.count("@"))] if "@" in written else []
    for m in MIXED.finditer(written):
        if m.group(0) not in ("codex-20260928T032110Z",):
            problems.append(("mixed-token", m.group(0)[:12]))
    for m in re.finditer(r"/Users/[^\s\"\\]*", written):
        if not m.group(0).startswith(RUN_PATH):
            problems.append(("path", m.group(0)[:24]))
    print(json.dumps({
        "streams": len(lines),
        "placeholders": len(ids),
        "needles_checked": len(needles),
        "problems": problems,
    }))
    if problems:
        sys.exit(1)


if __name__ == "__main__":
    main()
