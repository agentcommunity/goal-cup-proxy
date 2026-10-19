"""Behavioral checks for the bounded whole-module script-test runner."""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


RUNNER = Path(__file__).with_name("run-script-tests.py")


class ScriptRunnerTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def write(self, name: str, body: str) -> None:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)

    def run_runner(self, jobs: int = 2) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(RUNNER), "--jobs", str(jobs),
             "--start-directory", str(self.root)],
            capture_output=True, text=True, timeout=15,
        )

    def test_serial_and_parallel_run_the_same_discovered_tests_and_fixtures(self) -> None:
        self.write("test_check_connector.py", """
import unittest
from pathlib import Path
Path(__file__).with_suffix('.imported').write_text('yes')
def setUpModule():
    Path(__file__).with_suffix('.setup').write_text('yes')
def tearDownModule():
    Path(__file__).with_suffix('.teardown').write_text('yes')
class Cases(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        Path(__file__).with_suffix('.class').write_text('yes')
    def test_one(self): pass
    @unittest.skip('fixture skip')
    def test_two(self): pass
""")
        self.write("testcustom.py", """
import unittest
class Cases(unittest.TestCase):
    def test_three(self): pass
""")
        self.write("suite/__init__.py", """
import unittest
def load_tests(loader, tests, pattern):
    return loader.loadTestsFromName('suite.test_hook')
""")
        self.write("suite/test_hook.py", """
import unittest
class Hook(unittest.TestCase):
    def test_four(self): pass
""")
        for jobs in (1, 2):
            with self.subTest(jobs=jobs):
                result = self.run_runner(jobs)
                output = result.stdout + result.stderr
                self.assertEqual(result.returncode, 0, output)
                for case in ("test_one (test_check_connector.Cases.test_one)",
                             "test_two (test_check_connector.Cases.test_two)",
                             "test_three (testcustom.Cases.test_three)",
                             "test_four (suite.test_hook.Hook.test_four)"):
                    self.assertEqual(output.count(case), 1, output)
                self.assertIn("skipped", output)
                self.assertEqual((self.root / "test_check_connector.setup").read_text(), "yes")
                self.assertEqual((self.root / "test_check_connector.teardown").read_text(), "yes")
                self.assertEqual((self.root / "test_check_connector.class").read_text(), "yes")

    def test_failures_and_import_errors_are_reported_without_stopping_later_modules(self) -> None:
        self.write("test_check_connector.py", """
import unittest
class Cases(unittest.TestCase):
    def test_failure(self): self.fail('TEST FAILURE MARKER')
""")
        self.write("test_check_links.py", "raise RuntimeError('IMPORT FAILURE MARKER')\n")
        self.write("test_check_version.py", """
import unittest
class Cases(unittest.TestCase):
    def test_later(self): pass
""")
        self.write("test_custom.py", """
import unittest
def setUpModule(): raise RuntimeError('SETUP FAILURE MARKER')
class Cases(unittest.TestCase):
    def test_setup(self): pass
""")
        result = self.run_runner()
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        for marker in ("TEST FAILURE MARKER", "IMPORT FAILURE MARKER",
                       "SETUP FAILURE MARKER", "test_later (test_check_version.Cases.test_later)"):
            self.assertIn(marker, output)

    def test_empty_or_missing_discovery_fails(self) -> None:
        result = self.run_runner()
        self.assertNotEqual(result.returncode, 0)
        missing = subprocess.run(
            [sys.executable, str(RUNNER), "--start-directory", str(self.root / "missing")],
            capture_output=True, text=True, timeout=10,
        )
        self.assertNotEqual(missing.returncode, 0)

    def test_empty_helper_module_does_not_fail_a_nonempty_suite(self) -> None:
        self.write("test_support.py", "VALUE = 1\n")
        self.write("testcustom.py", """
import unittest
class Cases(unittest.TestCase):
    def test_real(self): pass
""")
        result = self.run_runner()
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, output)
        self.assertIn("test_real (testcustom.Cases.test_real)", output)

    def test_tree_with_only_empty_helper_modules_fails(self) -> None:
        self.write("test_support.py", "VALUE = 1\n")
        result = self.run_runner()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_import_time_skip_remains_a_skip(self) -> None:
        self.write("testcustom.py", """
import unittest
raise unittest.SkipTest('host prerequisite absent')
""")
        result = self.run_runner()
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, output)
        self.assertIn("host prerequisite absent", output)
        self.assertIn("skipped=1", output)

    def test_abnormal_worker_exit_fails_and_later_module_still_runs(self) -> None:
        self.write("testcustom.py", "import os\nos._exit(2)\n")
        self.write("test_check_connector.py", """
import unittest
class Cases(unittest.TestCase):
    def test_later(self): pass
""")
        result = self.run_runner()
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("test_later (test_check_connector.Cases.test_later)", output)
        self.assertIn("worker exited 2", output)

    def test_audited_modules_overlap_at_most_two_and_unknown_modules_run_alone(self) -> None:
        self.write("test_check_connector.py", """
import os, time, unittest
from pathlib import Path
root = Path(os.environ['FIXTURE_ROOT'])
class Cases(unittest.TestCase):
    def test_overlap(self):
        (root / 'first').write_text('started')
        deadline = time.monotonic() + 5
        while not (root / 'second').exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue((root / 'second').exists(), 'audited peer did not overlap')
        self.assertFalse((root / 'third').exists(), 'third worker overlapped')
""")
        self.write("test_check_links.py", """
import os, time, unittest
from pathlib import Path
root = Path(os.environ['FIXTURE_ROOT'])
class Cases(unittest.TestCase):
    def test_overlap(self):
        (root / 'second').write_text('started')
        deadline = time.monotonic() + 5
        while not (root / 'first').exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue((root / 'first').exists(), 'audited peer did not overlap')
        self.assertFalse((root / 'third').exists(), 'third worker overlapped')
""")
        self.write("test_check_version.py", """
import os, unittest
from pathlib import Path
class Cases(unittest.TestCase):
    def test_third(self):
        Path(os.environ['FIXTURE_ROOT'], 'third').write_text('started')
""")
        self.write("test_unknown.py", """
import os, unittest
from pathlib import Path
class Cases(unittest.TestCase):
    def test_exclusive(self):
        root = Path(os.environ['FIXTURE_ROOT'])
        self.assertFalse((root / 'first').exists())
        self.assertFalse((root / 'second').exists())
        self.assertFalse((root / 'third').exists())
""")
        env = {**os.environ, "FIXTURE_ROOT": str(self.root)}
        result = subprocess.run(
            [sys.executable, str(RUNNER), "--jobs", "2", "--start-directory", str(self.root)],
            env=env, capture_output=True, text=True, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def write_waiting_install_and_pending_peer(self) -> None:
        self.write("test_install.py", """
import os, time, unittest
from pathlib import Path
root = Path(os.environ['FIXTURE_ROOT'])
def tearDownModule():
    (root / 'cleaned').write_text('yes')
class Cases(unittest.TestCase):
    def test_wait(self):
        (root / 'active').write_text('yes')
        deadline = time.monotonic() + 10
        while not (root / 'release').exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue((root / 'release').exists())
        print('ACTIVE OUTPUT RETAINED', flush=True)
""")
        self.write("test_check_connector.py", """
import os, unittest
from pathlib import Path
class Cases(unittest.TestCase):
    def test_pending(self):
        Path(os.environ['FIXTURE_ROOT'], 'pending').write_text('started')
""")

    def run_with_injected_allocation(self, mode: str) -> tuple[int, str]:
        bootstrap = """
import os, runpy, signal, sys, tempfile
from pathlib import Path
mode, runner = sys.argv[1:3]
real = tempfile.TemporaryDirectory
calls = 0
def inject(*args, **kwargs):
    global calls
    calls += 1
    if calls == 2:
        Path(os.environ['FIXTURE_ROOT'], 'injected').write_text('yes')
        if mode == 'error':
            raise OSError('INJECTED TEMP FAILURE')
        os.kill(os.getpid(), signal.SIGINT)
    return real(*args, **kwargs)
tempfile.TemporaryDirectory = inject
sys.argv = [runner, '--jobs', '2', '--start-directory', os.environ['FIXTURE_ROOT']]
runpy.run_path(runner, run_name='__main__')
"""
        env = {**os.environ, "FIXTURE_ROOT": str(self.root)}
        proc = subprocess.Popen(
            [sys.executable, "-c", bootstrap, mode, str(RUNNER)],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 5
            while not ((self.root / "active").exists() and
                       (self.root / "injected").exists()) and time.monotonic() < deadline:
                time.sleep(.02)
            self.assertTrue((self.root / "active").exists(), "active fixture never started")
            self.assertTrue((self.root / "injected").exists(), "fault was not injected")
            (self.root / "release").write_text("yes")
            output, _ = proc.communicate(timeout=10)
            return proc.returncode, output
        finally:
            (self.root / "release").write_text("yes")
            if proc.poll() is None:
                proc.kill()
                proc.communicate(timeout=5)

    def test_resource_failure_drains_active_worker_and_retains_output(self) -> None:
        self.write_waiting_install_and_pending_peer()
        status, output = self.run_with_injected_allocation("error")
        self.assertNotEqual(status, 0, output)
        self.assertIn("INJECTED TEMP FAILURE", output)
        self.assertIn("ACTIVE OUTPUT RETAINED", output)
        self.assertTrue((self.root / "cleaned").exists(), output)
        self.assertFalse((self.root / "pending").exists(), output)

    @unittest.skipUnless(os.name == "posix", "signal mask requires POSIX")
    def test_signal_during_allocation_cannot_launch_pending_module(self) -> None:
        self.write_waiting_install_and_pending_peer()
        status, output = self.run_with_injected_allocation("signal")
        self.assertEqual(status, 130, output)
        self.assertIn("ACTIVE OUTPUT RETAINED", output)
        self.assertTrue((self.root / "cleaned").exists(), output)
        self.assertFalse((self.root / "pending").exists(), output)

    @unittest.skipUnless(os.name == "posix", "process group signals require POSIX")
    def test_interrupt_stops_dispatch_and_drains_active_module(self) -> None:
        self.check_two_worker_signal_drain(signal.SIGINT)

    @unittest.skipUnless(os.name == "posix", "process group signals require POSIX")
    def test_terminate_stops_dispatch_and_drains_active_module(self) -> None:
        self.check_two_worker_signal_drain(signal.SIGTERM)

    def check_two_worker_signal_drain(self, signum: int) -> None:
        for name in ("test_install", "test_check_connector"):
            self.write(f"{name}.py", f"""
import os, signal, subprocess, sys, time, unittest
from pathlib import Path
root = Path(os.environ['FIXTURE_ROOT'])
def tearDownModule():
    (root / '{name}.cleaned').write_text('yes')
class Cases(unittest.TestCase):
    def test_wait(self):
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(20)'])
        try:
            child.terminate()
            self.assertEqual(child.wait(timeout=2), -signal.SIGTERM,
                             'worker child inherited a blocked TERM')
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=2)
        (root / '{name}.active').write_text('yes')
        deadline = time.monotonic() + 10
        while not (root / '{name}.release').exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue((root / '{name}.release').exists())
        print('OUTPUT {name}', flush=True)
""")
        self.write("test_check_links.py", """
import os, unittest
from pathlib import Path
class Cases(unittest.TestCase):
    def test_pending(self):
        Path(os.environ['FIXTURE_ROOT'], 'pending').write_text('started')
""")
        env = {**os.environ, "FIXTURE_ROOT": str(self.root)}
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(20)"])
        proc = subprocess.Popen(
            [sys.executable, str(RUNNER), "--jobs", "2", "--start-directory", str(self.root)],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 5
            while not all((self.root / f"{name}.active").exists()
                          for name in ("test_install", "test_check_connector")) \
                    and time.monotonic() < deadline:
                time.sleep(.02)
            for name in ("test_install", "test_check_connector"):
                self.assertTrue((self.root / f"{name}.active").exists(),
                                f"{name} never started")
            os.killpg(proc.pid, signum)
            for name in ("test_install", "test_check_connector"):
                (self.root / f"{name}.release").write_text("yes")
            output, _ = proc.communicate(timeout=10)
            self.assertEqual(proc.returncode, 128 + signum, output)
            for name in ("test_install", "test_check_connector"):
                self.assertTrue((self.root / f"{name}.cleaned").exists(), output)
                self.assertIn(f"OUTPUT {name}", output)
            self.assertFalse((self.root / "pending").exists(), output)
            self.assertIsNone(unrelated.poll(), "unrelated process was signalled")
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.communicate(timeout=5)
            unrelated.terminate()
            unrelated.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
