#!/usr/bin/env python3
"""NON-SHIPPING E02 Part B independent cross-check (Goal Cup M2). Python 3.9+ stdlib only.

    early-crosscheck.py [--eps-s S] [--delta-s S] OUT [EVIDENCE_DIR]

Reads the finished run directory OUT: the chain parts in `OUT/trace` (`chain.jsonl`, then
`chain.<20 digits>.jsonl` by start offset, D55) raw, line by line, base64-decoding every v5
`segment`; `summary.json` (the E00/E01 runner's: endpoint counters, owner stall, sink);
`load-report.json` (`early-load.py run`); `native/e02-native.jsonl`. It compares:

- sent (the load's own report) = forwarded (the endpoint's independent counters) exactly;
- sent = retained outside the injected gap: every stream that ended `complete` without a gap
  must equal a sent body or WS message byte for byte (by sha256), or, for the planted secret
  sentinel, have the sent source length, an `anthropic_key` audit span and no sentinel bytes;
  every sent item without such a stream, and every chain stream that is not clean, must fall
  inside the gap window [stall trigger - 2 s, last Recovery + 2 s]; anything else is
  unexplained;
- the gap evidence: the runner's queue-full stall, a `queue_full` Recovery, gap records, and
  every item sent after the window retained;
- the native stream against the native file, line by line (sentinel lines redacted);
- no sentinel byte and no proxy-credential shape in any decoded segment or in any other file
  of OUT and EVIDENCE_DIR.

The match rule (F-16). A sent item matches a clean stream of the same category (request,
response, WS client or server message), path and digest class (its sha256, or for a sentinel
item its source length) whose Open lies in [start - eps, start + delta] of the item's start
(host wall clock on both sides). The window is anchored on the start only: an item's end says
nothing about where the next identical item's stream opens. Within one class, items and
streams are paired in order (both sorted by time), so a lost stream leaves its own item
unmatched and never shifts its neighbours. With `early-load.py`'s unique bodies every class
holds one item; the order rule is the fallback for identical bodies (old reports, sentinels).
`eps` covers clock mapping (measured Open - start down to -4.2 ms); `delta` covers connection
setup, upload and response head (measured up to +107 ms on the 30-minute run) and stays well
under the 0.625 s spacing of the closest identical streams.

Prints a one-line verdict and writes the full JSON to EVIDENCE_DIR/crosscheck.json.
Exit 0 = all six checks passed, 1 = a check failed, 3 = inputs unreadable, 64 = usage.
"""

import base64
import datetime
import hashlib
import importlib.util
import json
import os
import re
import sys
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
CRED_PATTERNS = [re.compile(rb"goalcup:[0-9a-fA-F]{16,}@"), re.compile(rb"Z29hbGN1cDo"),
                 re.compile(rb"[Pp]roxy-[Aa]uthorization")]
MAX_NATIVE = 64 << 20
WINDOW_BEFORE_S, WINDOW_AFTER_S = 2.0, 2.0
MATCH_EPS_S = 0.010
MATCH_DELTA_S = 0.250
PART_RE = re.compile(r"chain\.([0-9]{20})\.jsonl")
CHECKS = ("sent_equals_forwarded", "sent_equals_retained_outside_gap", "load_self_check",
          "gap_is_visible_and_recovered", "native_matches_file", "no_leak")
CAT_OF = {"wire_request": "http_request", "wire_response": "http_response",
          "ws_client": "ws_client", "ws_server": "ws_server"}


_LOAD = []


def load_module():
    if not _LOAD:
        spec = importlib.util.spec_from_file_location("early_load",
                                                      os.path.join(HERE, "early-load.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _LOAD.append(mod)
    return _LOAD[0]


def wall(text):
    """`2026-09-27T17:19:34.965391000Z` -> epoch seconds."""
    m = re.fullmatch(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d+))?Z", text)
    if not m:
        raise ValueError("not a UTC wall time")
    base = datetime.datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S").replace(
        tzinfo=datetime.timezone.utc).timestamp()
    return base + (float("0." + m.group(2)) if m.group(2) else 0.0)


