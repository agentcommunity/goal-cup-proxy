#!/usr/bin/env python3
"""Behavioral tests for scripts/research-staging.py.

    python3 -m unittest discover -s scripts -p 'test_research_staging.py'

No database and no R2: psql and the R2 list request are injected fakes (and, once, a fake psql on
PATH runs the script as a process). Sentinel credentials must never reach argv or output.
"""

from __future__ import annotations

import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
import urllib.parse
from pathlib import Path

SCRIPT = Path(__file__).with_name("research-staging.py")
SQL = Path(__file__).with_name("research-staging.sql")
_spec = importlib.util.spec_from_file_location("research_staging", SCRIPT)
rs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rs)

PASSWORD = "pw-sentinel-must-never-appear"
DB_URL = f"postgresql://reader:{PASSWORD}@db.invalid:5432/postgres?sslmode=require"
TOKEN = "cf-token-sentinel-must-never-appear"
ACCOUNT = "acc0123456789"
NOW = 1_800_000_000.0
RUN_A, RUN_B, RUN_OUT = "r00000000000000aa", "r00000000000000bb", "r00000000000000cc"
ENT_A, ENT_B = "e00000000000000aa", "e00000000000000bb"
PREFIX = "traces/test-staging/"
MEMBER = "11111111-2222-3333-4444-555555555555"


def key(run, entry, offset):
    return f"{PREFIX}{run}/{entry}/{offset:016d}-{'a' * 64}.zst"


def fixture_rows(staging=True, event="test-staging"):
    """What the SQL answers, section by section, for two runs (A with 3 chunks, B with none)."""
    def run(rid, start, last_beat, beats, max_gap, status="open"):
        return {"run": rid, "mode": "test", "status": status, "staging": staging, "official_start": start,
                "deadline_at": NOW + 3600, "replaced": False, "abandon_reason_given": False, "flag_reason": None,
                "last_heartbeat_at": last_beat, "heartbeats": beats, "first_beat_at": start,
                "last_beat_at": last_beat, "max_gap_s": max_gap, "p50_gap_s": max_gap, "p90_gap_s": max_gap}

    def entry(eid, rid, upload_offset, offset_local, acked_at_beat, beat_at):
        return {"run": rid, "entry": eid, "segment": 0, "upload_offset": upload_offset, "opened_at": NOW - 7200,
                "last_chunk_at": None, "final_at": None, "sealed": False, "model_requests": None,
                "beat_at": beat_at, "offset_local": offset_local, "beat_final": False, "recording": "ok",
                "acked_at_beat": acked_at_beat}

    chunks = [
        {"run": RUN_A, "entry": ENT_A, "offset": 0, "length": 1000, "key": key(RUN_A, ENT_A, 0), "at": NOW - 5400},
        {"run": RUN_A, "entry": ENT_A, "offset": 1000, "length": 3000, "key": key(RUN_A, ENT_A, 1000), "at": NOW - 3600},
        {"run": RUN_A, "entry": ENT_A, "offset": 4000, "length": 500, "key": key(RUN_A, ENT_A, 4000), "at": NOW - 1800},
    ]
    return {
        "event": [{"id": event, "mode": "test", "staging": staging, "run_hours": 168, "now": NOW}],
        "run": [run(RUN_A, NOW - 7200, NOW - 60, 10, 900.0), run(RUN_B, NOW - 3600, NOW - 1200, 2, 30.0)],
        "entry": [entry(ENT_A, RUN_A, 4500, 6000, 4000, NOW - 60), entry(ENT_B, RUN_B, 200, 100, 0, NOW - 1200)],
        "chunk": chunks,
        "notice": [{"kind": "silence", "state": "sent", "count": 1, "attempts": 1}],
        "summary": [{"runs": 2}],
    }


def fixture_objects():
    return {
        key(RUN_A, ENT_A, 0): 250,            # ratio 4
        key(RUN_A, ENT_A, 1000): 500,         # ratio 6
        # key(RUN_A, ENT_A, 4000) is missing from R2
        key(RUN_A, ENT_A, 9999): 77,          # stored but never indexed (a refused commit)
        f"{PREFIX}{RUN_A}/{ENT_A}/seal-{'b' * 64}.json": 11,
        key(RUN_OUT, "e00000000000000cc", 0): 1_000_000,  # a run outside the selection
    }


def psql_output(rows):
    lines = ["BEGIN"]  # stray non-JSON lines are ignored
    for section, items in rows.items():
        for item in items:
            lines.append(json.dumps(dict(item, section=section)))
    return "\n".join(lines) + "\n"


