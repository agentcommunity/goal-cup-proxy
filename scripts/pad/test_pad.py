"""Tests of the test pad's own hands (PRD-12 "Automated tests", ledger B10): key files, the push token, no key
value in any argument or file the pad builds (an env key launch included), the pane replacement, cleanup and
check.py's crash rows. No network; gate fast runs python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'.
"""
import base64
import importlib.machinery
import importlib.util
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import time
import unittest
import urllib.parse
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


pad, check, padrun = load("pad", "pad"), load("padcheck", "check.py"), load("padrun", "pad-run")

SENTINEL = "sk-ant-PADSENTINEL-9f3c1e"
PADCFG = {"keys": {"claude": "CLAUDE_API_KEY", "gateway": "AI_GATEWAY_API_KEY", "cf": "CLOUDFLARE_API_TOKEN"}}
BASE = {"harness": "claude-code-cli", "key": "claude", "mode": "cycle"}


def evaluate(files):
    """check.py's row for a run folder holding these JSON (or JSONL, for a name ending in l) files."""
    with tempfile.TemporaryDirectory() as run:
        for name, doc in files.items():
            os.makedirs(os.path.dirname(os.path.join(run, name)), exist_ok=True)
            with open(os.path.join(run, name), "w") as f:
                f.write("\n".join(map(json.dumps, doc)) if name.endswith("l") else json.dumps(doc))
        return check.evaluate(run)


def key_file(folder, mode=0o600, value=SENTINEL):
    path = os.path.join(folder, "claude")
    with open(path, "w") as f:
        f.write(value)
    os.chmod(path, mode)
    return path


class KeyFiles(unittest.TestCase):
    def setUp(self):
        self.out, self.repo = (self.enterContext(tempfile.TemporaryDirectory()) for _ in range(2))

    def test_key_file_rules(self):
        pad.check_key_file(key_file(self.out), self.repo)
        link = os.path.join(self.out, "link")
        os.symlink(key_file(self.out), link)
        for path in (key_file(self.out, 0o644), link, key_file(self.repo)):
            with self.assertRaises(pad.Refused, msg=path):
                pad.check_key_file(path, self.repo)

    def test_push_token_rule(self):
        for value in ("gho_abc", "ghp_abc", "GH_TOKEN=ghp_abc", " ghp_abc", "GH_TOKEN= ghp_abc",
                      "﻿ghp_abc", "github_pat_a\ngithub_pat_b", "github_pat_a b"):
            with self.assertRaises(pad.Refused, msg=repr(value)):
                pad.check_push_token(key_file(self.out, value=value), self.repo)
        for value in ("github_pat_abc", "GH_TOKEN=github_pat_abc\n"):
            pad.check_push_token(key_file(self.out, value=value), self.repo)


class Settings(unittest.TestCase):
    def test_only_model_keys_enter_a_box(self):
        with self.assertRaises(pad.Usage):
            pad.load_settings(dict(BASE, key="cf"), PADCFG)


class RowOverrides(unittest.TestCase):
    """Wave 1: a row may set cap_usd, box_extra_minutes and box_vcpus; pad.toml stays the default."""

    def test_row_values_replace_pad_toml(self):
        s = pad.load_settings(dict(BASE, cap_usd=3, box_extra_minutes=10, box_vcpus=2), PADCFG)
        got = pad.row_pad(s, dict(PADCFG, cap_usd=10, box_extra_minutes=30, box_vcpus=4))
        self.assertEqual((got["cap_usd"], got["box_extra_minutes"], got["box_vcpus"]), (3, 10, 2))
        self.assertEqual(got["keys"], PADCFG["keys"])

    def test_absent_keeps_pad_toml(self):
        s = pad.load_settings(dict(BASE), PADCFG)
        self.assertEqual(pad.row_pad(s, dict(PADCFG, cap_usd=10))["cap_usd"], 10)

    def test_bad_values_are_usage(self):
        for k, v in (("cap_usd", 0), ("cap_usd", "3"), ("cap_usd", True), ("box_vcpus", 3), ("box_vcpus", 2.5),
                     ("box_extra_minutes", -1), ("box_extra_minutes", 2.5)):
            with self.assertRaises(pad.Usage, msg=(k, v)):
                pad.load_settings(dict(BASE, **{k: v}), PADCFG)

    def test_main_passes_row_values_to_every_run(self):
        d = self.enterContext(tempfile.TemporaryDirectory())
        f = os.path.join(d, "s.toml")
        with open(f, "w") as fh:
            fh.write('harness = "codex-cli"\nkey = "openai"\nmode = "micro"\ncase = "all"\ncap_usd = 3\nbox_vcpus = 2\n')
        seen = []
        with mock.patch.object(pad, "run_all", lambda s, p, h, o, c: seen.append((p, c)) or [("PASS", "")] * len(c)):
            pad.main(["pad", f])
        p, cases = seen[0]
        self.assertEqual((p["cap_usd"], p["box_vcpus"]), (3, 2))
        self.assertEqual(cases, ["direct", "gateway", "dodger"])

    def test_mini_template_loads(self):
        s = pad.load_settings(pad.read_toml(os.path.join(HERE, "runs", "mini.toml")), pad.read_toml(os.path.join(HERE, "pad.toml")))
        self.assertEqual((s["mode"], s["minutes"], s["server"], s["steps"], s["cap_usd"]), ("micro", 10, "staging", ["keep-going"], 3))


class NotVerified(unittest.TestCase):
    """PRD-12 section 9: a stated reason means nothing runs, exit 3, and the table shows that reason."""

    def test_stated_reason_exits_3_before_any_box(self):
        d = self.enterContext(tempfile.TemporaryDirectory())
        f = os.path.join(d, "s.toml")
        with open(f, "w") as fh:
            fh.write('harness = "amp-cli"\nkey = "none"\nmode = "cycle"\n')
        with mock.patch.object(subprocess, "run", side_effect=AssertionError("started a box")):
            self.assertEqual(pad.main(["pad", f]), 3)

    def test_table_shows_the_reason_over_old_rows(self):
        tbl = load("padtable", "table")
        row = {"connector": "amp-cli", "case": "direct", "mode": "full", "result": "FAIL"}
        self.assertEqual(tbl.tier_for("amp-cli", [row]), ("not verified", "", "no key"))


class CrushPermissions(unittest.TestCase):
    def test_crush_runs_with_its_permission_file(self):
        h = pad.read_toml(os.path.join(HERE, "harnesses", "crush-cli.toml"))
        self.assertNotIn("not_verified", h)
        for case in ("direct", "gateway"):
            self.assertEqual(h[case]["files"][".config/crush/crushrc"], "direct/crush-cli-crushrc", case)
        lines = open(os.path.join(HERE, "harnesses", "direct", "crush-cli-crushrc")).read().splitlines()
        allow = [ln for ln in lines if ln.startswith("permissions allow ")]
        self.assertEqual(len(allow), 1)
        self.assertIn("bash", allow[0].split())
        self.assertEqual(load("padtable2", "table").tier_for("crush-cli", []), ("not verified", "", "not yet run on the pad"))


class Walk(unittest.TestCase):
    """A sentinel key in the environment: no argument the pad builds, and no file it writes, holds it."""

    HARNESS = {"version": "1.2.3", "version_cmd": "x --version", "keep_going": True, "gateway": {}}  # a gateway route

    def fake_call(self, argv, timeout=None, env=None):
        if argv[0] == "sh":  # fetch-run: no organizer secret but its own reaches it
            self.assertFalse({"GH_TOKEN", *pad.APPROVAL} & set(env))
            os.makedirs(os.path.join(argv[-1], argv[-2]))
            with open(os.path.join(argv[-1], argv[-2], "fetch.json"), "w") as f:
                f.write("{}")
        elif argv[0] != "node":  # check.py runs for real on the run folder
            return self.real_call(argv, timeout, env)
        self.calls.append(list(argv))
        if argv[2] in self.hang:
            raise subprocess.TimeoutExpired(argv, timeout)
        if argv[2] == "get":  # the box half's tar: meta-box.json only, as a run with no build leaves it
            with tempfile.TemporaryDirectory() as d, tarfile.open(argv[-1], "w") as t:
                with open(os.path.join(d, "meta-box.json"), "w") as f:
                    f.write("{}")
                t.add(os.path.join(d, "meta-box.json"), "meta-box.json")
        if argv[2] == "exec":  # pad-run answers by step, as the fake box is told; by default as with no build
            with open(argv[argv.index("--script") + 1]) as f:
                step = f.read().split()[-1]
            out = "PADSTEP " + json.dumps(self.answers.get(step, {"result": "skip", "saw": "no goalcup build"}))
        return subprocess.CompletedProcess(argv, 0, out if argv[2] == "exec" else "", "")

    def walk(self, hang=(), answers=None, extra_env=None, **settings):
        self.calls, self.hang, self.real_call, self.answers = [], hang, pad.call, answers or {}
        env = {"CLAUDE_API_KEY": SENTINEL, "AI_GATEWAY_API_KEY": SENTINEL + "gw", **(extra_env or {})}
        with tempfile.TemporaryDirectory() as out, mock.patch.dict(os.environ, env), \
                mock.patch.object(pad, "call", self.fake_call):
            s = pad.load_settings(dict(BASE, **settings), PADCFG)
            runs = pad.run_all(s, PADCFG, self.HARNESS, out, pad.case_list(s, self.HARNESS)[0])
            with open(os.path.join(out, "spend.jsonl")) as f:  # section 7: one ledger line per run, however it ended
                self.assertEqual(len(f.readlines()), len(runs))
            for root, _, files in os.walk(out):
                for name in files:
                    with open(os.path.join(root, name), "rb") as f:
                        self.assertNotIn(b"PADSENTINEL", f.read(), name)
        self.assertFalse([a for c in self.calls for a in c if "PADSENTINEL" in a])
        return [c[2:] for c in self.calls]

    def test_sandbox_passes_names_only(self):
        calls = self.walk(case="gateway")
        self.assertEqual(sorted(c[c.index("--env") + 1] for c in calls if c[0] == "put-env"),
                         ["AI_GATEWAY_API_KEY", "CLAUDE_API_KEY"])
        self.assertFalse([c for c in self.walk(case="gateway", key="none") if c[0] == "put-env"])

    def test_create_timeout_still_removes_the_box(self):
        calls = self.walk(hang=("create",))
        name = calls[0][calls[0].index("--name") + 1]
        self.assertIn(["rm", name], calls)
        self.assertIn(["list", "--prefix", "pad-"], calls)

    def test_spans_and_upload_suffix_still_end_with_a_table(self):  # section 1a: no step stops the rig
        self.walk(steps=["cut-worker@1+10", "kill-both@5+upload", "freeze@2+3", "route-drop@4", "kill-watchdog@8", "fd-limit@0"])

    def test_staging_approval_values_reach_only_the_request(self):  # rung 2; the scan in walk covers argv and files
        ok = {"result": "ok", "saw": ""}
        resp = mock.MagicMock(status=200)
        resp.__enter__.return_value = resp
        with mock.patch.object(pad.urllib.request, "urlopen", return_value=resp) as urlopen:
            self.walk(mode="micro", server="staging", answers={
                "setup": ok, "start": dict(ok, code="BCDF-GHJK"), "paste": ok, "wait-upload": dict(ok, entry="e" + "0" * 16)},
                extra_env={"GOALCUP_TEST_APPROVAL_KEY": SENTINEL + "ap", "GOALCUP_TEST_MEMBER_ID": "PADSENTINEL-m1",
                           "GH_TOKEN": SENTINEL + "gh"})
        req = urlopen.call_args.args[0]
        self.assertEqual(req.get_header("Authorization"), "Bearer " + SENTINEL + "ap")
        self.assertEqual(json.loads(req.data)["member_id"], "PADSENTINEL-m1")


