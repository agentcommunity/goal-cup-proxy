"""Tests for longsum.py (long-runs plan Task 4). Run: python3 -m unittest discover -s <this dir> -p 'test_longsum.py'"""
import datetime
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import longsum  # noqa: E402

T0 = datetime.datetime(2026, 11, 2, 14, 0, tzinfo=datetime.timezone.utc)


def iso(minute, seconds=0):
    return (T0 + datetime.timedelta(minutes=minute, seconds=seconds)).isoformat()


def gap(minute, ms, cause="recorder_down"):
    start = T0 + datetime.timedelta(minutes=minute)
    return {"cause": cause, "start": start.isoformat(),
            "end": (start + datetime.timedelta(milliseconds=ms)).isoformat(), "ms": ms}


def stub(gaps):
    class Run:
        def __init__(self, folder, meta):
            pass

        def gaps(self):
            return (gaps, {})

    class Mod:
        pass

    Mod.Run = Run
    return Mod


def put(path, doc):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(doc, f)


def fixture(tmp, **over):
    d = os.path.join(tmp, "20261102T140000Z-pad-abc123")
    steps = [["start", "ok", "0:01", ""], ["kill-recorder@10", "ok", "0:10", ""],
             ["kill-recorder@30", "ok", "0:30", ""], ["kill-watchdog-recover@45", "ok", "0:45", ""],
             ["seal", "ok", "1:01", ""]]
    put(os.path.join(d, "meta.json"), {"steps": steps, "over_cap": over.get("over_cap", False), "entry": "e1",
                                       "minutes": 60})
    facts = {"tokens": {"input": 6000000, "cached": 30000000, "output": 120000}, "raw_bytes": 2000000000,
             "compressed_bytes": 50000000, "subagents": over.get("subagents", 16), "threads": 17,
             "wire_requests": 900, "spend_usd_est": 6.0, "recorder_rss_peak_mb": 180.0,
             "models": ["claude-haiku-5-5"], "over_cap": over.get("over_cap", False)}
    put(os.path.join(d, "row.json"), {"facts": facts, "why": over.get("why", [])})
    threads = [{"parent": "p", "first": iso(10), "last": iso(40)} for _ in range(10)]
    threads.append({"parent": "p", "first": iso(60), "last": iso(65)})
    threads.append({"parent": None, "first": iso(0), "last": iso(60)})
    put(os.path.join(d, "verify", "usage", "e1.json"), {"threads": threads})
    return d


def live_fixture(tmp, name, points):
    d = os.path.join(tmp, name)
    os.makedirs(os.path.join(d, "watch"))
    with open(os.path.join(d, "watch", "spend.jsonl"), "w") as f:
        for at, usd in points:
            f.write(json.dumps({"at": at, "usd": usd}) + "\n")
    return d


class T(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.addCleanup(self._t.cleanup)
        self.t = self._t.name

    def test_per_hour(self):
        s = longsum.summary(fixture(self.t), stub([gap(10, 900), gap(30, 1200), gap(46, 40000)]))
        self.assertEqual((s["hours"], s["usd_per_hour"], s["tokens_per_hour"]["input"], s["raw_mb_per_hour"],
                          s["compressed_mb_per_hour"]), (1.0, 6.0, 6000000.0, 2000.0, 50.0))

    def test_max_parallel(self):
        self.assertEqual(longsum.summary(fixture(self.t), stub([]))["max_parallel"], 10)

    def test_x4_pass(self):
        s = longsum.summary(fixture(self.t), stub([gap(10, 900), gap(30, 1200), gap(46, 40000)]))
        self.assertEqual(s["x4"], "PASS")

    def test_x4_allows_5s_under_load(self):
        d = fixture(self.t, why=["faults FAIL: a recorder_down gap of 3100 ms, not under 2 s"])
        self.assertEqual(longsum.summary(d, stub([gap(10, 3100), gap(30, 900), gap(46, 40000)]))["x4"], "PASS")

    def test_x4_fails_over_5s(self):
        d = fixture(self.t)
        self.assertTrue(longsum.summary(d, stub([gap(10, 5200), gap(30, 900), gap(46, 40000)]))["x4"].startswith("FAIL"))
        self.assertTrue(longsum.summary(d, stub([gap(10, 900), gap(46, 40000)]))["x4"].startswith("FAIL"))

    def test_x3_memory_guard_fails(self):
        s = longsum.summary(fixture(self.t), stub([gap(10, 900, cause="memory_guard")]))
        self.assertEqual(s["x3"], "FAIL")

    def test_over_cap_marks_x2_inconclusive(self):
        s = longsum.summary(fixture(self.t, subagents=8, over_cap=True), stub([]))
        self.assertEqual(s["x2"], "INCONCLUSIVE: sealed at the spend cap")

    def test_live_projection(self):
        d = live_fixture(self.t, "20261102T200000Z-pad-abc123", [("2026-11-02T20:30:00+00:00", 4.0)])
        self.assertEqual(longsum.live(d, 60), {"minutes": 30.0, "usd": 4.0, "usd_per_hour": 8.0, "projected_usd": 8.0})

    def test_markdown_has_run1_and_no_model_id(self):
        md = longsum.markdown(longsum.summary(fixture(self.t), stub([])))
        self.assertIn("| Raw MB per hour | 2000.0 | 495.9 |", md)
        self.assertNotIn("haiku", md)


if __name__ == "__main__":
    unittest.main()
