"""Tests of pad-run's per-case first-run keys and the kill-recorder@0 timing (PRD-12 sections 3, 3.1; ledger B10).
No network, no tmux, no recorder: the run object is built bare and its hands are replaced."""
import importlib.machinery
import importlib.util
import os
import tempfile
import tomllib
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


padrun, pad = load("padrun_case", "pad-run"), load("pad_case", "pad")


class Stop(Exception):
    pass


def bare():
    r = object.__new__(padrun.Run)
    r.st, r.s, r.h, r.sec = {"pasted": 0.0}, {"case": "gateway"}, {}, {}
    r.need = lambda *a, **k: None
    r.at = lambda m: None
    return r


class PreKeys(unittest.TestCase):
    def sent(self, h, sec):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        open(os.path.join(d.name, "prompt.txt"), "w").write("task")
        r = bare()
        r.h, r.sec, r.tmux, r.env = h, sec, "t", {"GOALCUP_HOME": d.name}
        r.seed = lambda: None
        r.pane = lambda *a: ""
        r.path = lambda *a: os.path.join(d.name, *a)
        keys = []

        def sh(cmd, *a, **k):
            if "send-keys" in cmd:
                keys.append(cmd[-1])
            if "list-panes" in cmd:  # past the keys and the prompt read
                raise Stop
            return mock.Mock(stdout="")
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun.time, "sleep", lambda s: None):
            with self.assertRaises(Stop):
                r.type_prompt()
        return keys

    def test_section_wins_over_top_level(self):
        self.assertEqual(self.sent({"pre_keys": ["\\r"]}, {"pre_keys": ["\\r", "\\r"]}), ["\r", "\r"])

    def test_top_level_when_section_has_none(self):
        self.assertEqual(self.sent({"pre_keys": "\\r"}, {}), ["\r"])

    def test_claude_code_gateway_list(self):
        with open(os.path.join(HERE, "harnesses", "claude-code-cli.toml"), "rb") as f:
            doc = tomllib.load(f)
        self.assertEqual(len(doc["gateway"]["pre_keys"]), 4)
        self.assertIn("pre_keys", doc)


class KillTiming(unittest.TestCase):
    def kill(self, m, counts):
        r = bare()
        it = iter(counts)
        last = [counts[-1]]
        r.work = lambda st=None: next(it, last[0])
        r.kill_rec = lambda m, quiet=False: (7, 1.0) if not quiet else (7, None)
        limits = []

        def wait(ok, limit, every):
            limits.append(limit)
            return ok()
        with mock.patch.object(padrun, "wait", wait):
            saw = r.kill_recorder(m)[1]
        return saw, limits

    def test_parses_at_zero(self):
        self.assertEqual(padrun.parse(["kill-recorder@0"]), ("kill-recorder", None, "0"))
        self.assertEqual(pad.parse_steps(["kill-recorder@0"])[0][1], 0)

    def test_zero_fires_on_first_written_request(self):
        saw, _ = self.kill(0, [1, 1])  # one request already written, no second one needed
        self.assertNotIn("no request in flight", saw)

    def test_three_keeps_the_wait_for_a_new_request(self):
        saw, _ = self.kill(3, [5, 5, 5])
        self.assertIn("no request in flight", saw)
        saw, _ = self.kill(3, [5, 6])
        self.assertNotIn("no request in flight", saw)


class Recovery(unittest.TestCase):
    def test_quiet_kill_judged_on_first_three(self):
        r = bare()
        r.st["kill"] = dict(pid=1, at=0.0, requests=4, off_spans=0, off_ms=0, quiet=True)
        st = {}
        r.status = lambda: st
        r.work = lambda s=None: 4
        r.left = lambda: 5
        dig = {"s4_pid": 2, "s4_mode": "recording", "s4_spans": 1, "s4_since": None, "s4_down_ms": 10}
        with mock.patch.object(padrun.pc, "dig", lambda d, n: dig.get(n)), mock.patch.object(padrun, "wait", lambda ok, l, e: ok()):
            with self.assertRaises(padrun.Step) as cm:
                r.wait_recovery(None)
        self.assertEqual(cm.exception.args[0], "INCONCLUSIVE")  # not FAIL: the first three held


if __name__ == "__main__":
    unittest.main()
