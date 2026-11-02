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

    def test_no_pre_keys_when_ready_text_never_shows(self):
        with mock.patch.object(padrun, "wait", lambda *a, **k: False):
            self.assertEqual(self.sent({"pre_keys": "n", "ready_text": "dialog"}, {}), [])

    def test_pre_keys_after_ready_text_shows(self):
        with mock.patch.object(padrun, "wait", lambda *a, **k: True):
            self.assertEqual(self.sent({"pre_keys": "n", "ready_text": "dialog"}, {}), ["n"])

    def enters(self, name):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        open(os.path.join(d.name, "prompt.txt"), "w").write("task")
        with open(os.path.join(HERE, "harnesses", name + ".toml"), "rb") as f:
            h = tomllib.load(f)
        r = bare()
        r.h, r.tmux, r.env, r.run = h, "t", {"GOALCUP_HOME": d.name}, d.name
        r.seed = lambda: None
        r.pane = lambda *a, **k: ""
        r.path = lambda *a: os.path.join(d.name, *a)
        keys = []

        def sh(cmd, *a, **k):
            if "send-keys" in cmd:
                keys.append(cmd[-1])
            return mock.Mock(stdout="")
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun.time, "sleep", lambda s: None), \
                mock.patch.object(padrun.subprocess, "Popen", side_effect=Stop):
            with self.assertRaises(Stop):
                r.type_prompt()
        return keys.count("Enter")

    def test_openclaw_sends_a_second_enter_after_a_collapsed_paste_and_others_do_not(self):  # S4
        self.assertEqual(self.enters("openclaw-cli"), 2)
        self.assertEqual(self.enters("codex-cli"), 1)

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

    def test_a_timed_kill_waits_past_the_keep_going_idle_for_work(self):
        # A2 passes 3 and 4: the minute-30 kill came after 60 s of idle and was quiet; the watch types keep going after
        # 2 min idle, so the kill waits long enough for that work to start.
        r = bare()
        r.work = lambda st=None: 5
        r.kill_rec = lambda m, quiet=False: (7, 1.0)
        limits = []
        with mock.patch.object(padrun, "wait", lambda ok, limit, every: limits.append(limit) or ok()):
            r.kill_recorder(30)
        self.assertGreaterEqual(limits[0], 300)


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

    def test_quiet_kill_waits_for_the_keep_going_work(self):
        r = bare()
        r.st["kill"] = dict(pid=1, at=0.0, requests=4, off_spans=0, off_ms=0, quiet=True)
        r.status = lambda: {}
        r.work = lambda s=None: 4
        r.left = lambda: 9999
        limits = []
        dig = {"s4_pid": 2, "s4_mode": "recording", "s4_spans": 1, "s4_since": None, "s4_down_ms": 10}
        with mock.patch.object(padrun.pc, "dig", lambda d, n: dig.get(n)), \
                mock.patch.object(padrun, "wait", lambda ok, l, e: limits.append(l) or ok()):
            with self.assertRaises(padrun.Step):
                r.wait_recovery(None)
        self.assertGreaterEqual(limits[-1], 240)


    def test_idle_harness_after_a_kill_is_inconclusive_not_fail(self):  # A2: recorder back in 44 ms, no new request follows
        r = bare()
        r.st["kill"] = dict(pid=1, at=padrun.time.time() - 0.1, requests=4, off_spans=0, off_ms=0, quiet=False)
        r.status = lambda: {}
        r.work = lambda s=None: 4
        r.left = lambda: 5
        dig = {"s4_pid": 2, "s4_mode": "recording", "s4_spans": 1, "s4_since": None, "s4_down_ms": 10}
        with mock.patch.object(padrun.pc, "dig", lambda d, n: dig.get(n)), mock.patch.object(padrun, "wait", lambda ok, l, e: ok()):
            with self.assertRaises(padrun.Step) as cm:
                r.wait_recovery(None)
        self.assertEqual(cm.exception.args[0], "INCONCLUSIVE")
        self.assertRegex(cm.exception.args[1], r"back in 0\.\d s")


class KeepGoing(unittest.TestCase):
    def run_nudges(self, count, times):
        r = bare()
        r.s = {"keep_going": 2, "keep_going_count": count}
        r.tmux, r.run = "t", "/nonexistent"
        r.pane = lambda *a, **k: "same screen"
        r.status = lambda: {}
        told = []
        r.fault = lambda kind, **kw: told.append(kind)
        keys = []
        clock = [0.0]
        sent = []
        for t in times:
            clock[0] = float(t)
            with mock.patch.object(padrun.time, "time", lambda: clock[0]), mock.patch.object(padrun.time, "sleep", lambda s: None), \
                    mock.patch.object(padrun, "sh", lambda cmd, **k: keys.append(cmd[-1])), \
                    mock.patch.object(padrun.pc, "load", lambda *a: [{"kind": k} for k in told]):
                sent.append(bool(r.nudge()))
        return sent, keys

    def test_types_again_after_a_second_idle_period_up_to_the_count(self):
        sent, keys = self.run_nudges(2, [0, 60, 130, 140, 200, 262, 400, 600])
        self.assertEqual(sent, [False, False, True, False, False, True, False, False])
        self.assertEqual([k for k in keys if k == "keep going"], ["keep going", "keep going"])  # never more than the count

    def test_the_run_files_count_is_loaded(self):
        s = pad.load_settings(dict(harness="codex-cli", key="openai", mode="full", keep_going_count=12), {"keys": {"openai": "OPENAI_API_KEY"}})
        self.assertEqual((s["keep_going_count"], s["keep_going"]), (12, 1))
        with self.assertRaises(pad.Usage):
            pad.load_settings(dict(harness="codex-cli", key="openai", mode="full", keep_going_count=0), {"keys": {"openai": "OPENAI_API_KEY"}})