class FakePsql:
    def __init__(self, rows, stderr_echo=None):
        self.rows, self.calls, self.stderr_echo = rows, [], stderr_echo

    def __call__(self, argv, sql, env):
        conf = env.get("PGSERVICEFILE")
        mode = stat.S_IMODE(os.stat(conf).st_mode) if conf else None
        with open(conf) as f:
            service = f.read()
        self.calls.append({"argv": list(argv), "sql": sql, "env": dict(env), "mode": mode, "service": service, "conf": conf})
        if self.stderr_echo is not None:
            raise rs.Fail("psql failed: " + self.stderr_echo)
        return psql_output(self.rows)


class FakeR2:
    def __init__(self, objects, page_size=2):
        items = sorted(objects.items())
        self.pages = [items[i:i + page_size] for i in range(0, len(items), page_size)] or [[]]
        self.urls, self.tokens = [], []

    def __call__(self, url, token):
        self.urls.append(url)
        self.tokens.append(token)
        q = urllib.parse.parse_qs(urllib.parse.urlsplit(url).query)
        cursor = q.get("cursor", [None])[0]
        i = 0 if cursor is None else int(cursor[len("cur-"):])
        last = i == len(self.pages) - 1
        # Like R2, only keys under the prefix asked for.
        return {"success": True, "errors": [], "messages": [],
                "result": [{"key": k, "size": v, "etag": "x"} for k, v in self.pages[i] if k.startswith(q["prefix"][0])],
                "result_info": {"cursor": "" if last else f"cur-{i + 1}", "is_truncated": not last, "per_page": 1000}}


def env(**over):
    e = {"DATABASE_URL": DB_URL, "CLOUDFLARE_ACCOUNT_ID": ACCOUNT, "CLOUDFLARE_API_TOKEN": TOKEN, "PATH": os.environ.get("PATH", "")}
    e.update(over)
    return e


def run_main(args, rows=None, objects=None, environ=None, psql=None):
    psql = psql or FakePsql(rows if rows is not None else fixture_rows())
    r2 = FakeR2(objects if objects is not None else fixture_objects())
    out, err = io.StringIO(), io.StringIO()
    code = rs.main(args, env=environ or env(), psql=psql, fetch=r2, stdout=out, stderr=err)
    return code, out.getvalue(), err.getvalue(), psql, r2


class Aggregates(unittest.TestCase):
    def setUp(self):
        self.rep = rs.analyze(fixture_rows(), fixture_objects(), PREFIX, top=5)

    def test_join_and_ratio_math(self):
        o = self.rep["overall"]
        self.assertEqual((o["chunks"], o["logical_bytes"]), (3, 4500))
        # Only the two chunks with an object are in the ratio: (1000 + 3000) / (250 + 500).
        self.assertEqual(o["compressed_bytes"], 750)
        self.assertEqual(o["ratio"], round(4000 / 750, 3))
        self.assertEqual(o["chunks_missing_object"], 1)
        self.assertEqual(o["chunk_logical"], {"n": 3, "p50": 1000, "p90": 3000, "max": 3000})
        self.assertEqual(o["chunk_compressed"], {"n": 2, "p50": 250, "p90": 500, "max": 500})
        self.assertEqual(o["chunk_ratio"]["max"], 6.0)
        a = next(e for e in self.rep["entries"] if e["entry"] == ENT_A[:8] and e["run"] == RUN_A[:8])
        self.assertEqual((a["chunks"], a["logical_bytes"], a["compressed_bytes"], a["chunks_missing_object"]), (3, 4500, 750, 1))

    def test_objects_outside_the_index_are_classified_not_joined(self):
        r2 = self.rep["r2"]
        self.assertEqual(r2["objects_listed"], 5)
        self.assertEqual(r2["objects_in_scope"], 4)  # the other run's object is out of scope
        self.assertEqual((r2["chunk_objects_without_index_row"], r2["unindexed_bytes"]), (1, 77))
        self.assertEqual((r2["documents"], r2["document_bytes"]), (1, 11))
        self.assertEqual(r2["other_objects"], 0)

    def test_bytes_per_hour_over_the_active_span(self):
        a = self.rep["runs"][0]
        # official start NOW-7200 to the last chunk NOW-1800: 1.5 h.
        self.assertEqual(a["active_hours"], 1.5)
        self.assertEqual(a["logical_bytes_per_hour"], 3000)
        self.assertEqual(a["compressed_bytes_per_hour"], 500)
        b = self.rep["runs"][1]
        self.assertIsNone(b["logical_bytes_per_hour"])  # no chunks, no rate
        self.assertIsNone(b["ratio"])

    def test_ack_lag(self):
        a = self.rep["ack_lag"]
        self.assertEqual(a["entries_with_heartbeat"], 2)
        # A: 6000 - 4500 now, 6000 - 4000 at the beat. B: 100 - 200 now (uploaded since), 100 at the beat.
        self.assertEqual(sorted([a["now"]["p50"], a["now"]["max"]]), [-100, 1500])
        self.assertEqual(a["negative_now"], 1)
        self.assertEqual(a["at_heartbeat"]["max"], 2000)
        self.assertEqual([w["entry"] for w in a["worst"]], [ENT_A[:8], ENT_B[:8]])
        self.assertEqual(len(rs.analyze(fixture_rows(), None, PREFIX, top=1)["ack_lag"]["worst"]), 1)

    def test_heartbeat_health(self):
        h = self.rep["heartbeats"]
        self.assertEqual(h["runs_beating_10m"], 1)       # A beat 60 s ago, B 1200 s ago
        self.assertEqual(h["runs_open_silent_10m"], 1)
        self.assertEqual(h["runs_with_gap_over_10m"], 1)  # A's 900 s gap
        self.assertEqual(h["max_gap_s"]["max"], 900.0)

    def test_without_r2_there_are_no_compressed_numbers(self):
        rep = rs.analyze(fixture_rows(), None, PREFIX)
        self.assertNotIn("compressed_bytes", rep["overall"])
        self.assertIsNone(rep["r2"])
        self.assertTrue(rep["not_stored"])

    def test_percentile_is_nearest_rank(self):
        self.assertEqual(rs.pct([5, 1, 4, 2, 3], 0.5), 3)
        self.assertEqual(rs.pct(list(range(1, 11)), 0.9), 9)
        self.assertIsNone(rs.pct([], 0.5))


