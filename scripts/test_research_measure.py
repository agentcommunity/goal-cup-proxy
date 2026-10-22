"""Behavioral checks for the WP-0 chain measurer on synthetic v5 chains; no real traces."""

import base64
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "research_measure", Path(__file__).with_name("research-measure.py"))
MEASURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MEASURE)

SENTINEL = "SENTINEL-sk-ant-api03-DO-NOT-PRINT-7f3a"
PRODUCER = "13579bdf02468ace13579bdf02468ace"
NATIVE = "9d8c7b6a5f4e3d2c1b0a998877665544"
CONNECTION = "0f1e2d3c4b5a69788796a5b4c3d2e1f0"


def stamp(seconds):
    whole, frac = divmod(round(seconds * 1000), 1000)
    minutes, sec = divmod(whole, 60)
    hours, minutes = divmod(minutes, 60)
    text = "2026-10-02T%02d:%02d:%02d" % (12 + hours, minutes, sec)
    return text + ("Z" if not frac else ".%03dZ" % frac)


def observed(t, clock):
    return {"wall": stamp(t), "mono_ns": int(t * 1e9), "clock_id": clock}


def meta(source, label, hint=None, host=None, path=None, wire=None):
    return {"wire": wire, "label": label, "protocol_hint": hint, "method": None, "host": host,
            "path": path, "status": None, "content_type": None}


def open_rec(producer, stream, t, source, metadata):
    return {"kind": "open", "stream": {"producer": producer, "stream": stream},
            "observed": observed(t, producer), "source": source, "metadata": metadata}


def segment(producer, stream, t, payload, offset=0):
    return {"kind": "segment", "stream": {"producer": producer, "stream": stream}, "part": 0,
            "source_offset": offset, "source_len": len(payload), "first": observed(t, producer),
            "last": observed(t, producer),
            "data_b64": base64.b64encode(payload).decode(), "audit": {"spans": []}}


def end(producer, stream, t, reason="complete"):
    return {"kind": "end", "stream": {"producer": producer, "stream": stream},
            "observed": observed(t, producer), "reason": reason}


class Chain:
    """Compact v5 envelope lines, in the shape of the v5 corpus fixtures."""

    def __init__(self, entry="entry-v5-corpus"):
        self.lines, self.entry = [], entry

    def add(self, t, body):
        seq = len(self.lines)
        envelope = {"protocol_version": 5, "entry_id": self.entry, "source": "daemon",
                    "recorded_at": stamp(t), "seq": seq, "prev": "%064x" % seq,
                    "hash": "%064x" % (seq + 1), "body": body}
        self.lines.append(json.dumps(envelope, separators=(",", ":")).encode() + b"\n")

    def capture(self, t, record):
        self.add(t, {"kind": "capture", "content": {"record": record}})

    def bytes(self):
        return b"".join(self.lines)


def mixed_chain(entry="entry-v5-corpus", secret=b"hello"):
    chain = Chain(entry)
    chain.add(0, {"kind": "baseline", "content": {"manifest_sha256": "d" * 64, "file_count": 0}})
    chain.capture(1, {"kind": "connection", "producer": PRODUCER, "connection": CONNECTION,
                      "observed": observed(1, PRODUCER),
                      "event": {"kind": "open", "alpn": "http/1.1", "already_open": False}})
    wire = {"connection": CONNECTION, "exchange": 1, "message_index": None}
    chain.capture(1, open_rec(PRODUCER, 1, 1, "wire_request", meta(
        "wire_request", "POST /v1/responses", "openai-responses", "api.example.com",
        "/v1/responses", wire)))
    chain.capture(1, segment(PRODUCER, 1, 1, secret))          # 5 decoded bytes by default
    chain.capture(1, segment(PRODUCER, 1, 1, b"abcd", 5))      # 4: padding "=="
    chain.capture(1, end(PRODUCER, 1, 1))
    chain.capture(2, open_rec(PRODUCER, 2, 2, "wire_response", meta(
        "wire_response", "200", "openai-responses", "api.example.com", "/v1/responses", wire)))
    chain.capture(2, segment(PRODUCER, 2, 2, b"abcdef"))       # 6: no padding
    chain.capture(2, {"kind": "gap", "stream": {"producer": PRODUCER, "stream": 2},
                      "observed": observed(2, PRODUCER), "reason": "decode_failed",
                      "lost_bytes": None})
    chain.capture(2, end(PRODUCER, 2, 2, "interrupted"))
    chain.capture(3, {"kind": "gap", "stream": None, "observed": observed(3, "daemon-clock"),
                      "reason": "queue_full", "lost_bytes": None})
    return chain


class MeasureCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.trace = os.path.join(self.tmp.name, "trace-" + SENTINEL)
        os.mkdir(self.trace)

    def write(self, name, data):
        with open(os.path.join(self.trace, name), "wb") as stream:
            stream.write(data)

    def run_cli(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stderr(io.StringIO()):
            code = MEASURE.main(list(args), out=out)
        return code, out.getvalue()


class AccountingTest(MeasureCase):
    def test_kinds_streams_and_bytes_add_up(self):
        chain = mixed_chain()
        self.write("chain.jsonl", chain.bytes())
        report = MEASURE.measure(self.trace, compress=False)
        self.assertEqual(report["records"], len(chain.lines))
        self.assertEqual(report["bytes"], len(chain.bytes()))
        kinds = report["kinds"]
        self.assertEqual(kinds["segment"]["records"], 3)
        self.assertEqual(kinds["open"]["records"], 2)
        self.assertEqual(kinds["end"]["records"], 2)
        self.assertEqual(kinds["gap"]["records"], 2)
        self.assertEqual(kinds["connection.open"]["records"], 1)
        self.assertEqual(kinds["baseline"]["records"], 1)
        self.assertEqual(sum(k["bytes"] for k in kinds.values()), report["bytes"])
        self.assertAlmostEqual(sum(k["share"] for k in kinds.values()), 1.0, places=3)
        self.assertEqual(report["gap_reasons"], {"decode_failed": 1, "queue_full": 1})
        streams = report["streams"]
        request = streams["wire_request/openai-responses"]
        response = streams["wire_response/openai-responses"]
        self.assertEqual((request["streams"], request["records"], request["segments"]), (1, 4, 2))
        self.assertEqual((response["streams"], response["records"], response["segments"]),
                         (1, 4, 1))  # the stream gap counts with its stream
        self.assertEqual(streams["(no stream)"]["records"], 3)
        self.assertEqual(sum(s["bytes"] for s in streams.values()), report["bytes"])

    def test_envelope_versus_payload(self):
        chain = mixed_chain()
        self.write("chain.jsonl", chain.bytes())
        report = MEASURE.measure(self.trace, compress=False)
        request = report["streams"]["wire_request/openai-responses"]
        self.assertEqual(request["decoded_bytes"], 9)
        self.assertEqual(request["b64_bytes"], len("aGVsbG8=") + len("YWJjZA=="))
        self.assertEqual(request["envelope_bytes"], request["bytes"] - 9)
        segment_lines = len(chain.lines[3]) + len(chain.lines[4])
        self.assertEqual(request["segment_bytes"], segment_lines)
        self.assertEqual(request["segment_overhead_bytes"], segment_lines - 9)
        self.assertEqual(report["streams"]["wire_response/openai-responses"]["decoded_bytes"], 6)
        totals = report["segments"]
        self.assertEqual(totals["decoded_bytes"], 15)
        self.assertEqual(totals["envelope_bytes"], totals["line_bytes"] - 15)
        self.assertAlmostEqual(totals["line_bytes_per_payload_byte"],
                               totals["line_bytes"] / 15, places=2)

    def test_quiet_gap_distribution(self):
        chain = Chain()
        for t in (0, 0, 0.5, 2.5, 8.5, 23.5, 123.5, 623.5, 623.5, 624.5):
            chain.add(t, {"kind": "baseline", "content": {}})
        self.write("chain.jsonl", chain.bytes())
        q = MEASURE.measure(self.trace, compress=False)["quiet_gaps"]
        # Intervals: 0, 0.5, 2, 6, 15, 100, 500, 0, 1.
        self.assertEqual(q["intervals"], 9)
        self.assertEqual(q["zero"], 2)
        counts = {t["at_least_s"]: t["count"] for t in q["thresholds"]}
        self.assertEqual(counts, {1: 6, 5: 4, 15: 3, 30: 2, 60: 2, 300: 1})
        totals = {t["at_least_s"]: t["seconds_total"] for t in q["thresholds"]}
        self.assertEqual(totals[60], 600)
        self.assertEqual(q["p50_s"], 2)
        self.assertEqual(q["p90_s"], 500)
        self.assertEqual(q["p99_s"], 500)
        self.assertEqual(q["max_s"], 500)

    def test_span_rate_and_clock_going_back(self):
        chain = Chain()
        for t in (0, 3600, 1800, 7200):
            chain.add(t, {"kind": "baseline", "content": {}})
        self.write("chain.jsonl", chain.bytes())
        report = MEASURE.measure(self.trace, compress=False)
        self.assertEqual(report["wall_span_s"], 7200)
        self.assertEqual(report["bytes_per_hour"], round(len(chain.bytes()) / 2))
        self.assertEqual(report["quiet_gaps"]["clock_went_back"], 1)

    def test_rotated_parts_read_in_start_order(self):
        chain = mixed_chain()
        lines = chain.lines
        first, second = b"".join(lines[:4]), b"".join(lines[4:8])
        third = b"".join(lines[8:])
        # Written newest first; `chain.jsonl` sorts after `chain.0...` by name.
        self.write(MEASURE_PART(len(first) + len(second)), third)
        self.write(MEASURE_PART(len(first)), second)
        self.write("chain.jsonl", first)
        self.write("chain.00000000000000000000.jsonl", b"not a part\n")
        self.write("chain.123.jsonl", b"not a part\n")
        self.write("acks.jsonl", b"{}\n")
        report = MEASURE.measure(self.trace, compress=False)
        self.assertEqual(report["parts"], 3)
        self.assertTrue(report["parts_contiguous"])
        self.assertEqual((report["first_seq"], report["last_seq"]), (0, len(lines) - 1))
        self.assertEqual(report["records"], len(lines))
        self.assertEqual(report["unparsed_lines"], 0)
        self.assertEqual(report["quiet_gaps"]["clock_went_back"], 0)
        whole = MEASURE.measure(self._whole(chain), compress=False)
        self.assertEqual(report["streams"], whole["streams"])

    def _whole(self, chain):
        other = os.path.join(self.tmp.name, "whole")
        os.mkdir(other)
        with open(os.path.join(other, "chain.jsonl"), "wb") as stream:
            stream.write(chain.bytes())
        return other

    def test_pruned_head_and_missing_part_are_reported(self):
        chain = mixed_chain()
        lines = chain.lines
        head = len(b"".join(lines[:4]))
        middle = len(b"".join(lines[4:6]))
        self.write(MEASURE_PART(head), b"".join(lines[4:6]))
        self.write(MEASURE_PART(head + middle + 1), b"".join(lines[6:]))
        report = MEASURE.measure(self.trace, compress=False)
        self.assertEqual(report["first_part_start"], head)
        self.assertFalse(report["parts_contiguous"])
        self.assertEqual(report["first_seq"], 4)
        self.assertGreater(report["streams"]["(stream not opened here)"]["records"], 0)

    def test_torn_tail_and_oversize_line_counted_not_parsed(self):
        chain = mixed_chain()
        self.write("chain.jsonl", chain.bytes() + b"x" * (MEASURE.LINE_LIMIT * 2 + 5) + b"\n"
                   + b'{"protocol_version":5,"seq":')
        report = MEASURE.measure(self.trace, compress=False)
        self.assertEqual(report["records"], len(chain.lines))
        self.assertEqual(report["unparsed_lines"], 2)
        self.assertEqual(report["unterminated_lines"], 1)
        self.assertEqual(report["bytes"], os.path.getsize(os.path.join(self.trace, "chain.jsonl")))

    def test_snapshot_generations_sizes(self):
        chain = Chain()
        sizes = [100, 250, 400]
        for gen, size in enumerate(sizes, 1):
            label = "session:session_%s.json#g%d" % (SENTINEL, gen)
            chain.capture(gen, open_rec(NATIVE, gen, gen, "native", meta("native", label)))
            chain.capture(gen, segment(NATIVE, gen, gen, b"a" * (size - 40)))
            chain.capture(gen, segment(NATIVE, gen, gen, b"b" * 40, size - 40))
            chain.capture(gen, end(NATIVE, gen, gen))
        label = "rollout:x.jsonl#g1"
        chain.capture(9, open_rec(NATIVE, 9, 9, "native", meta("native", label, "codex-jsonl")))
        chain.capture(9, segment(NATIVE, 9, 9, b"z" * 7))
        self.write("chain.jsonl", chain.bytes())
        native = MEASURE.measure(self.trace, compress=False)["native"]
        self.assertEqual(native["files"], 2)
        self.assertEqual(native["generations"], 4)
        self.assertEqual(native["multi_generation_files"], 1)
        top = native["top_multi_generation"][0]
        self.assertEqual(top["bytes_per_generation"], sizes)
        self.assertEqual(top["ended_complete"], 3)
        self.assertEqual((top["first_gen_bytes"], top["last_gen_bytes"]), (100, 400))
        self.assertEqual(top["total_bytes"], 750)


def MEASURE_PART(start):
    return "chain.%020d.jsonl" % start


class NoContentTest(MeasureCase):
    def test_output_never_carries_payload_ids_labels_or_paths(self):
        secret = SENTINEL.encode()
        chain = mixed_chain(entry="entry-" + SENTINEL, secret=secret)
        chain.capture(5, open_rec(NATIVE, 3, 5, "native", meta(
            "native", "main:%s/x.jsonl#g1" % SENTINEL, SENTINEL)))
        chain.capture(5, segment(NATIVE, 3, 5, secret * 3))
        chain.capture(5, {"kind": SENTINEL, "stream": None})
        chain.capture(6, {"kind": "gap", "stream": None, "observed": observed(6, "d"),
                          "reason": SENTINEL, "lost_bytes": None})
        self.write("chain.jsonl", chain.bytes() + secret + b"\n")
        forbidden = [SENTINEL, SENTINEL.lower(), base64.b64encode(secret).decode()[:20],
                     PRODUCER, NATIVE, CONNECTION, "api.example.com", "/v1/responses",
                     self.tmp.name]
        for args in ([self.trace], [self.trace, "--json"]):
            code, output = self.run_cli(*args)
            self.assertEqual(code, 0)
            for word in forbidden:
                self.assertNotIn(word, output)
        report = json.loads(self.run_cli(self.trace, "--json")[1])["traces"][0]
        self.assertIn("native/other", report["streams"])
        self.assertIn("other", report["kinds"])

    def test_missing_dir_does_not_echo_its_path(self):
        code, output = self.run_cli(os.path.join(self.tmp.name, SENTINEL))
        self.assertEqual(code, 1)
        self.assertNotIn(SENTINEL, output)
        self.assertIn("error", output)


class CompressionTest(MeasureCase):
    def setUp(self):
        super().setUp()
        self.write("chain.jsonl", mixed_chain().bytes() * 20)
        self.bin = os.path.join(self.tmp.name, "bin")
        os.mkdir(self.bin)

    def test_zstd_absent_falls_back_to_labelled_proxy(self):
        with patch.dict(os.environ, {"PATH": self.bin}):
            code, output = self.run_cli(self.trace)
            report = json.loads(self.run_cli(self.trace, "--json")[1])["traces"][0]
        self.assertEqual(code, 0)
        self.assertIn("zstd not found", output)
        comp = report["compression"]
        self.assertEqual(comp["mode"], "fallback")
        self.assertEqual(len(comp["results"]), 2)
        for result in comp["results"]:
            self.assertIn("rough proxy", result["method"])
            self.assertGreater(result["ratio"], 1)

    def test_zstd_is_fed_the_whole_chain_at_both_settings(self):
        log = os.path.join(self.tmp.name, "zstd-args")
        fake = os.path.join(self.bin, "zstd")
        with open(fake, "w") as stream:
            stream.write('#!/bin/sh\necho "$*" >> "%s"\nexec cat\n' % log)
        os.chmod(fake, stat.S_IRWXU)
        with patch.dict(os.environ, {"PATH": self.bin + os.pathsep + "/usr/bin:/bin"}):
            report = json.loads(self.run_cli(self.trace, "--json")[1])["traces"][0]
        comp = report["compression"]
        self.assertEqual(comp["mode"], "zstd")
        for result in comp["results"]:
            self.assertEqual(result["compressed_bytes"], report["bytes"])  # `cat` echoes
            self.assertEqual(result["ratio"], 1.0)
        with open(log) as stream:
            calls = stream.read().splitlines()
        # Both compressors are fed at once, so they log in either order.
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertIn("-3", call.split())
        self.assertEqual(sorted("--long=27" in call.split() for call in calls), [False, True])

    def test_failing_compressor_is_reported_not_fatal(self):
        fake = os.path.join(self.bin, "zstd")
        with open(fake, "w") as stream:
            stream.write("#!/bin/sh\nexit 3\n")
        os.chmod(fake, stat.S_IRWXU)
        with patch.dict(os.environ, {"PATH": self.bin}):
            code, output = self.run_cli(self.trace, "--json")
        self.assertEqual(code, 0)
        for result in json.loads(output)["traces"][0]["compression"]["results"]:
            self.assertIsNone(result["ratio"])
            self.assertTrue(result["error"])

    def test_no_compress_skips(self):
        report = json.loads(self.run_cli(self.trace, "--json", "--no-compress")[1])
        self.assertEqual(report["traces"][0]["compression"], {"mode": "off", "results": []})


if __name__ == "__main__":
    unittest.main()