def list_parts(trace_dir):
    """The chain part paths in `trace_dir`, ordered by start offset (`export::chain_parts`)."""
    parts = []
    for name in os.listdir(trace_dir):
        if name == "chain.jsonl":
            start = 0
        else:
            m = PART_RE.fullmatch(name)
            if not m or int(m.group(1)) == 0:
                continue
            start = int(m.group(1))
        path = os.path.join(trace_dir, name)
        if os.path.isfile(path) and not os.path.islink(path):
            parts.append((start, path))
    if not parts:
        raise OSError("no chain part in %s" % trace_dir)
    return [p for _s, p in sorted(parts)]


def iter_lines(parts):
    """Every record line across the parts, as one stream; a part may end mid-line."""
    carry = b""
    for path in parts:
        with open(path, "rb") as fh:
            for raw in fh:
                if not raw.endswith(b"\n"):
                    carry += raw
                    continue
                line, carry = carry + raw, b""
                if line.strip():
                    yield line
    if carry.strip():
        yield carry


def observed_wall(rec, env):
    obs = rec.get("observed") or {}
    return wall(obs.get("wall") or env["recorded_at"])


def scan_chain(parts, prefix):
    streams, streamless, recoveries = {}, Counter(), []
    conns = {"open": Counter(), "reobserved": []}
    closed, finish, n_records, orphans = [], None, 0, 0
    sentinel_hits = cred_hits = 0
    native = bytearray()
    native_truncated = False
    for raw in iter_lines(parts):
        n_records += 1
        if prefix in raw:
            sentinel_hits += 1
        env = json.loads(raw)
        body = env["body"]
        if body["kind"] != "capture":
            continue
        rec = body["content"]["record"]
        kind = rec["kind"]
        key = None
        if kind in ("open", "segment", "end") or (kind == "gap" and rec.get("stream")):
            key = (rec["stream"]["producer"], rec["stream"]["stream"])
            if kind != "open" and key not in streams:
                orphans += 1
                continue
        if kind == "open":
            meta = rec.get("metadata") or {}
            w = meta.get("wire") or {}
            streams[key] = {"source": rec["source"], "path": meta.get("path"),
                            "host": meta.get("host"), "conn": w.get("connection"),
                            "exchange": w.get("exchange"), "message_index": w.get("message_index"),
                            "producer": key[0], "stream": key[1],
                            "open_wall": observed_wall(rec, env), "h": hashlib.sha256(),
                            "len": 0, "src_len": 0, "audit": Counter(), "end": None, "gaps": [],
                            "tail": b"", "sentinel": False}
        elif kind == "segment":
            s = streams[key]
            data = base64.b64decode(rec["data_b64"])
            s["h"].update(data)
            s["len"] += len(data)
            s["src_len"] += rec["source_len"]
            for span in (rec.get("audit") or {}).get("spans", []):
                s["audit"][span["kind"]] += 1
            window = s["tail"] + data
            if prefix in window:
                sentinel_hits += 1
                s["sentinel"] = True
            if any(p.search(window) for p in CRED_PATTERNS):
                cred_hits += 1
            s["tail"] = window[-64:]
            if s["source"] == "native":
                if len(native) + len(data) <= MAX_NATIVE:
                    native += data
                else:
                    native_truncated = True
        elif kind == "end":
            streams[key]["end"] = rec["reason"]
        elif kind == "gap":
            if key is not None:
                streams[key]["gaps"].append(rec["reason"])
            else:
                streamless[rec["reason"]] += 1
        elif kind == "recovery":
            recoveries.append({"previous": rec["previous_producer"], "producer": rec["producer"],
                               "reason": rec["reason"], "wall": observed_wall(rec, env)})
        elif kind == "connection":
            ev = rec.get("event") or {}
            if ev.get("kind") == "open":
                if ev.get("already_open"):
                    conns["reobserved"].append((rec["producer"], rec["connection"]))
                else:
                    conns["open"][(rec["producer"], rec["connection"])] += 1
        elif kind == "producer_closed":
            closed.append(rec["producer"])
        elif kind == "local_finish":
            finish = rec["reason"]
    for s in streams.values():
        s["sha256"] = s.pop("h").hexdigest()
        s.pop("tail")
    return {"streams": streams, "streamless": streamless, "recoveries": recoveries,
            "conns": conns, "closed": closed, "finish": finish, "records": n_records,
            "orphan_records": orphans, "parts": len(parts), "sentinel_hits": sentinel_hits,
            "cred_hits": cred_hits, "native": bytes(native), "native_truncated": native_truncated}