class R2Listing(unittest.TestCase):
    def test_pagination_follows_the_cursor_to_the_end(self):
        objects = {f"{PREFIX}r/e/{i:04d}.zst": i + 1 for i in range(7)}
        fake = FakeR2(objects, page_size=3)
        got = rs.list_objects(ACCOUNT, rs.STAGING_BUCKET, PREFIX, TOKEN, fake)
        self.assertEqual(got, objects)
        self.assertEqual(len(fake.urls), 3)
        first = urllib.parse.urlsplit(fake.urls[0])
        self.assertEqual(first.path, f"/client/v4/accounts/{ACCOUNT}/r2/buckets/{rs.STAGING_BUCKET}/objects")
        q = urllib.parse.parse_qs(first.query)
        self.assertEqual((q["prefix"], q["per_page"]), ([PREFIX], ["1000"]))
        self.assertNotIn("cursor", q)
        self.assertEqual(urllib.parse.parse_qs(urllib.parse.urlsplit(fake.urls[2]).query)["cursor"], ["cur-2"])

    def test_string_sizes_are_read(self):
        page = {"success": True, "result": [{"key": PREFIX + "x.zst", "size": "12"}], "result_info": {"is_truncated": False}}
        self.assertEqual(rs.list_objects(ACCOUNT, "b-staging", PREFIX, TOKEN, lambda u, t: page), {PREFIX + "x.zst": 12})

    def test_bad_answers_fail(self):
        cases = [
            {"success": False, "errors": [{"code": 10006, "message": "x"}]},
            {"success": True, "result": [{"key": "elsewhere/x", "size": 1}]},
            {"success": True, "result": [{"key": PREFIX + "x"}]},
            {"success": True, "result": [], "result_info": {"cursor": "same", "is_truncated": True}},
        ]
        for page in cases:
            with self.subTest(page=page), self.assertRaises(rs.Fail):
                rs.list_objects(ACCOUNT, "b", PREFIX, TOKEN, lambda u, t, p=page: p)


