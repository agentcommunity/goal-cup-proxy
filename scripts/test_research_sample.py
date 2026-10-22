"""Behavioral checks for the WP-F soak sampler; a fake config dir and fake process readers."""

import contextlib
import csv
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch


SPEC = importlib.util.spec_from_file_location(
    "research_sample", Path(__file__).with_name("research-sample.py"))
SAMPLE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SAMPLE)

SENTINEL = "SENTINEL-sk-ant-api03-DO-NOT-PRINT-7f3a"
NOW = 1_791_000_000  # 2026-10-02T...Z


def line(seq, recorded_at="2026-10-02T12:00:00Z", pad=0):
    return (json.dumps({"protocol_version": 5, "entry_id": "entry-" + SENTINEL,
                        "source": "daemon", "recorded_at": recorded_at, "seq": seq,
                        "prev": "0" * 64, "hash": "1" * 64,
                        "body": {"kind": "capture", "content": {"record": {
                            "kind": "producer_closed", "producer": SENTINEL,
                            "final_seq": 999, "pad": "x" * pad}}}},
                       separators=(",", ":")) + "\n").encode()


class FakeProcesses:
    def __init__(self, listed, rss=None, fds=None):
        self.listed, self.rss, self.fds = listed, rss or {}, fds or {}

    def list(self):
        return self.listed

    def rss_bytes(self, pid):
        value = self.rss.get(pid)
        if isinstance(value, Exception):
            raise value
        return value

    def open_files(self, pid):
        value = self.fds.get(pid)
        if isinstance(value, Exception):
            raise value
        return value


NO_PROCS = FakeProcesses([])


class SamplerCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.config = os.path.join(self.tmp.name, "cfg-" + SENTINEL)
        os.makedirs(os.path.join(self.config, "runs"))
        self.out = os.path.join(self.tmp.name, "samples.csv")

    def add_run(self, run_id, started, traces, state=None):
        run_dir = os.path.join(self.config, "runs", run_id)
        os.makedirs(run_dir)
        entries = [{"entry_id": "e%016d" % i, "trace_dir": trace} for i, trace in enumerate(traces)]
        with open(os.path.join(run_dir, "run.json"), "w") as stream:
            json.dump({"run_id": run_id, "server_url": "https://%s.example" % SENTINEL,
                       "started_at_unix": started, "entries": entries,
                       "workspace": "/home/%s/ws" % SENTINEL}, stream)
        if state is not None:
            with open(os.path.join(run_dir, "state.json"), "w") as stream:
                json.dump(state, stream)
        return run_dir

    def trace(self, name, parts, acks=None):
        trace_dir = os.path.join(self.tmp.name, name + SENTINEL)
        os.makedirs(trace_dir)
        for part_name, data in parts.items():
            with open(os.path.join(trace_dir, part_name), "wb") as stream:
                stream.write(data)
        if acks is not None:
            with open(os.path.join(trace_dir, "acks.jsonl"), "wb") as stream:
                stream.write(acks)
        return trace_dir

    def once(self, procs=NO_PROCS, clock=lambda: NOW):
        with contextlib.redirect_stderr(io.StringIO()) as err:
            code = SAMPLE.main(["--config", self.config, "--out", self.out, "--once"],
                               procs=procs, clock=clock, sleep=Mock())
        return code, err.getvalue()

    def rows(self):
        with open(self.out, newline="") as stream:
            return list(csv.DictReader(stream))


