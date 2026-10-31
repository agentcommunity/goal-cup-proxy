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
        with tempfile.TemporaryDirectory() as out, mock.patch.object(pad, "call", fake):
            r = self.run_for(out, server="staging")
            r.meta["entry"] = "e" + "0" * 16
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

    def test_env_key_launch_holds_no_value(self):
        run = self.enterContext(tempfile.TemporaryDirectory())
        os.makedirs(os.path.join(run, "keys"))
        key = key_file(os.path.join(run, "keys"))
        for name, doc in (("settings.json", {"server": "off", "harness_id": "claude-code-cli", "keep": True, "harness": {}}),
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
            self.assertNotIn("PADSENTINEL", f.read())
        self.assertFalse([a for argv in argvs for a in argv if "PADSENTINEL" in a])

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

    def row(self, kind, gap, samples=UP, recovery=(("ok", "10:03"),), seal="harness_exit", over_cap=False,
            words=check.RECOVER, more=(), pid=9, code=1):
        told = [{"kind": k, "exit": 0, "said": w, "at": "2026-10-30T10:%s+00:00" % t}
                for k, w, t in zip(("recover", "recover", "status"), words + (check.STATUS_RECOVER,), ("02:30", "04:00", "03:30"))]
        kills = [{"kind": k, "pid": pid, "at": "2026-10-30T10:02:00+00:00"} for k in ("kill-watchdog", "kill-recorder")]
        rec = [{"kind": "wait-recovery", "result": x, "at": "2026-10-30T%s:00+00:00" % t} for x, t in recovery]
        files = {"meta.json": {"connector": "codex-cli", "case": "direct", "variant": [kind + "@3"], "server": "off",
                               "keep": True, "commits_since_start": 1, "entry": "e1", "over_cap": over_cap,
                               "started": "2026-10-30T10:00:00+00:00"},
                 "verify/check-local.json": {"exit": code, "entries": [{"entry": "e1", "seal_reason": seal,
                     "counts": {"model_requests": 5, "gaps": {"recorder_down": 1}}, "checks": [
                         {"id": "gap_has_end", "result": "pass"}, {"id": "history_complete", "result": "pass"},
                         {"id": "inv6", "result": "named", "facts": {"gaps": [gap] if gap else []}}]}]},
                 "verify/usage/e1.json": {"tokens": {"merged": {"input": 9, "output": 9}}, "coverage": {"unattributed": 0},
                                          "threads": [{"parent": None, "models": {"m": 3}, "wire_calls": 3},
                                                      {"parent": "t0", "models": {"m": 2}, "wire_calls": 2}]},
                 "watch/status.jsonl": [{"at": "2026-10-30T10:0%d:00+00:00" % (4 * i), "status": x} for i, x in enumerate(samples)],
                 "watch/faults.jsonl": told + kills + rec + list(more)}
        return evaluate(files)

    def test_kill_recorder(self):
        gap = {"start": "2026-10-30T10:00:00Z", "end": "2026-10-30T10:00:01.2Z"}
        good = self.row("kill-recorder", gap)
        self.assertEqual((good["result"] != "FAIL", good["faults"]), (True, "PASS"))
        down = (("FAIL", "10:04"),)  # did not stand back up
        for bad in ({"gap": dict(gap, cause="null_mode")}, {"gap": dict(gap, end="2026-10-30T10:00:02.5Z")}, {"recovery": down},
                    {"samples": self.UP[:1] * 2}):  # no model request written after the kill
            self.assertEqual(self.row("kill-recorder", **dict({"gap": gap}, **bad))["result"], "FAIL", bad)

    def test_recover_keeps_harness_pid_and_port(self):
        gap = {"cause": "watchdog_lost", "start": "2026-10-30T10:01:00Z", "end": "2026-10-30T10:02:00Z"}
        for pid, port, fails in ((7, 5, False), (8, 5, True), (7, 6, True)):
            samples = [{"port": 5, "harness": {"pid": 7}}, {"port": port, "harness": {"pid": pid}}]
            self.assertEqual(self.row("kill-watchdog-recover", gap, samples)["result"] == "FAIL", fails, (pid, port))
        for over_cap, fails in ((True, False), (False, True)):  # section 7: a seal at the spend cap is not a failure
            self.assertEqual(self.row("kill-watchdog-recover", gap, seal="command", over_cap=over_cap)["result"] == "FAIL",
                             fails, over_cap)

    def test_seal_at_the_spend_cap_is_not_a_fault_failure(self):  # section 7, ledger T6: the pad's own seal
        gap = {"start": "2026-10-30T10:00:00Z", "end": "2026-10-30T10:00:01.2Z"}
        down = (("FAIL", "10:04"),)
        wgap = {"cause": "watchdog_lost", "start": "2026-10-30T10:01:00Z", "end": "2026-10-30T10:02:00Z"}
        for at, over_cap, code, fails in (("10:03", True, 0, False), ("10:03", False, 0, True), ("10:20", True, 0, True),
                                          ("10:03", True, 1, True)):  # a seal-now that did not seal is not the cap seal
            seal = [{"kind": "seal-now", "exit": code, "at": "2026-10-30T%s:00+00:00" % at}]
            self.assertEqual(self.row("kill-recorder", gap, recovery=down, over_cap=over_cap, more=seal)["result"] == "FAIL",
                             fails, (at, over_cap, code))
            self.assertEqual(self.row("kill-watchdog-recover", wgap, words=(check.RECOVER[0], "sealed"), over_cap=over_cap,
                                      more=seal)["result"] == "FAIL", fails, (at, over_cap, code))
        # Sealed at the cap before the kill: nothing left to kill, no gap, verifier exit 0. Not goalcup's failure.
        early = [{"kind": "seal-now", "exit": 0, "at": "2026-10-30T10:01:00+00:00"}]
        got = self.row("kill-recorder", None, pid=None, code=0, recovery=(("skip", "10:02"),), over_cap=True, more=early)
        self.assertEqual((got["result"] != "FAIL", got["faults"]), (True, "INCONCLUSIVE"))


class SecretCheck(unittest.TestCase):
    """C3 (point 4, section 5): a known-value hit fails, and so does a planted shape with no MASK. Only C3 is asserted."""

    PLANTED = {"openai_key": 1, "github_token": 1, "generic_secret_assignment": 1, "jwt": 1, "private_key": 1,
               "url_password": 1, "auth": 8}

    def c3(self, masks, secrets_exit=1, plain_exit=0):
        return evaluate({"meta.json": {"connector": "codex-cli", "case": "direct", "known_box": True, "entry": "e1"},
                         "verify/secrets.json": {"masks": masks}, "verify/secrets-conductor.json": {"hits": 0},
                         "verify/exits.json": {"secrets": secrets_exit, "secrets-out": 0, "secrets-plain": plain_exit}}
                        )["checks"]["C3"]

    def test_known_value_hit_fails(self):
        self.assertEqual(self.c3(self.PLANTED, secrets_exit=2), "FAIL")
        self.assertEqual(self.c3(dict(self.PLANTED, jwt=0)), "FAIL")

    def test_conductor_scan_counts_its_own_secrets(self):  # section 7: the approval key in the run folder is a hit
        run = mock.Mock(dir=self.enterContext(tempfile.TemporaryDirectory()))
        with open(os.path.join(run.dir, "calls.log"), "w") as f:
            f.write("approve " + SENTINEL + "ap\n")
        with mock.patch.dict(os.environ, {"GOALCUP_TEST_APPROVAL_KEY": SENTINEL + "ap"}):
            pad.Run.scan_out(run)
        self.assertEqual(check.Run(run.dir, {}).hits("secrets-conductor"), 1)


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


if __name__ == "__main__":
    unittest.main()
