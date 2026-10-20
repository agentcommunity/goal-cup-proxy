#!/usr/bin/env python3
"""Append one CSV row of soak measurements per interval: disk, chain, ACK lag, state, processes.

WP-F sampler, test-only. Reads `<config>/runs/<run_id>/run.json` and `state.json`, each
entry's trace directory (chain parts and `acks.jsonl`) and the goalcup processes. A row holds
counts, sizes, seconds and closed-set words only: never ids, paths, labels or record text.
"""

import argparse
import csv
from datetime import datetime, timezone
import json
import os
import re
import subprocess
import sys
import time


PART_RE = re.compile(r"chain\.([0-9]{20})\.jsonl")
SEQ_RE = re.compile(rb'"seq":([0-9]+)')
TIME_RE = re.compile(rb'"recorded_at":"([^"]{1,64})"')
STATE_MAX_BYTES = 64 * 1024
RUN_MAX_BYTES = 1024 * 1024
TAIL_START = 256 * 1024
TAIL_MAX = 4 * 1024 * 1024
WORDS = re.compile(r"[a-z_]{1,32}")

COLUMNS = [
    "utc", "sample_errors", "disk_free_bytes",
    "runs", "entries", "chain_parts", "chain_bytes",
    # The newest run's newest entry.
    "entry_parts", "entry_bytes", "entry_first_part_start", "chain_last_seq", "chain_idle_s",
    "acked_seq", "ack_lag_seq", "ack_age_s",
    "state_age_s", "recording", "harness", "ended", "upload_last_ok_age_s", "upload_problem",
    "upload_stopped", "heartbeat_failures", "state_disk_free_bytes", "state_disk_reserve_bytes",
    "state_disk_low",
    "daemon_procs", "daemon_rss_bytes", "daemon_open_files",
    "cli_procs", "cli_rss_bytes", "cli_open_files",
    "other_procs", "other_rss_bytes", "other_open_files",
]


def utc_now():
    return datetime.now(timezone.utc)