class SampleTest(SamplerCase):
    def test_one_row_per_once_with_chain_ack_state_and_processes(self):
        first = line(0) + line(1)
        second = line(2) + line(3, "2026-10-02T12:00:30.250Z")
        trace = self.trace("t", {
            "chain.jsonl": first,
            "chain.%020d.jsonl" % len(first): second,
            "chain.%020d.jsonl" % (len(first) + len(second)): b"",  # just rotated
        }, acks=b'{"entry_id":"x","seq":1,"hash":"h","acked_at":"2026-10-02T12:00:10Z",'
                b'"server_signature":"s"}\n{"seq":2,"acked_at":"2026-10-02T12:00:20Z"}\n{torn')
        old = self.trace("old", {"chain.jsonl": line(0)})
        self.add_run("older", 100, [old])
        self.add_run("newer", 200, [trace], state={
            "updated_at": NOW - 30, "entry_id": "e", "harness": {"exited": {"status": SENTINEL}},
            "recording": "gaps", "ended": False,
            "upload": {"last_ok_at": NOW - 90, "problem": "refused by %s" % SENTINEL,
                       "stopped": False},
            "heartbeat": {"failures": 2, "last_failure": SENTINEL},
            "disk": {"free_bytes": 7, "reserve_bytes": 1 << 30, "low": True},
            "sleep_lock": {"Err": SENTINEL}})
        procs = FakeProcesses(
            [(11, ["/opt/bin/goalcup-daemon", "--serve"]),
             (12, ["/opt/bin/goalcup", "start", SENTINEL]),
             (13, ["/opt/bin/goalcup", "upload"]),
             (14, ["python3", "scripts/research-sample.py", "--pid-match", "goalcup"]),
             (15, ["node", "/home/goalcup-run/codex"]),
             (16, ["bash"]),
             (os.getpid(), ["goalcup"])],
            rss={11: 4096, 12: 1000, 13: OSError("gone"), 15: 5},
            fds={11: 9, 12: 4, 13: 3, 15: PermissionError()})
        code, err = self.once(procs)
        self.assertEqual(code, 0)
        rows = self.rows()
        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertEqual(row["sample_errors"], "0")
        self.assertGreater(int(row["disk_free_bytes"]), 0)
        self.assertEqual((row["runs"], row["entries"]), ("2", "2"))
        self.assertEqual(row["chain_parts"], "4")
        self.assertEqual(int(row["chain_bytes"]), len(first) + len(second) + len(line(0)))
        self.assertEqual(row["entry_parts"], "3")
        self.assertEqual(int(row["entry_bytes"]), len(first) + len(second))
        self.assertEqual(row["chain_last_seq"], "3")  # from the part before the empty one
        self.assertEqual(row["acked_seq"], "2")
        self.assertEqual(row["ack_lag_seq"], "1")
        tail_at = SAMPLE.parse_time("2026-10-02T12:00:30.250Z")
        self.assertAlmostEqual(float(row["chain_idle_s"]), NOW - tail_at, places=0)
        self.assertEqual(float(row["ack_age_s"]),
                         round(NOW - SAMPLE.parse_time("2026-10-02T12:00:20Z"), 1))
        self.assertEqual(row["state_age_s"], "30")
        self.assertEqual(row["recording"], "gaps")
        self.assertEqual(row["harness"], "exited")
        self.assertEqual(row["upload_last_ok_age_s"], "90")
        self.assertEqual(row["upload_problem"], "1")
        self.assertEqual(row["heartbeat_failures"], "2")
        self.assertEqual(row["state_disk_free_bytes"], "7")
        self.assertEqual(row["state_disk_low"], "1")
        self.assertEqual((row["daemon_procs"], row["daemon_rss_bytes"], row["daemon_open_files"]),
                         ("1", "4096", "9"))
        self.assertEqual((row["cli_procs"], row["cli_rss_bytes"], row["cli_open_files"]),
                         ("2", "1000", "7"))
        self.assertEqual((row["other_procs"], row["other_rss_bytes"], row["other_open_files"]),
                         ("1", "5", ""))
        with open(self.out) as stream:
            text = stream.read()
        self.assertNotIn(SENTINEL, text + err)
        self.assertNotIn(self.tmp.name, text + err)

    def test_header_written_once_and_rows_appended(self):
        self.add_run("r", 1, [self.trace("t", {"chain.jsonl": line(0)})])
        self.once()
        self.once()
        with open(self.out) as stream:
            lines = stream.read().splitlines()
        self.assertEqual(len(lines), 3)
        self.assertEqual(lines[0].split(","), SAMPLE.COLUMNS)

    def test_tail_window_grows_past_a_long_last_line(self):
        big = line(41, pad=SAMPLE.TAIL_START + 1000)
        trace = self.trace("t", {"chain.jsonl": line(40) + big + line(42)[:30]})
        self.add_run("r", 1, [trace])
        self.once()
        self.assertEqual(self.rows()[0]["chain_last_seq"], "41")

    def test_missing_files_are_tolerated(self):
        gone = os.path.join(self.tmp.name, "gone")
        os.makedirs(os.path.join(self.config, "runs", "torn"))
        with open(os.path.join(self.config, "runs", "torn", "run.json"), "w") as stream:
            stream.write("{ torn")
        run_dir = self.add_run("r", 1, [gone])
        with open(os.path.join(run_dir, "state.json"), "w") as stream:
            stream.write("not json")
        code, err = self.once()
        self.assertEqual(code, 0)
        row = self.rows()[0]
        self.assertEqual((row["runs"], row["entries"], row["chain_parts"]), ("1", "1", "0"))
        self.assertEqual(row["sample_errors"], "2")  # parts and state
        self.assertEqual(row["acked_seq"], "")
        self.assertNotIn(self.tmp.name, err)

    def test_empty_config_still_writes_a_row(self):
        os.rmdir(os.path.join(self.config, "runs"))
        code, _err = self.once(procs=Mock(list=Mock(side_effect=OSError)))
        self.assertEqual(code, 0)
        row = self.rows()[0]
        self.assertEqual(row["runs"], "0")
        self.assertEqual(row["sample_errors"], "2")  # runs and process list

    def test_loop_survives_a_bad_sample_and_paces_itself(self):
        self.add_run("r", 1, [self.trace("t", {"chain.jsonl": line(0)})])
        real = SAMPLE.sample
        calls = Mock(side_effect=[RuntimeError("boom " + SENTINEL), real(self.config, NO_PROCS,
                                                                          ["goalcup"], NOW)])
        sleep = Mock(side_effect=[None, KeyboardInterrupt])
        with patch.object(SAMPLE, "sample", calls), \
                contextlib.redirect_stderr(io.StringIO()) as err:
            code = SAMPLE.main(["--config", self.config, "--out", self.out, "--every", "60"],
                               procs=NO_PROCS, clock=lambda: NOW, sleep=sleep)
        self.assertEqual(code, 130)
        rows = self.rows()
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["sample_errors"], "1")
        self.assertEqual(rows[1]["entries"], "1")
        self.assertEqual(sleep.call_args_list[0].args, (60,))
        self.assertNotIn(SENTINEL, err.getvalue())

    def test_bad_arguments_refused(self):
        for args in (["--out", self.out], ["--config", self.config],
                     ["--config", self.config, "--out", self.out, "--every", "0"],
                     ["--config", os.path.join(self.tmp.name, "nope"), "--out", self.out]):
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    SAMPLE.main(args, procs=NO_PROCS)
                self.assertEqual(raised.exception.code, 2)
        self.assertFalse(os.path.exists(self.out))


class ProcessReaderTest(unittest.TestCase):
    @unittest.skipUnless(os.path.isdir("/proc/self/fd"), "Linux /proc only")
    def test_linux_reader_sees_this_process(self):
        procs = SAMPLE.Processes()
        self.assertIn(os.getpid(), [pid for pid, _argv in procs.list()])
        self.assertGreater(procs.rss_bytes(os.getpid()), 0)
        self.assertGreater(procs.open_files(os.getpid()), 0)


if __name__ == "__main__":
    unittest.main()
