#!/usr/bin/env python3
"""Run script unittest modules with at most two audited modules at a time.

Cancellation stops new dispatch and waits for active modules to complete their normal
cleanup. A hung module can therefore keep cancellation waiting; no worker group is killed.
"""

from __future__ import annotations

import argparse
from fnmatch import fnmatch
import importlib
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time
from typing import TextIO
import unittest
from unittest.loader import _make_skipped_test


AUDITED = frozenset({
    "test_install", "test_check_connector", "test_check_links", "test_check_version",
    "test_ci_scope", "test_gate", "test_linux_check", "test_release",
    "test_research_replay", "test_research_watch",
})
MODULE_NAME = re.compile(r"[_a-z]\w*\.py\Z", re.IGNORECASE)
STOP_SIGNALS = {signal.SIGINT, signal.SIGTERM}


def report(message: str) -> None:
    try:
        print(message, file=sys.stderr, flush=True)
    except Exception:
        # A broken diagnostic stream must not interrupt worker reaping.
        pass


def discover_units(start: Path) -> list[tuple[str, str]]:
    """Partition unittest's top-level discovery entries into whole-module jobs."""
    if not start.is_dir():
        raise ValueError(f"start directory does not exist: {start}")
    units = []
    for path in sorted(start.iterdir()):
        if path.is_file() and MODULE_NAME.match(path.name) and fnmatch(path.name, "test*.py"):
            units.append(("module", path.stem))
        elif path.is_dir() and (path / "__init__.py").is_file():
            # A package may supply load_tests, which can suppress ordinary recursion.
            units.append(("package", path.name))
    if len({name for _, name in units}) != len(units):
        raise ValueError(f"ambiguous unittest module and package names in {start}")
    if not units:
        raise ValueError(f"no unittest modules found in {start}")
    return units


def run_worker(start: Path, kind: str, name: str, result_file: Path) -> int:
    # The supervisor blocks signals during Popen. exec preserves that mask, so clear
    # it before a test module or any of its subprocesses runs.
    signal.pthread_sigmask(signal.SIG_UNBLOCK, STOP_SIGNALS)
    sys.path.insert(0, str(start))
    loader = unittest.TestLoader()
    # Discovery supplies this context to module load_tests hooks. A hook's nested
    # discover call needs the same top-level directory to preserve test IDs.
    loader._top_level_dir = str(start)
    try:
        if kind == "module":
            try:
                module = importlib.import_module(name)
            except unittest.SkipTest as exc:
                # Match discover's import-time skip rather than treating it as an error.
                suite = _make_skipped_test(name, exc, loader.suiteClass)
            else:
                expected = (start / f"{name}.py").resolve()
                imported = getattr(module, "__file__", None)
                if imported is None or Path(imported).resolve() != expected:
                    raise ImportError(f"{name} was imported from {imported}, expected {expected}")
                suite = loader.loadTestsFromModule(module, pattern="test*.py")
        else:
            suite = loader.discover(str(start / name), pattern="test*.py", top_level_dir=str(start))
    except Exception:
        import traceback
        traceback.print_exc()
        return 1
    count = suite.countTestCases()
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    result_file.write_text(str(count))
    return 0 if result.wasSuccessful() and not loader.errors else 1


