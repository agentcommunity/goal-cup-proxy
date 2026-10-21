"""Behavioral checks for the bounded research watchdog; no real waiting or agents."""

import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch


SPEC = importlib.util.spec_from_file_location("research_watch", Path(__file__).with_name("research-watch.py"))
WATCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WATCH)
ENTRY = "e0123456789abcdef"


class ResearchWatchTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.config = str(Path(self.tmp.name).resolve())
        self.args = ["--config", self.config, "--entry", ENTRY]

    def invoke(self, extra=(), **kwargs):
        with contextlib.redirect_stderr(io.StringIO()) as output:
            result = WATCH.main(self.args + list(extra), **kwargs)
        return result, output.getvalue()

    def test_waits_monotonically_then_seals_exact_entry_and_config(self):
        runner = Mock(return_value=subprocess.CompletedProcess([], 0))
        sleep = Mock()
        with patch.dict(os.environ, {"GOALCUP_CONFIG_DIR": "/wrong", "WATCH_TEST": "preserved"}):
            result, log = self.invoke(
                ["--seconds", "2", "--goalcup", "/binary path/goalcup"],
                clock=Mock(side_effect=[100, 100, 101.8, 102]), sleep=sleep, runner=runner,
            )
        self.assertEqual(result, 0)
        self.assertEqual(sleep.call_count, 2)
        self.assertTrue(all(0 < call.args[0] <= 0.5 for call in sleep.call_args_list))
        args, options = runner.call_args
        self.assertEqual(args, (["/binary path/goalcup", "seal", "--yes", ENTRY],))
        self.assertEqual(options["env"]["GOALCUP_CONFIG_DIR"], self.config)
        self.assertEqual(options["env"]["WATCH_TEST"], "preserved")
        self.assertEqual(options["timeout"], 300)
        self.assertEqual(options["stdin"], subprocess.DEVNULL)
        self.assertNotIn("shell", options)
        self.assertEqual(runner.call_count, 1)
        self.assertIn("Seal command succeeded", log)

    def test_default_duration_is_1800(self):
        runner = Mock(return_value=subprocess.CompletedProcess([], 0))
        result, log = self.invoke(clock=Mock(side_effect=[0, 1800]), runner=runner)
        self.assertEqual(result, 0)
        self.assertIn("after 1800 seconds", log)

    def test_bad_ids_durations_and_missing_explicit_selection_refused(self):
        cases = [
            ["--config", self.config],
            ["--entry", ENTRY],
            *[self.args + ["--seconds", value] for value in ["0", "1801", "-1", "1.5", "nan"]],
            *[["--config", self.config, "--entry", value] for value in ["latest", "e1", "e0123456789abcdeF", ENTRY + ";echo bad", "--all"]],
            ["--config", "relative", "--entry", ENTRY],
        ]
        for args in cases:
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()):
                runner = Mock()
                with self.assertRaises(SystemExit) as raised:
                    WATCH.main(args, runner=runner)
                self.assertEqual(raised.exception.code, 2)
                runner.assert_not_called()

    def test_failure_is_nonzero_and_not_retried(self):
        runner = Mock(return_value=subprocess.CompletedProcess([], 7))
        result, log = self.invoke(clock=Mock(side_effect=[0, 1800]), runner=runner)
        self.assertEqual(result, 1)
        self.assertIn("exit 7", log)
        self.assertIn("unconfirmed", log)
        self.assertEqual(runner.call_count, 1)

    def test_interrupt_while_waiting_never_seals(self):
        runner = Mock()
        result, log = self.invoke(clock=lambda: 0, sleep=Mock(side_effect=KeyboardInterrupt), runner=runner)
        self.assertEqual(result, 130)
        runner.assert_not_called()
        self.assertIn("Manually seal", log)

    def test_timeout_does_not_claim_stopped(self):
        runner = Mock(side_effect=subprocess.TimeoutExpired("goalcup", 300))
        result, log = self.invoke(clock=Mock(side_effect=[0, 1800]), runner=runner)
        self.assertEqual(result, 124)
        self.assertIn("stopped state is unconfirmed", log)

    def test_missing_binary_requires_manual_stop(self):
        runner = Mock(side_effect=FileNotFoundError)
        result, log = self.invoke(clock=Mock(side_effect=[0, 1800]), runner=runner)
        self.assertEqual(result, 1)
        self.assertIn("Manually seal", log)


if __name__ == "__main__":
    unittest.main()