def item_id(e, leg):
    if e["lane"] == "sparse":
        return "sparse stream %s phase %s req %s %s" % (e.get("stream"), e.get("phase"),
                                                       e.get("req"), leg)
    if e.get("tick") is not None:
        return "%s tick %s %s" % (e["lane"], e["tick"], leg)
    return "%s %s" % (e["lane"], leg)


def expected_items(rep):
    t0 = rep["t0_wall"]
    out = []
    for e in rep["exchanges"]:
        if e["lane"] == "auth":
            continue
        a, b = t0 + e["start_s"], t0 + e["end_s"]
        for cat, leg, n, digest in (("http_request", "request", e["req_len"], e["req_sha256"]),
                                    ("http_response", "response", e["resp_len"],
                                     e["resp_sha256"])):
            out.append({"cat": cat, "path": e["path"], "len": n, "sha256": digest,
                        "sentinel": e["sentinel"], "a": a, "b": b, "ok": e["ok"],
                        "id": item_id(e, leg)})
    for m in rep["ws_messages"]:
        if m["dir"] in ("client", "server"):
            out.append({"cat": "ws_" + m["dir"], "path": "/v1/ws", "len": m["len"],
                        "sha256": m["sha256"], "sentinel": m["sentinel"],
                        "a": t0 + m["t_s"], "b": t0 + m["end_s"], "ok": m["ok"],
                        "id": "ws tick %s m %s %s" % (m["tick"], m["m"], m["dir"])})
    return out


def item_class(x):
    k = (x["cat"], x["path"])
    return k + (("sentinel", x["len"]) if x["sentinel"] else ("sha256", x["sha256"]))


def match(expected, clean, eps=MATCH_EPS_S, delta=MATCH_DELTA_S):
    """Start-anchored, order-preserving match (see the module doc).

    Returns (pairs [(item, stream)], missing items, unmatched streams, ambiguous item count).
    """
    by_class = defaultdict(list)
    for st in clean:
        k = (CAT_OF[st["source"]], st["path"])
        by_class[k + ("sha256", st["sha256"])].append(st)
        if st["audit"].get("anthropic_key") and not st["sentinel"]:
            by_class[k + ("sentinel", st["src_len"])].append(st)
    items = defaultdict(list)
    for x in expected:
        items[item_class(x)].append(x)
    used, pairs, missing, ambiguous = set(), [], [], 0
    for cls in sorted(items, key=repr):
        cands = sorted(by_class.get(cls, []), key=lambda st: st["open_wall"])
        n, j = len(cands), 0
        for x in sorted(items[cls], key=lambda x: x["a"]):
            lo, hi = x["a"] - eps, x["a"] + delta
            # A stream that opened before this item's window can serve no later item either.
            while j < n and (id(cands[j]) in used or cands[j]["open_wall"] < lo):
                j += 1
            in_win, k = [], j
            while k < n and cands[k]["open_wall"] <= hi:
                if id(cands[k]) not in used:
                    in_win.append(k)
                k += 1
            if len(in_win) > 1:
                ambiguous += 1
            if in_win:
                st = cands[in_win[0]]
                used.add(id(st))
                pairs.append((x, st))
                j = in_win[0] + 1
            else:
                missing.append(x)
    extra = [st for st in clean if id(st) not in used]
    return pairs, missing, extra, ambiguous


def sse_events_sent(e, plan):
    if e.get("sse_events") is not None:
        return e["sse_events"]
    # A report from before the tag event: the plan's own event count per request.
    if e["lane"] == "sse":
        return plan["dense"]["sse_events"]
    return plan["sparse"]["events_per_request"]