def run_group(start: Path, units: list[tuple[str, str]], jobs: int,
              cancelled: list[int]) -> tuple[bool, bool]:
    failed = False
    found_tests = False
    logged_cancel = False
    active: list[tuple[subprocess.Popen[bytes], TextIO, str, Path,
                       tempfile.TemporaryDirectory[str]]] = []
    pending = list(units)

    def resource_error(name: str, action: str, exc: Exception) -> None:
        nonlocal failed
        failed = True
        pending.clear()
        report(f"{name}: {action}: {exc}")

    def close_resource(name: str, action: str, resource: object) -> None:
        try:
            if action == "close output":
                resource.close()
            else:
                resource.cleanup()
        except Exception as exc:
            resource_error(name, action, exc)

    while pending or active:
        if cancelled[0]:
            pending.clear()
            if not logged_cancel:
                report(f"script tests: draining active modules after signal {cancelled[0]}")
                logged_cancel = True
        while pending and len(active) < jobs and not cancelled[0]:
            kind, name = pending[0]
            output = None
            result_dir = None
            started = False
            try:
                output = tempfile.TemporaryFile(mode="w+t")
                result_dir = tempfile.TemporaryDirectory()
                result_file = Path(result_dir.name) / "count"
                # Treat the final cancellation check and spawn as one transition.
                previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, STOP_SIGNALS)
                try:
                    if cancelled[0] or signal.sigpending() & STOP_SIGNALS:
                        pending.clear()
                    else:
                        process = subprocess.Popen(
                            [sys.executable, str(Path(__file__).resolve()), "--_worker", kind,
                             name, "--start-directory", str(start),
                             "--_result-file", str(result_file)],
                            stdout=output, stderr=subprocess.STDOUT, start_new_session=True,
                        )
                        active.append((process, output, name, result_file, result_dir))
                        pending.pop(0)
                        started = True
                finally:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
            except Exception as exc:
                resource_error(name, "could not start worker", exc)
            finally:
                if not started:
                    if output is not None:
                        close_resource(name, "close output", output)
                    if result_dir is not None:
                        close_resource(name, "remove result directory", result_dir)
        finished = [job for job in active if job[0].poll() is not None]
        for process, output, name, result_file, result_dir in finished:
            process.wait()  # Reap only a worker this runner started.
            active.remove((process, output, name, result_file, result_dir))
            try:
                output.seek(0)
                diagnostics = output.read()
            except Exception as exc:
                resource_error(name, "could not read worker output", exc)
            else:
                try:
                    print(f"=== {name} ===", flush=True)
                    print(diagnostics, end="", flush=True)
                except Exception as exc:
                    resource_error(name, "could not print worker output", exc)
            close_resource(name, "close output", output)
            try:
                count = int(result_file.read_text())
                if count < 0:
                    raise ValueError("negative test count")
            except (OSError, ValueError, UnicodeError):
                count = 0
                failed = True
                report(f"{name}: worker did not complete normally")
            close_resource(name, "remove result directory", result_dir)
            found_tests |= count > 0
            if process.returncode != 0:
                failed = True
                report(f"{name}: worker exited {process.returncode}")
        if active and not finished:
            time.sleep(0.02)
    return failed, found_tests


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, choices=(1, 2), default=2)
    parser.add_argument("--start-directory", type=Path, default=Path(__file__).parent)
    parser.add_argument("--_worker", nargs=2, metavar=("KIND", "NAME"),
                        help=argparse.SUPPRESS)
    parser.add_argument("--_result-file", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    start = args.start_directory.resolve()
    if args._worker:
        if args._result_file is None:
            parser.error("internal worker requires a result file")
        return run_worker(start, *args._worker, args._result_file)
    try:
        units = discover_units(start)
    except (OSError, ValueError) as exc:
        print(f"script tests: {exc}", file=sys.stderr)
        return 1

    cancelled = [0]

    def on_signal(signum: int, _frame: object) -> None:
        if not cancelled[0]:
            cancelled[0] = signum

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    if args.jobs == 1:
        failed, found_tests = run_group(start, units, 1, cancelled)
    else:
        exclusive = [unit for unit in units if unit[0] != "module" or unit[1] not in AUDITED]
        audited = [unit for unit in units if unit[0] == "module" and unit[1] in AUDITED]
        audited.sort(key=lambda unit: (unit[1] != "test_install", unit[1]))
        failed, found_tests = run_group(start, exclusive, 1, cancelled)
        if not cancelled[0]:
            cohort_failed, cohort_found = run_group(start, audited, 2, cancelled)
            failed |= cohort_failed
            found_tests |= cohort_found
    if cancelled[0]:
        report(f"script tests: interrupted by signal {cancelled[0]}")
        return 128 + cancelled[0]
    if not found_tests:
        print(f"script tests: no tests discovered in {start}", file=sys.stderr)
        return 1
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