class HereAndFetch(unittest.TestCase):
    """Section 7: here is the box, so no organizer secret reaches pad-run; a killed fetch-run leaves no work folder."""

    SECRETS = {"GOALCUP_TEST_APPROVAL_KEY": SENTINEL + "ap", "GH_TOKEN": SENTINEL + "gh", "DATABASE_URL": SENTINEL + "db"}

    def run_for(self, out, **settings):
        s = pad.load_settings(dict(BASE, mode="micro", **settings), PADCFG)
        r = pad.Run(s, "direct", PADCFG, Walk.HARNESS, out)
        self.addCleanup(shutil.rmtree, r.tmp, True)
        return r

    def test_here_pad_run_gets_no_organizer_secret(self):
        seen = []
        def fake(argv, timeout=None, env=None):
            seen.append(env)
            return subprocess.CompletedProcess(argv, 0, 'PADSTEP {"result": "ok"}', "")
        with tempfile.TemporaryDirectory() as out, mock.patch.dict(os.environ, self.SECRETS), \
                mock.patch.object(pad, "call", fake):
            r = self.run_for(out, where="here")
            r.box_dir = out
            r.box("setup", 10)
        self.assertIsNotNone(seen[0])
        self.assertFalse(set(pad.NEVER_IN_BOX) & set(seen[0]))

    def test_killed_fetch_run_leaves_no_work_folder(self):
        def fake(argv, timeout=None, env=None):
            work = os.path.join(argv[-1], ".fetch-run.abc123")
            os.makedirs(work)
            with open(os.path.join(work, "pg_service.conf"), "w") as f:
                f.write("password=" + SENTINEL)
            raise subprocess.TimeoutExpired(argv, timeout)
        with tempfile.TemporaryDirectory() as out, mock.patch.object(pad, "call", fake), \
                mock.patch.dict(os.environ, {"DATABASE_URL": "postgres://fake-not-a-secret"}):
            r = self.run_for(out, server="staging")
            r.meta["entry"] = "e" + "0" * 16
            r.fetch_index = lambda *a: None  # the index box is test_pad_fetch's; here fetch-run is the one that hangs
            with self.assertRaises(subprocess.TimeoutExpired):
                r.step_fetch("fetch", 10)
            self.assertEqual(os.listdir(os.path.join(r.dir, "fetched")), [])


