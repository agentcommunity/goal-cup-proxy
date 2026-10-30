"""Tests of the test pad's own hands (PRD-12 "Automated tests", ledger B10): key files, the push token, no key
value in any argument or file the pad builds (an env key launch included), the pane replacement, cleanup and
check.py's crash rows. Python 3.11, no network.

    python3.11 -m unittest scripts/pad/test_pad.py
"""
import base64
import importlib.machinery
import importlib.util
import json
import os
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

    HARNESS = {"version": "1.2.3", "version_cmd": "x --version", "keep_going": True}

    def fake_call(self, argv, timeout=None, env=None):
        if argv[0] != "node":  # check.py runs for real on the run folder
            return self.real_call(argv, timeout, env)
        self.calls.append(list(argv))
        if argv[2] in self.hang:
            raise subprocess.TimeoutExpired(argv, timeout)
        if argv[2] == "get":  # the box half's tar: meta-box.json only, as a run with no build leaves it
            with tempfile.TemporaryDirectory() as d, tarfile.open(argv[-1], "w") as t:
                with open(os.path.join(d, "meta-box.json"), "w") as f:
                    f.write("{}")
                t.add(os.path.join(d, "meta-box.json"), "meta-box.json")
        out = 'PADSTEP {"result": "skip", "saw": "no goalcup build"}\n' if argv[2] == "exec" else ""
        return subprocess.CompletedProcess(argv, 0, out, "")

    def walk(self, hang=(), **settings):
        self.calls, self.hang, self.real_call = [], hang, pad.call
        env = {"CLAUDE_API_KEY": SENTINEL, "AI_GATEWAY_API_KEY": SENTINEL + "gw"}
        with tempfile.TemporaryDirectory() as out, mock.patch.dict(os.environ, env), \
                mock.patch.object(pad, "call", self.fake_call):
            pad.run_all(pad.load_settings(dict(BASE, **settings), PADCFG), PADCFG, self.HARNESS, out)
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


class CrashRows(unittest.TestCase):
    """Section 3.1, ledger R3a and R3b: one closed recorder_down gap per kill under 2 s, no null_mode gap; the
    recover row keeps the harness pid and port. Only result values are asserted."""

    def row(self, kind, gap, samples=()):
        recover = [{"kind": "recover", "exit": 0, "said": w} for w in check.RECOVER]
        files = {"meta.json": {"connector": "codex-cli", "case": "direct", "variant": [kind + "@3"], "server": "off",
                               "keep": True, "commits_since_start": 1, "entry": "e1"},
                 "verify/check-local.json": {"exit": 1, "entries": [{"entry": "e1", "seal_reason": "harness_exit",
                     "counts": {"model_requests": 5, "gaps": {"recorder_down": 1}}, "checks": [
                         {"id": "gap_has_end", "result": "pass"}, {"id": "history_complete", "result": "pass"},
                         {"id": "inv6", "result": "named", "facts": {"gaps": [gap]}}]}]},
                 "verify/usage/e1.json": {"tokens": {"merged": {"input": 9, "output": 9}}, "coverage": {"unattributed": 0},
                                          "threads": [{"parent": None, "models": {"m": 3}, "wire_calls": 3},
                                                      {"parent": "t0", "models": {"m": 2}, "wire_calls": 2}]},
                 "watch/status.jsonl": samples, "watch/faults.jsonl": recover}
        return evaluate(files)

    def test_kill_recorder(self):
        gap = {"start": "2026-10-30T10:00:00Z", "end": "2026-10-30T10:00:01.2Z"}
        self.assertNotEqual(self.row("kill-recorder", gap)["result"], "FAIL")
        for bad in (dict(gap, cause="null_mode"), dict(gap, end="2026-10-30T10:00:02.5Z")):
            self.assertEqual(self.row("kill-recorder", bad)["result"], "FAIL", bad)

    def test_recover_keeps_harness_pid_and_port(self):
        gap = {"cause": "watchdog_lost", "start": "2026-10-30T10:01:00Z", "end": "2026-10-30T10:02:00Z"}
        for pid, port, fails in ((7, 5, False), (8, 5, True), (7, 6, True)):
            samples = [{"port": 5, "harness": {"pid": 7}}, {"port": port, "harness": {"pid": pid}}]
            self.assertEqual(self.row("kill-watchdog-recover", gap, samples)["result"] == "FAIL", fails, (pid, port))


class SecretCheck(unittest.TestCase):
    """C3 (point 4, section 5): a known-value hit fails; the planted shapes need a MASK each, the shell rule
    standing for a NAME_API_KEY= line; the auth header's MASK alone is no secret exercise. Only C3 is asserted."""

    PLANTED = {"openai_key": 1, "github_token": 1, "generic_secret_assignment": 1, "jwt": 1, "private_key": 1,
               "url_password": 1, "auth": 8}

    def c3(self, masks, secrets_exit=1, plain_exit=0):
        return evaluate({"meta.json": {"connector": "codex-cli", "case": "direct", "known_box": True, "entry": "e1"},
                         "verify/secrets.json": {"masks": masks},
                         "verify/exits.json": {"secrets": secrets_exit, "secrets-out": 0, "secrets-plain": plain_exit}}
                        )["checks"]["C3"]

    def test_planted_values_masked_pass(self):  # exit 1 (unsealed, a named GAP) is never a hit
        self.assertEqual(self.c3(self.PLANTED), "PASS")

    def test_known_value_hit_fails(self):
        self.assertEqual(self.c3(self.PLANTED, secrets_exit=2), "FAIL")
        self.assertEqual(self.c3(dict(self.PLANTED, jwt=0)), "FAIL")

    def test_exercise_not_done_is_inconclusive(self):
        self.assertEqual(self.c3({"auth": 8}), "INCONCLUSIVE")
        self.assertEqual(self.c3({"auth": 8}, plain_exit=2), "FAIL")  # the script ran, yet no planted shape is masked


if __name__ == "__main__":
    unittest.main()
