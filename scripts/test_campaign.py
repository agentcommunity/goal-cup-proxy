"""Tests for scripts/campaign.py. No docker, no network: the runner and the clock are fakes."""

from datetime import datetime, timedelta, timezone
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from contextlib import redirect_stdout

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("campaign", HERE / "campaign.py")
campaign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(campaign)

SENTINEL = "SENTINEL-SECRET-VALUE-0123456789"
TOKEN = "FAKE-GITHUB-TOKEN-for-tests-only-0123456789"
T0 = datetime(2026, 10, 23, 8, 0, 0, tzinfo=timezone.utc)


class FakeClock:
    """Waits on worker threads advance fake time at once; the main thread's waits do not."""

    def __init__(self):
        self.t = 1000.0
        self.lock = threading.Lock()
        self.hold = None        # timeout -> True: really block until the event (a drill in progress)
        self.interrupt = None   # () -> True: the main thread's wait raises KeyboardInterrupt
        self.fail_on = None     # timeout -> True: raise RuntimeError (an exception mid-drill)

    def monotonic(self):
        return self.t

    def utcnow(self):
        return T0 + timedelta(seconds=self.t - 1000.0)

    def wait(self, event, timeout):
        if threading.current_thread() is threading.main_thread():
            if self.interrupt and self.interrupt():
                raise KeyboardInterrupt
            event.wait(0.005)
            return event.is_set()
        if self.fail_on and self.fail_on(timeout):
            raise RuntimeError("boom mid-drill")
        if self.hold and self.hold(timeout):
            event.wait(20)
            return event.is_set()
        with self.lock:
            self.t += timeout
        return event.is_set()


class FakeProc:
    _pid = 4000

    def __init__(self, lines=(), rc=0, finish=True):
        FakeProc._pid += 1
        self.pid = FakeProc._pid
        self.stdout = [l.encode() + b"\n" for l in lines]
        self.rc = rc
        self.done = threading.Event()
        self.returncode = None
        self.signals = []
        if finish:
            self.finish(rc)

    def finish(self, rc):
        self.returncode = rc
        self.done.set()

    def wait(self, timeout=None):
        if not self.done.wait(timeout if timeout is not None else 30):
            raise subprocess.TimeoutExpired("fake", timeout)
        return self.returncode

    def poll(self):
        return self.returncode if self.done.is_set() else None


class FakeRunner:
    def __init__(self, networks=("bridge",)):
        self.calls = []          # every argv, in order (popen and run)
        self.envs = []           # every env given to a child
        self.procs = []
        self.lock = threading.Lock()
        self.networks = networks
        self.fail = {}           # verb -> how many times it fails
        self.on_run = None       # argv -> None, after recording
        self.make_proc = lambda argv: FakeProc(["linux-matrix: codex-cli linux-x86_64 0.1 PASS 2026-10-23"])
        self.signals = []
        self.order = []          # ("docker", verb) and ("signal", sig), in order
        self.real_kit = False    # run scripts/kit_settle.py for real (it only writes files)

    def popen(self, argv, env=None, cwd=None, stdout=None):
        with self.lock:
            self.calls.append(list(argv))
            self.envs.append(dict(env or {}))
        if argv[0] == "docker" or "research-sample.py" in " ".join(argv):
            proc = FakeProc(finish=False)
        else:
            proc = self.make_proc(argv)
        self.procs.append((argv, proc))
        return proc

    def run(self, argv, env=None, timeout=60):
        with self.lock:
            self.calls.append(list(argv))
            if env:
                self.envs.append(dict(env))
        if str(campaign.KIT) in argv:
            return campaign.RealRunner().run(argv, env, timeout) if self.real_kit else (0, "")
        verb = argv[1] if len(argv) > 1 else ""
        self.order.append((argv[0], verb))
        if verb == "network":
            verb = argv[2]
        try:
            if self.fail.get(verb, 0) > 0:
                self.fail[verb] -= 1
                return 1, f"Error: {verb} failed"
            if argv[:2] == ["docker", "inspect"] and "{{.State.Running}}" in argv:
                return 0, "true\n"
            if argv[:2] == ["docker", "inspect"]:
                return 0, json.dumps({n: {} for n in self.networks})
            return 0, ""
        finally:
            if self.on_run:
                self.on_run(argv)

    def interactive(self, argv, env=None, cwd=None):
        with self.lock:
            self.calls.append(list(argv))
            self.envs.append(dict(env or {}))
        return 0

    def signal_group(self, proc, sig):
        self.signals.append((proc.pid, sig))
        self.order.append(("signal", sig))
        if sig in (signal.SIGTERM, signal.SIGKILL) and not proc.done.is_set():
            proc.finish(-sig)

    def verbs(self):
        return [" ".join(c[1:3]) for c in self.calls if c[0] == "docker"]


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.key = self.root / "approval-key"
        self.key.write_text(SENTINEL + "\n")
        self.prompt = self.root / "prompt.txt"
        self.prompt.write_text("build a thing\n")
        self.token = self.root / "github-token"
        self.token.write_text(TOKEN + "\n")
        os.environ.pop(campaign.KEY_ENV, None)

    def tearDown(self):
        self.tmp.cleanup()

    def config(self, cells, **defaults):
        d = {"approval_key_file": str(self.key), "github_token_file": str(self.token),
             "connector": "codex-cli", "duration_s": 3600, "prompt_file": str(self.prompt)}
        d.update(defaults)
        return campaign.normalize_config({"name": "t", "defaults": d, "cells": cells}, self.root)

    def host_cell(self, cid, member, **kw):
        cfgdir = self.root / f"cfg-{cid}"
        cfgdir.mkdir(exist_ok=True)
        c = {"id": cid, "where": "host", "test_member_id": member, "config_dir": str(cfgdir)}
        c.update(kw)
        return c

    def docker_cell(self, cid, member, **kw):
        c = {"id": cid, "where": "docker", "test_member_id": member}
        c.update(kw)
        return c

    def errors(self, cfg, parallel=4):
        return campaign.validate(cfg, cfg["cells"], parallel)[0]