class BoxHalf(unittest.TestCase):
    """pad-run: known values never reach a pane file, and an env key launch only exports the value."""

    def test_pane_replacement(self):
        v = "PADSENTINEL-9f3c1e/+x"
        forms = [v, base64.b64encode(v.encode()).decode(), urllib.parse.quote(v, safe="")]
        safe, n = padrun.replace_known(" | ".join(forms), ["other-known-value", v])
        self.assertEqual(n, 3)
        for form in forms:
            self.assertNotIn(form, safe)
        self.assertIn("[known:2]", safe)

    def test_key_tail_a_harness_shows_is_replaced(self):  # point 4: Claude Code shows the last 20 characters of the key
        v = "sk-ant-api03-" + "Zq7xLm2VbN8cRt4HwK1d" + "PADTAIL0123456789abcd"
        shown = "Use this API key?  ...%s  (No / Yes)" % v[-20:]
        safe, n = padrun.replace_known(shown + " " + v[-12:], [v])
        self.assertNotIn(v[-20:], safe)
        self.assertNotIn(v[-12:], safe)
        self.assertGreaterEqual(n, 2)

    def test_provider_file_keeps_the_rows_setup_wrote(self):  # Codex needs approval_policy and sandbox_mode from setup
        old = 'approval_policy = "never"\nsandbox_mode = "danger-full-access"\n[projects."/p"]\ntrust_level = "trusted"\n'
        new = 'model = "m"\nmodel_reasoning_effort = "low"\n'
        out = padrun.merge_config("config.toml", old, new)
        for row in ('approval_policy = "never"', 'sandbox_mode = "danger-full-access"', 'model = "m"', 'trust_level = "trusted"'):
            self.assertIn(row, out)
        self.assertLess(out.index('model = "m"'), out.index("[projects"))  # top-level rows stay above the tables
        merged = json.loads(padrun.merge_config("a.json", json.dumps({"x": {"a": 1}, "k": 1}), json.dumps({"x": {"b": 2}})))
        self.assertEqual(merged, {"x": {"a": 1, "b": 2}, "k": 1})

    def launch(self, steps=()):
        """launch.sh as start writes it for an env-route key, and the argv start passed to its fake tmux."""
        run = self.enterContext(tempfile.TemporaryDirectory())
        os.makedirs(os.path.join(run, "keys"))
        key = key_file(os.path.join(run, "keys"))
        for name, doc in (("settings.json", {"server": "off", "harness_id": "claude-code-cli", "keep": True, "harness": {},
                                             "steps": list(steps)}),
                          ("state.json", {"key_route": "env", "key_env": "ANTHROPIC_API_KEY", "key_path": key})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        argvs = []
        fake = lambda argv, **kw: argvs.append(argv) or subprocess.CompletedProcess(argv, 0, "", "")  # noqa: E731
        with mock.patch.object(padrun, "sh", fake), mock.patch.object(padrun, "tool", lambda n: "/bin/true"), \
                mock.patch.object(padrun, "wait", lambda *a: False):
            with self.assertRaises(padrun.Step):  # the fake tmux never shows "Recording": the step ends after the launch
                padrun.Run(run, "start").start(None)
        with open(os.path.join(run, "launch.sh")) as f:
            text = f.read()
        self.assertNotIn("PADSENTINEL", text)
        self.assertFalse([a for argv in argvs for a in argv if "PADSENTINEL" in a])
        return text

    def test_env_key_launch_holds_no_value(self):
        self.assertIn("rm -f", self.launch())

    def test_kill_both_keeps_the_env_key_copy_for_continue(self):  # cont.sh is launch.sh less its last line: the key must still be there
        self.assertNotIn("rm -f", self.launch(["kill-both@5"]))

    def test_exec_env_keeps_no_key(self):  # section 3.1: /proc/<pid>/environ holds the harness's keys
        environ = b"\0".join([b"HTTPS_PROXY=http://127.0.0.1:4100", b"SSL_CERT_FILE=/run/ca.pem", b"PAD_CA=/run/ca2.pem",
                              b"OPENAI_API_KEY=" + SENTINEL.encode(), b"GOALCUP_TEST_APPROVAL_KEY=gca-PADSENTINEL-2",
                              b"PATH=/usr/bin", b"HOME=/iso", b"GOALCUP_TIME_LEFT_FILE=/e/upload.json", b"LANG=C", b""])
        got = padrun.exec_env(environ, ["PAD_CA"])  # PRD-10 acceptance 7 adds HOME, PATH and the time-left file
        self.assertEqual(set(got), {"HTTPS_PROXY", "SSL_CERT_FILE", "PAD_CA", "PATH", "HOME", "GOALCUP_TIME_LEFT_FILE"})
        self.assertNotIn("PADSENTINEL", json.dumps(got))


class FaultRestore(unittest.TestCase):
    """Point 3: a fault never outlives its step. hold() undoes in a finally, with a detached backstop for a killed pad-run,
    and the frozen tree never holds pad-run, the shell running the step, or tmux."""

    def test_hold_undoes_when_the_fault_fails(self):
        undone = []
        r = padrun.Run(self.enterContext(tempfile.TemporaryDirectory()), "freeze")
        def boom():
            raise OSError("no sudo")
        with mock.patch.object(padrun.subprocess, "Popen") as backstop, mock.patch.object(padrun.signal, "signal"):
            with self.assertRaises(OSError):
                r.hold(1, boom, lambda: undone.append(1), "kill -CONT 1")
        self.assertEqual((undone, backstop.call_args.kwargs["start_new_session"]), ([1], True))

    def test_frozen_tree_leaves_this_process_and_its_parents(self):
        got = padrun.Run(self.enterContext(tempfile.TemporaryDirectory()), "freeze").tree([os.getppid(), os.getpid()])
        self.assertFalse({os.getpid(), os.getppid()} & set(got))


class CrashRows(unittest.TestCase):
    """Section 3.1, ledger R3a and R3b: one closed recorder_down gap per kill under 2 s, no null_mode gap; the
    recover row keeps the harness pid and port. Only result values are asserted."""

    UP = ({"written": {"model_requests": 2}}, {"written": {"model_requests": 5}})  # status samples before and after

    @staticmethod
    def row(kind, gap, samples=UP, recovery=(("ok", "10:03"),), seal="harness_exit", over_cap=False,
            words=check.RECOVER, more=(), pid=9, code=1, case="direct", quiet=False, commits=1, extra=None, known_box=False):
        told = [{"kind": k, "exit": 0, "said": w, "at": "2026-10-30T10:%s+00:00" % t}
                for k, w, t in zip(("recover", "recover", "status"), words + (check.STATUS_RECOVER,), ("02:30", "04:00", "03:30"))]
        kills = [dict({"kind": k, "pid": pid, "at": "2026-10-30T10:02:00+00:00"}, **({"quiet": True} if quiet else {}))
                 for k in ("kill-watchdog", "kill-recorder")]
        rec = [{"kind": "wait-recovery", "result": x, "at": "2026-10-30T%s:00+00:00" % t} for x, t in recovery]
        files = {"meta.json": {"connector": "codex-cli", "case": case, "model_host": "api.example", "variant": [kind + "@3"], "server": "off",
                               "keep": True, "commits_since_start": commits, "entry": "e1", "over_cap": over_cap,
                               "started": "2026-10-30T10:00:00+00:00", **({"known_box": True} if known_box else {})},
                 "verify/check-local.json": {"exit": code, "entries": [{"entry": "e1", "seal_reason": seal,
                     "counts": {"model_requests": 5, "gaps": {"recorder_down": 1}}, "checks": [
                         {"id": "gap_has_end", "result": "pass"}, {"id": "history_complete", "result": "pass"},
                         {"id": "inv6", "result": "named", "facts": {"gaps": [gap] if gap else []}}]}]},
                 "verify/usage/e1.json": {"tokens": {"merged": {"input": 9, "output": 9}}, "coverage": {"unattributed": 0},
                                          "threads": [{"parent": None, "models": {"m": 3}, "wire_calls": 3},
                                                      {"parent": "t0", "models": {"m": 2}, "wire_calls": 2}]},
                 "watch/status.jsonl": [{"at": "2026-10-30T10:0%d:00+00:00" % (4 * i), "status": x} for i, x in enumerate(samples)],
                 "watch/faults.jsonl": told + kills + rec + list(more)}
        files.update(extra or {})
        return evaluate(files)

    GAP = {"start": "2026-10-30T10:01:59.9Z", "end": "2026-10-30T10:02:01.2Z"}  # the kill is at 10:02:00

    def test_kill_recorder(self):
        gap = self.GAP
        good = self.row("kill-recorder", gap)
        self.assertEqual((good["result"] != "FAIL", good["faults"]), (True, "PASS"))
        down = (("FAIL", "10:04"),)  # did not stand back up
        for bad in ({"gap": dict(gap, cause="null_mode")}, {"gap": dict(gap, end="2026-10-30T10:02:02.5Z")}, {"recovery": down},
                    {"samples": self.UP[:1] * 2}):  # no model request written after the kill
            self.assertEqual(self.row("kill-recorder", **dict({"gap": gap}, **bad))["result"], "FAIL", bad)

    def test_gap_is_under_2s_and_ends_within_2s_of_the_kill(self):  # PRD-02 R1, PRD-12 section 3.1; a quiet kill times only the end
        def gap(start, end):
            return {"start": "2026-10-30T%sZ" % start, "end": "2026-10-30T%sZ" % end}
        for start, end, quiet, fails in (("10:01:59.5", "10:02:01.2", False, False), ("10:01:00", "10:02:01.2", False, True),
                                         ("10:01:00", "10:02:01.2", True, False), ("10:01:59.5", "10:02:02.5", False, True),
                                         ("10:01:59.5", "10:02:02.5", True, True), ("10:01:00", "10:01:59", True, True)):
            res = self.row("kill-recorder", gap(start, end), quiet=quiet)["result"]
            self.assertEqual(res == "FAIL", fails, (start, end, quiet))
            if quiet and not fails:
                self.assertEqual(res, "INCONCLUSIVE", (start, end))

    LOST = {"kind": "status", "exit": 0, "at": "2026-10-30T10:03:40+00:00",
            "said": "Problem. Recording is off since 10:02: goalcup is not running. To go on, run: goalcup continue r1"}

    def test_unrelated_continue_text_is_not_a_lost_terminal(self):  # R5-2: a Codex status shows an unrelated 'goalcup continue' line
        other = {"kind": "note", "said": "not recorded ... goalcup seal, then goalcup continue", "at": "2026-10-30T10:03:41+00:00"}
        got = self.row("kill-watchdog-recover", self.GAP, more=[other])
        self.assertNotEqual(got["faults"], "INCONCLUSIVE")
        self.assertEqual(self.row("kill-watchdog-recover", self.GAP, more=[self.LOST])["faults"], "INCONCLUSIVE")

    def test_no_work_is_inconclusive(self):  # R5-3
        got = self.row("kill-watchdog-recover", self.GAP, more=[self.LOST], commits=0)
        self.assertEqual(got["faults"], "INCONCLUSIVE")

    def test_dodger_progress_is_tunnel_connections(self):
        def tun(n):
            return {"written": {"model_requests": 0, "tunnel_hosts": {"api.example": {"conns": n}}}}
        for samples, kw, want in (((tun(1), tun(4)), {}, "PASS"), ((tun(2), tun(2)), {}, "FAIL"),
                                  ((tun(2), tun(2)), {"quiet": True}, "INCONCLUSIVE")):
            self.assertEqual(self.row("kill-recorder", self.GAP, samples, case="dodger", **kw)["faults"], want, kw)

    def test_recover_keeps_harness_pid_and_port(self):
        gap = {"cause": "watchdog_lost", "start": "2026-10-30T10:01:00Z", "end": "2026-10-30T10:02:00Z"}
        for pid, port, fails in ((7, 5, False), (8, 5, True), (7, 6, True)):
            samples = [{"port": 5, "harness": {"pid": 7}}, {"port": port, "harness": {"pid": pid}}]
            self.assertEqual(self.row("kill-watchdog-recover", gap, samples)["result"] == "FAIL", fails, (pid, port))
        for over_cap, fails in ((True, False), (False, True)):  # section 7: a seal at the spend cap is not a failure
            self.assertEqual(self.row("kill-watchdog-recover", gap, seal="command", over_cap=over_cap)["result"] == "FAIL",
                             fails, over_cap)

    def test_seal_at_the_spend_cap_is_not_a_fault_failure(self):  # section 7, ledger T6: the pad's own seal
        gap = self.GAP
        down = (("FAIL", "10:04"),)
        wgap = {"cause": "watchdog_lost", "start": "2026-10-30T10:01:00Z", "end": "2026-10-30T10:02:00Z"}
        for at, over_cap, code, fails in (("10:03", True, 0, False), ("10:03", False, 0, True), ("10:20", True, 0, True),
                                          ("10:03", True, 1, True)):  # a seal-now that did not seal is not the cap seal
            seal = [{"kind": "seal-now", "exit": code, "at": "2026-10-30T%s:00+00:00" % at}]
            self.assertEqual(self.row("kill-recorder", gap, recovery=down, over_cap=over_cap, more=seal)["result"] == "FAIL",
                             fails, (at, over_cap, code))
            self.assertEqual(self.row("kill-watchdog-recover", wgap, words=(check.RECOVER[0], "sealed"), over_cap=over_cap,
                                      more=seal)["result"] == "FAIL", fails, (at, over_cap, code))
        late = [{"kind": "seal-now", "exit": "timeout", "began": "2026-10-30T10:03:00+00:00", "at": "2026-10-30T10:40:00+00:00"}]
        self.assertNotEqual(self.row("kill-recorder", gap, recovery=down, over_cap=True, more=late)["result"], "FAIL")  # a seal cut by the step limit still counts
        # Sealed at the cap before the kill: nothing left to kill, no gap, verifier exit 0. Not goalcup's failure.
        early = [{"kind": "seal-now", "exit": 0, "at": "2026-10-30T10:01:00+00:00"}]
        got = self.row("kill-recorder", None, pid=None, code=0, recovery=(("skip", "10:02"),), over_cap=True, more=early)
        self.assertEqual((got["result"] != "FAIL", got["faults"]), (True, "INCONCLUSIVE"))


class FaultRows(unittest.TestCase):
    """The rows the record decides, one case each: a row passes only on evidence that the fault took and held (point 3)."""

    SAMPLES = [{"at": "2026-10-30T10:%02d:00+00:00" % (i * 8), "status": {"written": {"model_requests": 2 + 3 * i}}} for i in (0, 1)]

    def row(self, kinds, faults, gaps=(), counts=None, seal="harness_exit", **files):
        kinds = [kinds] if isinstance(kinds, str) else kinds
        entry = {"entry": "e1", "seal_reason": seal, "counts": {"model_requests": 5, "gaps": counts or {}}, "checks": [
            {"id": "gap_has_end", "result": "pass"}, {"id": "history_complete", "result": "pass"},
            {"id": "listed_hosts_decrypted", "result": "pass"}, {"id": "inv6", "result": "named", "facts": {"gaps": list(gaps)}}]}
        use = {"tokens": {"merged": {"input": 9, "output": 9}}, "coverage": {"unattributed": 0},
               "threads": [{"parent": None, "models": {"m": 3}, "wire_calls": 3}, {"parent": "t0", "models": {"m": 2}, "wire_calls": 2}]}
        meta = {"connector": "codex-cli", "case": "direct", "variant": kinds, "server": "off", "keep": True,
                "commits_since_start": 1, "entry": "e1", "started": "2026-10-30T10:00:00+00:00"}
        lines = [dict(f, at="2026-10-30T10:%s+00:00" % f["at"]) for f in faults]
        return evaluate({"meta.json": meta, "verify/check-local.json": {"exit": 1, "entries": [entry]},
                         "verify/usage/e1.json": use, "watch/status.jsonl": self.SAMPLES, "watch/faults.jsonl": lines,
                         **files})["faults"]

    def test_a2_recorder_kill_then_recover_has_two_gaps(self):  # PRD-16 A2: kill-recorder@30 and kill-watchdog-recover@45
        plain = {"start": "2026-10-30T10:00:30Z", "end": "2026-10-30T10:00:31Z"}
        own = {"cause": "watchdog_lost", "start": "2026-10-30T10:02:00Z", "end": "2026-10-30T10:03:00Z"}
        faults = [{"kind": k, "exit": 0, "said": w, "at": at} for k, w, at in (
            ("recover", check.RECOVER[0], "02:30"), ("recover", check.RECOVER[1], "04:00"), ("status", check.STATUS_RECOVER, "03:30"))]
        faults += [{"kind": k, "pid": 9, "at": t} for k, t in (("kill-recorder", "00:30"), ("kill-watchdog", "02:00"), ("kill-recorder", "02:00"))]
        faults += [{"kind": "wait-recovery", "result": "ok", "at": "01:00"}]
        both = ["kill-recorder@0", "kill-watchdog-recover@3"]
        self.assertEqual(self.row(both, faults, [plain, own], counts={"recorder_down": 2}), "PASS")
        self.assertEqual(self.row(both, faults, [dict(plain, cause="null_mode"), own], counts={"recorder_down": 2}), "FAIL")
        self.assertEqual(self.row(both, faults, [own], counts={"recorder_down": 1}), "FAIL")  # the plain kill left no gap

    def test_kill_both_abandoned_gap_may_start_a_minute_before_the_kill(self):  # it starts at the last LINK (PRD-01 section 10)
        both = [{"kind": "kill-both", "pid": 3, "watchdog": 4, "at": "05:00"}]
        for start, want in (("10:04:20", "PASS"), ("10:02:00", "FAIL")):
            gap = {"cause": "abandoned", "start": "2026-10-30T%sZ" % start, "end": "2026-10-30T10:06:00Z"}
            self.assertEqual(self.row("kill-both@5", both, [gap], counts={"recorder_down": 1}, seal="recovered"), want, start)

    def test_kill_both_seal_is_judged_from_the_record(self):  # rung 5b: sealed recovered, one abandoned gap, script exit 0
        def gap(start, cause="abandoned"):
            return {"cause": cause, "start": "2026-10-30T%sZ" % start, "end": "2026-10-30T10:06:00Z"}

        def ran(code):
            return [{"kind": "exec", "script": "kill-both-seal.sh", "exit": code, "at": "05:10", "began": "2026-10-30T10:04:30Z"}]
        one = gap("10:04:20")
        for code, seal, gaps, want in ((0, "recovered", [one], "PASS"), (0, "harness_exit", [one], "FAIL"),
                                       (0, "recovered", [], "FAIL"), (3, "recovered", [one], "FAIL"),
                                       (0, "recovered", [gap("10:01:00", "recorder_down"), one], "FAIL"),  # two gaps
                                       (0, "recovered", [gap("10:01:00")], "FAIL"),  # long before the kill
                                       (0, "recovered", [gap("10:05:30")], "FAIL")):  # after the script ended
            got = self.row("exec kill-both-seal.sh@2", ran(code), gaps, counts={"recorder_down": 1}, seal=seal)
            self.assertEqual(got, want, (code, seal, gaps))

    def test_kill_watchdog_seal_is_timed_from_the_record(self):  # not from status.json, which is rewritten until the box ends
        faults = [{"kind": "kill-watchdog", "pid": 9, "at": "02:00"}, {"kind": "harness-exit", "seen": True, "at": "05:00"}]
        for sealed, want in (("2026-10-30T10:05:20Z", "PASS"), ("2026-10-30T10:09:00Z", "FAIL")):
            self.assertEqual(self.row("kill-watchdog@2", faults, **{"verify/seal.json": {"wall": sealed},
                                                                     "status.json": {"updated": "2026-10-30T10:09:00Z"}}), want)
        self.assertEqual(self.row("kill-watchdog@2", faults), "INCONCLUSIVE")  # no seal time read

    def test_route_drop_needs_a_drop_that_took_and_names_loss_only_inside_it(self):
        def drop(took):
            return [{"kind": "route-drop", "took": took, "at": "03:00"}, {"kind": "route-back", "default": True, "at": "04:00"}]
        decode = {"decode_failed": 1}
        for start, want in (("10:03:30", "PASS"), ("10:12:00", "FAIL")):
            lost = [{"reason": "decode_failed", "start": "2026-10-30T%sZ" % start, "end": "2026-10-30T%sZ" % start}]
            self.assertEqual(self.row("route-drop@3", drop(True), counts=decode, **{"verify/gaps.json": lost}), want, start)
        self.assertEqual(self.row("route-drop@3", drop(False)), "INCONCLUSIVE")

    def test_freeze_that_stopped_nothing_is_not_a_pass(self):
        for pids, want in ((3, "PASS"), (0, "INCONCLUSIVE")):
            self.assertEqual(self.row("freeze@2", [{"kind": "freeze", "pids": pids, "at": "02:00"}, {"kind": "thaw", "at": "12:00"}]), want)


class StatedLoss(unittest.TestCase):
    """C3 and the kill row's exit rule for a shape found only in a joined SSE deltas body (PRD-05 Risks "Model echo")."""

    DELTAS = {"kind": "generic_secret_assignment", "in": "e1/1.3/deltas", "sha12": "a" * 12, "n": 1}
    PLAIN = {"kind": "generic_secret_assignment", "in": "e1/1.3", "sha12": "a" * 12, "n": 1}
    KNOWN = {"form": "raw", "in": "e1/1.3/deltas", "sha12": "b" * 12, "n": 1}
    KNOWN_BODY = {"form": "raw", "in": "e1/1.3", "sha12": "b" * 12, "n": 1}  # a request body or a contiguous response stream

    def run_row(self, known=(), shapes=(), code=2):
        extra = {"verify/secrets.json": {"known_hits": list(known), "shape_hits": list(shapes), "big_masks": [],
                                         "masks": dict(SecretCheck.PLANTED)},
                 "verify/secrets-conductor.json": {"hits": 0},
                 "verify/exits.json": {"secrets": 2 if known or shapes else 0, "secrets-out": 0, "secrets-plain": 0}}
        gap = {"start": "2026-10-30T10:01:59.9Z", "end": "2026-10-30T10:02:01.2Z"}
        return CrashRows.row("kill-recorder", gap, code=code, extra=extra, known_box=True)

    def plain_row(self, shapes, code, counts, extra_checks=()):
        checks = ([{"id": "secrets", "result": "fail"}] if shapes else []) + list(extra_checks)
        return evaluate({"meta.json": {"connector": "codex-cli", "case": "direct", "known_box": True, "entry": "e1", "commits_since_start": 1},
                         "verify/check-local.json": {"exit": code, "entries": [{"entry": "e1", "counts": {"model_requests": 5, "gaps": counts},
                                                                                 "checks": checks}]},
                         "verify/secrets.json": {"known_hits": [], "shape_hits": list(shapes), "big_masks": [], "masks": dict(SecretCheck.PLANTED)},
                         "verify/secrets-conductor.json": {"hits": 0},
                         "verify/exits.json": {"secrets": 2 if shapes else 0, "secrets-out": 0, "secrets-plain": 0}})

    def test_deltas_only_shape_is_a_pass_with_the_stated_loss_note(self):  # owner answer D1
        row = self.run_row(shapes=[self.DELTAS])
        self.assertEqual(row["checks"]["C3"], "PASS", row["why"])
        self.assertTrue(any("stated loss: shape in streamed reply" in w for w in row["why"]), row["why"])
        self.assertNotEqual(row["faults"], "FAIL")
        self.assertNotEqual(row["result"], "FAIL")

    def test_stated_loss_does_not_hide_an_unexplained_gap(self):
        gaps = {"s6_gaps": {"memory_guard": 1}}
        for shapes, code in (([], 1), ([self.DELTAS], 2)):
            row = self.plain_row(shapes, code, gaps["s6_gaps"])
            self.assertEqual(row["checks"]["C1"], "FAIL", (shapes, code, row["why"]))
        for shapes, code in (([], 1), ([self.DELTAS], 2)):  # no gap at all, but a named history orphan
            row = self.plain_row(shapes, code, {}, [{"id": "history_complete", "result": "named"}])
            self.assertEqual(row["checks"]["C1"], "FAIL", (shapes, code, row["why"]))
        for shapes, code in (([], 1), ([self.DELTAS], 2)):  # no gap at all, another check named
            row = self.plain_row(shapes, code, {}, [{"id": "gaps_named", "result": "named"}])
            self.assertEqual(row["checks"]["C1"], "FAIL", (shapes, code, row["why"]))

    def test_any_other_hit_still_fails(self):
        for known, shapes in (([self.KNOWN_BODY], [self.DELTAS]), ([self.KNOWN, self.KNOWN_BODY], []), ([], [self.PLAIN]), ([], [self.DELTAS, self.PLAIN]),
                              ([], [dict(self.DELTAS, **{"in": None})])):
            row = self.run_row(known, shapes)
            self.assertEqual((row["checks"]["C3"], row["faults"]), ("FAIL", "FAIL"), (known, shapes))

    def planted_row(self, known, shapes, masks):
        extra = {"verify/secrets.json": {"known_hits": list(known), "shape_hits": list(shapes), "big_masks": [], "masks": masks},
                 "verify/secrets-conductor.json": {"hits": 0},
                 "verify/exits.json": {"secrets": 2, "secrets-out": 0, "secrets-plain": 0}}
        gap = {"start": "2026-10-30T10:01:59.9Z", "end": "2026-10-30T10:02:01.2Z"}
        return CrashRows.row("kill-recorder", gap, code=2, extra=extra)

    def test_stated_loss_with_a_planted_kind_unmasked_is_open_not_a_fail(self):
        masks = dict(SecretCheck.PLANTED, jwt=0, private_key=0)
        row = self.planted_row([], [self.DELTAS], masks)
        self.assertEqual(row["checks"]["C3"], "INCONCLUSIVE")
        self.assertNotIn(row["result"], ("FAIL", "PASS"))

    def test_known_or_pathless_hit_with_a_planted_kind_unmasked_still_fails(self):
        masks = dict(SecretCheck.PLANTED, jwt=0)
        for known, shapes in (([self.KNOWN_BODY], [self.DELTAS]), ([], [self.PLAIN])):
            self.assertEqual(self.planted_row(known, shapes, masks)["checks"]["C3"], "FAIL", (known, shapes))

    def test_known_value_only_in_streamed_reply_is_a_stated_loss_pass(self):  # T8, owner answer 2 Nov (split-echo gap)
        for known, shapes in (([self.KNOWN], []), ([self.KNOWN], [self.DELTAS])):
            row = self.run_row(known, shapes)
            self.assertEqual(row["checks"]["C3"], "PASS", row["why"])
            self.assertTrue(any("known value split in streamed reply" in w for w in row["why"]), row["why"])
            self.assertEqual(row["facts"]["stated_loss_known_hits"], 1)
        self.assertEqual(self.run_row([self.KNOWN_BODY], [self.DELTAS])["checks"]["C3"], "FAIL")  # same hit in a request body

    def test_exit_2_without_a_stated_loss_still_fails(self):
        self.assertEqual(self.run_row(code=2)["faults"], "FAIL")  # exit 2 with no stated loss behind it


class UpstreamRedaction(unittest.TestCase):
    """C3 for a harness file with redacts_tool_output (point 4): no hit and no planted MASK is masked upstream, a PASS."""

    DELTAS = StatedLoss.DELTAS

    def c3(self, up, masks, known=(), shapes=()):
        return evaluate({"meta.json": {"connector": "hermes-cli", "case": "direct", "known_box": True, "entry": "e1", "redacts_tool_output": up,
                                      "redacts_kinds": ["openai_key", "github_token"]},
                         "verify/secrets.json": {"masks": masks, "known_hits": list(known), "shape_hits": list(shapes), "big_masks": []},
                         "verify/secrets-conductor.json": {"hits": 0},
                         "verify/exits.json": {"secrets": 2 if known or shapes else 1, "secrets-out": 0, "secrets-plain": 0}}
                        )["checks"]["C3"]

    def test_no_hit_and_no_planted_mask_passes_upstream(self):
        rest = {"google_key": 1, "jwt": 1, "private_key": 1, "url_password": 1}  # kinds the vendor rule does not name
        self.assertEqual(self.c3(True, dict(rest, masked_key=3)), "PASS")
        self.assertEqual(self.c3(True, dict(SecretCheck.PLANTED, jwt=0, masked_key=3)), "INCONCLUSIVE")  # jwt is no vendor-named kind
        self.assertEqual(self.c3(True, {"masked_key": 3}), "INCONCLUSIVE")

    def test_no_exercise_evidence_is_not_a_pass(self):  # nothing shows the planted file reached the harness
        self.assertEqual(self.c3(True, {}), "INCONCLUSIVE")
        self.assertEqual(self.c3(True, {"auth": 8}), "INCONCLUSIVE")

    def test_a_plain_hit_with_exact_off_is_evidence_not_less(self):
        files = {"meta.json": {"connector": "hermes-cli", "case": "direct", "known_box": True, "entry": "e1", "redacts_tool_output": True,
                           "redacts_kinds": ["openai_key", "github_token"]},
                 "verify/secrets.json": {"masks": {"masked_key": 3, "google_key": 1, "jwt": 1, "private_key": 1, "url_password": 1}, "known_hits": [], "shape_hits": [], "big_masks": []},
                 "verify/secrets-conductor.json": {"hits": 0},
                 "verify/exits.json": {"secrets": 1, "secrets-out": 0, "secrets-plain": 2}}
        self.assertEqual(evaluate(files)["checks"]["C3"], "PASS")

    def test_a_harness_without_the_flag_stays_open(self):  # control: same files
        self.assertEqual(self.c3(False, {"masked_key": 3}), "INCONCLUSIVE")
        self.assertEqual(self.c3(False, {}), "INCONCLUSIVE")

    def test_a_known_hit_still_fails(self):
        self.assertEqual(self.c3(True, {}, known=[StatedLoss.KNOWN_BODY]), "FAIL")

    def test_deltas_only_shape_is_a_stated_loss_not_a_fail(self):
        self.assertEqual(self.c3(True, SecretCheck.PLANTED, shapes=[self.DELTAS]), "PASS")
        self.assertEqual(self.c3(True, {}, shapes=[self.DELTAS]), "INCONCLUSIVE")  # still no exercise evidence


class SecretCheck(unittest.TestCase):
    """C3 (point 4, section 5): a known-value hit fails, and so does a planted shape with no MASK. Only C3 is asserted."""

    PLANTED = {"openai_key": 1, "github_token": 1, "google_key": 1, "jwt": 1, "private_key": 1,
               "url_password": 1, "auth": 8}

    def c3(self, masks, secrets_exit=1, plain_exit=0):
        return evaluate({"meta.json": {"connector": "codex-cli", "case": "direct", "known_box": True, "entry": "e1"},
                         "verify/secrets.json": {"masks": masks}, "verify/secrets-conductor.json": {"hits": 0},
                         "verify/exits.json": {"secrets": secrets_exit, "secrets-out": 0, "secrets-plain": plain_exit}}
                        )["checks"]["C3"]

    def test_known_value_hit_fails(self):
        self.assertEqual(self.c3(self.PLANTED, secrets_exit=2), "FAIL")
        self.assertEqual(self.c3(dict(self.PLANTED, jwt=0), secrets_exit=2), "FAIL")  # a hit in a kind with no MASK
        self.assertEqual(self.c3(dict(self.PLANTED, jwt=0)), "INCONCLUSIVE")  # no hit, no MASK: not exercised

    def exact(self, masks, plain_exit=0, on=True, skipped=0):
        meta = {"connector": "codex-cli", "case": "direct", "known_box": True, "entry": "e1"}
        if on:
            meta["mask_values"] = {"workspace": 1, "env": 0, "harness": 0, "skipped": skipped}
        return evaluate({"meta.json": meta, "verify/secrets.json": {"masks": masks}, "verify/secrets-conductor.json": {"hits": 0},
                         "verify/exits.json": {"secrets": 1, "secrets-out": 0, "secrets-plain": plain_exit}})["checks"]["C3"]

    def test_exact_values_on(self):
        self.assertEqual(self.exact(dict(self.PLANTED, run_secret=2), plain_exit=2), "FAIL")  # the planted value reached the record
        self.assertEqual(self.exact(self.PLANTED), "PASS")
        self.assertEqual(self.exact({"auth": 8, "run_secret": 4}), "INCONCLUSIVE")  # run_secret is no proof: env or auth can make it
        self.assertEqual(self.exact({"auth": 8}), "INCONCLUSIVE")
        self.assertEqual(self.exact(dict(self.PLANTED, jwt=0)), "INCONCLUSIVE")  # a kind with no MASK is not exercised
        self.assertEqual(self.exact(self.PLANTED, plain_exit=2, skipped=1), "INCONCLUSIVE")  # a stated cut

    def hermes(self, masks):
        meta = {"connector": "hermes-cli", "case": "direct", "known_box": True, "entry": "e1", "redacts_tool_output": True,
                "redacts_kinds": ["openai_key", "github_token"],
                "mask_values": {"workspace": 4, "env": 0, "harness": 0, "skipped": 0}}
        out = evaluate({"meta.json": meta, "verify/secrets.json": {"masks": masks}, "verify/secrets-conductor.json": {"hits": 0},
                        "verify/exits.json": {"secrets": 1, "secrets-out": 0, "secrets-plain": 0}})
        return out["checks"]["C3"], out["facts"].get("masked_upstream")

    def test_run_secret_never_stands_for_a_planted_kind(self):
        # Redacting harness, exact values on: masked_key and run_secret with no openai/github/google MASK is the unconfirmed
        # masked-upstream PASS (refused by the release check), never a plain PASS; run_secret alone is no evidence.
        shaped = {"google_key": 1, "jwt": 1, "private_key": 1, "url_password": 1, "masked_key": 3, "run_secret": 1, "auth": 8}
        self.assertEqual(self.hermes(shaped), ("PASS", True))
        self.assertEqual(self.hermes({"run_secret": 4, "auth": 8})[0], "INCONCLUSIVE")

    def test_exact_values_off_keeps_a_plain_hit_as_proof(self):
        self.assertEqual(self.exact(self.PLANTED, plain_exit=2, on=False), "PASS")
        self.assertEqual(self.exact({"auth": 8}, plain_exit=2, on=False), "INCONCLUSIVE")  # plain proves exercise, kinds unmasked

    FILES = {"meta.json": {"connector": "codex-cli", "case": "direct", "known_box": True, "entry": "e1"},
             "verify/secrets-conductor.json": {"hits": 0},
             "verify/exits.json": {"secrets": 0, "secrets-out": 0, "secrets-plain": 0}}

    def test_kind_with_no_mask_and_no_hit_is_not_a_pass(self):  # a run never passes on a missing exercise
        files = dict(self.FILES, **{"verify/secrets.json": {"masks": dict(self.PLANTED, jwt=0)}})
        self.assertEqual(evaluate(files)["checks"]["C3"], "INCONCLUSIVE")

    def test_unscanned_pane_copies_are_not_a_pass(self):  # exit 3 and no report: no scan
        files = dict(self.FILES, **{"verify/secrets.json": {"masks": self.PLANTED},
                                    "verify/exits.json": {"secrets": 0, "secrets-out": 0, "secrets-plain": 0, "secrets-panes": 3}})
        self.assertNotEqual(evaluate(files)["checks"]["C3"], "PASS")

    def test_known_value_on_a_pane_copy_fails(self):  # the rig's own replacement missed a value
        files = dict(self.FILES, **{"verify/secrets.json": {"masks": self.PLANTED}, "verify/secrets-panes.json": {"known_hits": [{}]},
                                    "verify/exits.json": {"secrets": 0, "secrets-out": 0, "secrets-plain": 0, "secrets-panes": 2}})
        self.assertEqual(evaluate(files)["checks"]["C3"], "FAIL")

    def test_conductor_scan_counts_its_own_secrets(self):  # section 7: the approval key in the run folder is a hit
        run = mock.Mock(dir=self.enterContext(tempfile.TemporaryDirectory()))
        with open(os.path.join(run.dir, "calls.log"), "w") as f:
            f.write("approve " + SENTINEL + "ap\n")
        with mock.patch.dict(os.environ, {"GOALCUP_TEST_APPROVAL_KEY": SENTINEL + "ap"}):
            pad.Run.scan_out(run)
        self.assertEqual(check.Run(run.dir, {}).hits("secrets-conductor"), 1)


class OutScan(unittest.TestCase):
    """The secrets-out scan leaves the pad's pane copies out and keeps goalcup's own folders in."""

    def test_pane_copies_are_not_scanned_but_the_record_is(self):
        with tempfile.TemporaryDirectory() as d:
            for f in ("watch/pane-w1.txt", "watch/pane-3.txt", "watch/faults.jsonl", "verify/pane-x.json", "status.json"):
                os.makedirs(os.path.dirname(os.path.join(d, f)), exist_ok=True)
                open(os.path.join(d, f), "w").close()
            run = mock.Mock(run=d)
            run.path = lambda *p: os.path.join(d, *p)
            got = padrun.Run.out_paths(run)
        self.assertEqual(sorted(got), ["status.json", "verify/pane-x.json", "watch/faults.jsonl"])


class PreRows(unittest.TestCase):
    """Section 3.1, proxy-setting.sh, a row the record decides: it passes when start refused and named the file and key,
    with the output and conductor scans read; no scan is never a pass (point 4; PRD-11 acceptance 4)."""

    SCANNED = {"verify/exits.json": {"secrets-out": 0}, "verify/secrets-conductor.json": {"hits": 0}}

    def row(self, script, lines, scanned=True, **files):
        pre = {"kind": "pre", "script": script, "exit": 0}
        meta = {"connector": "qwen-code", "case": "direct", "variant": ["pre " + script], "server": "off"}
        return evaluate({"meta.json": meta, "watch/faults.jsonl": [pre] + lines, **(self.SCANNED if scanned else {}), **files})

    def test_start_refusal_names_file_and_key(self):
        said = {"kind": "start-refused", "exit": 1, "named_file": True, "named_key": True, "value_shown": False}
        self.assertEqual(self.row("proxy-setting.sh", [said])["result"], "PASS")  # no run began: no run certificate key to miss
        for bad in (dict(said, named_key=False), dict(said, exit=0), dict(said, value_shown=True)):
            self.assertEqual(self.row("proxy-setting.sh", [bad])["result"], "FAIL", bad)
        self.assertEqual(self.row("proxy-setting.sh", [])["result"], "INCONCLUSIVE")  # start never ended

    def test_start_refusal_with_no_scan_is_not_a_pass(self):  # point 4: no scan of the box's output is no evidence
        said = {"kind": "start-refused", "exit": 1, "named_file": True, "named_key": True, "value_shown": False}
        got = self.row("proxy-setting.sh", [said], scanned=False)
        self.assertEqual((got["result"], got["checks"]["C3"]), ("INCONCLUSIVE", "INCONCLUSIVE"))
        hit = self.row("proxy-setting.sh", [said], **{"verify/exits.json": {"secrets-out": 2}})
        self.assertEqual((hit["result"], hit["checks"]["C3"]), ("FAIL", "FAIL"))


class StartRefusal(unittest.TestCase):
    """pad-run: the pane's words about a refused start become booleans."""

    def run_for(self, **harness):
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"harness": harness}), ("state.json", {"home": run})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        return run, padrun.Run(run, "verify")

    def test_refusal_records_what_the_screen_showed(self):
        run, r = self.run_for()
        seen = []
        def fake(argv, **kw):
            out = "1 1" if argv[1] == "display-message" else "settings.json has a proxy key: http://" + padrun.PROXY_VALUE
            return subprocess.CompletedProcess(argv, 0, out, "")
        with mock.patch.object(padrun, "sh", fake), mock.patch.object(r, "fault", lambda kind, **kw: seen.append((kind, kw))), \
                mock.patch.object(r, "pane", lambda keep=True, tag="": fake(["tmux", "capture-pane"]).stdout):
            r.refusal()
        self.assertEqual({k: seen[0][1][k] for k in ("named_file", "named_key", "value_shown")},
                         {"named_file": True, "named_key": True, "value_shown": True})

    def test_home_is_not_beside_project(self):  # PRD-10 section 3 step 2
        run, r = self.run_for()
        self.assertNotEqual(os.path.dirname(os.path.realpath(r.project)), os.path.dirname(os.path.realpath(r.env["GOALCUP_HOME"])))

    def test_own_goalcup_calls_never_default_to_production(self):  # ledger section 7: unset is production
        for settings, want in (({"server": "off"}, "off"), ({"server": "on", "server_url": "http://srv.test"}, "http://srv.test"),
                               ({"server": "on"}, "off")):
            run = self.enterContext(tempfile.TemporaryDirectory())
            for name, doc in (("settings.json", settings), ("state.json", {})):
                with open(os.path.join(run, name), "w") as f:
                    json.dump(doc, f)
            r = padrun.Run(run, "verify")
            self.assertEqual(r.env["GOALCUP_SERVER"], want)
            self.assertEqual(r.gc_env()["GOALCUP_SERVER"], want)

    def test_slow_seal_ends_the_step_not_the_rig(self):  # section 1a: no step stops the rig
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"minutes": 1, "harness": {}, "steps": []}), ("state.json", {"pasted": 0})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        r = padrun.Run(run, "verify")
        def fake(argv, **kw):
            raise subprocess.TimeoutExpired(argv, 1)
        with mock.patch.object(padrun, "sh", fake), mock.patch.object(padrun, "tool", lambda n: "/bin/true"), \
                mock.patch.object(padrun, "wait", lambda *a: False), mock.patch.object(r, "dead", lambda: False), \
                mock.patch.object(r, "pane", lambda *a, **k: ""), mock.patch.object(r, "status", lambda: {}), \
                mock.patch.object(r, "harness_files", lambda: None), mock.patch.object(r, "wipe_secrets", lambda **k: None):
            self.assertEqual(r.seal(None)[0], "timeout")


    def test_end_screen_count_ignores_the_no_request_warnings(self):  # a warning left on the pane is not the end screen
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"minutes": 1, "harness": {}, "steps": []}), ("state.json", {"pasted": 0})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        r = padrun.Run(run, "verify")
        for screen, want in (("No model request was written yet.", None), ("No model request was written for 10 minutes, x", None),
                             ("No model request was written. h may not use the proxy", 0), ("3 requests to model hosts written", 3)):
            with mock.patch.object(padrun, "wait", lambda *a: True), mock.patch.object(padrun, "tool", lambda n: "/bin/true"), \
                    mock.patch.object(r, "sealed", lambda: True), \
                    mock.patch.object(r, "pane", lambda *a, **k: screen), mock.patch.object(r, "status", lambda: {}), \
                    mock.patch.object(r, "harness_files", lambda: None), mock.patch.object(r, "wipe_secrets", lambda **k: None):
                r.seal(None)
            self.assertEqual(r.st["end_screen_requests"], want, screen)