def crosscheck(out, ev=None, eps=MATCH_EPS_S, delta=MATCH_DELTA_S, prefix=None):
    """(result dict, exit code). Raises OSError/ValueError/KeyError on unreadable inputs."""
    if prefix is None:
        prefix = load_module().SENTINEL_PREFIX
    with open(os.path.join(out, "load-report.json"), encoding="utf-8") as fh:
        rep = json.load(fh)
    with open(os.path.join(out, "summary.json"), encoding="utf-8") as fh:
        summ = json.load(fh)
    trace_dir = os.path.join(out, "trace")
    parts = list_parts(trace_dir)
    chain = scan_chain(parts, prefix)
    with open(os.path.join(out, "native", "e02-native.jsonl"), "rb") as fh:
        native_file = fh.read()
    t0 = rep["t0_wall"]
    res = {"schema": "e02-early-crosscheck/2", "chain_records": chain["records"],
           "chain_parts": chain["parts"], "orphan_records": chain["orphan_records"]}
    checks = {}

    # -- the gap window
    stall = rep.get("stall") or {}
    trigger = t0 + stall["start_s"] if stall.get("start_s") is not None else None
    rec_walls = [r["wall"] for r in chain["recoveries"]]
    lo = trigger - WINDOW_BEFORE_S if trigger is not None else None
    hi = (max(rec_walls) + WINDOW_AFTER_S) if rec_walls else None
    res["gap_window"] = {"trigger_wall": trigger, "recoveries": chain["recoveries"],
                         "from_wall": lo, "to_wall": hi}

    def in_window(a, b):
        return lo is not None and hi is not None and a <= hi and b >= lo

    # -- expected items (sent) and chain streams (retained)
    expected = expected_items(rep)
    wire = [s for s in chain["streams"].values() if s["source"] in CAT_OF
            and not (s["source"].startswith("wire_") and s["path"] == "/v1/ws")]
    clean = [s for s in wire if s["end"] == "complete" and not s["gaps"]]
    unclean = [s for s in wire if not (s["end"] == "complete" and not s["gaps"])]
    pairs, missing, extra, ambiguous = match(expected, clean, eps, delta)
    matched = Counter(x["cat"] + ("_sentinel" if x["sentinel"] else "") for x, _st in pairs)
    lead = [st["open_wall"] - x["a"] for x, st in pairs]
    classes = Counter(item_class(x) for x in expected)
    miss_in = [x for x in missing if in_window(x["a"], x["b"])]
    miss_out = [x for x in missing if not in_window(x["a"], x["b"])]
    unclean_in = [s for s in unclean if in_window(s["open_wall"], s["open_wall"])]
    unclean_out = [s for s in unclean if not in_window(s["open_wall"], s["open_wall"])]
    res["retained_vs_sent"] = {
        "sent": dict(Counter(x["cat"] for x in expected)),
        "matched": dict(matched),
        "match_rule": "start-anchored, order-preserving per digest class",
        "match_eps_s": eps, "match_delta_s": delta,
        "digest_classes": len(classes),
        "items_in_shared_classes": sum(n for n in classes.values() if n > 1),
        "ambiguous_windows": ambiguous,
        "open_minus_start_s": [round(min(lead), 4), round(max(lead), 4)] if lead else None,
        "missing_in_gap_window": dict(Counter(x["cat"] for x in miss_in)),
        "missing_in_gap_window_ids": [x["id"] for x in miss_in[:50]],
        "missing_outside_window": [{k: x[k] for k in ("id", "cat", "path", "len", "sentinel", "a")}
                                   for x in miss_out[:50]],
        "missing_outside_window_count": len(miss_out),
        "chain_clean_unmatched": [{"source": s["source"], "path": s["path"],
                                   "open_wall": s["open_wall"], "sha256": s["sha256"]}
                                  for s in extra[:50]],
        "chain_clean_unmatched_count": len(extra),
        "chain_unclean_in_window": len(unclean_in),
        "chain_unclean_outside_window": [{"source": s["source"], "path": s["path"],
                                          "end": s["end"], "gaps": s["gaps"],
                                          "open_wall": s["open_wall"]} for s in unclean_out[:50]],
    }
    checks["sent_equals_retained_outside_gap"] = not miss_out and not extra and not unclean_out
    checks["load_self_check"] = bool(rep.get("ok"))

    # -- forwarded (endpoint counters) against sent
    ex = [e for e in rep["exchanges"] if e["ok"]]
    ws = [m for m in rep["ws_messages"] if m.get("ok") and m["dir"] in ("client", "server")]
    sse = [e for e in ex if e["lane"] in ("sse", "sparse")]
    c = (summ.get("endpoint") or {}).get("counters") or {}
    want = {
        "echo_requests": sum(1 for e in ex if e["lane"] == "echo"),
        "echo_bytes_in": sum(e["req_len"] for e in ex if e["lane"] == "echo"),
        "echo_bytes_out": sum(e["resp_len"] for e in ex if e["lane"] == "echo"),
        "sse_requests": len(sse),
        "sse_events": sum(sse_events_sent(e, rep["plan"]) for e in sse),
        "sse_bytes_out": sum(e["resp_len"] for e in sse),
        "bulk_requests": sum(1 for e in ex if e["lane"] == "bulk"),
        "auth_requests": sum(1 for e in ex if e["lane"] == "auth"),
        "ws_messages_in": sum(1 for m in ws if m["dir"] == "client"),
        "ws_bytes_in": sum(m["len"] for m in ws if m["dir"] == "client"),
        "ws_messages_out": sum(1 for m in ws if m["dir"] == "server"),
        "ws_bytes_out": sum(m["len"] for m in ws if m["dir"] == "server"),
    }
    res["forwarded_vs_sent"] = {k: {"sent": v, "forwarded": c.get(k)} for k, v in want.items()}
    checks["sent_equals_forwarded"] = all(c.get(k) == v for k, v in want.items())

    # -- gap evidence
    sink = summ.get("sink") or {}
    owner = summ.get("owner") or {}
    lost = {r["previous"] for r in chain["recoveries"]}
    fresh = {r["producer"] for r in chain["recoveries"]}
    opened_by_lost = {cid for (p, cid) in chain["conns"]["open"] if p in lost}
    reobs = [(p, cid) for p, cid in chain["conns"]["reobserved"] if p in fresh]
    after = [x for x in expected if hi is not None and x["a"] > hi]
    missing_ids = {id(x) for x in missing}
    after_missing = [x for x in after if id(x) in missing_ids]
    stream_gaps = Counter(g for s in chain["streams"].values() for g in s["gaps"])
    res["gap_evidence"] = {
        "stall": owner.get("stall"), "refused_queue_full": sink.get("refused_queue_full"),
        "recoveries": len(chain["recoveries"]), "lost_producers": sorted(lost),
        "stream_gaps": dict(stream_gaps), "streamless_gaps": dict(chain["streamless"]),
        "reobserved_under_fresh": len(reobs),
        "reobserved_ids_opened_under_lost": sum(1 for _p, cid in reobs if cid in opened_by_lost),
        "sent_after_window": len(after),
        "sent_after_window_missing": [x["id"] for x in after_missing[:50]],
        "lost_producer_closed": sorted(p for p in chain["closed"] if p in lost),
    }
    checks["gap_is_visible_and_recovered"] = bool(
        (owner.get("stall") or {}).get("released_by") == "queue_full"
        and any(r["reason"] == "queue_full" for r in chain["recoveries"])
        and (stream_gaps or chain["streamless"])
        and after and not after_missing)

    # -- native, line by line
    nat_streams = [s for s in chain["streams"].values() if s["source"] == "native"]
    retained_lines = chain["native"].split(b"\n")
    file_lines = native_file.split(b"\n")
    equal = redacted_ok = differ = 0
    for i, fl in enumerate(file_lines):
        rl = retained_lines[i] if i < len(retained_lines) else None
        if prefix in fl:
            if rl is not None and prefix not in rl and rl != fl:
                redacted_ok += 1
            else:
                differ += 1
        elif rl == fl:
            equal += 1
        else:
            differ += 1
    res["native"] = {"streams": len(nat_streams), "file_bytes": len(native_file),
                     "file_sha256": hashlib.sha256(native_file).hexdigest(),
                     "load_reported_sha256": (rep.get("native") or {}).get("sha256"),
                     "retained_bytes": len(chain["native"]),
                     "source_bytes": sum(s["src_len"] for s in nat_streams),
                     "lines_equal": equal, "sentinel_lines_redacted": redacted_ok,
                     "lines_differ": differ, "retained_line_count": len(retained_lines),
                     "file_line_count": len(file_lines), "truncated": chain["native_truncated"],
                     "gaps": [g for s in nat_streams for g in s["gaps"]],
                     "ends": [s["end"] for s in nat_streams]}
    checks["native_matches_file"] = (differ == 0 and len(retained_lines) == len(file_lines)
                                     and not chain["native_truncated"]
                                     and res["native"]["file_sha256"]
                                     == res["native"]["load_reported_sha256"])

    # -- sentinel and credential scans over every other file
    chain_paths = set(parts)
    scanned, file_hits = 0, []
    for root in [out] + ([ev] if ev else []):
        for base, _dirs, files in os.walk(root):
            for name in files:
                p = os.path.join(base, name)
                if p in chain_paths or os.path.islink(p):
                    continue
                if ev and root == out and p.startswith(ev + os.sep):
                    continue
                with open(p, "rb") as fh:
                    data = fh.read()
                scanned += 1
                if (prefix in data and name != "e02-native.jsonl") or any(
                        pat.search(data) for pat in CRED_PATTERNS):
                    file_hits.append(os.path.relpath(p, root))
    res["leak_scan"] = {"chain_sentinel_hits": chain["sentinel_hits"],
                        "chain_credential_hits": chain["cred_hits"], "files_scanned": scanned,
                        "file_hits": file_hits,
                        "note": "the native source file itself holds the sentinel by design"}
    checks["no_leak"] = chain["sentinel_hits"] == 0 and chain["cred_hits"] == 0 and not file_hits

    # -- the runner's own measured peaks (targets are prospective, R19)
    res["runner_peaks"] = {"max_flush_interval_ms": owner.get("max_flush_interval_ms"),
                           "max_sync_interval_ms": owner.get("max_sync_interval_ms"),
                           "sink_high_water_payload_bytes": sink.get("high_water_payload_bytes"),
                           "sink_high_water_control_bytes": sink.get("high_water_control_bytes"),
                           "resources": summ.get("resources"),
                           "final_storage": summ.get("final_storage"),
                           "integrity": summ.get("integrity")}
    assert set(checks) == set(CHECKS)
    res["checks"] = checks
    res["ok"] = all(checks.values())
    return res, 0 if res["ok"] else 1


def main(argv):
    eps, delta, rest = MATCH_EPS_S, MATCH_DELTA_S, []
    it = iter(argv)
    try:
        for a in it:
            if a == "--eps-s":
                eps = float(next(it))
            elif a == "--delta-s":
                delta = float(next(it))
            elif a.startswith("-"):
                raise ValueError(a)
            else:
                rest.append(a)
    except (StopIteration, ValueError):
        rest = []
    if len(rest) not in (1, 2) or eps < 0 or delta <= 0:
        sys.stderr.write(__doc__)
        return 64
    out = os.path.abspath(rest[0])
    ev = os.path.abspath(rest[1]) if len(rest) == 2 else None
    try:
        res, code = crosscheck(out, ev, eps, delta)
    except (OSError, ValueError, KeyError, TypeError) as e:
        sys.stderr.write("early-crosscheck: inputs unreadable: %s\n" % type(e).__name__)
        return 3
    text = json.dumps(res, indent=1, sort_keys=True, default=list)
    if ev:
        with open(os.path.join(ev, "crosscheck.json"), "w", encoding="utf-8") as fh:
            fh.write(text)
    print(json.dumps({"ok": res["ok"], "checks": res["checks"]}))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