class ConfigValidation(Base):
    def test_valid_config_passes(self):
        cfg = self.config([self.docker_cell("a", "m1"), self.host_cell("b", "m2")])
        self.assertEqual(self.errors(cfg), [])

    def test_shared_member_refused_when_cells_run_together(self):
        cfg = self.config([self.docker_cell("a", "m1"), self.docker_cell("b", "m1")])
        errs = self.errors(cfg, parallel=2)
        self.assertTrue(any("share test_member_id" in e for e in errs), errs)

    def test_shared_member_allowed_in_parallel_once_test_runs_do_not_replace(self):
        cfg = self.config([self.docker_cell("a", "m1"), self.docker_cell("b", "m1")],
                          test_runs_parallel=True)
        self.assertEqual(self.errors(cfg, parallel=2), [])

    def test_shared_member_allowed_one_at_a_time(self):
        cfg = self.config([self.docker_cell("a", "m1"), self.docker_cell("b", "m1")])
        self.assertEqual(self.errors(cfg, parallel=1), [])

    def test_production_refused(self):
        for server in ("https://ingest.agentcommunity.org", "https://ingest.agentcommunity.org/"):
            cfg = self.config([self.docker_cell("a", "m1")], server=server)
            self.assertTrue(any("production" in e for e in self.errors(cfg)), server)

    def test_plain_http_refused(self):
        cfg = self.config([self.docker_cell("a", "m1")], server="http://example.workers.dev")
        self.assertTrue(any("https" in e for e in self.errors(cfg)))

    def test_bad_ids_refused(self):
        for bad in ("A", "a_b", "-a", "", "a b", "x" * 41):
            cfg = self.config([self.docker_cell(bad, "m1")])
            self.assertTrue(any("id must match" in e for e in self.errors(cfg)), bad)

    def test_duplicate_ids_refused(self):
        cfg = self.config([self.docker_cell("a", "m1"), self.docker_cell("a", "m2")])
        self.assertTrue(any("duplicate id" in e for e in self.errors(cfg)))

    def test_host_config_dirs_distinct(self):
        a = self.host_cell("a", "m1")
        b = self.host_cell("b", "m2", config_dir=a["config_dir"])
        cfg = self.config([a, b])
        self.assertTrue(any("also cell a's" in e for e in self.errors(cfg)))

    def test_host_needs_config_dir_and_docker_refuses_one(self):
        cfg = self.config([{"id": "a", "where": "host", "test_member_id": "m1"},
                           self.docker_cell("b", "m2", config_dir=str(self.root))])
        errs = self.errors(cfg)
        self.assertTrue(any("needs config_dir" in e for e in errs))
        self.assertTrue(any("config_dir is for host" in e for e in errs))

    def test_matrix_timeout_slack_checked(self):
        for bad in ("3600", -1, 1.5, True):
            with self.subTest(slack=bad):
                cfg = self.config([self.docker_cell("a", "m1")], matrix_timeout_slack_s=bad)
                self.assertTrue(any("matrix_timeout_slack_s" in e for e in self.errors(cfg)))
        cfg = self.config([self.docker_cell("a", "m1")], matrix_timeout_slack_s=0)
        self.assertEqual(self.errors(cfg), [])

    def test_missing_member_refused(self):
        cfg = self.config([{"id": "a", "where": "docker"}])
        self.assertTrue(any("test_member_id is required" in e for e in self.errors(cfg)))

    def test_key_file_must_be_readable(self):
        cfg = self.config([self.docker_cell("a", "m1")],
                          approval_key_file=str(self.root / "missing"))
        self.assertTrue(any("approval key file" in e for e in self.errors(cfg)))

    def test_key_in_environment_refused(self):
        os.environ["GOALCUP_TEST_APPROVAL_KEY"] = "x"
        try:
            cfg = self.config([self.docker_cell("a", "m1")])
            self.assertTrue(any("in the environment" in e for e in self.errors(cfg)))
        finally:
            del os.environ["GOALCUP_TEST_APPROVAL_KEY"]

    def test_drills_checked(self):
        cfg = self.config([self.docker_cell("a", "m1", drills=[
            {"at_s": 100, "kind": "network_cut", "seconds": 600},
            {"at_s": 200, "kind": "pause", "seconds": 60},
            {"at_s": 10, "kind": "reboot", "seconds": 1},
            {"at_s": 3500, "kind": "pause", "seconds": 600},
        ])])
        errs = self.errors(cfg)
        self.assertTrue(any("overlap" in e for e in errs), errs)
        self.assertTrue(any("kind must be" in e for e in errs), errs)
        self.assertTrue(any("after the cell's duration" in e for e in errs), errs)

    def test_check_command_json(self):
        path = self.root / "c.json"
        path.write_text(json.dumps({"defaults": {"approval_key_file": str(self.key),
                                                 "github_token_file": str(self.token)},
                                    "cells": [{"id": "a", "where": "docker", "connector": "codex-cli",
                                               "test_member_id": "m1", "duration_s": 600}]}))
        runner = FakeRunner()
        out = []
        args = campaign.build_parser().parse_args(["check", str(path), "--json"])
        rc = campaign.cmd_check(args, runner, say=out.append)
        doc = json.loads(out[0])
        self.assertEqual(rc, 0, doc)
        self.assertIn(["docker", "volume", "inspect", "goalcup-linux-matrix-codex-cli-a"],
                      runner.calls)


def in_thread(fn):
    """Run fn on a worker thread (the fake clock advances only off the main thread)."""
    box = {}

    def target():
        try:
            box["value"] = fn()
        except BaseException as e:  # surfaced below
            box["error"] = e
    t = threading.Thread(target=target)
    t.start()
    t.join(30)
    if t.is_alive():
        raise AssertionError("did not finish")
    if "error" in box:
        raise box["error"]
    return box["value"]


class DrillScheduling(Base):
    def schedule(self, drills, runner, clock, stop=None, alive=lambda: True):
        ex = campaign.DockerDrills(runner, "ctr", clock)
        stop = stop or threading.Event()
        sched = campaign.DrillScheduler("c", drills, ex, clock, clock.monotonic(), clock.utcnow(),
                                        stop, alive)
        return sched, in_thread(sched.run)

    def test_network_cut_disconnects_then_reconnects_every_network(self):
        runner, clock = FakeRunner(networks=("bridge", "extra")), FakeClock()
        times = {}
        runner.on_run = lambda argv: times.setdefault(" ".join(argv[1:4]), clock.monotonic())
        _, recs = self.schedule([{"at_s": 600, "kind": "network_cut", "seconds": 300}], runner, clock)
        v = runner.verbs()
        self.assertEqual([x for x in v if x.startswith("network")],
                         ["network disconnect", "network disconnect",
                          "network connect", "network connect"])
        self.assertGreaterEqual(times["network disconnect bridge"], 1000 + 600)
        self.assertGreaterEqual(times["network connect bridge"] - times["network disconnect bridge"], 300)
        self.assertEqual(recs[0]["status"], "done")
        self.assertEqual(recs[0]["planned_utc"], "2026-10-23T08:10:00Z")

    def test_pause_then_unpause(self):
        runner, clock = FakeRunner(), FakeClock()
        _, recs = self.schedule([{"at_s": 5, "kind": "pause", "seconds": 60}], runner, clock)
        self.assertEqual(runner.verbs(), ["pause ctr", "unpause ctr"])
        self.assertEqual(recs[0]["status"], "done")

    def test_reconnect_happens_when_the_wait_raises(self):
        runner, clock = FakeRunner(), FakeClock()
        clock.fail_on = lambda timeout: timeout == 300
        _, recs = self.schedule([{"at_s": 0, "kind": "network_cut", "seconds": 300}], runner, clock)
        self.assertEqual(runner.verbs()[-1], "network connect")
        self.assertEqual(recs[0]["status"], "error")
        self.assertIn("boom", recs[0]["error"])

    def test_unpause_happens_when_pause_fails(self):
        runner, clock = FakeRunner(), FakeClock()
        runner.fail["pause"] = 1
        _, recs = self.schedule([{"at_s": 0, "kind": "pause", "seconds": 30}], runner, clock)
        self.assertEqual(runner.verbs(), ["pause ctr", "unpause ctr"])
        self.assertEqual(recs[0]["status"], "error")

    def test_reconnect_attempted_when_disconnect_fails(self):
        runner, clock = FakeRunner(), FakeClock()
        runner.fail["disconnect"] = 1
        self.schedule([{"at_s": 0, "kind": "network_cut", "seconds": 30}], runner, clock)
        self.assertEqual(runner.verbs()[-1], "network connect")

    def test_failed_undo_is_retried_and_reported(self):
        runner, clock = FakeRunner(), FakeClock()
        runner.fail["unpause"] = 5
        _, recs = self.schedule([{"at_s": 0, "kind": "pause", "seconds": 30}], runner, clock)
        self.assertEqual(runner.verbs().count("unpause ctr"), campaign.DockerDrills.RETRIES)
        self.assertIn("UNDO FAILED", recs[0]["undo_error"])
        self.assertEqual(recs[0]["status"], "error")

    def test_stop_cuts_a_drill_short_and_still_undoes_it(self):
        runner, clock = FakeRunner(), FakeClock()
        clock.hold = lambda timeout: timeout == 999
        stop = threading.Event()
        runner.on_run = lambda argv: argv[1] == "pause" and threading.Timer(0.05, stop.set).start()
        _, recs = self.schedule([{"at_s": 0, "kind": "pause", "seconds": 999}], runner, clock, stop)
        self.assertEqual(runner.verbs(), ["pause ctr", "unpause ctr"])
        self.assertEqual(recs[0]["status"], "cut short")

    def test_drills_after_the_cell_ended_are_skipped(self):
        runner, clock = FakeRunner(), FakeClock()
        _, recs = self.schedule([{"at_s": 10, "kind": "pause", "seconds": 5}], runner, clock,
                                alive=lambda: False)
        self.assertEqual(runner.verbs(), [])
        self.assertEqual(recs[0]["status"], "skipped")

    def test_host_drill_prints_and_never_runs_anything(self):
        runner, clock, said = FakeRunner(), FakeClock(), []
        ex = campaign.HostDrills(said.append, "mac", clock, platform="darwin")
        sched = campaign.DrillScheduler("mac", [{"at_s": 60, "kind": "network_cut", "seconds": 600}],
                                        ex, clock, clock.monotonic(), clock.utcnow(),
                                        threading.Event())
        recs = in_thread(sched.run)
        self.assertEqual(runner.calls, [])
        self.assertTrue(said[0].startswith("DRILL-NOW cell=mac kind=network_cut"))
        self.assertIn("networksetup -setairportpower en0 off", said[0])
        self.assertTrue(said[1].startswith("DRILL-END cell=mac kind=network_cut"))
        self.assertEqual(recs[0]["status"], "manual")