class PasteAndMask(unittest.TestCase):
    """RIG-8: paste waits for status.json to say recording. RIG-9: fetch credentials are never shown or written."""

    ENV = {"DATABASE_URL": "postgres://padtester:Zq7fakePw@db.fake-host-123.example.com:6543/postgres",
           "CLOUDFLARE_ACCOUNT_ID": "acct0fakeid42", "CLOUDFLARE_API_TOKEN": "tok-madeup-not-real-77"}
    VALUES = ("padtester", "Zq7fakePw", "db.fake-host-123.example.com", "acct0fakeid42", "tok-madeup-not-real-77")

    def failing(self):
        err = "psql: could not translate host name; " + " ".join(self.VALUES) + " refused"
        return subprocess.CompletedProcess(["x"], 1, "", "noise\n" + err)

    def test_said_hides_every_fetch_credential(self):
        with mock.patch.dict(os.environ, self.ENV):
            got = pad.said("server copy not fetched", self.failing())
        for v in self.VALUES:
            self.assertNotIn(v, got)
        self.assertIn("refused", got)

    def test_failed_fetch_writes_no_credential(self):
        with tempfile.TemporaryDirectory() as out, mock.patch.dict(os.environ, self.ENV), \
                mock.patch.object(pad, "call", lambda argv, timeout=None, env=None: self.failing()):
            s = pad.load_settings(dict(BASE, mode="micro", server="staging"), PADCFG)
            r = pad.Run(s, "direct", PADCFG, Walk.HARNESS, out)
            self.addCleanup(shutil.rmtree, r.tmp, True)
            r.meta["entry"] = "e" + "0" * 16
            result, text = r.step_fetch("fetch", 10)
            r.log("fetch-run host " + self.VALUES[2])
            logged = open(os.path.join(r.dir, "calls.log")).read()
        self.assertEqual(result, "FAIL")
        for v in self.VALUES:
            self.assertNotIn(v, text + logged)

    def paste_run(self, mode, dead):
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"harness": {}}), ("state.json", {"home": run})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        r = padrun.Run(run, "verify")
        r.end = time.time() + 5
        typed = []
        status = {}
        for key in check.SEAM["s4_mode"][::-1]:
            status = {key: mode if status == {} else status}
        patches = [mock.patch.object(padrun, "tool", lambda n: "/bin/true"), mock.patch.object(padrun, "wait", lambda fn, *a: fn()),
                   mock.patch.object(r, "status", lambda: status), mock.patch.object(r, "pane_exit", lambda: (dead, "1")),
                   mock.patch.object(r, "capture", lambda *a: "a cleared screen"), mock.patch.object(r, "refusal", lambda: None),
                   mock.patch.object(r, "pane", lambda *a, **k: ""), mock.patch.object(r, "daemon_check", lambda **k: None),
                   mock.patch.object(r, "type_prompt", lambda: typed.append(1) or ("ok", "typed"))]
        for p in patches:
            self.enterContext(p)
        return r, typed

    def test_paste_goes_ahead_when_status_says_recording(self):
        r, typed = self.paste_run("recording", False)
        self.assertEqual(r.paste(None)[0], "ok")
        self.assertEqual(typed, [1])

    def test_paste_on_a_dead_pane_without_recording_is_refused_at_once(self):
        r, typed = self.paste_run(None, True)
        with self.assertRaises(padrun.Step) as got:
            r.paste(None)
        self.assertEqual((got.exception.args[0], typed), ("INCONCLUSIVE", []))


