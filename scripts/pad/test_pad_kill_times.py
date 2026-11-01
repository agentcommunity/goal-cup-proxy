"""Kill minutes per harness (PRD-12 section 3.1, note of 2026-11-01): the harness file's kill_minutes replaces the
kill-recorder@8 and @10 pair; Codex gets 3 and 5. No network."""
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


pad, table = load("padkt", "pad"), load("padkttable", "table")
PADCFG = {"keys": {"openai": "OPENAI_API_KEY", "claude": "CLAUDE_API_KEY", "gateway": "AI_GATEWAY_API_KEY"}}
PAIR = ["kill-recorder@8", "kill-recorder@10"]


def plan_for(harness_id, key, extra=None, **over):
    s = pad.load_settings(dict(harness=harness_id, key=key, mode="full", steps=list(PAIR), **over), PADCFG)
    return pad.apply_kill_minutes(s, pad.read_toml(os.path.join(HERE, "harnesses", harness_id + ".toml")) if extra is None else extra)


def minutes(s):
    return [m for kind, m, r in s["plan"] if r.startswith("kill-recorder@")]


class KillTimes(unittest.TestCase):
    def test_codex_kills_at_3_and_5_claude_code_at_8_and_10(self):
        self.assertEqual(minutes(plan_for("codex-cli", "openai")), [3, 5])
        self.assertEqual(minutes(plan_for("claude-code-cli", "claude")), [8, 10])

    def test_bad_values_are_a_usage_error(self):
        for bad in ([5, 3], [3, 3], [3, 4], [3, 30], [0, 5], [3], [3, 5, 7], ["3", "5"], [3.0, 5], [True, 5], "3,5", 3):
            with self.assertRaises(pad.Usage, msg=repr(bad)):
                plan_for("codex-cli", "openai", extra={"kill_minutes": bad})

    def test_codex_full_row_has_both_kills_in_time_order_each_with_wait_recovery(self):
        s = plan_for("codex-cli", "openai")
        seq = pad.step_list(s, pad.read_toml(os.path.join(HERE, "harnesses", "codex-cli.toml")))
        kills = [i for i, x in enumerate(seq) if x.startswith("kill-recorder@")]
        self.assertEqual([seq[i] for i in kills], ["kill-recorder@3", "kill-recorder@5"])
        for i in kills:
            self.assertEqual(seq[i + 1], "wait-recovery")

    def test_a_run_without_the_pair_keeps_its_steps(self):
        s = pad.load_settings(dict(harness="codex-cli", key="openai", mode="full", steps=["kill-recorder@0"]), PADCFG)
        pad.apply_kill_minutes(s, {"kill_minutes": [3, 5]})
        self.assertEqual(minutes(s), [0])

    def test_the_second_kill_is_before_the_end_of_the_run(self):
        with self.assertRaises(pad.Usage):
            plan_for("codex-cli", "openai", extra={"kill_minutes": [28, 30]}, minutes=30)

    def test_settings_name_the_same_minutes(self):
        self.assertEqual(plan_for("codex-cli", "openai")["steps"], ["kill-recorder@3", "kill-recorder@5"])

    def test_cycle_own_crash_is_not_doubled(self):
        s = pad.load_settings(dict(harness="codex-cli", key="openai", mode="cycle", steps=list(PAIR)), PADCFG)
        pad.apply_kill_minutes(s, {"kill_minutes": [3, 5]})
        self.assertEqual(minutes(s), [3, 5])


if __name__ == "__main__":
    unittest.main()