SPEC_SAMPLE = importlib.util.spec_from_file_location("research_sample", HERE / "research-sample.py")
research_sample = importlib.util.module_from_spec(SPEC_SAMPLE)
SPEC_SAMPLE.loader.exec_module(research_sample)
COLUMNS = research_sample.COLUMNS


def write_samples(path, rows):
    """rows: dicts of research-sample.py's columns (missing ones left empty)."""
    import csv as _csv
    with open(path, "w", newline="") as f:
        w = _csv.DictWriter(f, fieldnames=COLUMNS, extrasaction="raise")
        w.writeheader()
        for r in rows:
            w.writerow(r)


class PassLines(Base):
    def stamp(self, s):
        return campaign.utc(T0 + timedelta(seconds=s))

    def line(self, lines, name):
        return next(l for l in lines if l["name"] == name)

    def info(self, **kw):
        d = {"verdict": "PASS", "exit_code": 0, "final_row": "x PASS", "elapsed_s": 4000,
             "duration_s": 3600, "drills": [], "start_utc": self.stamp(0)}
        d.update(kw)
        return d

    def evaluate(self, rows, **kw):
        path = self.root / "s.csv"
        write_samples(path, rows)
        return campaign.evaluate_lines(self.info(**kw), str(path))

    def test_slope_math(self):
        pts = [(h * 3600, 5 + 2 * h) for h in range(5)]
        self.assertAlmostEqual(campaign.slope_per_hour(pts), 2.0)
        self.assertIsNone(campaign.slope_per_hour([(0, 1)]))

    def test_flat_rss_and_files_pass_growing_fail(self):
        rows = []
        for i in range(0, 4 * 6 + 1):  # every 10 min for 4 h
            t = i * 600
            rows.append({"utc": self.stamp(t),
                         "daemon_procs": 1, "daemon_rss_bytes": 100 * 2 ** 20 + (i % 2) * 4096,
                         "daemon_open_files": 20,
                         "cli_procs": 1, "cli_rss_bytes": 200 * 2 ** 20 + int(t / 3600 * 2 * 2 ** 20),
                         "cli_open_files": 30 + i})
        lines = self.evaluate(rows)
        rss = self.line(lines, "rss_slope")
        self.assertEqual(rss["status"], "fail")
        self.assertAlmostEqual(rss["value"], 2.0, places=2)
        self.assertEqual(rss["per_process"]["daemon (procs=1)"]["status"], "pass")
        files = self.line(lines, "open_files_slope")
        self.assertEqual(files["status"], "fail")
        self.assertEqual(files["per_process"]["daemon (procs=1)"]["status"], "pass")

    def test_a_change_in_process_count_is_not_a_leak(self):
        rows = []
        for i in range(0, 25):
            n = 1 if i < 12 else 2  # a second daemon from then on: the sum doubles
            rows.append({"utc": self.stamp(i * 600), "daemon_procs": n,
                         "daemon_rss_bytes": n * 50 * 2 ** 20, "daemon_open_files": n * 10})
        lines = self.evaluate(rows)
        self.assertNotEqual(self.line(lines, "rss_slope")["status"], "fail")
        self.assertNotEqual(self.line(lines, "open_files_slope")["status"], "fail")

    def test_roles_without_processes_are_ignored(self):
        rows = [{"utc": self.stamp(i * 600), "other_procs": 0, "daemon_procs": 1,
                 "daemon_rss_bytes": 1000, "daemon_open_files": 5} for i in range(19)]
        rss = self.line(self.evaluate(rows), "rss_slope")
        self.assertEqual(list(rss["per_process"]), ["daemon (procs=1)"])
        self.assertEqual(rss["status"], "pass")

    def test_too_few_samples_is_insufficient(self):
        rows = [{"utc": self.stamp(t), "daemon_procs": 1, "daemon_rss_bytes": 1000,
                 "daemon_open_files": 10} for t in (0, 600, 1200, 1800)]
        lines = self.evaluate(rows)
        self.assertEqual(self.line(lines, "rss_slope")["status"], campaign.INSUFFICIENT)
        self.assertEqual(self.line(lines, "open_files_slope")["status"], campaign.INSUFFICIENT)
        self.assertEqual(self.line(lines, "chain_bytes_max")["status"], campaign.INSUFFICIENT)
        no_file = campaign.evaluate_lines(self.info(), str(self.root / "missing.csv"))
        self.assertEqual(self.line(no_file, "rss_slope")["status"], campaign.INSUFFICIENT)

    def test_chain_bytes_capped_with_entry_bytes_and_free_disk(self):
        gib = 1024 ** 3
        rows = [{"utc": self.stamp(0), "chain_bytes": gib, "entry_bytes": gib // 2,
                 "disk_free_bytes": 50 * gib},
                {"utc": self.stamp(600), "chain_bytes": 2 * gib + 1, "entry_bytes": gib,
                 "disk_free_bytes": 49 * gib}]
        line = self.line(self.evaluate(rows), "chain_bytes_max")
        self.assertEqual(line["value"], 2 * gib + 1)
        self.assertEqual(line["status"], "fail")
        self.assertEqual(line["entry_bytes_max"], gib)
        self.assertEqual(line["min_free_disk_bytes"], 49 * gib)
        rows[1]["chain_bytes"] = gib + 5
        self.assertEqual(self.line(self.evaluate(rows), "chain_bytes_max")["status"], "pass")

    def cut(self, end_s):
        return {"kind": "network_cut", "seconds": 600, "status": "done",
                "started_utc": self.stamp(end_s - 600), "ended_utc": self.stamp(end_s)}

    def ack(self, rows, end_s=3000):
        rows = [{"utc": self.stamp(t), "ack_lag_seq": lag, "ack_age_s": age}
                for t, lag, age in rows]
        return self.line(self.evaluate(rows, drills=[self.cut(end_s)]), "ack_recovery")

    def test_ack_recovers_when_every_record_is_acknowledged(self):
        line = self.ack([(2900, 50, 700), (3000, 80, 800), (3300, 90, 1100), (3900, 0, 5),
                         (6000, 0, 3)])
        self.assertEqual(line["status"], "pass")
        self.assertEqual(line["value"], 900)
        self.assertIn("ack_lag_seq == 0", line["rule"])

    def test_ack_recovers_on_fresh_acks_with_a_shrinking_backlog(self):
        line = self.ack([(3000, 80, 800), (3600, 40, 30), (6000, 3, 10)])
        self.assertEqual(line["status"], "pass")
        self.assertEqual(line["value"], 600)

    def test_fresh_acks_with_a_growing_backlog_are_not_recovery(self):
        line = self.ack([(3000, 80, 800), (3600, 90, 30), (4200, 120, 30), (4800, 150, 30),
                         (5400, 170, 20)])
        self.assertEqual(line["status"], "fail")

    def test_ack_never_recovers_fails(self):
        line = self.ack([(3000 + i * 300, 100 + i, 600 + i * 300) for i in range(8)])
        self.assertEqual(line["status"], "fail")

    def test_ack_window_not_observed_is_insufficient(self):
        line = self.ack([(3000, 100, 900), (3300, 120, 1200)])
        self.assertEqual(line["status"], campaign.INSUFFICIENT)
        rows = [{"utc": self.stamp(0), "ack_lag_seq": 0, "ack_age_s": 1}]
        none = self.line(self.evaluate(rows), "ack_recovery")
        self.assertEqual(none["status"], "not applicable")

    def test_early_stop_fails_even_when_a_drill_caused_it(self):
        log = self.root / "run.log"
        log.write_text("2026-10-23T08:00:00Z | codex was stopped: the disk is almost full\n")
        lines = campaign.evaluate_lines(self.info(), None, str(log))
        self.assertEqual(self.line(lines, "no_early_stop")["status"], "fail")
        log.write_text("2026-10-23T09:00:00Z | Time is up. codex was stopped.\n"
                       "2026-10-23T09:00:00Z | [campaign] DRILL-END cell=a kind=pause do: x was stopped: y\n")
        lines = campaign.evaluate_lines(self.info(), None, str(log))
        self.assertEqual(self.line(lines, "no_early_stop")["status"], "pass")
        short = campaign.evaluate_lines(self.info(elapsed_s=100), None, str(log))
        self.assertEqual(self.line(short, "no_early_stop")["status"], "fail")

    def test_upload_stopped_or_ended_before_the_duration_is_an_early_stop(self):
        for flag in ("upload_stopped", "ended"):
            rows = [{"utc": self.stamp(600), flag: 0}, {"utc": self.stamp(1200), flag: 1}]
            line = self.line(self.evaluate(rows), "no_early_stop")
            self.assertEqual(line["status"], "fail", flag)
            self.assertEqual(line["early_stop_samples"][0]["utc"], self.stamp(1200))
        rows = [{"utc": self.stamp(3700), "ended": 1}]  # after start + duration: the deadline
        self.assertEqual(self.line(self.evaluate(rows), "no_early_stop")["status"], "pass")

    def test_upload_facts(self):
        rows = [{"utc": self.stamp(0), "upload_stopped": 0, "upload_problem": 0,
                 "heartbeat_failures": 0, "ended": 0},
                {"utc": self.stamp(600), "upload_stopped": 0, "upload_problem": 1,
                 "heartbeat_failures": 4, "ended": 0},
                {"utc": self.stamp(1200), "upload_stopped": 1, "upload_problem": 1,
                 "heartbeat_failures": 2, "ended": 1}]
        path = self.root / "s.csv"
        write_samples(path, rows)
        facts = campaign.upload_facts_for(str(path))
        self.assertEqual(facts["upload_stopped_samples"], 1)
        self.assertEqual(facts["first_upload_stopped_utc"], self.stamp(1200))
        self.assertEqual(facts["upload_problem_samples"], 2)
        self.assertEqual(facts["first_upload_problem_utc"], self.stamp(600))
        self.assertEqual(facts["heartbeat_failures_max"], 4)
        self.assertEqual(facts["heartbeat_failures_last"], 2)
        self.assertEqual(facts["ended_samples"], 1)

    def test_time_parsing(self):
        for text in ("2026-10-23T08:00:00Z", "2026-10-23T08:00:00.1234567+00:00",
                     "2026-10-23 08:00:00", str(T0.timestamp())):
            self.assertEqual(campaign.parse_time(text).replace(microsecond=0), T0, text)
        self.assertIsNone(campaign.parse_time("nope"))


