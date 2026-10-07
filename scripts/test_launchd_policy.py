#!/usr/bin/env python3
"""The launchd user agent's restart policy, and the scripts that install and remove it.

Static checks only: this file parses the plist template with `plistlib`, reads the shell
scripts as text and runs `sh -n` on them. It never runs `install-assisted.sh`,
`test-m2-pilot.sh` or `launchctl`.

    python3 scripts/test_launchd_policy.py
"""

from __future__ import annotations

import plistlib
import re
import subprocess
import unittest
from pathlib import Path

MACOS = Path(__file__).resolve().parents[1] / "packaging" / "macos"
PLIST = MACOS / "com.agentcommunity.goalcup.plist"
OWNED_PREFIX = "com.agentcommunity.goalcup.m2pilot."


class PlistPolicy(unittest.TestCase):
    def setUp(self) -> None:
        with PLIST.open("rb") as handle:
            self.plist = plistlib.load(handle)

    def test_keepalive_restarts_a_crash_and_nothing_else(self) -> None:
        # `KeepAlive = true` restarts a daemon that refuses its configuration (exit 1) forever.
        keepalive = self.plist["KeepAlive"]
        self.assertIsInstance(keepalive, dict, "KeepAlive must be a dict, not a bare boolean")
        self.assertIs(keepalive.get("Crashed"), True)
        self.assertEqual(set(keepalive), {"Crashed"}, "no other KeepAlive condition")

    def test_throttle_interval_is_at_least_ten_seconds(self) -> None:
        interval = self.plist["ThrottleInterval"]
        self.assertIsInstance(interval, int)
        self.assertGreaterEqual(interval, 10)

    def test_the_template_is_still_a_user_agent_template(self) -> None:
        self.assertEqual(self.plist["Label"], "__GOALCUP_LABEL__")
        self.assertIs(self.plist["RunAtLoad"], True)


class TeardownIsScopedToTheOwnedLabel(unittest.TestCase):
    def test_every_script_parses(self) -> None:
        for name in ("install-assisted.sh", "test-m2-pilot.sh", "uninstall-assisted.sh"):
            with self.subTest(script=name):
                done = subprocess.run(
                    ["sh", "-n", str(MACOS / name)], capture_output=True, text=True
                )
                self.assertEqual(done.returncode, 0, done.stderr)

    def test_installer_shell_is_a_working_public_entry_point(self) -> None:
        done = subprocess.run(
            ["sh", str(MACOS / "install-assisted.sh"), "--help"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("--bundle", done.stdout)
        self.assertIn("--prefix", done.stdout)
        self.assertIn("--label", done.stdout)

    def test_the_pilot_boots_out_only_its_own_label(self) -> None:
        text = (MACOS / "test-m2-pilot.sh").read_text(encoding="utf-8")
        assigns = dict(re.findall(r'^(label|prepublish_label|postpublish_label)="([^"]*)"$', text, re.M))
        self.assertEqual(set(assigns), {"label", "prepublish_label", "postpublish_label"})
        for value in assigns.values():
            self.assertTrue(value.startswith(OWNED_PREFIX), value)
        bootouts = re.findall(r'launchctl bootout ("[^"]*")', text)
        self.assertEqual(set(bootouts), {'"gui/$(id -u)/$owned_label"', '"gui/$(id -u)/$label"'})
        cleanup = text[text.index("cleanup() {") : text.index("trap cleanup")]
        for name in assigns:
            self.assertIn(f'bootout_disposable_label "${name}"', cleanup)

    def test_the_pilot_cleanup_refuses_a_label_it_did_not_mint(self) -> None:
        # If the label were ever edited to the real agent's, the cleanup must not boot it out.
        text = (MACOS / "test-m2-pilot.sh").read_text(encoding="utf-8")
        helper = text[text.index("bootout_disposable_label() {") : text.index("cleanup() {")]
        self.assertIn(OWNED_PREFIX, helper, "cleanup helper checks the owned label prefix")
        self.assertLess(helper.index(OWNED_PREFIX), helper.index("launchctl bootout"))

    def test_the_uninstaller_boots_out_only_the_recorded_label(self) -> None:
        text = (MACOS / "uninstall-assisted.sh").read_text(encoding="utf-8")
        for target in re.findall(r'launchctl bootout ("[^"]*")', text):
            self.assertEqual(target, '"gui/$me/$label"')
        self.assertIn('label="$recorded_label"', text)


if __name__ == "__main__":
    unittest.main()
