"""scripts/pad/release-rows: templates, settings, dry run and the preflight's timeout, with tmux stubbed. No network,
no build, no row. Run: python3.11 -m unittest discover -s scripts/pad -p 'test_release_rows.py'"""
import glob
import importlib.machinery
import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


rr = load("release_rows", "release-rows")
pad = load("pad_for_release_rows", "pad")
URL = "https://goalcup-ingest-staging.example.workers.dev"


def run(argv, env):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = rr.main(argv, env)
    return code, out.getvalue(), err.getvalue()


class Templates(unittest.TestCase):
    def test_one_template_per_row_and_each_fills_to_valid_settings(self):
        rows = [r for rows in rr.CHAINS.values() for r in rows]
        files = sorted(os.path.basename(p)[:-5] for p in glob.glob(os.path.join(rr.TEMPLATES, "*.toml")))
        self.assertEqual(sorted(rows), files)
        with tempfile.TemporaryDirectory() as t:
            installs = os.path.join(t, "installs")
            for v in ("0.22.5", "0.22.6"):
                os.makedirs(os.path.join(installs, "hermes-cli-%s" % v, "bin"))
            values = {"BUILD": os.path.join(t, "build"), "SIGNIN_HOME": os.path.join(t, "signin"),
                      "EXEC_HOME": os.path.join(t, "exec"), "HARNESS_INSTALLS": installs}
            padcfg = pad.read_toml(os.path.join(HERE, "pad.toml"))
            for row in rows:
                with open(os.path.join(rr.TEMPLATES, row + ".toml")) as f:
                    text = rr.fill(f.read(), values)
                self.assertFalse(rr.PLACEHOLDER.search(text), row)
                path = os.path.join(t, row + ".toml")
                with open(path, "w") as f:
                    f.write(text)
                with mock.patch("os.path.isdir", lambda d: True):  # /opt/homebrew/opt/curl/bin is the Mac's
                    s = pad.load_settings(pad.read_toml(path), padcfg)
                self.assertEqual((s["where"], s["build"]), ("here", values["BUILD"]), row)

    def test_unknown_placeholder_refused(self):
        with self.assertRaises(rr.Refused):
            rr.fill('build = "@NOPE@"\n', {"BUILD": "/b"})

    def test_no_personal_path_in_templates(self):
        for p in glob.glob(os.path.join(rr.TEMPLATES, "*.toml")):
            with open(p) as f:
                self.assertNotRegex(f.read(), r"/(Users|home)/", p)


class Main(unittest.TestCase):
    def test_missing_staging_url_refused(self):
        code, out, err = run(["--dry-run"], {})
        self.assertEqual(code, 2)
        self.assertIn("PAD_STAGING_URL", err)

    def test_dry_run_lists_14_rows_in_4_chains(self):
        code, out, _ = run(["--dry-run"], {"PAD_STAGING_URL": URL, "HARNESS_INSTALLS": "/nowhere"})
        self.assertEqual(code, 0)
        self.assertIn("14 rows in 4 chains", out)
        for name, rows in rr.CHAINS.items():
            self.assertIn("%s: %s" % (name, " ".join(rows)), out)
        self.assertIn("missing folder: /nowhere/hermes-cli-0.22.5/bin", out)

    def test_only_picks_chains_and_refuses_unknown(self):
        code, out, _ = run(["--only", "gateway", "--dry-run"], {"PAD_STAGING_URL": URL})
        self.assertEqual(code, 0)
        self.assertIn("2 rows in 1 chains", out)
        code, _, err = run(["--only", "nope", "--dry-run"], {"PAD_STAGING_URL": URL})
        self.assertEqual(code, 2)
        self.assertIn("--only takes chain names", err)

    def test_template_error_is_one_line_exit_2(self):
        with tempfile.TemporaryDirectory() as t, mock.patch.object(rr, "TEMPLATES", t):
            code, out, err = run(["--dry-run"], {"PAD_STAGING_URL": URL})
        self.assertEqual(code, 2)
        self.assertNotIn("Traceback", err)
        self.assertEqual(len(err.strip().splitlines()), 1, err)
        self.assertTrue(err.startswith("release-rows: "), err)


class Finish(unittest.TestCase):
    """The exit code covers the new batch, not only the committed rows' release check."""

    def finish(self, results, check_rc=0):
        done = lambda a, **kw: subprocess.CompletedProcess(a, check_rc if "--release-check" in a else 0, "check\n", "")
        out = io.StringIO()
        with redirect_stdout(out), mock.patch.object(rr.subprocess, "run", done):
            return rr.finish(results, {"EXEC_HOME": "/x", "build": "/b"}), out.getvalue()

    def test_green_batch_and_check_pass(self):
        self.assertEqual(self.finish({"a": (0, "result PASS f/\n"), "b": (3, "result INCONCLUSIVE f/\n")})[0], 0)

    def test_a_new_fail_row_fails_even_when_the_check_passes(self):
        code, out = self.finish({"a": (0, "result PASS f/\n"), "b": (1, "result FAIL f/\n")})
        self.assertEqual(code, 1)
        self.assertIn("b", out.splitlines()[-1])

    def test_a_pad_exit_outside_0_3_fails(self):
        self.assertEqual(self.finish({"a": (2, "pad: bad settings\n")})[0], 1)

    def test_a_failing_check_still_fails(self):
        self.assertEqual(self.finish({"a": (0, "result PASS f/\n")}, check_rc=1)[0], 1)


