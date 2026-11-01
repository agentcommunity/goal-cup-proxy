"""Tests of the conductor's fetch (PRD-12 sections 3.2, 7; ledger B10): DATABASE_URL reaches only the put-env call of a
separate short-lived box, that box is always removed, the SQL is fetch-run.sh's own block, and a fetch that does not
finish keeps the entry's rec/ before the pad box goes. A fake sbx and fetch-run; nothing real runs.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from test_pad import BASE, HERE, PADCFG, SENTINEL, Walk, pad  # noqa: E402

ENTRY = "e" + "0123456789abcdef"
DB = SENTINEL + "db"
SECRETS = {"DATABASE_URL": DB, "GH_TOKEN": SENTINEL + "gh", "GOALCUP_TEST_APPROVAL_KEY": SENTINEL + "ap",
           "CLOUDFLARE_ACCOUNT_ID": "acct12345678", "CLOUDFLARE_API_TOKEN": SENTINEL + "cf", "VERCEL_TOKEN": "vercel-fake-token"}
FETCH_RUN = os.path.join(os.path.dirname(os.path.dirname(HERE)), "scripts", "fetch-run.sh")


class Fetch(unittest.TestCase):
    def setUp(self):
        self.calls = []  # (argv, env) of every child
        self.fail_at, self.hang_at, self.fetch_exit = None, None, 0
        self.sent_sql = None

    def fake(self, argv, timeout=None, env=None):
        self.calls.append((list(argv), dict(env or {})))
        if argv[0] == "sh":  # fetch-run
            if self.hang_at == "sh":
                raise subprocess.TimeoutExpired(argv, timeout)
            if not self.fetch_exit:
                os.makedirs(os.path.join(argv[-1], argv[-2]))
                with open(os.path.join(argv[-1], argv[-2], "fetch.json"), "w") as f:
                    f.write("{}")
            return subprocess.CompletedProcess(argv, self.fetch_exit, "", "boom " + DB if self.fetch_exit else "")
        verb = argv[2]
        if verb == self.hang_at:
            raise subprocess.TimeoutExpired(argv, timeout)
        if verb == "put" and argv[4].endswith("query.sql"):
            with open(argv[argv.index("--file") + 1]) as f:
                self.sent_sql = f.read()
        if verb == "get" and argv[4].startswith("~/idx/"):
            with open(argv[-1], "w") as f:
                f.write("[]")
        if verb == "get" and "rec-kept" in argv[4]:
            with tempfile.TemporaryDirectory() as d, tarfile.open(argv[-1], "w") as t:
                with open(os.path.join(d, "x"), "w") as f:
                    f.write("recording")
                t.add(os.path.join(d, "x"), "rec/x")
        code = int(verb == self.fail_at or (self.fail_at == "get-rec" and verb == "get" and "rec-kept" in argv[4]))
        return subprocess.CompletedProcess(argv, code, "", "psql: " + DB if code else "")

    def run_for(self, out, **env):
        s = pad.load_settings(dict(BASE, mode="cycle"), PADCFG)
        r = pad.Run(s, "direct", PADCFG, Walk.HARNESS, out)
        self.addCleanup(shutil.rmtree, r.tmp, True)
        r.tried, r.meta["entry"] = True, ENTRY
        return r

    def go(self, *, env=SECRETS, cleanup=False, **kw):
        with tempfile.TemporaryDirectory() as out, mock.patch.dict(os.environ, env, clear=False), \
                mock.patch.object(pad, "call", self.fake):
            if "DATABASE_URL" not in env:
                os.environ.pop("DATABASE_URL", None)
            r = self.run_for(out)
            try:
                res = r.step_fetch("fetch", 60)
            except subprocess.TimeoutExpired:
                res = "timeout"
            if cleanup:
                r.step_cleanup()
            self.log = open(os.path.join(r.dir, "calls.log")).read() if os.path.exists(os.path.join(r.dir, "calls.log")) else ""
            self.kept = os.path.isfile(os.path.join(r.dir, "rec-kept", ENTRY, "rec", "x"))
            return res

    def sbx(self):
        return [(a[2:], e) for a, e in self.calls if a[0] == "node"]

    def verbs(self):
        return [a[0] for a, _ in self.sbx()]

    def test_database_url_reaches_only_the_put_env_call(self):
        res = self.go()
        self.assertEqual(res[0], "ok")
        envs = [(a, e) for a, e in self.sbx() if "DATABASE_URL" in e]
        self.assertEqual([a[0] for a, _ in envs], ["put-env"])
        put = envs[0][1]
        self.assertFalse({"GH_TOKEN", "GOALCUP_TEST_APPROVAL_KEY", "CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID"} & set(put))
        self.assertIn("VERCEL_TOKEN", put)
        for argv, env in self.calls:
            self.assertFalse([a for a in argv if DB in a])
            if argv[0] == "sh":
                self.assertNotIn("DATABASE_URL", env)
                self.assertIn("CLOUDFLARE_API_TOKEN", env)
                self.assertNotIn("GH_TOKEN", env)
                self.assertIn("--index", argv)
        self.assertNotIn(DB, self.log)

    def test_sql_is_the_fetch_run_block_and_extraction_fails_closed(self):
        self.go()
        with open(FETCH_RUN) as f:
            block = re.search(r"<<'SQL'\n(.*?)\nSQL\n", f.read(), re.S).group(1) + "\n"
        self.assertEqual(self.sent_sql, block)
        self.assertTrue(block.startswith("BEGIN READ ONLY") and block.rstrip().endswith("ROLLBACK;"))
        with tempfile.TemporaryDirectory() as d:
            for name, text in (("none", "echo hi\n"), ("open", "x <<'SQL'\nSELECT 1;\n"), ("write", "x <<'SQL'\nDELETE FROM t;\nSQL\n"),
                               ("two", "x <<'SQL'\nBEGIN READ ONLY;\nROLLBACK;\nSQL\ny <<'SQL'\nSQL\n")):
                with open(os.path.join(d, name), "w") as f:
                    f.write(text)
                with self.assertRaises(pad.Refused, msg=name):
                    pad.fetch_sql(os.path.join(d, name))

    def index_box_removed(self):
        rms = [a for a, _ in self.sbx() if a[0] == "rm"]
        created = [a[a.index("--name") + 1] for a, _ in self.sbx() if a[0] == "create"]
        self.assertEqual(len(created), 1)
        self.assertTrue(created[0].startswith("pad-dbf-"))
        self.assertEqual([a[1] for a in rms], created)  # the pad box is not removed by the fetch

    def test_index_box_is_removed_on_success_failure_and_timeout(self):
        self.go()
        self.index_box_removed()
        for kind in ("fail", "hang"):
            self.setUp()
            if kind == "fail":
                self.fail_at = "exec"
            else:
                self.hang_at = "put-env"
            res = self.go()
            self.assertEqual(res[0] if kind == "fail" else res, "FAIL" if kind == "fail" else "timeout")
            self.index_box_removed()

    def test_failed_fetch_keeps_rec_before_the_pad_box_goes(self):
        self.fetch_exit = 1
        res = self.go(cleanup=True)
        self.assertEqual(res[0], "FAIL")
        self.assertNotIn(DB, res[1])
        self.assertTrue(self.kept)
        calls = [a for a, _ in self.sbx()]
        got = next(i for i, a in enumerate(calls) if a[0] == "get" and "rec-kept" in a[2])
        pad_rm = next(i for i, a in enumerate(calls) if a[0] == "rm" and not a[1].startswith("pad-dbf-"))
        self.assertLess(got, pad_rm)

    def test_timed_out_fetch_also_keeps_rec(self):
        self.hang_at = "sh"
        self.assertEqual(self.go(), "timeout")
        self.assertTrue(self.kept)

    def test_successful_fetch_copies_no_rec(self):
        self.assertEqual(self.go(cleanup=True)[0], "ok")
        self.assertFalse(self.kept)
        self.assertFalse([a for a, _ in self.sbx() if a[0] == "get" and "rec-kept" in a[2]])

    def test_failed_rec_copy_is_said_and_changes_nothing(self):
        self.fetch_exit, self.fail_at = 1, "get-rec"
        res = self.go()
        self.assertEqual(res[0], "FAIL")
        self.assertFalse(self.kept)

    def test_no_database_url_makes_no_box(self):
        res = self.go(env={})
        self.assertEqual(res[0], "FAIL")
        self.assertNotIn("create", self.verbs())
        self.assertFalse([a for a, _ in self.calls if a[0] == "sh"])


if __name__ == "__main__":
    unittest.main()