class RealSamplerIntegration(Base):
    """research-sample.py run for real (only its CLI), then `campaign.py evaluate` for real."""

    def chain_line(self, seq):
        return json.dumps({"seq": seq, "recorded_at": campaign.utc(datetime.now(timezone.utc)),
                           "body": {"kind": "capture"}}, separators=(",", ":")) + "\n"

    def set_state(self, **kw):
        state = {"updated_at": int(time.time()), "recording": "ok", "ended": False,
                 "upload": {"last_ok_at": int(time.time()), "problem": None, "stopped": False},
                 "heartbeat": {"failures": 0}}
        for k, v in kw.items():
            if k in ("problem", "stopped"):
                state["upload"][k] = v
            elif k == "failures":
                state["heartbeat"]["failures"] = v
            else:
                state[k] = v
        (self.run_dir / "state.json").write_text(json.dumps(state))

    def write_chain(self, last_seq, acked):
        (self.trace / "chain.jsonl").write_text("".join(self.chain_line(i) for i in range(last_seq + 1)))
        now = campaign.utc(datetime.now(timezone.utc))
        (self.trace / "acks.jsonl").write_text(json.dumps({"seq": acked, "acked_at": now}) + "\n")

    def sample_once(self, csv_path):
        done = subprocess.run(
            [sys.executable, str(HERE / "research-sample.py"), "--config", str(self.cfg),
             "--out", str(csv_path), "--once", "--pid-match", "no-such-process-campaign-test"],
            stdin=subprocess.DEVNULL, capture_output=True, timeout=60, check=False)
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_evaluate_reads_what_the_real_sampler_writes(self):
        self.cfg = self.root / "cfg"
        self.trace = self.root / "trace"
        self.run_dir = self.cfg / "runs" / "run-1"
        self.trace.mkdir()
        self.run_dir.mkdir(parents=True)
        (self.run_dir / "run.json").write_text(json.dumps({
            "run_id": "run-1", "started_at_unix": int(time.time()),
            "entries": [{"entry_id": "e0000000000000000", "trace_dir": str(self.trace)}]}))
        out = self.root / "campaign-out"
        cell = out / "c"
        cell.mkdir(parents=True)
        csv_path = cell / "samples.csv"
        start = datetime.now(timezone.utc) - timedelta(seconds=30)
        cut_end = campaign.utc(datetime.now(timezone.utc) - timedelta(seconds=2))

        self.write_chain(4, 1)                     # 3 records behind
        self.set_state(problem="the server did not answer", failures=3)
        self.sample_once(csv_path)
        self.write_chain(6, 2)                     # 4 behind: the backlog grows
        self.sample_once(csv_path)
        self.write_chain(6, 6)                     # every record acknowledged
        self.set_state()
        self.sample_once(csv_path)
        self.set_state(ended=True, stopped=True)   # long before the duration
        self.sample_once(csv_path)

        (out / "campaign.json").write_text(json.dumps({"name": "it", "cells": ["c"]}))
        (cell / "run.log").write_text("2026-10-23T08:00:00Z | x linux 0 PASS d\n")
        (cell / "result.json").write_text(json.dumps({
            "cell": "c", "where": "host", "connector": "codex-cli", "verdict": "PASS",
            "exit_code": 0, "final_row": "x linux 0 PASS d", "duration_s": 3600,
            "start_utc": campaign.utc(start), "elapsed_s": 3700, "interrupted": False,
            "run_log": str(cell / "run.log"), "sampler": {"path": str(csv_path)},
            "drills": [{"kind": "network_cut", "seconds": 60, "status": "done",
                        "started_utc": cut_end, "ended_utc": cut_end}]}))
        done = subprocess.run([sys.executable, str(HERE / "campaign.py"), "evaluate", str(out),
                               "--json"], capture_output=True, timeout=60, check=False)
        self.assertEqual(done.returncode, 1, done.stderr)  # the early stop fails the cell
        res = json.loads(done.stdout)[0]
        lines = {l["name"]: l for l in res["pass_lines"]}
        chain_size = (self.trace / "chain.jsonl").stat().st_size
        self.assertEqual(lines["chain_bytes_max"]["value"], chain_size)
        self.assertEqual(lines["chain_bytes_max"]["entry_bytes_max"], chain_size)
        self.assertIsNotNone(lines["chain_bytes_max"]["min_free_disk_bytes"])
        self.assertEqual(lines["ack_recovery"]["status"], "pass")
        self.assertEqual(lines["ack_recovery"]["per_cut"][0]["ack_lag_seq"], 0)
        self.assertEqual(lines["no_early_stop"]["status"], "fail")
        self.assertTrue(lines["no_early_stop"]["early_stop_samples"][0]["ended"])
        self.assertEqual(res["upload"]["samples"], 4)
        self.assertEqual(res["upload"]["upload_stopped_samples"], 1)
        self.assertEqual(res["upload"]["upload_problem_samples"], 2)  # samples 1 and 2
        self.assertEqual(res["upload"]["heartbeat_failures_max"], 3)
        self.assertEqual(res["upload"]["ended_samples"], 1)
        # evaluate rewrote result.json with the same findings.
        saved = json.loads((cell / "result.json").read_text())
        self.assertEqual(saved["overall"], "FAIL")
        self.assertEqual(saved["upload"], res["upload"])


