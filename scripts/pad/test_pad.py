"""Tests of the test pad's own hands (PRD-12 "Automated tests", ledger B10): key files, the push
token, no key value in any argument the pad builds, and the server allowlist. Python 3.9, no
network, no harness.

    python3 -m unittest scripts/pad/test_pad.py
"""
import importlib.machinery
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
_loader = importlib.machinery.SourceFileLoader("pad", os.path.join(HERE, "pad"))
_spec = importlib.util.spec_from_loader("pad", _loader)
pad = importlib.util.module_from_spec(_spec)
_loader.exec_module(pad)

SENTINEL = "sk-ant-PADSENTINEL-9f3c1e"
PADCFG = {"staging": "https://goalcup-ingest-staging.b-9a5.workers.dev",
          "keys": {"claude": "CLAUDE_API_KEY", "gateway": "AI_GATEWAY_API_KEY"}}


def key_file(folder, mode=0o600, value=SENTINEL):
    path = os.path.join(folder, "claude")
    with open(path, "w") as f:
        f.write(value)
    os.chmod(path, mode)
    return path


class KeyFiles(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.out = self.tmp.name
        self.repo = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()
        self.repo.cleanup()

    def test_good_file_passes(self):
        pad.check_key_file(key_file(self.out), self.repo.name)

    def test_group_or_world_readable_refused(self):
        with self.assertRaises(pad.Refused):
            pad.check_key_file(key_file(self.out, 0o644), self.repo.name)

    def test_symlink_refused(self):
        link = os.path.join(self.out, "link")
        os.symlink(key_file(self.out), link)
        with self.assertRaises(pad.Refused):
            pad.check_key_file(link, self.repo.name)

    def test_inside_checkout_refused(self):
        with self.assertRaises(pad.Refused):
            pad.check_key_file(key_file(self.repo.name), self.repo.name)

    def test_gh_login_and_classic_push_tokens_refused(self):
        for value in ("gho_abc123", "ghp_abc123", "GH_TOKEN=ghp_abc123"):
            with self.assertRaises(pad.Refused):
                pad.check_push_token(key_file(self.out, value=value), self.repo.name)
        pad.check_push_token(key_file(self.out, value="github_pat_abc123"), self.repo.name)


class Servers(unittest.TestCase):
    def test_production_and_unknown_refused(self):
        for url in ("https://ingest.agentcommunity.org", "https://INGEST.agentcommunity.org/",
                    "https://example.com", "http://example.com", "https://staging.example.com",
                    "http://localhost.evil.example", "https://goalcup-ingest-staging.x.workers.dev.evil.example"):
            with self.assertRaises(pad.Refused, msg=url):
                pad.check_server(url)

    def test_staging_and_loopback_allowed(self):
        for url in (PADCFG["staging"], "http://127.0.0.1:8787", "http://localhost", "http://[::1]:9"):
            pad.check_server(url)

    def test_no_server_address_setting(self):
        base = {"harness": "claude-code-cli", "key": "claude", "mode": "micro"}
        for server in ("https://ingest.agentcommunity.org", PADCFG["staging"]):
            with self.assertRaises(pad.Usage):
                pad.load_settings(dict(base, server=server), PADCFG)
        with self.assertRaises(pad.Usage):
            pad.load_settings(dict(base, server="staging"), dict(PADCFG, staging="https://ingest.agentcommunity.org"))


class NoKeyInArgv(unittest.TestCase):
    """A sentinel key in the environment and in the key file: no argument the pad builds, and no
    file it writes, holds it. The harness takes its key by `key_route = "env"`."""

    HARNESS = {"version": "1.2.3", "install": "npm i -g @example/cli@1.2.3", "key_route": "env",
               "key_env": "ANTHROPIC_API_KEY", "keep_going": True}

    def fake_call(self, argv, timeout=None, env=None):
        self.calls.append(list(argv))
        verb = argv[2] if len(argv) > 2 and argv[0] == "node" else "local"
        out = {"create": json.dumps({"id": "sbx_test"}), "exec": "PADVER=1.2.3\n",
               "local": "PADVER=1.2.3\n", "list": ""}.get(verb, "")
        return subprocess.CompletedProcess(argv, 0, out, "")

    def walk(self, where, case="direct"):
        self.calls = []
        with tempfile.TemporaryDirectory() as home, tempfile.TemporaryDirectory() as out:
            os.makedirs(os.path.join(home, ".pad-keys"))
            key_file(os.path.join(home, ".pad-keys"))
            env = {"HOME": home, "CLAUDE_API_KEY": SENTINEL, "AI_GATEWAY_API_KEY": SENTINEL + "gw"}
            s = pad.load_settings({"harness": "claude-code-cli", "key": "claude", "mode": "cycle",
                                   "case": case, "where": where, "steps": ["kill-recorder@5"]}, PADCFG)
            with mock.patch.dict(os.environ, env), mock.patch.object(pad, "call", self.fake_call), \
                    mock.patch.object(pad, "read_image_id", lambda quiet=False: "snap_test"):
                pad.run_all(s, PADCFG, self.HARNESS, out)
            for argv in self.calls:
                for a in argv:
                    self.assertNotIn("PADSENTINEL", a)
            for root, _, files in os.walk(out):
                for name in files:
                    with open(os.path.join(root, name), "rb") as f:
                        self.assertNotIn(b"PADSENTINEL", f.read(), name)
        return self.calls

    def test_sandbox_passes_names_only(self):
        calls = self.walk("sandbox", case="gateway")
        put = [c for c in calls if c[2] == "put-env"]
        self.assertEqual(sorted(c[c.index("--env") + 1] for c in put), ["AI_GATEWAY_API_KEY", "CLAUDE_API_KEY"])
        self.assertTrue(any(c[2] == "rm" for c in calls))

    def test_here_reads_no_key(self):
        self.walk("here")


if __name__ == "__main__":
    unittest.main()
