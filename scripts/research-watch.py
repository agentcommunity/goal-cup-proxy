#!/usr/bin/env python3
"""Seal one explicitly selected research entry after a monotonic time budget."""

import argparse
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import subprocess
import sys
import time


SEAL_TIMEOUT_SECONDS = 300


def entry_id(value):
    if not re.fullmatch(r"e[0-9a-f]{16}", value):
        raise argparse.ArgumentTypeError("entry must be e followed by 16 lowercase hex digits")
    return value


def duration(value):
    try:
        seconds = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError("seconds must be an integer from 1 to 1800") from None
    if not 1 <= seconds <= 1800:
        raise argparse.ArgumentTypeError("seconds must be an integer from 1 to 1800")
    return seconds


def config_dir(value):
    path = Path(value)
    if not path.is_absolute() or not path.is_dir():
        raise argparse.ArgumentTypeError("config must be an existing absolute directory")
    return str(path)


def log(message):
    stamp = datetime.now(timezone.utc).isoformat(timespec="seconds")
    print(f"{stamp} {message}", file=sys.stderr, flush=True)


def main(argv=None, *, clock=time.monotonic, sleep=time.sleep, runner=subprocess.run):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=config_dir)
    parser.add_argument("--entry", required=True, type=entry_id)
    parser.add_argument("--seconds", type=duration, default=1800)
    parser.add_argument("--goalcup", default="goalcup", help="goalcup executable path or name")
    args = parser.parse_args(argv)
    deadline = clock() + args.seconds
    log(f"Watching entry {args.entry}; seal requested after {args.seconds} seconds.")
    try:
        while True:
            remaining = deadline - clock()
            if remaining <= 0:
                break
            sleep(min(remaining, 0.5))
        log(f"Deadline reached; requesting seal for entry {args.entry}.")
        env = os.environ.copy()
        env["GOALCUP_CONFIG_DIR"] = args.config
        # No shell, status discovery, input injection, or retry. Suppress child output so
        # this watchdog's log never becomes a second trace or an environment dump.
        result = runner(
            [args.goalcup, "seal", "--yes", args.entry],
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=SEAL_TIMEOUT_SECONDS,
            check=False,
        )
    except KeyboardInterrupt:
        log(f"Watchdog cancelled for entry {args.entry}; no further seal will be requested. "
            "The agent may still run. Manually seal this entry using the same config directory.")
        return 130
    except subprocess.TimeoutExpired:
        log(f"Seal timed out for entry {args.entry}; stopped state is unconfirmed. "
            "Inspect and manually seal this entry using the same config directory.")
        return 124
    except OSError:
        log(f"Could not execute seal for entry {args.entry}; the agent may still run. "
            "Manually seal this entry using the same config directory.")
        return 1
    if result.returncode != 0:
        log(f"Seal failed for entry {args.entry} (exit {result.returncode}); "
            "stopped state is unconfirmed. Inspect and manually seal using the same config directory.")
        return 1
    log(f"Seal command succeeded for entry {args.entry}; inspect the sealed termination outcome.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