class EndScreen(unittest.TestCase):
    def test_watchdog_row_reads_the_end_screen_from_goalcup_status(self):  # A2: nothing prints it into the pane once the watchdog is dead
        r = bare()
        r.s = {"minutes": 1}
        r.project, r.env, r.run = "/nonexistent", {}, "/nonexistent"
        r.sealed = lambda: True
        r.dead = lambda: True
        r.has = lambda *a: True
        r.exit_harness = lambda: None
        r.known = lambda: []
        r.pane = lambda *a, **k: ""
        r.left = lambda: 100
        r.status = lambda: {}
        r.harness_files = lambda: None
        r.wipe_secrets = lambda **k: None
        calls = []

        def sh(cmd, **k):
            calls.append(cmd)
            return mock.Mock(stdout="217 requests to model hosts written: 3 gaps", returncode=0)
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun, "tool", lambda n: n), \
                mock.patch.object(padrun.pc, "load", lambda *a: []), mock.patch.object(padrun, "wait", lambda ok, l, e: ok()):
            r.seal(None)
        self.assertEqual(r.st["end_screen_requests"], 217)
        self.assertIn(["goalcup", "status"], calls)


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


class GoalcupLog(unittest.TestCase):
    def run_with(self, write_log):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        run, entry = os.path.join(d.name, "run"), os.path.join(d.name, "entry")
        os.makedirs(run)
        os.makedirs(entry)
        if write_log:
            open(os.path.join(entry, "goalcup.log"), "w").write("line\n")
        r = bare()
        r.run = run
        r.path = lambda *a: os.path.join(run, *a)
        r.copy_log(entry)
        r.copy_log(None)  # no entry: quiet
        return r, run

    def test_log_is_copied_and_scanned(self):
        r, run = self.run_with(True)
        self.assertTrue(os.path.isfile(os.path.join(run, "goalcup.log")))
        self.assertIn("goalcup.log", r.out_paths())

    def test_missing_log_is_quiet_and_second_entry_is_carried(self):
        r, run = self.run_with(False)
        self.assertFalse(os.path.exists(os.path.join(run, "goalcup.log")))
        self.assertNotIn("goalcup.log", r.out_paths())
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        open(os.path.join(d.name, "goalcup.log"), "w").write("recover\n")
        r.copy_log(d.name, "goalcup-new.log")
        self.assertIn("goalcup-new.log", r.out_paths())


class PaneBeforePaste(unittest.TestCase):
    def test_pane_copied_before_the_paste(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        open(os.path.join(d.name, "prompt.txt"), "w").write("task")
        r = bare()
        r.h, r.tmux, r.env = {}, "t", {"GOALCUP_HOME": d.name}
        r.seed = lambda: None
        r.path = lambda *a: os.path.join(d.name, *a)
        order = []
        r.pane = lambda *a, **k: order.append("pane") or ""

        def sh(cmd, *a, **k):
            if "paste-buffer" in cmd:
                order.append("paste")
                raise Stop
            return mock.Mock(stdout="")
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun.time, "sleep", lambda s: None):
            with self.assertRaises(Stop):
                r.type_prompt()
        self.assertEqual(order, ["pane", "paste"])


class MaskValues(unittest.TestCase):
    """The start event's mask_values reaches check.py: the dump's event lines (as goalcup-verify prints them) are read up to the start."""

    def read(self, lines):
        proc = mock.MagicMock()
        proc.__enter__.return_value = proc
        proc.stdout = iter(lines)
        r = bare()
        r.run, r.left = ".", lambda: 5
        with mock.patch.object(padrun.subprocess, "Popen", return_value=proc):
            return r.mask_values("e")

    def test_start_event_line_is_read(self):
        mv = {"workspace": 3, "env": 0, "harness": 1, "skipped": 0}
        got = self.read(['{"k": "event", "o": 9, "ev": "other"}\n',
                         '{"k": "event", "o": 10, "ev": "start", "mask_values": {"workspace": 3, "env": 0, "harness": 1, "skipped": 0}}\n',
                         'not read'])
        self.assertEqual(got, mv)
        self.assertIsNone(self.read(['{"k": "event", "ev": "start"}\n']))


if __name__ == "__main__":
    unittest.main()