class CampaignRun(Base):
    def make(self, cells, runner, clock=None, parallel=4, **defaults):
        cfg = self.config(cells, **defaults)
        said = []
        camp = campaign.Campaign(cfg, cfg["cells"], self.root / "out", parallel, runner=runner,
                                 clock=clock or FakeClock(), say=said.append)
        return camp, said

    def all_text(self):
        out = []
        for p in (self.root / "out").rglob("*"):
            if p.is_file():
                out.append(p.read_text(errors="replace"))
        return "\n".join(out)

    def test_results_and_summaries(self):
        runner = FakeRunner()
        cells = [self.docker_cell("lin", "m1"), self.host_cell("mac", "m2")]

        def make_proc(argv):
            if "matrix.sh" in argv[1] and "linux" not in argv[1]:
                return FakeProc(["hello", "codex-cli darwin-arm64 0.1 FAIL 2026-10-23",
                                 "log: /tmp/matrix.log"], rc=1)
            return FakeProc(["linux-matrix: build", "linux-matrix: codex-cli linux-x86_64 0.1 PASS 2026-10-23"])
        runner.make_proc = make_proc
        camp, said = self.make(cells, runner)
        rc = camp.run()
        self.assertEqual(rc, 1)
        out = self.root / "out"
        lin = json.loads((out / "lin" / "result.json").read_text())
        self.assertEqual(lin["verdict"], "PASS")
        self.assertEqual(lin["final_row"], "codex-cli linux-x86_64 0.1 PASS 2026-10-23")
        self.assertEqual(lin["exit_code"], 0)
        self.assertEqual(lin["container"], "goalcup-linux-matrix-codex-cli-lin")
        self.assertTrue(lin["sampler"]["path"].endswith("lin/logs/samples.csv"))
        for key in ("cell", "connector", "where", "start_utc", "end_utc", "drills", "pass_lines"):
            self.assertIn(key, lin)
        mac = json.loads((out / "mac" / "result.json").read_text())
        self.assertEqual(mac["verdict"], "FAIL")
        self.assertEqual(mac["exit_code"], 1)
        summary = json.loads((out / "summary.json").read_text())
        self.assertEqual({c["cell"]: c["verdict"] for c in summary["cells"]},
                         {"lin": "PASS", "mac": "FAIL"})
        md = (out / "summary.md").read_text()
        self.assertIn("| lin | docker | codex-cli | PASS |", md)
        self.assertIn("2026-10-23T08:00:00Z | hello", (out / "mac" / "run.log").read_text())
        # The docker cell: linux-matrix with the cell's name, its sampler inside the container.
        run_argv = next(c for c in runner.calls if "linux-matrix.sh" in " ".join(c))
        self.assertEqual(run_argv[2:9], ["run", "codex-cli", "--", "--test", "--duration", "3600",
                                         "--timeout"])
        env = runner.envs[runner.calls.index(run_argv)]
        self.assertEqual(env["LINUX_MATRIX_CELL"], "lin")
        self.assertEqual(env["LINUX_MATRIX_PREBUILT"], "1")
        self.assertEqual(env["GOALCUP_TEST_MEMBER_ID"], "m1")
        self.assertEqual(env["LINUX_MATRIX_LOG_DIR"], str(out / "lin" / "logs"))
        host_argv = next(c for c in runner.calls if c[1].endswith("/matrix.sh"))
        self.assertIn("--test", host_argv)
        host_env = runner.envs[runner.calls.index(host_argv)]
        self.assertEqual(host_env["MATRIX_CONFIG_DIR"], cells[1]["config_dir"])
        self.assertEqual(mac["final_row"], "codex-cli darwin-arm64 0.1 FAIL 2026-10-23")
        # The host sampler counts only the processes of the cell's own configuration directory.
        host_sampler = next(c for c in runner.calls
                            if "research-sample.py" in " ".join(c) and c[0] != "docker")
        i = host_sampler.index("--pid-match")
        self.assertEqual(host_sampler[i + 1], cells[1]["config_dir"] + "/")
        self.assertEqual(host_env["MATRIX_PROMPT_FILE"], str(self.prompt))
        # status and evaluate read the files back.
        st = campaign.status_rows(out)
        self.assertEqual([c["state"] for c in st["cells"]], ["done", "done"])
        res = campaign.evaluate_dir(out)
        self.assertEqual([r["cell"] for r in res], ["lin", "mac"])

    def test_docker_sampler_waits_for_the_container(self):
        runner = FakeRunner()
        gate = threading.Event()
        proc = FakeProc(["linux-matrix: x linux-x86_64 0.1 PASS 2026-10-23"], finish=False)
        runner.make_proc = lambda argv: proc
        runner.on_run = lambda argv: "{{.State.Running}}" in argv and (gate.set(), proc_done())

        def proc_done():
            threading.Timer(0.2, lambda: proc.finish(0)).start()
        camp, _ = self.make([self.docker_cell("lin", "m1")], runner)
        camp.run()
        exec_argv = next(c for c in runner.calls if c[:2] == ["docker", "exec"])
        self.assertEqual(exec_argv[:8], ["docker", "exec", "-u", "dev",
                                         "goalcup-linux-matrix-codex-cli-lin", "python3",
                                         "/src/scripts/research-sample.py", "--config"])
        self.assertIn("/logs/samples.csv", exec_argv)
        inspect_i = next(i for i, c in enumerate(runner.calls) if "{{.State.Running}}" in c)
        self.assertLess(inspect_i, runner.calls.index(exec_argv))
        # The sampler is stopped when the cell ends.
        sampler = next(p for a, p in runner.procs if a == exec_argv)
        self.assertIn((sampler.pid, signal.SIGTERM), runner.signals)

    def test_pause_drill_runs_in_a_campaign(self):
        runner = FakeRunner()
        proc = FakeProc(["linux-matrix: x linux-x86_64 0.1 PASS 2026-10-23"], finish=False)
        runner.make_proc = lambda argv: proc
        runner.on_run = lambda argv: argv[1] == "unpause" and proc.finish(0)
        camp, said = self.make([self.docker_cell("lin", "m1", drills=[
            {"at_s": 60, "kind": "pause", "seconds": 120}])], runner)
        camp.run()
        res = json.loads((self.root / "out" / "lin" / "result.json").read_text())
        self.assertEqual([d["status"] for d in res["drills"]], ["done"])
        self.assertTrue(res["drills"][0]["started_utc"] and res["drills"][0]["ended_utc"])
        self.assertTrue(any(l.startswith("DRILL-NOW cell=lin kind=pause") for l in said))
        # The drill's lines are in run.log, tagged, and never taken for the matrix row.
        self.assertIn("| [campaign] DRILL-NOW cell=lin", (self.root / "out" / "lin" / "run.log").read_text())
        self.assertEqual(res["final_row"], "x linux-x86_64 0.1 PASS 2026-10-23")
        self.assertEqual(res["verdict"], "PASS")
        events = [json.loads(l)["event"] for l in
                  (self.root / "out" / "lin" / "events.jsonl").read_text().splitlines()]
        self.assertIn("drill-start", events)
        self.assertIn("drill-end", events)

    def test_one_cell_failing_never_stops_the_others(self):
        runner = FakeRunner()
        real_popen = runner.popen

        def popen(argv, env=None, cwd=None, stdout=None):
            if "bad" in (env or {}).get("LINUX_MATRIX_CELL", ""):
                raise OSError("exec format error")
            return real_popen(argv, env, cwd, stdout)
        runner.popen = popen
        camp, _ = self.make([self.docker_cell("bad", "m1"), self.docker_cell("good", "m2")], runner)
        self.assertEqual(camp.run(), 1)
        out = self.root / "out"
        bad = json.loads((out / "bad" / "result.json").read_text())
        good = json.loads((out / "good" / "result.json").read_text())
        self.assertEqual(bad["verdict"], "FAIL")
        self.assertIn("exec format error", bad["error"])
        self.assertEqual(good["verdict"], "PASS")

    def test_parallel_limit(self):
        runner = FakeRunner()
        running, peak = [0], [0]
        lock = threading.Lock()

        def make_proc(argv):
            p = FakeProc(["linux-matrix: x linux-x86_64 0 PASS 2026-10-23"], finish=False)
            with lock:
                running[0] += 1
                peak[0] = max(peak[0], running[0])

            def end():
                with lock:
                    running[0] -= 1
                p.finish(0)
            threading.Timer(0.1, end).start()
            return p
        runner.make_proc = make_proc
        cells = [self.docker_cell(f"c{i}", f"m{i}") for i in range(4)]
        camp, _ = self.make(cells, runner, parallel=2)
        camp.run()
        self.assertLessEqual(peak[0], 2)
        self.assertEqual([r.result["verdict"] for r in camp.runs], ["PASS"] * 4)

    def test_no_secret_value_or_key_path_in_argv_logs_or_files(self):
        runner = FakeRunner()
        cells = [self.docker_cell("lin", "m1"), self.host_cell("mac", "m2", drills=[
            {"at_s": 0, "kind": "note", "text": "hi"}])]
        camp, said = self.make(cells, runner)
        camp.run()
        for argv in runner.calls:
            joined = " ".join(argv)
            self.assertNotIn(SENTINEL, joined)
            self.assertNotIn(str(self.key), joined)
        for env in runner.envs:
            for k, v in env.items():
                self.assertNotIn(SENTINEL, v)
                if str(self.key) in v:
                    self.assertEqual(k, campaign.KEY_ENV)
        # Samplers do not get the key path at all.
        for argv, env in zip(runner.calls, runner.envs):
            if "research-sample.py" in " ".join(argv):
                self.assertNotIn(campaign.KEY_ENV, env)
        text = self.all_text() + "\n".join(said)
        self.assertNotIn(SENTINEL, text)
        self.assertNotIn(str(self.key), text)

    def test_the_github_token_reaches_github_json_and_nowhere_else(self):
        runner = FakeRunner()
        runner.real_kit = True
        cells = [self.docker_cell("lin", "m1"), self.host_cell("mac", "m2")]
        camp, said = self.make(cells, runner)
        camp.run()
        cfgdir = Path(cells[1]["config_dir"])
        gh = json.loads((cfgdir / "github.json").read_text())
        self.assertEqual(gh, {"repo_url": "https://github.com/agentcommunity/goalcup-matrix-empty",
                              "token": TOKEN})
        for name in ("github.json", "prep.json"):
            self.assertEqual((cfgdir / name).stat().st_mode & 0o777, 0o600, name)
        # The host cell settled before its matrix.sh started.
        settle = next(i for i, c in enumerate(runner.calls) if str(campaign.KIT) in c)
        host = next(i for i, c in enumerate(runner.calls) if c[1].endswith("/matrix.sh"))
        self.assertLess(settle, host)
        # The docker cell gets the path, for linux-matrix.sh to mount.
        lin_env = next(e for e in runner.envs if e.get("LINUX_MATRIX_CELL") == "lin")
        self.assertEqual(lin_env[campaign.TOKEN_ENV], str(self.token))
        for argv in runner.calls:
            self.assertNotIn(TOKEN, " ".join(argv))
            self.assertNotIn(str(self.token), " ".join(argv))
        for env in runner.envs:
            for k, v in env.items():
                self.assertNotIn(TOKEN, v)
                if str(self.token) in v:
                    self.assertEqual(k, campaign.TOKEN_ENV)
        self.assertNotIn(TOKEN, self.all_text() + "\n".join(said))

    def test_a_host_cell_whose_settle_is_refused_never_starts(self):
        self.token.write_text("short\n")  # not a token: the helper refuses it
        runner = FakeRunner()
        runner.real_kit = True
        camp, _ = self.make([self.host_cell("mac", "m1")], runner)
        self.assertEqual(camp.run(), 1)
        self.assertFalse(any(c[1].endswith("/matrix.sh") for c in runner.calls if len(c) > 1))
        res = json.loads((self.root / "out" / "mac" / "result.json").read_text())
        self.assertEqual(res["verdict"], "FAIL")
        self.assertIn("settle exited 1", res["error"])

    def host_run(self, cell, writes):
        """One host cell run with the real kit helper; the fake matrix.sh calls writes(cfgdir)
        and prints a PASS row. The cell's result.json."""
        runner = FakeRunner()
        runner.real_kit = True

        def make_proc(argv):
            writes(Path(cell["config_dir"]))
            return FakeProc([f"{cell.get('connector', 'codex-cli')} darwin-arm64 0.1 PASS 2026-10-23"])
        runner.make_proc = make_proc
        camp, _ = self.make([cell], runner)
        camp.run()
        return json.loads((self.root / "out" / cell["id"] / "result.json").read_text())

    @staticmethod
    def rollout(cfgdir, *turns):
        d = cfgdir / "capture-homes/codex-cli/sessions/2026/10/23"
        d.mkdir(parents=True, exist_ok=True)
        (d / "rollout-a.jsonl").write_text("".join(
            json.dumps({"type": "turn_context", "payload": {"model": m, "effort": e}}) + "\n"
            for m, e in turns))

    @staticmethod
    def transcript(cfgdir, *models):
        d = cfgdir / "capture-homes/claude-code-cli/projects/-w"
        d.mkdir(parents=True, exist_ok=True)
        (d / "s.jsonl").write_text("".join(
            json.dumps({"type": "assistant", "message": {"model": m}}) + "\n" for m in models))

    def test_a_host_cells_homes_get_the_cheapest_model_rules(self):
        cell = self.host_cell("mac-codex", "m1")
        res = self.host_run(cell, lambda d: self.rollout(d, ("gpt-5.6-luna", "low")))
        self.assertEqual(res["verdict"], "PASS", res["final_row"])
        home = Path(cell["config_dir"]) / "capture-homes"
        self.assertEqual((home / "codex-cli/config.toml").read_text().splitlines()[:2],
                         ['model = "gpt-5.6-luna"', 'model_reasoning_effort = "low"'])
        cell = self.host_cell("mac-claude", "m2", connector="claude-code-cli")
        res = self.host_run(cell, lambda d: self.transcript(d, "claude-haiku-4-5"))
        self.assertEqual(res["verdict"], "PASS", res["final_row"])
        settings = json.loads((Path(cell["config_dir"]) /
                               "capture-homes/claude-code-cli/settings.json").read_text())
        self.assertEqual((settings["model"], settings["alwaysThinkingEnabled"]), ("haiku", False))

    def test_a_turn_on_another_model_fails_the_host_cell(self):
        cell = self.host_cell("mac-codex", "m1")
        res = self.host_run(cell, lambda d: self.rollout(d, ("gpt-5.6-luna", "low"),
                                                         ("gpt-6-astra", "low")))
        self.assertEqual(res["verdict"], "FAIL")
        self.assertTrue(res["final_row"].startswith("FAIL owner rule: "), res["final_row"])
        self.assertIn("gpt-6-astra", res["final_row"])
        cell = self.host_cell("mac-claude", "m2", connector="claude-code-cli")
        res = self.host_run(cell, lambda d: self.transcript(d, "claude-sonnet-4-5"))
        self.assertEqual(res["verdict"], "FAIL")
        res = self.host_run(self.host_cell("mac-none", "m3"), lambda d: None)
        self.assertEqual(res["verdict"], "FAIL")
        self.assertIn("could not be confirmed", res["final_row"])

    def matrix_sh_tail(self, result):
        """What the real scripts/matrix.sh prints last: its print_row function, run."""
        src = (HERE / "matrix.sh").read_text()
        fn = re.search(r"^print_row\(\) \{\n.*?^\}\n", src, re.S | re.M).group(0)
        script = ("exec 3>/dev/null\nlog() { :; }\n" + fn
                  + f"ROW_PRINTED=0\nHV='0.1 beta'\nRESULT={result}\nFAIL_REASON=\nCONNECTOR=codex-cli\n"
                  + "PLATFORM=darwin-arm64\nDATE=2026-10-23\nLOG=/tmp/m.log\nprint_row\n")
        out = subprocess.run(["sh", "-c", script], capture_output=True, text=True, check=True)
        lines = out.stdout.splitlines()
        self.assertTrue(lines[-1].startswith("log: "), lines)  # the shape this test is about
        return lines

    def test_a_host_cell_passes_on_the_real_matrix_sh_output(self):
        for result, rc, verdict in (("PASS", 0, "PASS"), ("FAIL", 1, "FAIL")):
            with self.subTest(result=result):
                runner = FakeRunner()
                lines = ["matrix: something"] + self.matrix_sh_tail(result)
                runner.make_proc = lambda argv: FakeProc(lines, rc=rc)
                camp, _ = self.make([self.host_cell("mac", "m1")], runner)
                camp.run()
                res = json.loads((self.root / "out" / "mac" / "result.json").read_text())
                self.assertEqual(res["final_row"], f"codex-cli darwin-arm64 0.1_beta {result} 2026-10-23")
                self.assertEqual(res["verdict"], verdict)

    def test_a_linux_matrix_fail_after_the_row_is_the_final_row(self):
        runner = FakeRunner()
        runner.make_proc = lambda argv: FakeProc([
            "codex-cli linux-x86_64 0.1 PASS 2026-10-23", "log: /logs/m.log",
            "linux-matrix: FAIL owner rule: luna"], rc=1)
        camp, _ = self.make([self.docker_cell("lin", "m1")], runner)
        camp.run()
        res = json.loads((self.root / "out" / "lin" / "result.json").read_text())
        self.assertEqual(res["final_row"], "FAIL owner rule: luna")
        self.assertEqual(res["verdict"], "FAIL")

    def test_a_second_ctrl_c_cannot_cut_the_shutdown_short(self):
        runner = FakeRunner()
        seen = []
        real_signal_group = runner.signal_group

        def signal_group(proc, sig):
            seen.append((signal.getsignal(signal.SIGINT), signal.getsignal(signal.SIGTERM)))
            real_signal_group(proc, sig)
        runner.signal_group = signal_group
        started = threading.Event()

        def make_proc(argv):
            started.set()
            return FakeProc(["matrix: running"], finish=False)
        runner.make_proc = make_proc
        clock = FakeClock()
        clock.interrupt = started.is_set
        before = signal.getsignal(signal.SIGINT)
        camp, _ = self.make([self.host_cell("mac", "m1")], runner, clock)
        self.assertEqual(camp.run(), 130)
        self.assertTrue(seen)
        for handlers in seen:
            self.assertEqual(handlers, (signal.SIG_IGN, signal.SIG_IGN))
        self.assertEqual(signal.getsignal(signal.SIGINT), before)

    def test_ctrl_c_undoes_the_drill_and_stops_every_child(self):
        runner = FakeRunner()
        clock = FakeClock()
        clock.hold = lambda timeout: timeout == 999
        paused = threading.Event()
        runner.on_run = lambda argv: argv[1] == "pause" and paused.set()
        clock.interrupt = paused.is_set
        procs = []

        def make_proc(argv):
            p = FakeProc(["linux-matrix: build"], finish=False)
            procs.append(p)
            return p
        runner.make_proc = make_proc
        cells = [self.docker_cell("lin", "m1", drills=[{"at_s": 30, "kind": "pause", "seconds": 999}]),
                 self.host_cell("mac", "m2")]
        camp, said = self.make(cells, runner, clock)
        rc = camp.run()
        self.assertEqual(rc, 130)
        v = runner.verbs()
        self.assertIn("unpause goalcup-linux-matrix-codex-cli-lin", v)
        self.assertLess(v.index("pause goalcup-linux-matrix-codex-cli-lin"),
                        v.index("unpause goalcup-linux-matrix-codex-cli-lin"))
        self.assertTrue(procs and all(p.done.is_set() for p in procs))
        for p in procs:
            self.assertIn((p.pid, signal.SIGTERM), runner.signals)
        # The unpause came before any SIGTERM.
        unpause_at = runner.order.index(("docker", "unpause"))
        first_term = min(i for i, o in enumerate(runner.order) if o == ("signal", signal.SIGTERM))
        self.assertLess(unpause_at, first_term)
        for cid in ("lin", "mac"):
            res = json.loads((self.root / "out" / cid / "result.json").read_text())
            self.assertTrue(res["interrupted"])
            self.assertEqual(res["verdict"], "FAIL")
        lin = json.loads((self.root / "out" / "lin" / "result.json").read_text())
        self.assertEqual(lin["drills"][0]["status"], "cut short")