class QuietKillRecovery(unittest.TestCase):
    """A quiet kill is judged on pid, mode and the closed gap; a missing new request is never a FAIL."""

    def run_recovery(self, quiet):
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"harness": {}}), ("state.json", {"home": run})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        r = padrun.Run(run, "verify")
        r.end = time.time() + 5
        r.st["kill"] = {"pid": 1, "at": time.time(), "requests": 3, "off_spans": 0, "off_ms": 0, "quiet": quiet}
        status = {}
        for p in (mock.patch.object(padrun, "wait", lambda fn, *a: fn()), mock.patch.object(r, "status", lambda: status),
                  mock.patch.object(r, "work", lambda *a: 3),
                  mock.patch.object(padrun.pc, "dig", lambda st, k: {"s4_pid": 2, "s4_mode": "recording", "s4_spans": 1,
                                                                      "s4_since": None, "s4_down_ms": 100}.get(k))):
            self.enterContext(p)
        return r

    def test_quiet_kill_without_new_work_is_inconclusive(self):
        with self.assertRaises(padrun.Step) as got:
            self.run_recovery(True).wait_recovery(None)
        self.assertEqual(got.exception.args[0], "INCONCLUSIVE")

    def test_busy_kill_without_new_work_fails(self):
        with self.assertRaises(padrun.Step) as got:
            self.run_recovery(False).wait_recovery(None)
        self.assertEqual(got.exception.args[0], "FAIL")


