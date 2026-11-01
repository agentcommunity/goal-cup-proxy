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
        self.assertTrue(doc["gateway"]["pre_keys"])
        self.assertIn("pre_keys", doc)


class KillTiming(unittest.TestCase):
    def kill(self, m, counts, last=None):
        r = bare()
        it = iter(counts)
        r.work = lambda st=None: next(it, counts[-1])
        r.status = lambda: {}
        quiets = []
        r.kill_rec = lambda m, quiet=False: quiets.append(quiet) or (7, 1.0)
        now = padrun.time.time()
        pc_when = (lambda s: mock.Mock(timestamp=lambda: now - last)) if last is not None else (lambda s: None)
        with mock.patch.object(padrun, "wait", lambda ok, limit, every: ok()), mock.patch.object(padrun.pc, "when", pc_when), \
                mock.patch.object(padrun.pc, "dig", lambda d, n: "t"):
            r.kill_recorder(m)
        return quiets[0]

    def test_parses_at_zero(self):
        self.assertEqual(padrun.parse(["kill-recorder@0"]), ("kill-recorder", None, "0"))
        self.assertEqual(pad.parse_steps(["kill-recorder@0"])[0][1], 0)

    def test_zero_is_quiet_unless_a_request_was_just_written(self):
        self.assertFalse(self.kill(0, [9], last=2))  # written 2 s ago
        self.assertTrue(self.kill(0, [9], last=300))  # a count above 0 alone proves nothing
        self.assertTrue(self.kill(0, [9], last=None))

    def test_three_keeps_the_wait_for_a_new_request(self):
        self.assertTrue(self.kill(3, [5, 5, 5]))
        self.assertFalse(self.kill(3, [5, 6]))


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


class EarlyPane(unittest.TestCase):
    def test_pane_is_copied_before_recording_seen(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        r = bare()
        r.s.update(server="off", harness_id="h")
        r.project, r.tmux, r.env = d.name, "t", {}
        r.has = lambda k: False
        r.gc_env = lambda: {}
        r.path = lambda *a: os.path.join(d.name, *a)
        order = []
        r.capture = lambda *a: "goalcup 1.0"
        r.pane = lambda *a, **k: order.append("pane") or ""
        r.recording_seen = lambda why: order.append("wait") or (_ for _ in ()).throw(Stop())
        sh = lambda cmd, *a, **k: mock.Mock(stdout="")
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun, "tool", lambda n: n):
            with self.assertRaises(Stop):
                r.start(None)
        self.assertEqual(order, ["pane", "wait"])


if __name__ == "__main__":
    unittest.main()