def parse_time(raw):
    """RFC 3339 (`...Z`, any fraction) to unix seconds, or None."""
    try:
        text = raw.decode() if isinstance(raw, bytes) else raw
        match = re.fullmatch(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z", text)
        if not match:
            return None
        base = datetime.strptime(match.group(1), "%Y-%m-%dT%H:%M:%S")
        value = base.replace(tzinfo=timezone.utc).timestamp()
        return value + (float(match.group(2)) if match.group(2) else 0.0)
    except (ValueError, UnicodeDecodeError, AttributeError):
        return None


def read_json(path, limit):
    with open(path, "rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("too large")
    return json.loads(data)


def list_parts(trace_dir):
    """[(start, path, size)] by start: `chain.jsonl` is 0, `chain.<20 digits>.jsonl` after it."""
    parts = []
    for name in os.listdir(trace_dir):
        match = PART_RE.fullmatch(name)
        if name == "chain.jsonl":
            start = 0
        elif match and int(match.group(1)) > 0:
            start = int(match.group(1))
        else:
            continue
        path = os.path.join(trace_dir, name)
        try:
            parts.append((start, path, os.stat(path).st_size))
        except OSError:
            continue  # pruned between the listing and the stat
    parts.sort()
    return parts


def last_line(path, size):
    """The last complete line of a file, reading backwards from its end in growing windows."""
    window = TAIL_START
    with open(path, "rb") as stream:
        while True:
            start = max(0, size - window)
            stream.seek(start)
            data = stream.read(size - start)
            end = data.rfind(b"\n")
            if end < 0:
                if start == 0 or window >= TAIL_MAX:
                    return None
                window *= 2
                continue
            begin = data.rfind(b"\n", 0, end) + 1
            if begin > 0 or start == 0:
                return data[begin:end]
            if window >= TAIL_MAX:
                return None
            window *= 2


def chain_tail(parts):
    """(seq, recorded_at seconds) of the last complete record, or (None, None)."""
    for _start, path, size in reversed(parts):
        if size == 0:
            continue
        line = last_line(path, size)
        if line:
            seq = SEQ_RE.search(line)
            when = TIME_RE.search(line)
            return (int(seq.group(1)) if seq else None,
                    parse_time(when.group(1)) if when else None)
    return None, None


def last_ack(trace_dir):
    """(seq, acked_at seconds) of the last parseable line of `acks.jsonl`, or (None, None)."""
    path = os.path.join(trace_dir, "acks.jsonl")
    try:
        size = os.stat(path).st_size
    except OSError:
        return None, None
    if size == 0:
        return None, None
    with open(path, "rb") as stream:
        stream.seek(max(0, size - TAIL_START))
        lines = stream.read().split(b"\n")
    for line in reversed(lines):
        try:
            ack = json.loads(line)
        except ValueError:
            continue
        if isinstance(ack, dict) and isinstance(ack.get("seq"), int):
            return ack["seq"], parse_time(ack.get("acked_at"))
    return None, None


def find_runs(config):
    """[(started_at, run_dir, record)] for every readable run.json, oldest first."""
    runs_dir = os.path.join(config, "runs")
    found = []
    for name in sorted(os.listdir(runs_dir)):
        run_dir = os.path.join(runs_dir, name)
        try:
            record = read_json(os.path.join(run_dir, "run.json"), RUN_MAX_BYTES)
        except (OSError, ValueError):
            continue
        if isinstance(record, dict):
            started = record.get("started_at_unix")
            found.append((started if isinstance(started, int) else 0, run_dir, record))
    found.sort(key=lambda run: run[0])
    return found


def word(value):
    return value if isinstance(value, str) and WORDS.fullmatch(value) else ""


def state_fields(run_dir, now):
    row = {}
    state = read_json(os.path.join(run_dir, "state.json"), STATE_MAX_BYTES)
    if not isinstance(state, dict):
        return row
    if isinstance(state.get("updated_at"), int):
        row["state_age_s"] = int(now) - state["updated_at"]
    row["recording"] = word(state.get("recording"))
    harness = state.get("harness")
    row["harness"] = word(harness) if isinstance(harness, str) else (
        "exited" if isinstance(harness, dict) and "exited" in harness else "")
    row["ended"] = int(bool(state.get("ended")))
    upload = state.get("upload") if isinstance(state.get("upload"), dict) else {}
    if isinstance(upload.get("last_ok_at"), int):
        row["upload_last_ok_age_s"] = int(now) - upload["last_ok_at"]
    # The problem is a sentence that may name a host: only whether there is one.
    row["upload_problem"] = int(upload.get("problem") is not None)
    row["upload_stopped"] = int(bool(upload.get("stopped")))
    heartbeat = state.get("heartbeat") if isinstance(state.get("heartbeat"), dict) else {}
    if isinstance(heartbeat.get("failures"), int):
        row["heartbeat_failures"] = heartbeat["failures"]
    disk = state.get("disk") if isinstance(state.get("disk"), dict) else {}
    for key in ("free_bytes", "reserve_bytes"):
        if isinstance(disk.get(key), int):
            row["state_disk_" + key] = disk[key]
    if "low" in disk:
        row["state_disk_low"] = int(bool(disk.get("low")))
    return row


class Processes:
    """Process readers; the tests replace this with a fake."""

    def __init__(self):
        self.linux = os.path.isdir("/proc/self/fd")

    def list(self):
        """[(pid, argv list)] of every process this user can see."""
        if self.linux:
            found = []
            for name in os.listdir("/proc"):
                if not name.isdigit():
                    continue
                try:
                    with open("/proc/%s/cmdline" % name, "rb") as stream:
                        raw = stream.read(65536)
                except OSError:
                    continue
                argv = [a.decode("utf-8", "replace") for a in raw.split(b"\0") if a]
                if argv:
                    found.append((int(name), argv))
            return found
        out = subprocess.run(["ps", "-axww", "-o", "pid=,command="], capture_output=True,
                             timeout=30, check=False).stdout.decode("utf-8", "replace")
        found = []
        for line in out.splitlines():
            fields = line.strip().split(None, 1)
            if len(fields) == 2 and fields[0].isdigit():
                found.append((int(fields[0]), fields[1].split()))
        return found

    def rss_bytes(self, pid):
        if self.linux:
            with open("/proc/%d/status" % pid, "rb") as stream:
                for line in stream:
                    if line.startswith(b"VmRSS:"):
                        return int(line.split()[1]) * 1024
            return None
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True,
                             timeout=30, check=False).stdout.strip()
        return int(out) * 1024 if out.isdigit() else None

    def open_files(self, pid):
        if self.linux:
            return len(os.listdir("/proc/%d/fd" % pid))
        result = subprocess.run(["lsof", "-n", "-P", "-p", str(pid), "-F", "f"],
                                capture_output=True, timeout=60, check=False)
        if result.returncode != 0 and not result.stdout:
            return None
        return sum(1 for line in result.stdout.splitlines() if line.startswith(b"f"))


def role(argv):
    program = os.path.basename(argv[0]) if argv else ""
    if program == "goalcup-daemon":
        return "daemon"
    if program == "goalcup":
        return "cli"
    return "other"


def process_fields(procs, patterns, errors):
    row = {}
    for name in ("daemon", "cli", "other"):
        row[name + "_procs"] = 0
    me = os.getpid()
    try:
        listed = procs.list()
    except (OSError, subprocess.SubprocessError, ValueError):
        errors.append("process list")
        return row
    for pid, argv in listed:
        line = " ".join(argv)
        if pid == me or "research-sample" in line:
            continue
        if not any(pattern in line for pattern in patterns):
            continue
        kind = role(argv)
        row[kind + "_procs"] += 1
        for column, reader in (("_rss_bytes", procs.rss_bytes), ("_open_files", procs.open_files)):
            try:
                value = reader(pid)
            except (OSError, subprocess.SubprocessError, ValueError):
                value = None  # gone, or not ours to read
            if value is not None:
                row[kind + column] = row.get(kind + column, 0) + value
    return row


def sample(config, procs, patterns, now=None):
    """One row as a dict. Every part fails on its own and counts in `sample_errors`."""
    now = time.time() if now is None else now
    errors = []
    row = {"utc": datetime.fromtimestamp(now, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")}
    try:
        stat = os.statvfs(config)
        row["disk_free_bytes"] = stat.f_bavail * stat.f_frsize
    except OSError:
        errors.append("disk")
    try:
        runs = find_runs(config)
    except OSError:
        runs = []
        errors.append("runs")
    row["runs"] = len(runs)
    entries, total_parts, total_bytes = 0, 0, 0
    newest = None
    for _started, run_dir, record in runs:
        refs = record.get("entries") if isinstance(record.get("entries"), list) else []
        for ref in refs:
            trace_dir = ref.get("trace_dir") if isinstance(ref, dict) else None
            if not isinstance(trace_dir, str):
                continue
            entries += 1
            try:
                parts = list_parts(trace_dir)
            except OSError:
                parts = None
                errors.append("parts")
            if parts is not None:
                total_parts += len(parts)
                total_bytes += sum(part[2] for part in parts)
            newest = (run_dir, trace_dir, parts)
    row.update(entries=entries, chain_parts=total_parts, chain_bytes=total_bytes)
    if newest:
        run_dir, trace_dir, parts = newest
        if parts is not None:
            row["entry_parts"] = len(parts)
            row["entry_bytes"] = sum(part[2] for part in parts)
            if parts:
                row["entry_first_part_start"] = parts[0][0]
            try:
                seq, at = chain_tail(parts)
                if seq is not None:
                    row["chain_last_seq"] = seq
                if at is not None:
                    row["chain_idle_s"] = round(now - at, 1)
            except OSError:
                errors.append("tail")
        try:
            acked, acked_at = last_ack(trace_dir)
            if acked is not None:
                row["acked_seq"] = acked
                if "chain_last_seq" in row:
                    row["ack_lag_seq"] = row["chain_last_seq"] - acked
            if acked_at is not None:
                row["ack_age_s"] = round(now - acked_at, 1)
        except OSError:
            errors.append("acks")
        try:
            row.update(state_fields(run_dir, now))
        except FileNotFoundError:
            pass
        except (OSError, ValueError):
            errors.append("state")
    row.update(process_fields(procs, patterns, errors))
    row["sample_errors"] = len(errors)
    return row, errors


def append_row(out_path, row):
    new = not os.path.exists(out_path) or os.path.getsize(out_path) == 0
    with open(out_path, "a", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=COLUMNS, extrasaction="ignore")
        if new:
            writer.writeheader()
        writer.writerow(row)
        stream.flush()
        os.fsync(stream.fileno())


def log(message):
    stamp = utc_now().isoformat(timespec="seconds")
    print("%s %s" % (stamp, message), file=sys.stderr, flush=True)


def seconds(value):
    try:
        number = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError("seconds must be an integer of at least 1") from None
    if number < 1:
        raise argparse.ArgumentTypeError("seconds must be an integer of at least 1")
    return number


def main(argv=None, *, procs=None, sleep=time.sleep, clock=time.time):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, help="the GOALCUP_CONFIG_DIR to sample")
    parser.add_argument("--out", required=True, help="CSV file to append to")
    parser.add_argument("--every", type=seconds, default=600, help="seconds between samples")
    parser.add_argument("--once", action="store_true", help="write one row and exit")
    parser.add_argument("--pid-match", action="append", default=None,
                        help="substring of a process command line to sample (repeatable; "
                             "default goalcup)")
    args = parser.parse_args(argv)
    if not os.path.isdir(args.config):
        parser.error("config must be an existing directory")
    patterns = args.pid_match or ["goalcup"]
    procs = procs or Processes()
    try:
        while True:
            started = clock()
            try:
                row, errors = sample(args.config, procs, patterns, now=started)
            except Exception as error:  # one bad sample never ends the loop
                row = {"utc": datetime.fromtimestamp(started, timezone.utc)
                       .strftime("%Y-%m-%dT%H:%M:%SZ"), "sample_errors": 1}
                errors = [type(error).__name__]
            try:
                append_row(args.out, row)
            except OSError as error:
                log("could not append the row: %s" % type(error).__name__)
                if args.once:
                    return 1
            if errors:
                log("sample incomplete: %s" % ", ".join(errors))
            if args.once:
                return 0
            sleep(max(1.0, args.every - (clock() - started)))
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
