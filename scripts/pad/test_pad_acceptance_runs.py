"""The committed run files (plan Task 3): the one-hour composite runs, the core rows and the Codex kill fallback.
No network. The prompt paths in the run files are relative to the repository root, so the tests run from there."""
import importlib.machinery
import importlib.util
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


pad = load("padacc", "pad")
PADCFG = pad.read_toml(os.path.join(HERE, "pad.toml"))


def harness(s):
    return pad.read_toml(os.path.join(HERE, "harnesses", s["harness"] + ".toml"))


def plan(name):
    s = pad.load_settings(pad.read_toml(os.path.join(HERE, "runs", name)), PADCFG)
    pad.apply_kill_minutes(s, harness(s))
    return s


def kills(s):
    return [m for kind, m, r in s["plan"] if r.startswith("kill-recorder@")]


class AcceptanceRuns(unittest.TestCase):
    def setUp(self):
        self.cwd = os.getcwd()
        os.chdir(ROOT)

    def tearDown(self):
        os.chdir(self.cwd)

    def test_composite_runs(self):
        for name in ("a2.toml", "a3.toml", "a5.toml"):
            s = plan(name)
            self.assertEqual((kills(s), s["minutes"], s["cap_usd"], s["keep_going"], s["keep_going_count"], s["box_extra_minutes"]),
                             ([10, 30], 60, 15, 2, 12, 15), name)
            self.assertIn("kill-watchdog-recover@45", s["steps"])
            self.assertTrue(s["prompt"].endswith("many-subagents.txt"))

    def test_a5_carries_release_items(self):
        s = plan("a5.toml")
        steps = [r for _, _, r in s["plan"]]
        self.assertEqual((s["harness"], s["case"], s["mode"]), ("codex-cli", "direct", "full"))
        self.assertIn("exec h2-parallel.sh@15", steps)
        self.assertIn("exec long-tunnel.sh@20", steps)
        self.assertFalse(any("connector copy" in r for r in steps))

    def test_a1_two_hours_cap_15(self):
        s = plan("a1.toml")
        self.assertEqual((s["minutes"], s["cap_usd"], s["case"]), (120, 15, "direct"))

    def test_core_rows_give_tiers(self):
        for h in ("claude-code", "codex", "hermes"):
            s = plan("core-%s.toml" % h)
            self.assertEqual((s["case"], s["mode"], s["minutes"], s["box_extra_minutes"]), ("all", "full", 20, 10))
            self.assertEqual([x for x in pad.step_list(s, harness(s)) if x.split("@")[0] in pad.FAULTS], [])

    def test_codex_kill_fallback_uses_harness_minutes(self):
        self.assertEqual(kills(plan("kill-codex.toml")), [3, 5])


if __name__ == "__main__":
    unittest.main()