class Guards(unittest.TestCase):
    def test_refuses_the_production_bucket(self):
        code, out, err, psql, r2 = run_main(["--bucket", rs.PRODUCTION_BUCKET])
        self.assertEqual(code, 1)
        self.assertIn("refusing bucket", err)
        self.assertEqual((psql.calls, r2.urls), ([], []))  # nothing was read

    def test_production_needs_allow_production(self):
        rows = fixture_rows(staging=False, event="goalcup-dryrun-2026")
        code, _, err, _, r2 = run_main(["--event", "goalcup-dryrun-2026", "--no-r2"], rows=rows)
        self.assertEqual(code, 1)
        self.assertIn("--allow-production", err)
        self.assertEqual(r2.urls, [])
        code, out, err, _, r2 = run_main(
            ["--event", "goalcup-dryrun-2026", "--bucket", rs.PRODUCTION_BUCKET, "--allow-production"], rows=rows)
        self.assertEqual(code, 0, err)
        self.assertIn(f"/r2/buckets/{rs.PRODUCTION_BUCKET}/objects", r2.urls[0])

    def test_event_and_bucket_must_match(self):
        code, _, err, _, r2 = run_main(["--bucket", rs.PRODUCTION_BUCKET, "--allow-production"])
        self.assertEqual(code, 1)
        self.assertIn("not in bucket", err)
        self.assertEqual(r2.urls, [])

    def test_unknown_event(self):
        rows = fixture_rows()
        rows["event"] = []
        code, _, err, _, _ = run_main([], rows=rows)
        self.assertEqual(code, 1)
        self.assertIn("no event", err)

    def test_sql_sent_is_one_read_only_transaction(self):
        code, _, err, psql, _ = run_main(["--since", "2026-10-01"])
        self.assertEqual(code, 0, err)
        sql = psql.calls[0]["sql"]
        stmts = rs.statements(sql)
        self.assertTrue(stmts[0].startswith("begin") and "read only" in stmts[0])
        self.assertEqual(stmts[-1], "rollback")
        self.assertTrue(all(s.startswith(("select", "with", "set transaction read only", "set local")) for s in stmts[1:-1]))
        argv = psql.calls[0]["argv"]
        self.assertEqual(argv[:7], ["psql", "-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1"])
        self.assertIn("event=test-staging", argv)
        self.assertIn("since=2026-10-01T00:00:00Z", argv)

    def test_writing_sql_is_refused(self):
        good = SQL.read_text()
        rs.check_read_only(good)
        bad = [
            good.replace("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;", "BEGIN;"),
            good.replace("ROLLBACK;", "COMMIT;"),
            good.replace("ROLLBACK;", "INSERT INTO goal_cup_tracer.events VALUES ('x');\nROLLBACK;"),
            good.replace("ROLLBACK;", "COMMIT;\nSELECT 1;\nROLLBACK;"),
            good.replace("ROLLBACK;", "SET default_transaction_read_only = off;\nROLLBACK;"),
            good.replace("ROLLBACK;", "\\! touch /tmp/x\nROLLBACK;"),
            good.replace("ROLLBACK;", "SELECT 1 INTO TEMP t;\nROLLBACK;"),
        ]
        for sql in bad:
            with self.subTest(sql=sql[-80:]), self.assertRaises(rs.Fail):
                rs.check_read_only(sql)

    def test_every_sql_column_exists_in_the_schema(self):
        schema = (Path(__file__).resolve().parent.parent / "server/worker/sql/goal-cup-tracer-upload.sql").read_text()
        for column in ("upload_offset", "offset_local", "r2_key", "received_at", "last_heartbeat_at", "flag_reason",
                       "abandon_reason", "official_start", "deadline_at", "replaced_at", "recording", "last_error",
                       "skipped_at", "sent_at", "attempts", "sealed_at", "final_at", "last_chunk_at", "model_requests"):
            self.assertRegex(schema, rf"\n  {column}\s+\w", column)