class KillBothVerify(unittest.TestCase):
    """pad-run verify after kill-both: the row is the old entry's, so its server copy is the one compared and scanned."""

    def test_verify_judges_the_old_entry_and_its_server_copy(self):
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"server": "staging", "harness": {}, "steps": ["kill-both@2"]}),
                          ("state.json", {"home": run, "old": {"entry": "OLD", "run": "r1"}, "entry": "NEW", "run": "r1"})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        for n, age in (("OLD", 100), ("NEW", 0)):  # the newest entry is the one goalcup continue started
            for base in (os.path.join(run, "gc", "home", "runs", "r1", n), os.path.join(run, "fetched", n)):
                os.makedirs(base)
            with open(os.path.join(run, "gc", "home", "runs", "r1", n, "status.json"), "w") as f:
                f.write("{}")
            with open(os.path.join(run, "fetched", n, "rec.gcr.zst"), "w") as f:
                f.write("x")
            os.utime(os.path.join(run, "gc", "home", "runs", "r1", n, "status.json"), (1e9 - age, 1e9 - age))
        seen = []
        def fake(argv, **kw):
            seen.append(argv[4:])
            return subprocess.CompletedProcess(argv, 0, "{}", "")
        r = padrun.Run(run, "verify")
        with mock.patch.object(padrun, "sh", fake), mock.patch.object(padrun, "tool", lambda name: "/box/" + name), \
                mock.patch.object(r, "wipe_secrets"):
            r.verify(None)
        first = lambda word: next(a for a in seen if a[0] == word)
        check = next(a for a in seen if a[0] == "check" and "--server" in a)
        server = check[check.index("--server") + 1:]
        self.assertEqual((server[0], os.path.basename(server[2])), (os.path.join("fetched", "OLD"), "OLD"))
        self.assertIn(os.path.join("fetched", "OLD"), first("secrets"))
        self.assertEqual(first("usage")[1], os.path.join("fetched", "OLD"))
        with open(os.path.join(run, "meta-box.json")) as f:
            self.assertEqual(json.load(f)["entry"], "OLD")