class LiveRecorders(unittest.TestCase):
    PS = ("101 /bin/zsh -l\n"
          "202 /b/goalcup recorder GOALCUP_HOME=/h/signin/hm PATH=/usr/bin\n"
          "303 /b/goalcup recorder GOALCUP_HOME=/h/other/home\n"
          "404 /b/goalcup start --test GOALCUP_HOME=/h/signin/hm\n"
          "505 /h/signin/hm/bin/goalcup recorder\n"
          "606 /b/goalcup recorder GOALCUP_HOME=/h/signin/hm2\n")

    def test_only_recorders_under_the_signin_home_are_named(self):
        self.assertEqual([p for p, _ in rr.live_recorders("/h/signin/hm", self.PS)], ["202", "505"])

    def test_a_warning_line_names_them_before_each_signin_row_and_kills_nothing(self):
        out, rows = io.StringIO(), []
        with redirect_stdout(out), mock.patch.object(rr, "ps_text", lambda: self.PS), mock.patch.object(rr, "PAUSE", 0), \
                mock.patch.object(rr, "run_row", lambda d, row, lock, results: rows.append(row)), \
                mock.patch.object(rr.shutil, "which", lambda n: None), mock.patch.object(rr.os, "kill", side_effect=AssertionError("kills nothing")):
            rr.run_chains("/nonexistent", {"signin": ["a", "b"], "gateway": ["c"]}, "/h/signin/hm")
        lines = [ln for ln in out.getvalue().splitlines() if ln.startswith("warning:")]
        self.assertEqual(sorted(rows), ["a", "b", "c"])
        self.assertEqual(len(lines), 2)  # one before each sign-in row; none for the gateway chain
        for ln, row in zip(lines, "ab"):
            self.assertIn("before %s" % row, ln)
            self.assertTrue("202" in ln and "505" in ln and "303" not in ln and "606" not in ln, ln)


class Preflight(unittest.TestCase):
    def stub(self, screen, dead="0"):
        calls = []

        def tmux(*a):
            calls.append(a)
            out = screen if a[0] == "capture-pane" else dead if a[0] == "display-message" else ""
            return subprocess.CompletedProcess(a, 0, out, "")
        return calls, tmux

    def test_timeout_gives_the_little_snitch_message_and_cleans_up(self):
        calls, tmux = self.stub("Approve a test run of Claude Code in your browser.\n")
        with mock.patch.object(rr, "tmux", tmux), mock.patch.object(rr.time, "sleep", lambda s: None), \
                self.assertRaises(rr.Refused) as got:
            rr.preflight("/b", URL, wait=0)
        self.assertEqual(str(got.exception), rr.NO_REACH)
        self.assertIn("Little Snitch", rr.NO_REACH)
        self.assertEqual(calls[-1][0], "kill-session")

    def test_tmux_that_cannot_start_is_named(self):
        calls, tmux = self.stub("")
        fail = lambda *a: subprocess.CompletedProcess(a, 1, "", "no server") if a[0] == "new-session" else tmux(*a)
        with mock.patch.object(rr, "tmux", fail), mock.patch.object(rr.time, "sleep", lambda s: None), \
                self.assertRaises(rr.Refused) as got:
            rr.preflight("/b", URL, wait=5)
        self.assertIn("tmux could not start", str(got.exception))
        self.assertNotIn("Little Snitch", str(got.exception))

    def test_launch_sets_home(self):
        calls, tmux = self.stub("Open this address: x\n")
        seen = []

        def spy(*a):
            if a[0] == "respawn-pane":
                with open(a[-1].split(" ", 1)[1].strip("'")) as f:
                    seen.append(f.read())
            return tmux(*a)
        with mock.patch.object(rr, "tmux", spy), mock.patch.object(rr.time, "sleep", lambda s: None), \
                mock.patch.dict(os.environ, {"HOME": "/home/u"}):
            rr.preflight("/b", URL, wait=5)
        self.assertIn("HOME=/home/u ", seen[0])

    def test_address_seen_passes(self):
        calls, tmux = self.stub("Open this address: https://agentcommunity.org/goalcup/approve?code=X\n")
        with mock.patch.object(rr, "tmux", tmux), mock.patch.object(rr.time, "sleep", lambda s: None):
            rr.preflight("/b", URL, wait=5)
        self.assertEqual(calls[-1][0], "kill-session")

    def test_early_exit_names_the_last_line(self):
        _, tmux = self.stub("Claude Code is not installed.\n", dead="1")
        with mock.patch.object(rr, "tmux", tmux), mock.patch.object(rr.time, "sleep", lambda s: None), \
                self.assertRaises(rr.Refused) as got:
            rr.preflight("/b", URL, wait=5)
        self.assertIn("not installed", str(got.exception))


class ExecHelp(unittest.TestCase):
    def test_commands_name_the_entry_and_folder_from_the_log(self):
        log = ("pad hermes-cli direct micro   variant exec h2-parallel.sh@1   run r1  entry e0123456789abcdef  box -   3m 00s\n"
               "result            PASS           pad-out/hermes-cli/direct/s-b/\n")
        text = rr.exec_help({"EXEC_HOME": "/x/home", "build": "/b"}, log)
        self.assertIn("/b/goalcup-verify dump --kind open /x/home/runs/r1/e0123456789abcdef", text)
        self.assertIn("pad-out/hermes-cli/direct/s-b/exec/h2-parallel.sh.verdict", text)
        self.assertIn("0 h2_unsupported", text)
        self.assertIn("one long_tunnel event, 0 gaps", text)


if __name__ == "__main__":
    unittest.main()