class Commands(Base):
    def write(self, cells, token=True):
        defaults = {"approval_key_file": str(self.key), "connector": "codex-cli",
                    "duration_s": 600}
        if token:
            defaults["github_token_file"] = str(self.token)
        path = self.root / "c.json"
        path.write_text(json.dumps({"defaults": defaults, "cells": cells}))
        return str(path)

    def test_login_host_runs_login_then_prep_in_its_config_dir(self):
        cfgdir = self.root / "new-cfg"
        path = self.write([{"id": "mac", "where": "host", "test_member_id": "m1",
                            "config_dir": str(cfgdir)}])
        runner = FakeRunner()
        rc = campaign.main(["login", path, "mac"], runner=runner)
        self.assertEqual(rc, 0)
        self.assertEqual(runner.calls[0][:3], ["goalcup", "login", "--server"])
        self.assertEqual(runner.calls[1], ["goalcup", "prep", "--harness", "codex-cli"])
        self.assertEqual(runner.envs[0]["GOALCUP_CONFIG_DIR"], str(cfgdir))
        self.assertTrue(cfgdir.is_dir())
        self.assertNotIn(campaign.KEY_ENV, runner.envs[0])

    def test_host_login_leaves_prep_and_github_json(self):
        cfgdir = self.root / "new-cfg"
        path = self.write([{"id": "mac", "where": "host", "test_member_id": "m1",
                            "config_dir": str(cfgdir)}])
        runner = FakeRunner()
        runner.real_kit = True
        with redirect_stdout(io.StringIO()) as buf:
            rc = campaign.main(["login", path, "mac"], runner=runner)
        self.assertEqual(rc, 0, buf.getvalue())
        prep = json.loads((cfgdir / "prep.json").read_text())
        self.assertEqual(prep["repo_url"], "https://github.com/agentcommunity/goalcup-matrix-empty")
        self.assertEqual(prep["work_root"], str(cfgdir / "work"))
        self.assertTrue((cfgdir / "work").is_dir())
        self.assertEqual(json.loads((cfgdir / "github.json").read_text())["token"], TOKEN)
        self.assertEqual((cfgdir / "github.json").stat().st_mode & 0o777, 0o600)
        self.assertNotIn(TOKEN, buf.getvalue() + json.dumps(runner.calls))

    def test_a_cell_without_the_token_file_is_refused_before_anything_starts(self):
        for where in ("docker", "host"):
            with self.subTest(where=where):
                cell = {"id": "a", "where": where, "test_member_id": "m1"}
                if where == "host":
                    cell["config_dir"] = str(self.root)
                path = self.write([cell], token=False)
                runner = FakeRunner()
                with redirect_stdout(io.StringIO()) as buf:
                    rc = campaign.main(["run", path, "--out", str(self.root / "o")],
                                       runner=runner, clock=FakeClock())
                self.assertEqual(rc, 2)
                self.assertEqual(runner.calls, [])
                self.assertIn("no GitHub token file", buf.getvalue())
                with redirect_stdout(io.StringIO()):
                    self.assertEqual(campaign.main(["check", path, "--static"], runner=runner), 1)

    def test_login_docker_delegates_with_the_cell(self):
        path = self.write([{"id": "lin", "where": "docker", "test_member_id": "m1"}])
        runner = FakeRunner()
        self.assertEqual(campaign.main(["login", path, "lin"], runner=runner), 0)
        self.assertEqual(runner.calls[0][2:], ["login", "codex-cli"])
        self.assertEqual(runner.envs[0]["LINUX_MATRIX_CELL"], "lin")
        self.assertEqual(runner.envs[0][campaign.KEY_ENV], str(self.key))
        self.assertEqual(runner.envs[0][campaign.TOKEN_ENV], str(self.token))

    def test_build_runs_linux_matrix_build_once(self):
        path = self.write([{"id": "a", "where": "docker", "test_member_id": "m1"},
                           {"id": "b", "where": "docker", "test_member_id": "m2"}])
        runner = FakeRunner()
        self.assertEqual(campaign.main(["build", path], runner=runner), 0)
        self.assertEqual(len(runner.calls), 1)
        self.assertEqual(runner.calls[0][2:], ["build", "codex-cli"])
        self.assertNotIn(campaign.KEY_ENV, runner.envs[0])  # a build needs no key

    def test_build_comes_before_the_host_cells_sign_in(self):
        # LOCAL-CAMPAIGN orders build, then login; login makes a host cell's config_dir.
        path = self.write([{"id": "lin", "where": "docker", "test_member_id": "m1"},
                           {"id": "mac", "where": "host", "test_member_id": "m2",
                            "config_dir": str(self.root / "not-yet")}])
        runner = FakeRunner()
        self.assertEqual(campaign.main(["build", path], runner=runner), 0)
        self.assertEqual(runner.calls[0][2:], ["build", "codex-cli"])

    def test_run_refuses_shared_member_before_starting_anything(self):
        path = self.write([{"id": "a", "where": "docker", "test_member_id": "m1"},
                           {"id": "b", "where": "docker", "test_member_id": "m1"}])
        runner = FakeRunner()
        with redirect_stdout(io.StringIO()) as buf:
            rc = campaign.main(["run", path, "--out", str(self.root / "o")], runner=runner,
                               clock=FakeClock())
        self.assertEqual(rc, 2)
        self.assertEqual(runner.calls, [])
        self.assertIn("share test_member_id", buf.getvalue())

    def test_example_config_parses_and_is_well_formed(self):
        cfg = campaign.load_config(HERE / "campaign.example.json")
        errs, _ = campaign.validate(cfg, cfg["cells"], 4)
        # Only the machine-specific paths may fail here.
        for e in errs:
            self.assertTrue("approval key" in e or "GitHub token" in e or "prompt_file" in e
                            or "config_dir" in e, e)


if __name__ == "__main__":
    unittest.main()