class RigFaults(unittest.TestCase):
    """Rung 2 rig faults: a named user agent and a plain refusal text (RIG-5), a dead pane ends the wait (RIG-6),
    DATABASE_URL only for fetch-run (RIG-7)."""

    def test_approve_names_itself_and_hides_the_secrets(self):
        import http.server
        import threading
        seen = {}

        class H(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                seen["ua"] = self.headers.get("user-agent")
                self.rfile.read(int(self.headers.get("content-length") or 0))
                self.send_response(403)
                self.end_headers()
                self.wfile.write(b"error code: 1010\n")

            def log_message(self, *a):
                pass

        srv = http.server.HTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        env = {"GOALCUP_TEST_APPROVAL_KEY": "fake-key-not-a-secret", "GOALCUP_TEST_MEMBER_ID": "fake-member-not-real"}
        with mock.patch.dict(os.environ, env), mock.patch.dict(pad.SERVERS, {"t": "http://127.0.0.1:%d" % srv.server_port}):
            got = pad.approve("t", "ABCD-EFGH")
        self.assertTrue(seen["ua"].startswith("goalcup-pad"))
        self.assertIn("403", got)
        self.assertNotIn("fake-key-not-a-secret", got)
        self.assertNotIn("fake-member-not-real", got)

    def test_database_url_reaches_only_fetch_run(self):
        with mock.patch.dict(os.environ, {"DATABASE_URL": "postgres://fake-not-a-secret"}):
            self.assertIn("DATABASE_URL", pad.env_without(set(pad.NEVER_IN_BOX) - set(pad.FETCH)))
            self.assertNotIn("DATABASE_URL", pad.env_without())

    def test_dead_pane_ends_the_wait_at_once(self):
        run = self.enterContext(tempfile.TemporaryDirectory())
        for name, doc in (("settings.json", {"harness": {}}), ("state.json", {"home": run})):
            with open(os.path.join(run, name), "w") as f:
                json.dump(doc, f)
        r = padrun.Run(run, "verify")
        r.end = time.time() + 120

        def stub(screen):
            def fake(argv, **kw):
                return subprocess.CompletedProcess(argv, 0, "1 1" if argv[1] == "display-message" else screen, "")
            return fake
        with mock.patch.object(padrun, "sh", stub("starting\nsettings.json refused\nPane is dead (status 1, Sun Nov  1 10:00:00 2026)\n")), mock.patch.object(r, "fault", lambda *a, **k: None):
            t0 = time.time()
            with self.assertRaises(padrun.Step) as cm:
                r.until("Recording in chat mode", "none")
        self.assertLess(time.time() - t0, 5)
        self.assertTrue(cm.exception.args[1].startswith("start-refused"))
        self.assertIn("settings.json refused", cm.exception.args[1])
        self.assertNotIn("Pane is dead", cm.exception.args[1])
        with mock.patch.object(padrun, "sh", stub("x\n")):  # server off: the status wait ends on the dead pane too
            self.assertTrue(r.recording_or_dead())
        faults = []  # server off: a refused start is raised and written, not typed into a dead pane
        r.s = {"server": "off", "harness_id": "x"}
        typed = []
        with mock.patch.object(padrun, "sh", stub("starting\nsettings.json refused\nPane is dead (status 1, Sun Nov  1 10:00:00 2026)\n")), \
                mock.patch.object(padrun, "tool", lambda n: n), mock.patch.object(r, "need", lambda *a, **k: None), \
                mock.patch.object(r, "gc_env", lambda *a, **k: {}), mock.patch.object(r, "daemon_check", lambda: typed.append("d")), \
                mock.patch.object(r, "type_prompt", lambda: typed.append("t")), mock.patch.object(r, "fault", lambda *a, **k: faults.append(a)):
            t0 = time.time()
            with self.assertRaises(padrun.Step) as cm:
                r.start(None)
        self.assertLess(time.time() - t0, 5)
        self.assertTrue(cm.exception.args[1].startswith("start-refused"))
        self.assertEqual(typed, [])
        self.assertTrue(any(a and a[0] == "start-refused" for a in faults))
        with mock.patch.object(padrun, "sh", stub("Recording in chat mode\n")):
            self.assertTrue(r.until("Recording in chat mode", "none"))


class PinnedModel(unittest.TestCase):
    """The cheapest-model rule: the harness toml pins the main model, the row meta carries it, C5 warns of a different one."""

    @staticmethod
    def row(pinned, recorded, case="direct", read=True):
        use = {"tokens": {"merged": {"input": 9, "output": 9}}, "coverage": {"unattributed": 0},
               "threads": [{"parent": None, "models": {"m": 3}, "wire_calls": 3}, {"parent": "t0", "models": {"m": 2}, "wire_calls": 2}]}
        if read:
            use["models"] = [{"model": m, "tokens": {"input": 1}} for m in recorded]
        return evaluate({"meta.json": {"connector": "claude-code-cli", "case": case, "model_host": "api.example", "server": "off",
                                       "keep": True, "entry": "e1", "commits_since_start": 1, "pinned_model": pinned, "started": "2026-10-30T10:00:00+00:00"},
                         "verify/check-local.json": {"exit": 1, "entries": [{"entry": "e1", "counts": {"model_requests": 5}, "checks": []}]},
                         "verify/usage/e1.json": use})

    def test_c5_follows_the_pin(self):
        bad = self.row("claude-haiku-5-5", ["claude-opus-5-5"])
        self.assertEqual(bad["checks"]["C5"], "PASS")  # a warning line, never a verdict by itself
        self.assertIn("recorded model claude-opus-5-5, pinned claude-haiku-5-5", " ".join(bad["why"]))
        self.assertEqual(bad["facts"]["pinned_model"], "claude-haiku-5-5")
        self.assertEqual(self.row("claude-haiku-5-5", ["claude-haiku-5-5"])["checks"]["C5"], "PASS")
        self.assertEqual(self.row("anthropic/claude-haiku-5.5", ["claude-haiku-5.5"], "gateway")["checks"]["C5"], "PASS")
        self.assertEqual(self.row(" claude-haiku-5-5 ", ["anthropic/claude-haiku-5-5"])["checks"]["C5"], "PASS")
        self.assertEqual(self.row("claude-haiku-5-5", ["claude-haiku-5-5", "claude-opus-5-5"])["checks"]["C5"], "PASS")

    def test_no_models_read_or_no_pin_says_nothing(self):
        for pin, recorded, read in (("claude-haiku-5-5", [], True), ("claude-haiku-5-5", [], False), (None, ["claude-opus-5-5"], True)):
            self.assertNotIn("pinned", " ".join(self.row(pin, recorded, read=read)["why"]), (pin, recorded, read))


class MiniFixRound(unittest.TestCase):
    """Mini-wave fixes: goose plain count, task-log subagents, OpenClaw redaction data, setup script without provider keys."""

    def test_shape_only_plain_hit_does_not_undo_the_stated_loss(self):  # T3
        shape = {"kind": "generic_secret_assignment", "in": "e1/1.3/deltas", "sha12": "a" * 12, "n": 1}
        row = evaluate({"meta.json": {"connector": "goose-cli", "case": "direct", "known_box": True, "entry": "e1",
                                      "mask_values": {"workspace": 2, "env": 0, "harness": 0, "skipped": 0}},  # exact values on: the old plain count of a shape hit failed here
                        "verify/secrets.json": {"known_hits": [], "shape_hits": [shape], "big_masks": [], "masks": dict(SecretCheck.PLANTED)},
                        "verify/secrets-plain.json": {"known_hits": [], "shape_hits": [shape], "big_masks": []},
                        "verify/secrets-conductor.json": {"hits": 0},
                        "verify/exits.json": {"secrets": 2, "secrets-out": 0, "secrets-plain": 2}})
        self.assertEqual(row["checks"]["C3"], "PASS", row["why"])

    def usage_row(self, flag, harness="hermes-cli"):
        use = {"tokens": {"merged": {"input": 9, "output": 9}}, "coverage": {"unattributed": 0},
               "threads": [{"parent": None, "models": {"m": 3}, "wire_calls": 3}, {"parent": "t0", "models": {}, "wire_calls": 0}]}
        meta = {"connector": harness, "case": "direct", "known_box": True, "entry": "e1", "commits_since_start": 1}
        if flag:
            meta["subagents_by_task_log"] = True
        return evaluate({"meta.json": meta, "verify/usage/e1.json": use,
                         "verify/check-local.json": {"exit": 1, "entries": [{"entry": "e1", "counts": {"model_requests": 5}, "checks": []}]}})

    def test_task_log_subagents_need_no_calls(self):  # T4
        row = self.usage_row(True)
        self.assertNotEqual(row["checks"]["C2"], "FAIL", row["why"])
        self.assertNotEqual(row["checks"]["C5"], "FAIL", row["why"])
        self.assertTrue(any("subagents shown by their task logs, not tied to calls" in w for w in row["why"]), row["why"])
        other = self.usage_row(False, "gemini-cli")  # the rule stays for every other harness
        self.assertEqual(other["checks"]["C2"], "FAIL")
        self.assertEqual(other["checks"]["C5"], "FAIL")

    def test_pad_copies_the_task_log_flag_and_openclaw_claims_no_redaction(self):  # T4, T1
        import tomllib
        here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "harnesses")
        with open(os.path.join(here, "hermes-cli.toml"), "rb") as f:
            self.assertIs(tomllib.load(f).get("subagents_by_task_log"), True)
        with open(os.path.join(here, "openclaw-cli.toml"), "rb") as f:
            oc = tomllib.load(f)
        self.assertFalse(oc.get("redacts_kinds"))  # no kind passes C3 on an inference

    def test_setup_script_never_prints_the_environment(self):  # T7
        here = os.path.dirname(os.path.abspath(__file__))
        with open(os.path.join(here, "prompts", "pad-setup-check.sh")) as f:
            code = [ln for ln in f.read().splitlines() if not ln.lstrip().startswith("#")]
        text = "\n".join(code)
        for bad in ("env", "printenv", "set", "export", "declare", "API_KEY", "/proc/"):
            self.assertNotRegex(text, r"(^|[\s|;&(])%s($|[\s|;&)=])" % bad if bad.isalpha() else bad, bad)
        self.assertIn(".env.local", text)


