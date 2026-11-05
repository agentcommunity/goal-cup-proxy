"""pad-run on one shared signed-in GOALCUP_HOME: other rows' entries and files are not this row's."""
import importlib.machinery
import importlib.util
import json
import os
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


padrun = load("padrun_shared_home", "pad-run")


def bare(gc, **st):
    r = object.__new__(padrun.Run)
    r.st, r.s, r.quiet, r.tmux = dict(st), {}, True, "pad"
    r.env = {"GOALCUP_HOME": gc}
    r.pane_pids = lambda: []  # no tmux here: the pane's pids unknown unless a test gives them
    return r


def make_entry(home, run, entry, pid=None, mtime=None):
    d = os.path.join(home, "runs", run, entry)
    os.makedirs(d)
    p = os.path.join(d, "status.json")
    with open(p, "w") as f:
        json.dump({"pid": pid, "harness": {"pid": pid}}, f)
    if mtime:
        os.utime(p, (mtime, mtime))
    return d


class Entry(unittest.TestCase):
    def setUp(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        self.home = d.name

    def test_an_older_live_recorders_newer_status_is_not_this_rows_entry(self):
        old = make_entry(self.home, "r1", "e1")
        r = bare(self.home, entries_before=[old])
        os.utime(os.path.join(old, "status.json"), (time.time() + 50, time.time() + 50))  # the old recorder is still writing
        self.assertIsNone(r.entry())  # nothing new yet
        new = make_entry(self.home, "r2", "e2", mtime=time.time() - 10)
        self.assertEqual(r.entry(), new)
        self.assertEqual((r.st["run"], r.st["entry"]), ("r2", "e2"))

    def test_several_new_entries_go_by_the_panes_process(self):
        before = make_entry(self.home, "r0", "e0")
        mine = make_entry(self.home, "r2", "e2", pid=111, mtime=time.time() - 100)
        make_entry(self.home, "r3", "e3", pid=222)
        r = bare(self.home, entries_before=[before])
        with mock.patch.object(r, "pane_pids", lambda: [111, 112], create=True):
            self.assertEqual(r.entry(), mine)

    def test_several_new_entries_none_the_panes_is_refused(self):
        make_entry(self.home, "r2", "e2", pid=111)
        make_entry(self.home, "r3", "e3", pid=222)
        r = bare(self.home, entries_before=[])
        with mock.patch.object(r, "pane_pids", lambda: [999], create=True):
            with self.assertRaises(padrun.Step) as ctx:
                r.entry()
        self.assertIn("pane", ctx.exception.args[1])

    def test_one_new_entry_not_the_panes_is_not_taken(self):  # another row's recorder started after this one
        make_entry(self.home, "r2", "e2", pid=222)
        r = bare(self.home, entries_before=[])
        with mock.patch.object(r, "pane_pids", lambda: [111], create=True):
            self.assertIsNone(r.entry())

    def test_one_new_entry_the_panes_is_taken(self):
        mine = make_entry(self.home, "r2", "e2", pid=111)
        r = bare(self.home, entries_before=[])
        with mock.patch.object(r, "pane_pids", lambda: [111], create=True):
            self.assertEqual(r.entry(), mine)
        with mock.patch.object(r, "pane_pids", lambda: [999], create=True):  # the harness ended; the pane is dead
            self.assertEqual(r.entry(), mine)

    def test_a_status_mid_write_keeps_polling(self):
        mine = make_entry(self.home, "r2", "e2", pid=111)
        make_entry(self.home, "r3", "e3", pid=222)
        with open(os.path.join(mine, "status.json"), "w") as f:
            f.write('{"pid": 1')  # half written
        r = bare(self.home, entries_before=[])
        with mock.patch.object(r, "pane_pids", lambda: [111], create=True):
            self.assertIsNone(r.entry())  # not refused: the next poll reads it whole
            with open(os.path.join(mine, "status.json"), "w") as f:
                json.dump({"pid": 111, "harness": {"pid": 111}}, f)
            self.assertEqual(r.entry(), mine)


class HarnessFiles(unittest.TestCase):
    def test_a_file_from_before_this_row_started_is_not_listed(self):
        with tempfile.TemporaryDirectory() as d:
            root = os.path.join(d, "projects")
            os.makedirs(root)
            now = time.time()
            for name, age in (("previous.jsonl", 45 + 60), ("mine.jsonl", 5)):
                p = os.path.join(root, name)
                open(p, "w").write("x")
                os.utime(p, (now - age, now - age))
            r = bare(d, pasted=now, row_start=now - 60, home=d)
            r.s = {"native": [{"name": "n", "root": "{home}/projects", "pattern": "*.jsonl"}]}
            r.project = d
            written = {}
            r.path = lambda *a: os.path.join(d, *a)
            with mock.patch.object(padrun, "write", lambda p, t, m: written.update(t=t)):
                r.harness_files()
            self.assertEqual([f["path"] for f in json.loads(written["t"])], ["n:mine.jsonl"])


if __name__ == "__main__":
    unittest.main()
