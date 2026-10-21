"""Behavioral checks for the E02 cross-check (F-16) on synthetic load reports and v5 chains."""

import base64
import contextlib
import datetime
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest

E02 = Path(__file__).resolve().parent.parent / "spikes" / "m2-first-release" / "e02"


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, E02 / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


XC = load("early_crosscheck", "early-crosscheck.py")
LOAD = XC.load_module()
PREFIX = LOAD.SENTINEL_PREFIX
T0 = datetime.datetime(2026, 10, 2, 12, 0, 0, tzinfo=datetime.timezone.utc).timestamp()
P1, P2, PN = "1" * 32, "2" * 32, "9" * 32
CONNS = ["c%031d" % i for i in range(3)]
TICKS = 12
STALL_S, RECOVERY_S = 4.5, 5.5  # gap window [2.5, 7.5]; ticks 8..11 come after it
LOST_IN_WINDOW = 5  # the SSE response in flight at the refusal: a queue_full gap, no End
SENTINEL_TICK = 2
EXPECTED_FAIL = {"sent_equals_retained_outside_gap", "gap_is_visible_and_recovered"}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def stamp(t):
    d = datetime.datetime.fromtimestamp(T0 + t, tz=datetime.timezone.utc)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + "%06dZ" % d.microsecond


def redact(data):
    i = data.index(PREFIX)
    j = data.index(b" ", i)
    return data[:i] + b"[REDACTED:anthropic_key]" + data[j:], i, j