class PromptVariant(unittest.TestCase):
    """rung1x item 10, sweep2: opencode refused, qwen asked, copilot ran half the script. Same task, plants named as fake."""

    def test_fixture_prompt_keeps_the_smoke_task(self):
        path = os.path.join(HERE, "prompts", "smoke-fixture.txt")
        text = open(path).read()
        smoke = open(os.path.join(HERE, "prompts", "smoke.txt")).read()
        for part in ("sh ./pad-setup-check.sh", "one subagent", "count.txt", "commit count.txt", "fake", "Do not stop to ask"):
            self.assertIn(part, text)
        self.assertIn(" ".join(smoke.split()), " ".join(text.split()))  # the smoke task word for word
        self.assertEqual(pad.load_settings(dict(BASE, prompt=path), PADCFG)["prompt"], path)


class KeyFileMerge(unittest.TestCase):
    def kimi_line(self):
        return pad.read_toml(os.path.join(HERE, "harnesses", "kimi-code-cli.toml"))["key_line"].replace("{key}", "PADSENTINEL")

    def test_key_text_keeps_the_row_setup_wrote(self):
        out = padrun.key_text("/iso/config.toml", 'default_permission_mode = "auto"\n', self.kimi_line())
        self.assertIn('default_permission_mode = "auto"', out)
        self.assertIn('api_key = "PADSENTINEL"', out)
        self.assertLess(out.index("default_permission_mode"), out.index("[providers.deepseek]"))

    def test_key_text_same_key_once_pad_value_wins(self):
        out = padrun.key_text("/iso/config.toml", 'default_model = "other"\n', self.kimi_line())
        self.assertEqual(out.count("default_model"), 1)
        self.assertIn('default_model = "ds"', out)

    def test_key_text_with_no_old_file_is_the_line(self):
        self.assertEqual(padrun.key_text("/iso/.env", "", "AI_GATEWAY_API_KEY=PADSENTINEL"), "AI_GATEWAY_API_KEY=PADSENTINEL\n")
        self.assertEqual(json.loads(padrun.key_text("/h/auth.json", "", '{"deepseek": {"type": "api", "key": "K"}}')),
                         {"deepseek": {"type": "api", "key": "K"}})


class SnippetFill(unittest.TestCase):
    def test_fill_leaves_env_references_alone(self):
        t = '{"w": "{workspace}", "h": "{home}/x", "k": "${AI_GATEWAY_API_KEY}", "e": "{env:DEEPSEEK_API_KEY}"}'
        self.assertEqual(json.loads(padrun.fill(t, "/iso", "/run/project")),
                         {"w": "/run/project", "h": "/iso/x", "k": "${AI_GATEWAY_API_KEY}", "e": "{env:DEEPSEEK_API_KEY}"})

    def test_openclaw_works_in_the_project(self):
        for case in ("direct", "gateway"):
            with open(os.path.join(HERE, "harnesses", case, "openclaw-cli-config.json")) as f:
                d = json.load(f)
            self.assertEqual(d["agents"]["defaults"]["workspace"], "{workspace}", case)
            self.assertIn("model", d["agents"]["defaults"], case)  # the model pin is kept beside it


class ExitKeys(unittest.TestCase):
    def sent(self, keys, dies_after=None):
        r = object.__new__(padrun.Run)
        r.h, r.tmux, r.fault, r.status = {"exit_keys": keys}, "t", lambda *a, **k: None, lambda: {}
        sent, sleeps = [], []
        r.dead = lambda: dies_after is not None and len(sent) >= dies_after
        def sh(cmd, *a, **k):
            if "send-keys" in cmd:
                sent.append(cmd[-1])
            return subprocess.CompletedProcess(cmd, 0, "", "")
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun.time, "sleep", sleeps.append):
            r.exit_harness()
        return sent, sleeps

    def test_list_items_go_one_by_one_with_a_pause(self):
        sent, sleeps = self.sent(["/quit", "\\r"])
        self.assertEqual(sent, ["/quit", "Enter"])
        self.assertIn(1.0, sleeps)

    def test_string_form_is_unchanged(self):
        self.assertEqual(self.sent("/exit\\r")[0], ["/exit", "Enter"])
        self.assertEqual(self.sent("\\003\\003")[0], ["\x03\x03"])

    def test_harness_gone_mid_list_gets_no_more_keys(self):
        self.assertEqual(self.sent(["/quit", "\\r"], dies_after=1)[0], ["/quit"])

    def test_codex_data(self):
        h = pad.read_toml(os.path.join(HERE, "harnesses", "codex-cli.toml"))
        self.assertEqual(h["exit_keys"], ["/quit", "\\r"])
        cfg = pad.read_toml(os.path.join(HERE, "harnesses", "gateway", "codex-config.toml"))
        self.assertEqual(h["gateway"]["model"], cfg["model"])  # owner rule 19:17Z, conductor note 22:00Z, gw3 PASS 20:27Z


class SelftestOfflineC3(unittest.TestCase):
    """The selftest must prove a known-value hit maps to C3 FAIL and a deltas-only one to the stated-loss PASS, offline."""

    def test_selftest_has_the_two_offline_c3_controls_and_they_read_as_wanted(self):
        st = load("padselftest", "selftest")
        labels = [c[0] for c in st.OFFLINE]
        self.assertEqual(labels, ["C3-body", "C3-deltas"])
        got = st.run_offline_controls()
        self.assertEqual(got["C3-body"]["state"], "PASS", got["C3-body"])  # control passes = C3 read FAIL as wanted
        self.assertEqual(got["C3-deltas"]["state"], "PASS", got["C3-deltas"])
        self.assertIn("C3 FAIL", got["C3-body"]["saw"])
        self.assertIn("C3 PASS", got["C3-deltas"]["saw"])
        self.assertIn("split in streamed reply", got["C3-deltas"]["saw"])

    def test_an_offline_control_that_reads_wrong_is_a_fail(self):
        st = load("padselftest2", "selftest")
        st.OFFLINE[:] = [("C3-body", st.OFFLINE[0][1], "PASS", None)]  # wanted PASS, but a request-body hit reads FAIL
        self.assertEqual(st.run_offline_controls()["C3-body"]["state"], "FAIL")


if __name__ == "__main__":
    unittest.main()
