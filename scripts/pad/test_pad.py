"""Tests of the test pad's own hands (PRD-12 "Automated tests", ledger B10): key files, the push
token, no key value in any argument the pad builds, the fixed servers and cleanup. Python 3.11 or
later, no network, no harness.

    python3.11 -m unittest scripts/pad/test_pad.py
"""
import importlib.machinery
import importlib.util
import os
import subprocess
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
_loader = importlib.machinery.SourceFileLoader("pad", os.path.join(HERE, "pad"))
pad = importlib.util.module_from_spec(importlib.util.spec_from_loader("pad", _loader))
_loader.exec_module(pad)

SENTINEL = "sk-ant-PADSENTINEL-9f3c1e"
PADCFG = {"keys": {"claude": "CLAUDE_API_KEY", "gateway": "AI_GATEWAY_API_KEY", "cf": "CLOUDFLARE_API_TOKEN"}}
BASE = {"harness": "claude-code-cli", "key": "claude", "mode": "cycle"}


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
    def test_no_server_address_setting(self):
        for server in ("https://ingest.agentcommunity.org", "https://goalcup-ingest-staging.b-9a5.workers.dev"):
            with self.assertRaises(pad.Usage):
                pad.load_settings(dict(BASE, server=server), PADCFG)
        s = pad.load_settings(BASE, dict(PADCFG, staging="https://ingest.agentcommunity.org"))
        self.assertEqual(s["server_url"], "https://goalcup-ingest-staging.b-9a5.workers.dev")

    def test_only_model_keys_enter_a_box(self):
        with self.assertRaises(pad.Usage):
            pad.load_settings(dict(BASE, key="cf"), PADCFG)


class Walk(unittest.TestCase):
    """A sentinel key in the environment: no argument the pad builds, and no file it writes, holds it."""

    HARNESS = {"version": "1.2.3", "version_cmd": "x --version", "keep_going": True}

    def fake_call(self, argv, timeout=None, env=None):
        self.calls.append(list(argv))
        if argv[2] in self.hang:
            raise subprocess.TimeoutExpired(argv, timeout)
        out = {"create": '{"id":"x"}', "exec": "PADVER=1.2.3\n"}.get(argv[2], "")
        return subprocess.CompletedProcess(argv, 0, out, "")

    def walk(self, hang=(), **settings):
        self.calls, self.hang = [], hang
        env = {"CLAUDE_API_KEY": SENTINEL, "AI_GATEWAY_API_KEY": SENTINEL + "gw"}
        with tempfile.TemporaryDirectory() as out, mock.patch.dict(os.environ, env), \
                mock.patch.object(pad, "call", self.fake_call):
            (_, table), = pad.run_all(pad.load_settings(dict(BASE, **settings), PADCFG), PADCFG, self.HARNESS, out)
            for root, _, files in os.walk(out):
                for name in files:
                    with open(os.path.join(root, name), "rb") as f:
                        self.assertNotIn(b"PADSENTINEL", f.read(), name)
        self.assertFalse([a for c in self.calls for a in c if "PADSENTINEL" in a])
        return [c[2:] for c in self.calls], table

    def test_sandbox_passes_names_only(self):
        calls, table = self.walk(case="gateway")
        self.assertEqual(sorted(c[c.index("--env") + 1] for c in calls if c[0] == "put-env"),
                         ["AI_GATEWAY_API_KEY", "CLAUDE_API_KEY"])
        self.assertIn("kill-recorder@3", table.splitlines()[0])  # a cycle row is a fault row
        self.assertFalse([c for c in self.walk(case="gateway", key="none")[0] if c[0] == "put-env"])

    def test_create_timeout_still_removes_the_box(self):
        calls, table = self.walk(hang=("create",))
        name = calls[0][calls[0].index("--name") + 1]
        self.assertIn(["rm", name], calls)
        self.assertIn("list shows no live box", table)


if __name__ == "__main__":
    unittest.main()
