"""The canary names harness builds no pad row has verified (plan K.4). No network. Run: python3.11 -m unittest scripts/pad/test_canary.py"""
import importlib.machinery
import importlib.util
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


canary = load("padcanary", "canary")
KNOWN = {"claude-code-cli": "2.1.312", "codex-cli": "0.50.0", "amp-cli": "0.0.1793664053-ga6248e", "hermes-cli": "0.22.5"}


def fake(answers):
    """run() that answers by package or repo name; an Exception value is raised."""
    def run(argv):
        a = next(v for k, v in answers.items() if k in " ".join(argv))
        if isinstance(a, Exception):
            raise a
        return a
    return run


def all_known(**over):
    """Every harness verified at KNOWN and answering KNOWN, then the overrides."""
    tiers = {cid: {"verified": [{"version": v, "os": "macos"}]} for cid, v in KNOWN.items()}
    ans = {name: v for cid, v in KNOWN.items() for kind, name in [canary.SOURCES[cid]] if kind == "npm"}
    ans["NousResearch/hermes-agent"] = json.dumps({"tag_name": "v" + KNOWN["hermes-cli"]})
    ans.update(over)
    return tiers, ans


def one(cid, tiers, answer):
    return canary.report(tiers, fake({canary.SOURCES[cid][1]: answer}), only=[cid])


class Canary(unittest.TestCase):
    def test_new_build_exits_1(self):
        lines, code = one("claude-code-cli", {"claude-code-cli": {"verified": [{"version": "2.1.312", "os": "macos"}]}}, "2.1.320\n")
        self.assertEqual(lines, ["claude-code-cli latest 2.1.320 verified 2.1.312 (macos) NEW"])
        self.assertEqual(code, 1)

    def test_known_build_exits_0(self):
        lines, code = one("claude-code-cli", {"claude-code-cli": {"verified": [{"version": "2.1.312", "os": "macos"}]}}, "2.1.312\n")
        self.assertTrue(lines[0].endswith("known"), lines)
        self.assertEqual(code, 0)

    def test_other_os_still_known(self):
        lines, code = one("claude-code-cli", {"claude-code-cli": {"verified": [{"version": "2.1.312", "os": "linux"}]}}, "2.1.312")
        self.assertTrue(lines[0].endswith("known"), lines)
        self.assertEqual(code, 0)

    def test_amp_suffix_matches(self):
        tiers = {"amp-cli": {"verified": [{"version": "0.0.1793664053-ga6248e", "os": "macos"}]}}
        self.assertTrue(one("amp-cli", tiers, "0.0.1793664053-ga6248e\n")[0][0].endswith("known"))
        lines, code = one("amp-cli", tiers, "0.0.1793664099-gb1c2d3\n")
        self.assertTrue(lines[0].endswith("NEW"), lines)
        self.assertEqual(code, 1)

    def test_hermes_v_prefix(self):
        tiers = {"hermes-cli": {"verified": [{"version": "0.22.5", "os": "macos"}]}}
        lines, code = one("hermes-cli", tiers, json.dumps({"tag_name": "v0.22.6"}))
        self.assertIn("latest 0.22.6 ", lines[0])
        self.assertTrue(lines[0].endswith("NEW"), lines)
        self.assertEqual(code, 1)

    def test_nothing_verified_is_new(self):
        lines, code = one("codex-cli", {"codex-cli": {"tier": "not verified", "version": ""}}, "0.50.0\n")
        self.assertEqual(lines, ["codex-cli latest 0.50.0 verified none NEW"])
        self.assertEqual(code, 1)

    def test_lookup_failure_exits_2(self):
        tiers, ans = all_known(**{"@openai/codex": RuntimeError("registry unreachable")})
        lines, code = canary.report(tiers, fake(ans))
        self.assertIn("codex-cli lookup failed: registry unreachable", lines)
        self.assertEqual(len(lines), 4)
        self.assertEqual(code, 2)

    def test_failure_does_not_hide_new(self):
        tiers, ans = all_known(**{"@openai/codex": RuntimeError("registry unreachable"), "@anthropic-ai/claude-code": "2.1.320"})
        lines, code = canary.report(tiers, fake(ans))
        self.assertIn("codex-cli lookup failed: registry unreachable", lines)
        self.assertIn("claude-code-cli latest 2.1.320 verified 2.1.312 (macos) NEW", lines)
        self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