class Scenario:
    """A small E02 run: dense ticks of echo, SSE and WS, a stall at 4.5 s whose queue-full
    refusal loses the proxy producer, a Recovery at 5.5 s, and a native file.

    `unique=False` reproduces the old load: every SSE request is empty and every SSE response
    the same bytes. `unique=True` is the current load: each carries its tick's tag.
    """

    def __init__(self, unique):
        self.unique = unique
        self.exchanges, self.ws, self.specs = [], [], []
        self.native_lines = []
        for k in range(TICKS):
            self.tick(k)
        self.stall()
        self.native()

    def sse_bodies(self, k):
        events = b"".join(b'data: {"i":%d,"pad":"%s"}\n\n' % (i, b"ab cd" * 20) for i in range(4))
        if not self.unique:
            return b"", events
        tag = LOAD.sse_tag(k)
        return tag, events + b"data: " + tag + b"\n\n"

    def stream(self, t_open, t_end, source, path, data, **kw):
        producer = kw.pop("producer", None) or (P1 if t_open < RECOVERY_S else P2)
        spec = {"producer": producer, "open": t_open, "end_t": t_end, "source": source,
                "path": path, "data": data, "src_len": kw.pop("src_len", len(data)),
                "spans": kw.pop("spans", []), "end": kw.pop("end", "complete"),
                "gap": kw.pop("gap", None), "conn": kw.pop("conn", CONNS[0]),
                "pieces": kw.pop("pieces", None)}
        assert not kw
        self.specs.append(spec)
        return spec

    def exchange(self, lane, path, k, start, end, req, resp, sentinel=False, conn=0, **extra):
        e = {"lane": lane, "path": path, "tick": k, "conn": conn, "start_s": start, "end_s": end,
             "ok": True, "sentinel": sentinel, "req_len": len(req), "req_sha256": sha(req),
             "resp_len": len(resp), "resp_sha256": sha(resp)}
        e.update(extra)
        self.exchanges.append(e)
        for source, t, data in (("wire_request", start + 0.003, req),
                                ("wire_response", start + 0.004, resp)):
            kw = {"conn": CONNS[conn]}
            if sentinel:
                retained, i, j = redact(data)
                kw.update(src_len=len(data), spans=[{"kind": "anthropic_key", "start": i,
                                                     "end": j}])
                data = retained
            if lane == "sse" and source == "wire_response" and k == LOST_IN_WINDOW:
                kw.update(end=None, gap="queue_full")
            self.stream(t, end, source, path, data, **kw)

    def tick(self, k):
        body = b"e02 echo %07d " % k + b"lorem ipsum " * 8
        sentinel = k == SENTINEL_TICK
        if sentinel:
            body = body + b" " + LOAD.sentinel(k) + b" tail"
        self.exchange("echo", "/v1/echo", k, k + 0.001, k + 0.05, body, body, sentinel, conn=0)
        req, resp = self.sse_bodies(k)
        self.exchange("sse", "/v1/sse", k, k + 0.002, k + 0.85, req, resp, conn=1,
                      sse_events=5 if self.unique else 4)
        for m in range(2):
            t = k + 0.06 + m * 0.02
            payload = LOAD.ws_payload(k, m)[:256]
            for d, dt in (("client", 0.002), ("server", 0.010)):
                self.ws.append({"tick": k, "m": m, "session": 1, "conn": 2, "dir": d,
                                "len": len(payload), "sha256": sha(payload), "sentinel": False,
                                "ok": True, "t_s": t, "end_s": t + 0.015, "server_frames": 1})
                self.stream(t + dt, t + 0.015, "ws_" + d, "/v1/ws", payload, conn=CONNS[2])

    def stall(self):
        self.exchanges.append({"lane": "auth", "path": "/oauth/token", "conn": None,
                               "start_s": 0.0, "end_s": 0.01, "ok": True, "sentinel": False,
                               "req_len": 14, "req_sha256": None, "resp_len": None,
                               "resp_sha256": None})
        self.exchanges.append({"lane": "bulk", "path": "/v1/bulk", "conn": 9,
                               "start_s": STALL_S, "end_s": STALL_S + 0.02, "ok": True,
                               "sentinel": False, "req_len": 0, "req_sha256": sha(b""),
                               "resp_len": 0, "resp_sha256": sha(b"")})
        self.stream(STALL_S + 0.003, STALL_S + 0.02, "wire_request", "/v1/bulk", b"")
        self.stream(STALL_S + 0.004, STALL_S + 0.02, "wire_response", "/v1/bulk", b"")

    def native(self):
        for s in range(8):
            line = b'{"e02":"native","s":%d,"pad":"%s"}' % (s, b"x" * 40)
            if s == 3:
                line = b'{"e02":"native","s":3,"pad":"a ' + LOAD.sentinel(45) + b' b"}'
            self.native_lines.append(line + b"\n")
        retained = b"".join(redact(x)[0] if PREFIX in x else x for x in self.native_lines)
        self.stream(0.0, TICKS, "native", None, retained, producer=PN,
                    src_len=sum(map(len, self.native_lines)),
                    spans=[{"kind": "anthropic_key", "start": 0, "end": 1}],
                    pieces=[redact(x)[0] if PREFIX in x else x for x in self.native_lines])

    # -- mutations

    def drop(self, source, path, k):
        """Remove every record of tick k's stream: a loss the chain does not show."""
        lane = [e for e in self.exchanges if e["path"] == path and e.get("tick") == k][0]
        t = lane["start_s"] + (0.003 if source == "wire_request" else 0.004)
        before = len(self.specs)
        self.specs = [s for s in self.specs
                      if not (s["source"] == source and s["path"] == path
                              and abs(s["open"] - t) < 1e-9)]
        assert len(self.specs) == before - 1

    # -- the run directory

    def records(self):
        counters = {}
        opened = []
        for spec in sorted(self.specs, key=lambda s: s["open"]):
            counters[spec["producer"]] = counters.get(spec["producer"], 0) + 1
            opened.append(dict(spec, stream=counters[spec["producer"]]))
        ev = []
        for i, conn in enumerate(CONNS):
            ev.append((0.0001 * (i + 1), {"kind": "connection", "producer": P1, "connection": conn,
                                          "event": {"kind": "open", "alpn": "http/1.1",
                                                    "already_open": False}}))
        ev.append((RECOVERY_S, {"kind": "recovery", "previous_producer": P1, "producer": P2,
                                "reason": "queue_full"}))
        for i, conn in enumerate(CONNS):
            ev.append((RECOVERY_S + 0.0001 * (i + 1),
                       {"kind": "connection", "producer": P2, "connection": conn,
                        "event": {"kind": "open", "alpn": "http/1.1", "already_open": True}}))
        for s in opened:
            sid = {"producer": s["producer"], "stream": s["stream"]}
            wire = None if s["source"] == "native" else {
                "connection": s["conn"], "exchange": s["stream"], "message_index": None}
            ev.append((s["open"], {"kind": "open", "stream": sid, "source": s["source"],
                                   "metadata": {"wire": wire, "label": None,
                                                "protocol_hint": None, "method": None,
                                                "host": "api.e00.localhost", "path": s["path"],
                                                "status": None, "content_type": None}}))
            pieces = s["pieces"] or ([s["data"][:len(s["data"]) // 2],
                                      s["data"][len(s["data"]) // 2:]] if s["data"] else [])
            offset, src_left = 0, s["src_len"]
            for n, piece in enumerate(pieces):
                src = src_left if n == len(pieces) - 1 else len(piece)
                src_left -= src
                t = s["open"] + (n + 1) * (s["end_t"] - s["open"]) / (len(pieces) + 1)
                ev.append((t, {"kind": "segment", "stream": sid, "part": n,
                               "source_offset": offset, "source_len": src,
                               "data_b64": base64.b64encode(piece).decode(),
                               "audit": {"spans": s["spans"] if n == 0 else []}}))
                offset += src
            if s["gap"]:
                ev.append((RECOVERY_S - 0.0001, {"kind": "gap", "stream": sid,
                                                 "reason": s["gap"], "lost_bytes": None}))
            if s["end"]:
                ev.append((s["end_t"], {"kind": "end", "stream": sid, "reason": s["end"]}))
        ev.append((STALL_S + 0.5, {"kind": "gap", "stream": None, "reason": "queue_full",
                                   "lost_bytes": None}))
        ev.append((TICKS + 1, {"kind": "producer_closed", "producer": P2, "final_seq": 1}))
        ev.append((TICKS + 2, {"kind": "local_finish", "reason": "participant_stop"}))
        ev.sort(key=lambda x: x[0])
        lines = [{"protocol_version": 5, "entry_id": "entry-e02", "source": "daemon",
                  "recorded_at": stamp(-1), "seq": 0, "prev": "0" * 64, "hash": "0" * 64,
                  "body": {"kind": "baseline", "content": {"manifest_sha256": "0" * 64,
                                                           "file_count": 0}}}]
        for seq, (t, rec) in enumerate(ev, start=1):
            clock = rec.get("producer") or (rec.get("stream") or {}).get("producer") or "daemon"
            rec = dict(rec)
            obs = {"wall": stamp(t), "mono_ns": int(t * 1e9), "clock_id": clock}
            rec = dict([("kind", rec.pop("kind"))] + [("observed", obs)] + list(rec.items()))
            lines.append({"protocol_version": 5, "entry_id": "entry-e02", "source": "daemon",
                          "recorded_at": stamp(t + 0.001), "seq": seq, "prev": "0" * 64,
                          "hash": "0" * 64,
                          "body": {"kind": "capture", "content": {"record": rec}}})
        return [(json.dumps(x, separators=(",", ":")) + "\n").encode() for x in lines]

    def report(self):
        native = b"".join(self.native_lines)
        return {"schema": "e02-early-load/1", "t0_wall": T0, "ok": True,
                "plan": {"dense": {"sse_events": 4}, "sparse": {"events_per_request": 10}},
                "stall": {"sched_s": STALL_S, "start_s": STALL_S, "end_s": STALL_S + 0.02,
                          "ok": True},
                "native": {"bytes": len(native), "sha256": sha(native)},
                "exchanges": self.exchanges, "ws_messages": self.ws}

    def counters(self):
        ex = [e for e in self.exchanges if e["ok"]]
        sse = [e for e in ex if e["lane"] == "sse"]
        ws = self.ws
        return {"echo_requests": sum(e["lane"] == "echo" for e in ex),
                "echo_bytes_in": sum(e["req_len"] for e in ex if e["lane"] == "echo"),
                "echo_bytes_out": sum(e["resp_len"] for e in ex if e["lane"] == "echo"),
                "sse_requests": len(sse), "sse_events": sum(e["sse_events"] for e in sse),
                "sse_bytes_out": sum(e["resp_len"] for e in sse),
                "bulk_requests": 1, "auth_requests": 1,
                "ws_messages_in": sum(m["dir"] == "client" for m in ws),
                "ws_bytes_in": sum(m["len"] for m in ws if m["dir"] == "client"),
                "ws_messages_out": sum(m["dir"] == "server" for m in ws),
                "ws_bytes_out": sum(m["len"] for m in ws if m["dir"] == "server")}

    def write(self, out, parts=1, summary_extra=None):
        out = Path(out)
        (out / "trace").mkdir(parents=True)
        (out / "native").mkdir()
        lines = self.records()
        # Rotated parts (D55): `chain.jsonl` at offset 0, then `chain.<20 digits>.jsonl`.
        cuts = [len(lines) * i // parts for i in range(parts + 1)]
        offset = 0
        for i in range(parts):
            data = b"".join(lines[cuts[i]:cuts[i + 1]])
            name = "chain.jsonl" if i == 0 else "chain.%020d.jsonl" % offset
            (out / "trace" / name).write_bytes(data)
            offset += len(data)
        (out / "native" / "e02-native.jsonl").write_bytes(b"".join(self.native_lines))
        (out / "load-report.json").write_text(json.dumps(self.report()))
        summary = {"endpoint": {"counters": self.counters()},
                   "owner": {"stall": {"released_by": "queue_full", "held_ms": 900}},
                   "sink": {"refused_queue_full": 1}}
        summary.update(summary_extra or {})
        (out / "summary.json").write_text(json.dumps(summary))
        return out


def old_matcher_missing(expected, clean):
    """The frozen v1 rule (F-16): Open in [start - 1 s, end + 1 s], nearest start, greedy."""
    used, missing = set(), []
    for x in sorted(expected, key=lambda x: x["a"]):
        cands = [st for st in clean if id(st) not in used and st["path"] == x["path"]
                 and XC.CAT_OF[st["source"]] == x["cat"]
                 and (st["src_len"] == x["len"] and st["audit"].get("anthropic_key")
                      and not st["sentinel"] if x["sentinel"] else st["sha256"] == x["sha256"])
                 and x["a"] - 1.0 <= st["open_wall"] <= x["b"] + 1.0]
        if cands:
            used.add(id(min(cands, key=lambda st: abs(st["open_wall"] - x["a"]))))
        else:
            missing.append(x["id"])
    return missing


class CrosscheckTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.n = 0

    def run_dir(self, scenario, **kw):
        self.n += 1
        return str(scenario.write(Path(self.tmp.name) / ("run%d" % self.n), **kw))

    def check(self, scenario, **kw):
        out = self.run_dir(scenario, **{k: kw.pop(k) for k in ("parts", "summary_extra")
                                        if k in kw})
        res, code = XC.crosscheck(out, prefix=PREFIX, **kw)
        return res, code

    def failed(self, res):
        return {k for k, v in res["checks"].items() if not v}

    def missing_ids(self, res):
        r = res["retained_vs_sent"]
        return [x["id"] for x in r["missing_outside_window"]], r["missing_in_gap_window_ids"]

    # -- clean runs

    def test_clean_run_passes_all_six_checks(self):
        for unique in (True, False):
            with self.subTest(unique=unique):
                res, code = self.check(Scenario(unique))
                self.assertEqual(code, 0, res["checks"])
                self.assertEqual(set(res["checks"]), set(XC.CHECKS))
                self.assertEqual(self.missing_ids(res), ([], ["sse tick 5 response"]))
                self.assertEqual(res["retained_vs_sent"]["chain_unclean_in_window"], 1)
                self.assertEqual(res["retained_vs_sent"]["ambiguous_windows"], 0)
                self.assertEqual(res["native"]["sentinel_lines_redacted"], 1)
                self.assertEqual(res["retained_vs_sent"]["matched"]["http_response_sentinel"], 1)

    def test_loss_in_window_no_longer_cascades_out_of_it(self):
        # The 30-minute run (F-16): the in-flight SSE response lost at the refusal, followed by
        # identical responses. The old rule pinned the loss on the phase's last tick.
        s = Scenario(unique=False)
        out = self.run_dir(s)
        chain = XC.scan_chain(XC.list_parts(os.path.join(out, "trace")), PREFIX)
        clean = [st for st in chain["streams"].values() if st["source"] in XC.CAT_OF
                 and st["end"] == "complete" and not st["gaps"]]
        self.assertEqual(old_matcher_missing(XC.expected_items(s.report()), clean),
                         ["sse tick 11 response"])
        res, code = XC.crosscheck(out, prefix=PREFIX)
        self.assertEqual(code, 0, res["checks"])

    def test_unique_bodies_put_every_item_in_its_own_digest_class(self):
        res, _ = self.check(Scenario(unique=True))
        # Only the sentinel request and response share a class (by length), each with itself.
        self.assertEqual(res["retained_vs_sent"]["items_in_shared_classes"], 0)
        res, _ = self.check(Scenario(unique=False))
        # The old load: the empty SSE requests and the identical SSE responses.
        self.assertEqual(res["retained_vs_sent"]["items_in_shared_classes"], 2 * TICKS)

    # -- the F-16 mutant

    def test_identical_body_lost_outside_window_is_pinpointed_without_cascade(self):
        s = Scenario(unique=False)
        s.drop("wire_response", "/v1/sse", 9)
        res, code = self.check(s)
        self.assertEqual(code, 1)
        self.assertEqual(self.failed(res), EXPECTED_FAIL)
        self.assertEqual(self.missing_ids(res), (["sse tick 9 response"],
                                                 ["sse tick 5 response"]))
        self.assertEqual(res["gap_evidence"]["sent_after_window_missing"],
                         ["sse tick 9 response"])
        self.assertEqual(res["retained_vs_sent"]["chain_clean_unmatched_count"], 0)

    def test_unique_body_lost_outside_window_is_pinpointed(self):
        for source, leg in (("wire_response", "response"), ("wire_request", "request")):
            with self.subTest(source=source):
                s = Scenario(unique=True)
                s.drop(source, "/v1/sse", 9)
                res, code = self.check(s)
                self.assertEqual(code, 1)
                self.assertEqual(self.failed(res), EXPECTED_FAIL)
                self.assertEqual(self.missing_ids(res)[0], ["sse tick 9 %s" % leg])

    def test_loss_before_the_window_fails_only_the_retention_check(self):
        s = Scenario(unique=False)
        s.drop("wire_response", "/v1/sse", 1)
        res, _ = self.check(s)
        self.assertEqual(self.failed(res), {"sent_equals_retained_outside_gap"})
        self.assertEqual(self.missing_ids(res)[0], ["sse tick 1 response"])

    def test_changed_bytes_leave_an_unmatched_stream_and_a_missing_item(self):
        s = Scenario(unique=True)
        spec = [x for x in s.specs if x["source"] == "ws_server" and 8 < x["open"] < 9][0]
        spec["data"] = spec["data"][:-1] + b"!"
        res, _ = self.check(s)
        self.assertEqual(self.failed(res), EXPECTED_FAIL)
        self.assertEqual(self.missing_ids(res)[0], ["ws tick 8 m 0 server"])
        self.assertEqual(res["retained_vs_sent"]["chain_clean_unmatched_count"], 1)

    def test_match_is_start_anchored_with_a_configurable_bound(self):
        s = Scenario(unique=True)
        spec = [x for x in s.specs if x["source"] == "wire_response" and x["path"] == "/v1/echo"
                and 9 < x["open"] < 10][0]
        spec["open"] += 0.4  # inside [start, end + 1 s], outside start + 250 ms
        spec["end_t"] = spec["open"] + 0.01
        res, _ = self.check(s)
        self.assertIn("sent_equals_retained_outside_gap", self.failed(res))
        self.assertEqual(self.missing_ids(res)[0], ["echo tick 9 response"])
        res, code = self.check(s, delta=0.5)
        self.assertEqual(code, 0, res["checks"])
        # An Open before the start beyond the clock allowance does not match either.
        s = Scenario(unique=True)
        spec = [x for x in s.specs if x["source"] == "ws_client" and 9 < x["open"] < 10][0]
        spec["open"] -= 0.02
        res, _ = self.check(s)
        self.assertEqual(self.missing_ids(res)[0], ["ws tick 9 m 0 client"])
        res, code = self.check(s, eps=0.03)
        self.assertEqual(code, 0, res["checks"])

    def test_forwarded_counter_off_by_one_fails_only_forwarding(self):
        s = Scenario(unique=True)
        counters = s.counters()
        counters["sse_events"] += 1
        res, _ = self.check(s, summary_extra={"endpoint": {"counters": counters}})
        self.assertEqual(self.failed(res), {"sent_equals_forwarded"})

    def test_native_byte_changed_fails_only_native(self):
        s = Scenario(unique=True)
        native = [x for x in s.specs if x["source"] == "native"][0]
        native["pieces"][5] = native["pieces"][5].replace(b"x", b"y", 1)
        res, _ = self.check(s)
        self.assertEqual(self.failed(res), {"native_matches_file"})

    # -- rotated chain parts

    def test_rotated_parts_read_in_order_as_one_chain(self):
        s = Scenario(unique=False)
        one, _ = self.check(s)
        three, code = self.check(s, parts=3)
        self.assertEqual(code, 0, three["checks"])
        self.assertEqual(three["chain_parts"], 3)
        self.assertEqual(three["chain_records"], one["chain_records"])
        self.assertEqual(three["retained_vs_sent"], one["retained_vs_sent"])
        out = Path(self.run_dir(s, parts=3))
        names = [os.path.basename(p) for p in XC.list_parts(str(out / "trace"))]
        self.assertEqual(names[0], "chain.jsonl")
        self.assertEqual(len(names), 3)
        self.assertLess(int(names[1][6:26]), int(names[2][6:26]))

    def test_a_missing_middle_part_is_not_silently_passed(self):
        s = Scenario(unique=True)
        out = Path(self.run_dir(s, parts=3))
        os.remove(XC.list_parts(str(out / "trace"))[1])
        res, code = XC.crosscheck(str(out), prefix=PREFIX)
        self.assertEqual(code, 1)
        self.assertIn("sent_equals_retained_outside_gap", self.failed(res))

    def test_part_names_outside_the_grammar_are_ignored(self):
        s = Scenario(unique=True)
        out = Path(self.run_dir(s))
        (out / "trace" / "chain.00000000000000000000.jsonl").write_text("not json\n")
        (out / "trace" / "chain.123.jsonl").write_text("not json\n")
        res, code = XC.crosscheck(str(out), prefix=PREFIX)
        self.assertEqual(code, 0, res["checks"])

    # -- leaks

    def test_sentinel_in_retained_bytes_fails_no_leak(self):
        s = Scenario(unique=True)
        spec = [x for x in s.specs if x["source"] == "wire_request"
                and x["path"] == "/v1/echo" and SENTINEL_TICK < x["open"] < SENTINEL_TICK + 1][0]
        sent = [e for e in s.exchanges if e.get("tick") == SENTINEL_TICK and e["lane"] == "echo"]
        body = b"e02 echo %07d " % SENTINEL_TICK + b"lorem ipsum " * 8 + b" " \
            + LOAD.sentinel(SENTINEL_TICK) + b" tail"
        self.assertEqual(sha(body), sent[0]["req_sha256"])
        spec["data"], spec["spans"] = body, []  # the redactor missed it
        res, _ = self.check(s)
        self.assertIn("no_leak", self.failed(res))
        self.assertGreaterEqual(res["leak_scan"]["chain_sentinel_hits"], 1)

    def test_sentinel_split_across_segments_is_still_found(self):
        s = Scenario(unique=True)
        spec = [x for x in s.specs if x["source"] == "ws_client" and 7 < x["open"] < 8][0]
        leak = b"pad " + LOAD.sentinel(1) + b" pad"
        cut = leak.index(PREFIX) + 6
        spec["pieces"] = [leak[:cut], leak[cut:]]
        res, _ = self.check(s)
        self.assertIn("no_leak", self.failed(res))

    def test_sentinel_or_credential_in_another_file_fails_only_no_leak(self):
        for extra in ({"note": PREFIX.decode() + "x" * 30},
                      {"note": "Proxy-Authorization: Basic abc"}):
            with self.subTest(extra=list(extra.values())[0][:12]):
                res, _ = self.check(Scenario(unique=True), summary_extra=extra)
                self.assertEqual(self.failed(res), {"no_leak"})
                self.assertEqual(res["leak_scan"]["file_hits"], ["summary.json"])

    # -- the command

    def test_main_exit_codes_and_evidence_file(self):
        out = self.run_dir(Scenario(unique=True))
        ev = Path(self.tmp.name) / "evidence"
        ev.mkdir()
        with contextlib.redirect_stdout(io.StringIO()) as stdout:
            self.assertEqual(XC.main([out, str(ev)]), 0)
        self.assertTrue(json.loads(stdout.getvalue())["ok"])
        self.assertTrue(json.loads((ev / "crosscheck.json").read_text())["ok"])
        s = Scenario(unique=True)
        s.drop("wire_response", "/v1/sse", 9)
        bad = self.run_dir(s)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(XC.main(["--delta-s", "0.25", "--eps-s", "0.01", bad]), 1)
        os.remove(os.path.join(out, "summary.json"))
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(XC.main([out]), 3)
            self.assertEqual(XC.main([]), 64)
            self.assertEqual(XC.main(["--delta-s"]), 64)
            self.assertEqual(XC.main(["--bogus", out]), 64)


class LoadUniquenessTest(unittest.TestCase):
    def test_selfcheck_passes(self):
        with contextlib.redirect_stdout(io.StringIO()) as stdout:
            self.assertEqual(LOAD.selfcheck(), 0)
        self.assertEqual(json.loads(stdout.getvalue())["failures"], [])

    def test_totals_are_unchanged_by_the_tags(self):
        full = LOAD.plan("full")
        self.assertEqual(full["total_bytes"], 125706240)
        self.assertEqual(full["dense"]["sse_down"], 32768)
        self.assertEqual(full["sparse"]["request_bytes"], 2560)

    def test_sse_response_with_tag_validates_only_its_own_tag(self):
        size, n = LOAD.SPARSE_SIZE, 10
        events = b"".join(b'data: {"i":%d,"pad":"%s"}\n\n' % (i, b"ab c" * (size // 4))
                          for i in range(n))
        tag = LOAD.sparse_tag(3, 1, 2, n)
        body = events + b"data: " + tag + b"\n\n"
        self.assertEqual(len(body), 2560)
        self.assertTrue(LOAD.sse_ok(body, n, size, tail=tag))
        self.assertFalse(LOAD.sse_ok(body, n, size, tail=LOAD.sparse_tag(3, 1, 1, n)))
        self.assertFalse(LOAD.sse_ok(body, n, size))

    def test_every_tagged_body_differs_in_digest(self):
        bodies = [LOAD.echo_body(k) for k in range(50)] + [LOAD.ws_payload(k, m) for k in
                                                           range(50) for m in range(2)]
        self.assertEqual(len({sha(b) for b in bodies}), len(bodies))
        self.assertEqual({len(b) for b in bodies}, {LOAD.ECHO_BYTES})


if __name__ == "__main__":
    unittest.main()