class Secrets(unittest.TestCase):
    def assert_clean(self, *texts):
        for text in texts:
            for s in (DB_URL, PASSWORD, TOKEN):
                self.assertNotIn(s, text)

    def test_credentials_never_in_argv_or_output(self):
        for args in ([], ["--json"]):
            code, out, err, psql, r2 = run_main(args)
            self.assertEqual(code, 0, err)
            call = psql.calls[0]
            self.assert_clean(out, err, " ".join(call["argv"]), *r2.urls)
            # The connection reaches psql in a private service file only; no credential in its env.
            self.assertEqual(call["mode"], 0o600)
            self.assertIn(f"password={PASSWORD}", call["service"])
            self.assertFalse(os.path.exists(call["conf"]), "the service file outlives the run")
            self.assert_clean(*(f"{k}={v}" for k, v in call["env"].items()))
            self.assertEqual(r2.tokens, [TOKEN] * len(r2.urls))  # the token goes only to the request header

    def test_errors_are_scrubbed(self):
        psql = FakePsql(fixture_rows(), stderr_echo=f"could not connect to {DB_URL} (password {PASSWORD}) {TOKEN}")
        code, out, err, _, _ = run_main([], psql=psql)
        self.assertEqual(code, 1)
        self.assertIn("[redacted]", err)
        self.assert_clean(out, err)

    def test_no_identifying_data_in_output(self):
        code, out, err, _, _ = run_main(["--json"])
        self.assertEqual(code, 0, err)
        report = json.loads(out)
        self.assertEqual(report["overall"]["chunks"], 3)
        for full in (RUN_A, RUN_B, ENT_A, ENT_B, MEMBER, PREFIX, "traces/"):
            self.assertNotIn(full, out)
        code, out, err, _, _ = run_main([])
        for full in (RUN_A, RUN_B, ENT_A, ENT_B, PREFIX, "traces/"):
            self.assertNotIn(full, out)

    def test_the_real_psql_runner(self):
        """The script as a process, with a fake psql on PATH that records what it was given."""
        with tempfile.TemporaryDirectory() as d:
            log = Path(d, "log")
            psql = Path(d, "psql")
            psql.write_text(
                "#!/bin/sh\n"
                f"printf '%s\\n' \"$@\" >'{log}.argv'\n"
                f"env >'{log}.env'\n"
                f"cat >'{log}.sql'\n"
                f"cat '{d}/answer'\n")
            psql.chmod(0o755)
            Path(d, "answer").write_text(psql_output(fixture_rows()))
            e = env(PATH=f"{d}:{os.environ.get('PATH', '')}")
            p = subprocess.run([sys.executable, str(SCRIPT), "--no-r2", "--json"], env=e, capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(json.loads(p.stdout)["overall"]["logical_bytes"], 4500)
            self.assert_clean(p.stdout, p.stderr, Path(f"{log}.argv").read_text(), Path(f"{log}.env").read_text())
            self.assertTrue(rs.statements(Path(f"{log}.sql").read_text())[0].startswith("begin isolation level repeatable read read only"))


class SecretFiles(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db = Path(self.tmp.name, "db-url")
        self.db.write_text(DB_URL + "\n")
        self.token = Path(self.tmp.name, "token")
        self.token.write_text(TOKEN + "\n")

    def test_secrets_from_files_are_used_and_never_printed(self):
        environ = {"CLOUDFLARE_ACCOUNT_ID": ACCOUNT, "PATH": os.environ.get("PATH", "")}
        code, out, err, psql, r2 = run_main(
            ["--database-url-file", str(self.db), "--cloudflare-token-file", str(self.token)],
            environ=environ)
        self.assertEqual(code, 0, err)
        self.assertEqual(err, "")  # no warning: nothing came from the environment
        self.assertIn(f"password={PASSWORD}", psql.calls[0]["service"])
        self.assertEqual(r2.tokens, [TOKEN] * len(r2.urls))
        for s in (DB_URL, PASSWORD, TOKEN):
            self.assertNotIn(s, out + err + " ".join(psql.calls[0]["argv"]))

    def test_secrets_from_the_environment_warn(self):
        code, _, err, _, _ = run_main([])
        self.assertEqual(code, 0, err)
        self.assertIn("DATABASE_URL is taken from the environment", err)
        self.assertIn("CLOUDFLARE_API_TOKEN is taken from the environment", err)
        for s in (DB_URL, PASSWORD, TOKEN):
            self.assertNotIn(s, err)

    def test_an_unreadable_secret_file_fails_clearly(self):
        missing = str(Path(self.tmp.name, "nope"))
        code, _, err, psql, _ = run_main(["--no-r2", "--database-url-file", missing])
        self.assertEqual(code, 1)
        self.assertIn("cannot read --database-url-file", err)
        self.assertEqual(psql.calls, [])

    def test_an_unreadable_sql_file_or_bad_psql_answer_fails_clearly(self):
        code, _, err, _, _ = run_main(["--no-r2", "--sql", str(Path(self.tmp.name, "none.sql"))])
        self.assertEqual(code, 1)
        self.assertIn("cannot read the queries", err)
        self.assertNotIn("Traceback", err)

        class BadPsql(FakePsql):
            def __call__(self, argv, sql, env):
                return '{"section": "event", "staging": tru\n'
        code, _, err, _, _ = run_main(["--no-r2"], psql=BadPsql(fixture_rows()))
        self.assertEqual(code, 1)
        self.assertIn("is not a JSON object", err)


class Args(unittest.TestCase):
    def test_since_forms(self):
        from datetime import datetime, timezone
        now = datetime(2026, 10, 20, 12, 0, tzinfo=timezone.utc)
        self.assertEqual(rs.parse_since("24h", now), "2026-10-19T12:00:00Z")
        self.assertEqual(rs.parse_since("7d", now), "2026-10-13T12:00:00Z")
        self.assertEqual(rs.parse_since("2026-10-01T02:00:00+02:00", now), "2026-10-01T00:00:00Z")

    def test_bad_event_is_refused(self):
        with self.assertRaises(SystemExit), unittest.mock.patch("sys.stderr", io.StringIO()):
            rs.parser().parse_args(["--event", "x'; drop"])


if __name__ == "__main__":
    unittest.main()
