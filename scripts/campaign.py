#!/usr/bin/env python3
"""Run many long test cells in parallel, with drills, and write machine-readable reports.

    campaign.py check CONFIG [--cells a,b] [--parallel N] [--json] [--static]
    campaign.py build CONFIG
    campaign.py login CONFIG CELL
    campaign.py run CONFIG [--cells a,b] [--parallel N] [--out DIR]
    campaign.py status DIR [--json]
    campaign.py evaluate DIR [--json]

A cell is one organizer test run of one connector, either in Docker (scripts/linux-matrix.sh,
one container and one volume per cell) or on this machine (scripts/matrix.sh with a prepared
configuration directory of the cell's own). `run` starts every selected cell (at most --parallel
at once), starts scripts/research-sample.py beside it, plays the cell's drills at their times,
and writes, per cell, under DIR/<cell>/:

    run.log       the cell's output, each line prefixed with its UTC time and " | "
    events.jsonl  what the campaign did to the cell (start, drills, sampler, end)
    state.json    live state, rewritten as things happen (what `status` reads)
    result.json   the outcome and the pass lines, once the cell has ended
    samples.csv   (host) or logs/samples.csv (docker): the sampler's CSV

and DIR/summary.json plus DIR/summary.md over all cells. `evaluate DIR` recomputes the pass lines
from the files (also for cells still running), so a supervising agent can poll it.

Drills. Docker: `network_cut` disconnects the container from each of its networks and reconnects
it after `seconds`; `pause` is `docker pause` then `docker unpause` (a stand-in for sleep). A
drill is always undone (try/finally; on Ctrl-C too). Host: the campaign never runs a privileged
command; it prints `DRILL-NOW cell=<id> kind=<kind> do: <command>` for the person or agent at the
machine, then `DRILL-END ... do: <undo>` after `seconds`, and records both. `note` only prints.

Secrets. The approval key is only ever a FILE whose path goes to the child scripts in
GOALCUP_TEST_APPROVAL_KEY_FILE. This script never opens it, and the path is never in an argument.
Production (ingest.agentcommunity.org) is refused.

Before website migration 20261021100000 a test run replaces an earlier test run of the same
operator, so cells that can run at the same time must have distinct `test_member_id`s; `check` and
`run` refuse otherwise. Once the server has it, set `"test_runs_parallel": true` in `defaults` and
cells may share a member.
"""

import argparse
import copy
import csv
from datetime import datetime, timedelta, timezone
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parent

STAGING = "https://goalcup-ingest-staging.b-9a5.workers.dev"
PRODUCTION_HOST = "ingest.agentcommunity.org"
KEY_ENV = "GOALCUP_TEST_APPROVAL_KEY_FILE"
CELL_ID = re.compile(r"[a-z0-9][a-z0-9-]{0,39}")
CONNECTOR_ID = re.compile(r"[a-z0-9][a-z0-9-]*")
MEMBER_ID = re.compile(r"[A-Za-z0-9_-]+")
EFFORTS = ("low", "minimal")  # what linux-matrix.sh accepts (LINUX_MATRIX_CODEX_EFFORT)
DRILL_KINDS = ("network_cut", "pause", "note")
WHERE = ("docker", "host")

# Pass lines (docs/plans/2026-10-20-event-readiness.md, WP-F and section 5).
RSS_SLOPE_MIB_PER_H = 1.0
FD_SLOPE_PER_H = 1.0
WARMUP_S = 3600
MIN_POINTS = 3
MIN_SPAN_S = 1800
CHAIN_BYTES_MAX = 2 * 1024 ** 3
RESERVE_NOTE = "the recorder's 1 GiB disk reserve is kept on top of this and is not counted"
ACK_LAG_OK_S = 120
ACK_RECOVERY_WINDOW_S = 25 * 60
INSUFFICIENT = "insufficient data"
CAMPAIGN_TAG = "[campaign]"
# The matrix row (scripts/matrix.sh print_row): connector, platform-arch, harness version, result,
# date. matrix.sh prints `log: <path>` after it, and linux-matrix.sh prefixes it.
MATRIX_ROW = re.compile(r"^\S+ \S+-\S+ \S+ (PASS|FAIL) \d{4}-\d\d-\d\d$")
EARLY_STOP = re.compile(
    r"was stopped: |replaced by a newer run|harness exited|has exited \(|linux-matrix: FAIL")

DEFAULTS = {
    "server": STAGING,
    "duration_s": 7200,
    "sample_every_s": 600,
    "matrix_timeout_slack_s": 3600,
    "goalcup": "goalcup",
    "drills": [],
    "test_runs_parallel": False,
}


class ConfigError(Exception):
    pass


# --------------------------------------------------------------------------- injection points

class RealClock:
    def monotonic(self):
        return time.monotonic()

    def utcnow(self):
        return datetime.now(timezone.utc)

    def wait(self, event, timeout):
        """Wait up to timeout seconds; True when the event is set."""
        return event.wait(max(0.0, timeout))


class RealRunner:
    """Every child process goes through here; never a shell."""

    def popen(self, argv, env=None, cwd=None, stdout=None):
        return subprocess.Popen(
            argv, env=env, cwd=cwd, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE if stdout is None else stdout,
            stderr=subprocess.STDOUT, start_new_session=True)

    def run(self, argv, env=None, timeout=60):
        try:
            done = subprocess.run(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, timeout=timeout, check=False)
        except FileNotFoundError:
            return 127, f"{argv[0]}: not found"
        except subprocess.TimeoutExpired:
            return 124, f"{argv[0]}: timed out after {timeout}s"
        return done.returncode, done.stdout.decode("utf-8", "replace")

    def interactive(self, argv, env=None, cwd=None):
        try:
            return subprocess.call(argv, env=env, cwd=cwd)
        except FileNotFoundError:
            print(f"campaign: {argv[0]}: not found", file=sys.stderr)
            return 127

    def signal_group(self, proc, sig):
        try:
            os.killpg(os.getpgid(proc.pid), sig)
        except (ProcessLookupError, PermissionError, OSError):
            pass


# --------------------------------------------------------------------------- small helpers

def utc(dt):
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ") if dt else None


def parse_time(text):
    """ISO-8601 (Z or offset, any fraction) or epoch seconds; None when neither."""
    text = (text or "").strip()
    if not text:
        return None
    try:
        return datetime.fromtimestamp(float(text), timezone.utc)
    except ValueError:
        pass
    t = text.replace("Z", "+00:00").replace(" ", "T", 1)
    t = re.sub(r"(\.\d{6})\d+", r"\1", t)
    t = re.sub(r"\.(\d{1,5})(?=$|[+-])", lambda m: "." + m.group(1).ljust(6, "0"), t)
    try:
        dt = datetime.fromisoformat(t)
    except ValueError:
        return None
    return dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def write_json(path, doc):
    path = Path(path)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    os.replace(tmp, path)


def read_json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def child_env(extra):
    """The parent's environment without any approval-key variable, plus `extra` (a None value
    removes that variable)."""
    env = {k: v for k, v in os.environ.items()
           if k not in ("GOALCUP_TEST_APPROVAL_KEY", KEY_ENV)}
    for k, v in extra.items():
        if v is None:
            env.pop(k, None)
        else:
            env[k] = str(v)
    return env


def row_of(line):
    """The matrix row in one line of a child's output (without a `linux-matrix: ` prefix), or a
    linux-matrix FAIL / OWNER STEP line; None for anything else."""
    text = (line or "").strip()
    if text.startswith("linux-matrix: "):
        text = text[len("linux-matrix: "):]
        if text.startswith(("FAIL", "OWNER STEP")):
            return text
    return text if MATRIX_ROW.match(text) else None


def container_name(cell):
    return f"goalcup-linux-matrix-{cell['connector']}-{cell['id']}"


volume_name = container_name


# --------------------------------------------------------------------------- configuration

def load_config(path):
    path = Path(path).resolve()
    try:
        doc = json.loads(path.read_text())
    except OSError as e:
        raise ConfigError(f"cannot read {path}: {e.strerror}")
    except ValueError as e:
        raise ConfigError(f"{path} is not JSON: {e}")
    return normalize_config(doc, path.parent, str(path))


def _abs(base, value):
    if value is None:
        return None
    p = Path(os.path.expanduser(str(value)))
    return str(p if p.is_absolute() else (base / p))


def normalize_config(doc, base, source="<config>"):
    if not isinstance(doc, dict) or not isinstance(doc.get("cells"), list):
        raise ConfigError("the config is an object with a `cells` list")
    defaults = dict(DEFAULTS)
    defaults.update(doc.get("defaults") or {})
    cfg = {
        "source": source,
        "name": str(doc.get("name") or Path(source).stem),
        "server": str(defaults.get("server") or STAGING).rstrip("/"),
        "approval_key_file": _abs(base, defaults.get("approval_key_file") or os.environ.get(KEY_ENV)),
        "sample_every_s": defaults.get("sample_every_s"),
        "matrix_timeout_slack_s": defaults.get("matrix_timeout_slack_s"),
        "goalcup": defaults.get("goalcup"),
        "bin_dir": _abs(base, defaults.get("bin_dir")),
        "test_runs_parallel": defaults.get("test_runs_parallel") is True,
        "cells": [],
    }
    cell_keys = ("where", "connector", "duration_s", "test_member_id", "prompt_file", "config_dir",
                 "codex_effort", "drills")
    for raw in doc["cells"]:
        if not isinstance(raw, dict):
            raise ConfigError("every cell is an object")
        cell = {k: defaults.get(k) for k in cell_keys}
        cell.update(raw)
        cell["id"] = str(raw.get("id", ""))
        cell["prompt_file"] = _abs(base, cell.get("prompt_file"))
        cell["config_dir"] = _abs(base, cell.get("config_dir"))
        cell["drills"] = [dict(d) for d in (cell.get("drills") or [])]
        cfg["cells"].append(cell)
    return cfg


def select_cells(cfg, names):
    if not names:
        return list(cfg["cells"])
    want = [n.strip() for n in names.split(",") if n.strip()]
    known = {c["id"]: c for c in cfg["cells"]}
    missing = [n for n in want if n not in known]
    if missing:
        raise ConfigError("no such cell: " + ", ".join(missing))
    return [known[n] for n in want]


def validate(cfg, cells, parallel):
    """Static checks (no docker, no network). Returns (errors, warnings)."""
    errors, warnings = [], []
    server = cfg["server"]
    host = re.sub(r"^[a-z]+://", "", server).split("/")[0].split(":")[0].lower()
    if host == PRODUCTION_HOST or host.endswith("." + PRODUCTION_HOST) or PRODUCTION_HOST in server:
        errors.append(f"server {server} is production; a campaign runs on staging only")
    elif not server.startswith("https://") and host not in ("localhost", "127.0.0.1"):
        errors.append(f"server {server} must be https://")
    elif "staging" not in host and host not in ("localhost", "127.0.0.1"):
        warnings.append(f"server {server} does not look like staging")
    if os.environ.get("GOALCUP_TEST_APPROVAL_KEY"):
        errors.append("GOALCUP_TEST_APPROVAL_KEY in the environment is refused; give a file")
    key = cfg.get("approval_key_file")
    if not key:
        errors.append(f"no approval key file: set defaults.approval_key_file or {KEY_ENV}")
    elif not os.path.isabs(key) or ":" in key or "," in key:
        errors.append("the approval key file must be an absolute path without : or ,")
    elif not (os.path.isfile(key) and os.access(key, os.R_OK)):
        errors.append("the approval key file is not a regular file this user can read")
    for name in ("sample_every_s",):
        v = cfg.get(name)
        if not isinstance(v, int) or isinstance(v, bool) or v < 1:
            errors.append(f"defaults.{name} must be a positive integer")
    slack = cfg.get("matrix_timeout_slack_s")
    if slack is not None and (not isinstance(slack, int) or isinstance(slack, bool) or slack < 0):
        errors.append("defaults.matrix_timeout_slack_s must be an integer of at least 0")

    seen_ids, dirs, volumes, members = set(), {}, {}, {}
    for cell in cells:
        cid = cell["id"]
        tag = f"cell {cid or '?'}"
        if not CELL_ID.fullmatch(cid):
            errors.append(f"{tag}: id must match [a-z0-9][a-z0-9-]* (at most 40)")
        elif cid in seen_ids:
            errors.append(f"{tag}: duplicate id")
        seen_ids.add(cid)
        where = cell.get("where")
        if where not in WHERE:
            errors.append(f"{tag}: where must be docker or host")
        if not CONNECTOR_ID.fullmatch(str(cell.get("connector") or "")):
            errors.append(f"{tag}: connector must match [a-z0-9-]+")
        d = cell.get("duration_s")
        if not isinstance(d, int) or isinstance(d, bool) or d < 60:
            errors.append(f"{tag}: duration_s must be an integer of at least 60")
            d = None
        member = cell.get("test_member_id")
        if not member or not MEMBER_ID.fullmatch(str(member)):
            errors.append(f"{tag}: test_member_id is required (letters, digits, _ -)")
        else:
            members.setdefault(member, []).append(cid)
        pf = cell.get("prompt_file")
        if pf and not (os.path.isfile(pf) and os.access(pf, os.R_OK)):
            errors.append(f"{tag}: prompt_file {pf} is not a readable file")
        eff = cell.get("codex_effort")
        if eff is not None and eff not in EFFORTS:
            errors.append(f"{tag}: codex_effort must be one of {', '.join(EFFORTS)}")
        elif eff is not None and where == "host":
            warnings.append(f"{tag}: codex_effort is not applied to a host cell (its config_dir's "
                            "Codex home sets the effort)")
        if where == "host":
            cd = cell.get("config_dir")
            if not cd:
                errors.append(f"{tag}: a host cell needs config_dir")
            else:
                real = os.path.realpath(cd)
                if real in dirs:
                    errors.append(f"{tag}: config_dir {cd} is also cell {dirs[real]}'s")
                dirs[real] = cid
                if not os.path.isdir(cd):
                    errors.append(f"{tag}: config_dir {cd} does not exist (campaign.py login first)")
        elif where == "docker":
            if cell.get("config_dir"):
                errors.append(f"{tag}: config_dir is for host cells (a docker cell has a volume)")
            if cell.get("connector"):
                vol = volume_name(cell)
                if vol in volumes:
                    errors.append(f"{tag}: volume {vol} is also cell {volumes[vol]}'s")
                volumes[vol] = cid
        errors.extend(f"{tag}: {e}" for e in validate_drills(cell["drills"], d))
    if min(parallel, len(cells)) > 1 and not cfg.get("test_runs_parallel"):
        for member, ids in members.items():
            if len(ids) > 1:
                errors.append(
                    f"cells {', '.join(ids)} share test_member_id {member} and could run at the "
                    "same time; a test run replaces the same operator's earlier one. Give each "
                    "a distinct test operator, or use --parallel 1")
    return errors, warnings


def validate_drills(drills, duration):
    errors = []
    spans = []
    for i, d in enumerate(drills):
        kind = d.get("kind")
        at, secs = d.get("at_s"), d.get("seconds", 0)
        if kind not in DRILL_KINDS:
            errors.append(f"drill {i}: kind must be one of {', '.join(DRILL_KINDS)}")
            continue
        if not isinstance(at, (int, float)) or isinstance(at, bool) or at < 0:
            errors.append(f"drill {i}: at_s must be a number >= 0")
            continue
        if not isinstance(secs, (int, float)) or isinstance(secs, bool) or secs < 0:
            errors.append(f"drill {i}: seconds must be a number >= 0")
            continue
        if kind != "note" and secs <= 0:
            errors.append(f"drill {i}: a {kind} needs seconds > 0")
        if duration and at + secs > duration:
            errors.append(f"drill {i}: ends after the cell's duration")
        if kind != "note":
            spans.append((at, at + secs, i))
    spans.sort()
    for (a0, a1, i), (b0, _, j) in zip(spans, spans[1:]):
        if b0 < a1:
            errors.append(f"drills {i} and {j} overlap")
    return errors


def live_checks(cfg, cells, runner):
    """What can be seen without starting anything: docker, volumes, host sign-ins."""
    errors, warnings, info = [], [], []
    for script in ("matrix.sh", "linux-matrix.sh", "research-sample.py"):
        if not (HERE / script).is_file():
            warnings.append(f"scripts/{script} is missing")
    if any(c["where"] == "docker" for c in cells):
        rc, out = runner.run(["docker", "version", "--format", "{{.Server.Version}}"], timeout=30)
        if rc != 0:
            errors.append(f"docker is not usable: {out.strip()[:200]}")
        else:
            for c in cells:
                if c["where"] != "docker":
                    continue
                vol = volume_name(c)
                rc, _ = runner.run(["docker", "volume", "inspect", vol], timeout=30)
                if rc != 0:
                    warnings.append(f"cell {c['id']}: volume {vol} does not exist "
                                    f"(campaign.py login CONFIG {c['id']})")
                else:
                    info.append(f"cell {c['id']}: volume {vol} exists (sign-in not detectable "
                                "from outside a container)")
    for c in cells:
        if c["where"] != "host" or not c.get("config_dir"):
            continue
        prep = read_json(os.path.join(c["config_dir"], "prep.json")) or {}
        if prep.get("harness") == c["connector"] and prep.get("home_ready"):
            info.append(f"cell {c['id']}: {c['connector']} signed in (prep.json home_ready)")
        else:
            warnings.append(f"cell {c['id']}: prep.json does not record a {c['connector']} "
                            f"sign-in (campaign.py login CONFIG {c['id']})")
    return errors, warnings, info


# --------------------------------------------------------------------------- commands for a cell

def run_command(cfg, cell, cell_dir):
    """(argv, env) for one cell's run. The key path is only in env, never in argv."""
    d = int(cell["duration_s"])
    extra = []
    slack = cfg.get("matrix_timeout_slack_s")
    if slack:
        extra = ["--timeout", str(d + int(slack))]
    env = {KEY_ENV: cfg["approval_key_file"], "GOALCUP_TEST_MEMBER_ID": cell["test_member_id"]}
    if cell.get("prompt_file"):
        env["MATRIX_PROMPT_FILE"] = cell["prompt_file"]
        env["MATRIX_PROMPT"] = None  # matrix.sh refuses both
    if cell["where"] == "docker":
        logs = Path(cell_dir) / "logs"
        env.update({
            "LINUX_MATRIX_CELL": cell["id"],
            "LINUX_MATRIX_PREBUILT": "1",
            "LINUX_MATRIX_SERVER": cfg["server"],
            "LINUX_MATRIX_LOG_DIR": str(logs),
        })
        if cell.get("codex_effort"):
            env["LINUX_MATRIX_CODEX_EFFORT"] = cell["codex_effort"]
        argv = ["sh", str(HERE / "linux-matrix.sh"), "run", cell["connector"], "--",
                "--test", "--duration", str(d)] + extra
    else:
        env["MATRIX_CONFIG_DIR"] = cell["config_dir"]
        if cfg.get("bin_dir"):
            env["MATRIX_BIN_DIR"] = cfg["bin_dir"]
        argv = ["sh", str(HERE / "matrix.sh"), cell["connector"], cfg["server"],
                "--test", "--duration", str(d)] + extra + [
                "--log", str(Path(cell_dir) / "matrix.log")]
    return argv, env


def sampler_command(cfg, cell, cell_dir):
    """(argv, env, host path of the CSV)."""
    every = str(cfg["sample_every_s"])
    if cell["where"] == "docker":
        argv = ["docker", "exec", "-u", "dev", container_name(cell), "python3",
                "/src/scripts/research-sample.py", "--config", "/vol/cfg",
                "--out", "/logs/samples.csv", "--every", every]
        return argv, child_env({}), str(Path(cell_dir) / "logs" / "samples.csv")
    out = str(Path(cell_dir) / "samples.csv")
    # Only this cell's processes: its daemon runs as `goalcup-daemon --config <dir>/daemon.toml`,
    # so another cell's (or anyone's) goalcup on this machine is not counted.
    own = cell["config_dir"].rstrip("/") + "/"
    argv = [sys.executable, str(HERE / "research-sample.py"), "--config", cell["config_dir"],
            "--out", out, "--every", every, "--pid-match", own]
    return argv, child_env({}), out


def host_instructions(kind, seconds, platform=None):
    platform = platform or sys.platform
    wsl = False
    try:
        wsl = "microsoft" in Path("/proc/version").read_text().lower()
    except OSError:
        pass
    if kind == "network_cut":
        if platform == "darwin":
            return ("networksetup -setairportpower en0 off  (and unplug any cable)",
                    "networksetup -setairportpower en0 on")
        if wsl:
            return ("turn the Windows machine's network off (Wi-Fi off / cable out)",
                    "turn the Windows machine's network back on")
        return ("sudo nmcli networking off  (or: sudo ip link set <if> down)",
                "sudo nmcli networking on  (or: sudo ip link set <if> up)")
    if kind == "pause":
        if platform == "darwin":
            return (f"sudo pmset sleepnow  (sleep about {int(seconds)}s)", "wake the Mac")
        return (f"sudo systemctl suspend  (sleep about {int(seconds)}s)", "wake the machine")
    return ("note", "")


# --------------------------------------------------------------------------- drills

class DockerDrills:
    """Begin/end of a drill on a container. end() is idempotent and always attempted."""

    RETRIES = 3

    def __init__(self, runner, container, clock, say=None, cell_id=""):
        self.runner, self.container, self.clock = runner, container, clock
        self.say, self.cell_id = say or (lambda line: None), cell_id

    def networks(self):
        rc, out = self.runner.run(
            ["docker", "inspect", "-f", "{{json .NetworkSettings.Networks}}", self.container])
        if rc != 0:
            raise RuntimeError(f"docker inspect {self.container}: {out.strip()[:200]}")
        try:
            nets = sorted(json.loads(out.strip() or "{}").keys())
        except ValueError:
            raise RuntimeError("docker inspect gave no network list")
        if not nets:
            raise RuntimeError(f"{self.container} is on no network")
        return nets

    def begin(self, drill, ctx):
        kind = drill["kind"]
        self.say(f"DRILL-NOW cell={self.cell_id} kind={kind} seconds={drill.get('seconds', 0)} "
                 f"do: {(drill.get('text') or 'note') if kind == 'note' else 'automatic (docker)'}")
        if kind == "network_cut":
            nets = self.networks()
            ctx["networks"] = []
            for net in nets:
                ctx["networks"].append(net)
                rc, out = self.runner.run(["docker", "network", "disconnect", net, self.container])
                ctx.setdefault("actions", []).append(
                    {"utc": utc(self.clock.utcnow()), "do": f"disconnect {net}", "rc": rc})
                if rc != 0:
                    raise RuntimeError(f"disconnect {net}: {out.strip()[:200]}")
        elif kind == "pause":
            ctx["paused"] = True
            rc, out = self.runner.run(["docker", "pause", self.container])
            ctx.setdefault("actions", []).append(
                {"utc": utc(self.clock.utcnow()), "do": "pause", "rc": rc})
            if rc != 0:
                raise RuntimeError(f"pause: {out.strip()[:200]}")

    def _undo(self, argv, ok_text, what, ctx):
        for _ in range(self.RETRIES):
            rc, out = self.runner.run(argv)
            ctx.setdefault("actions", []).append(
                {"utc": utc(self.clock.utcnow()), "do": what, "rc": rc})
            if rc == 0 or any(t in out.lower() for t in ok_text):
                return True
            if "no such container" in out.lower():
                return True
        return False

    def end(self, drill, ctx):
        ok = True
        if drill["kind"] == "network_cut":
            for net in ctx.get("networks", []):
                ok &= self._undo(["docker", "network", "connect", net, self.container],
                                 ("already exists", "already connected"), f"connect {net}", ctx)
        elif drill["kind"] == "pause" and ctx.get("paused"):
            ok &= self._undo(["docker", "unpause", self.container], ("is not paused",),
                             "unpause", ctx)
        if not ok:
            raise RuntimeError("UNDO FAILED: the container may still be cut off or paused")


class HostDrills:
    """Prints the instruction for whoever is at the machine; never runs it."""

    def __init__(self, say, cell_id, clock, platform=None):
        self.say, self.cell_id, self.clock, self.platform = say, cell_id, clock, platform

    def begin(self, drill, ctx):
        do, undo = host_instructions(drill["kind"], drill.get("seconds", 0), self.platform)
        if drill["kind"] == "note":
            do = drill.get("text") or "note"
        ctx["manual"] = True
        ctx["instruction"] = do
        ctx["undo_instruction"] = undo
        self.say(f"DRILL-NOW cell={self.cell_id} kind={drill['kind']} "
                 f"seconds={drill.get('seconds', 0)} do: {do}")

    def end(self, drill, ctx):
        if drill["kind"] != "note":
            self.say(f"DRILL-END cell={self.cell_id} kind={drill['kind']} "
                     f"do: {ctx.get('undo_instruction', '')}")


class DrillScheduler:
    def __init__(self, cell_id, drills, executor, clock, start_mono, start_utc, stop,
                 is_alive=lambda: True, record=None, registry=None):
        self.cell_id, self.executor, self.clock = cell_id, executor, clock
        self.drills = sorted((dict(d) for d in drills), key=lambda d: d["at_s"])
        self.start_mono, self.start_utc, self.stop = start_mono, start_utc, stop
        self.is_alive = is_alive
        self.record = record or (lambda rec: None)
        self.registry = registry if registry is not None else {}
        self.performed = []

    def run(self):
        for drill in self.drills:
            target = self.start_mono + drill["at_s"]
            stopped = False
            while True:
                remaining = target - self.clock.monotonic()
                if remaining <= 0:
                    break
                if self.clock.wait(self.stop, min(remaining, 5.0)):
                    stopped = True
                    break
            if stopped or self.stop.is_set() or not self.is_alive():
                self._skip(drill, "the cell had ended or the campaign was stopped")
                continue
            self.perform(drill)
        return self.performed

    def _skip(self, drill, why):
        rec = self._rec(drill)
        rec.update(status="skipped", reason=why)
        self.performed.append(rec)
        self.record(rec)

    def _rec(self, drill):
        planned = self.start_utc + timedelta(seconds=drill["at_s"])
        return {"kind": drill["kind"], "at_s": drill["at_s"], "seconds": drill.get("seconds", 0),
                "planned_utc": utc(planned), "started_utc": None, "ended_utc": None,
                "status": "pending"}

    def perform(self, drill):
        rec = self._rec(drill)
        ctx = {"undone": False, "lock": threading.Lock()}
        key = (self.cell_id, len(self.performed))
        self.registry[key] = (self.executor, drill, ctx, rec)
        rec["started_utc"] = utc(self.clock.utcnow())
        rec["status"] = "running"
        self.record(dict(rec, event="drill-start"))
        try:
            self.executor.begin(drill, ctx)
            if drill.get("seconds", 0) > 0:
                self.clock.wait(self.stop, drill["seconds"])
            rec["status"] = "manual" if ctx.get("manual") else "done"
        except Exception as e:  # recorded; the finally undoes it
            rec["status"] = "error"
            rec["error"] = str(e)
        finally:
            undo_drill(self.executor, drill, ctx, rec)
            rec["ended_utc"] = utc(self.clock.utcnow())
            self.registry.pop(key, None)
            rec["actions"] = ctx.get("actions", [])
            for k in ("instruction", "undo_instruction"):
                if k in ctx:
                    rec[k] = ctx[k]
            if self.stop.is_set() and rec["status"] in ("done", "manual"):
                rec["status"] = "cut short"
            self.performed.append(rec)
            self.record(dict(rec, event="drill-end"))


def undo_drill(executor, drill, ctx, rec):
    with ctx["lock"]:
        if ctx["undone"]:
            return
        ctx["undone"] = True
        try:
            executor.end(drill, ctx)
        except Exception as e:
            rec["status"] = "error"
            rec["undo_error"] = str(e)


# --------------------------------------------------------------------------- sample analysis
#
# The CSV is scripts/research-sample.py's (its COLUMNS): one row per sample, one column set per
# process role. Units, from that script: *_bytes are bytes; *_s are seconds; ack_lag_seq is a
# number of RECORDS (chain_last_seq - acked_seq of the newest entry), not seconds; ack_age_s is
# seconds since the newest entry's last ACK; chain_bytes is the total on disk across all runs;
# entry_bytes is the newest entry's; *_rss_bytes and *_open_files are sums over the role's
# processes, with *_procs saying how many there were.

ROLES = ("daemon", "cli", "other")


def read_samples(path):
    """The sampler's CSV as a list of row dicts, in file order; [] when absent."""
    try:
        with open(path, newline="") as f:
            return [{(k or "").strip(): (v or "").strip() for k, v in row.items()}
                    for row in csv.DictReader(f)]
    except OSError:
        return []


def _num(text):
    try:
        v = float(text)
    except (TypeError, ValueError):
        return None
    return v if math.isfinite(v) else None


def series_from_samples(rows):
    """Explicit columns of research-sample.py, as time series of (unix seconds, value)."""
    out = {"rss": {}, "fd": {}, "chain": [], "entry": [], "free": [], "ack": [], "flags": [],
           "times": []}
    for row in rows:
        t = parse_time(row.get("utc", ""))
        if t is None:
            continue
        ts = t.timestamp()
        out["times"].append(ts)
        for role in ROLES:
            procs = _num(row.get(f"{role}_procs"))
            if not procs:
                continue
            # The columns are sums over the role's processes: a change in their number is a
            # different series, never a leak or a drop within one.
            key = f"{role} (procs={int(procs)})"
            for kind, col in (("rss", f"{role}_rss_bytes"), ("fd", f"{role}_open_files")):
                v = _num(row.get(col))
                if v is not None:
                    out[kind].setdefault(key, []).append((ts, v))
        for kind, col in (("chain", "chain_bytes"), ("entry", "entry_bytes"),
                          ("free", "disk_free_bytes")):
            v = _num(row.get(col))
            if v is not None:
                out[kind].append((ts, v))
        lag, age = _num(row.get("ack_lag_seq")), _num(row.get("ack_age_s"))
        if lag is not None or age is not None:
            out["ack"].append((ts, lag, age))
        out["flags"].append({
            "t": ts, "utc": row.get("utc"),
            "upload_stopped": _num(row.get("upload_stopped")) == 1,
            "ended": _num(row.get("ended")) == 1,
            "upload_problem": _num(row.get("upload_problem")) == 1,
            "heartbeat_failures": _num(row.get("heartbeat_failures")),
        })
    return out


def upload_facts(s):
    """upload_stopped, upload_problem, heartbeat_failures and ended, for result.json."""
    flags = s["flags"]
    first = lambda key: next((f["utc"] for f in flags if f[key]), None)  # noqa: E731
    hb = [f["heartbeat_failures"] for f in flags if f["heartbeat_failures"] is not None]
    last = flags[-1] if flags else {}
    return {
        "samples": len(flags),
        "upload_stopped_samples": sum(f["upload_stopped"] for f in flags),
        "first_upload_stopped_utc": first("upload_stopped"),
        "upload_problem_samples": sum(f["upload_problem"] for f in flags),
        "first_upload_problem_utc": first("upload_problem"),
        "heartbeat_failures_max": None if not hb else int(max(hb)),
        "heartbeat_failures_last": None if not hb else int(hb[-1]),
        "ended_samples": sum(f["ended"] for f in flags),
        "first_ended_utc": first("ended"),
        "last_sample_utc": last.get("utc"),
    }

def slope_per_hour(points):
    """Least-squares slope of (t seconds, y) in y per hour."""
    n = len(points)
    if n < 2:
        return None
    xs = [p[0] / 3600.0 for p in points]
    ys = [p[1] for p in points]
    mx, my = sum(xs) / n, sum(ys) / n
    var = sum((x - mx) ** 2 for x in xs)
    if var == 0:
        return None
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / var


def slope_line(name, series, threshold, unit, scale=1.0):
    per = {}
    worst = None
    short = []
    for key, pts in sorted(series.items()):
        if not pts:
            continue
        t0 = pts[0][0]
        late = [p for p in pts if p[0] >= t0 + WARMUP_S]
        if len(late) < MIN_POINTS or late[-1][0] - late[0][0] < MIN_SPAN_S:
            per[key] = {"status": INSUFFICIENT, "points_after_warmup": len(late)}
            short.append(key)
            continue
        s = slope_per_hour(late) / scale
        per[key] = {"slope": round(s, 4), "points_after_warmup": len(late),
                    "status": "pass" if s < threshold else "fail"}
        worst = s if worst is None else max(worst, s)
    if worst is None:
        status = INSUFFICIENT
    elif any(v["status"] == "fail" for v in per.values()):
        status = "fail"
    else:
        status = "pass"
    detail = (f"least-squares slope per process after the first {WARMUP_S // 60} min; "
              f"needs {MIN_POINTS}+ samples over {MIN_SPAN_S // 60}+ min")
    if short and worst is not None:
        detail += f"; too few samples for: {', '.join(short)}"
    return {"name": name, "value": None if worst is None else round(worst, 4), "unit": unit,
            "threshold": f"< {threshold} {unit}", "status": status, "per_process": per,
            "detail": detail}


def evaluate_lines(info, samples_path, run_log=None):
    """The pass lines for one cell. `info` holds the result fields (verdict, exit_code, ...)."""
    lines = []
    running = info.get("state") == "running"
    rows = read_samples(samples_path) if samples_path else []
    s = series_from_samples(rows)
    note = f"{len(s['times'])} samples in {samples_path}" if samples_path else "no sampler file"
    # 1. The run ended PASS.
    row = info.get("final_row")
    if running:
        st = "running"
    else:
        st = "pass" if info.get("verdict") == "PASS" else "fail"
    lines.append({"name": "run_pass", "value": row, "unit": "matrix row",
                  "threshold": "exit 0 and a PASS row", "status": st,
                  "detail": f"exit code {info.get('exit_code')}"})
    # 2. No stop before the duration (a drill-caused stop is still a FAIL): a stop line in the
    # log, a sample with upload_stopped=1 or ended=1 before start + duration, or a short run.
    markers = []
    if run_log:
        try:
            with open(run_log, errors="replace") as f:
                for line in f:
                    text = line.split(" | ", 1)[-1].rstrip("\n")
                    if not text.startswith(CAMPAIGN_TAG) and EARLY_STOP.search(text):
                        markers.append(text[:200])
        except OSError:
            pass
    elapsed = info.get("elapsed_s")
    duration = info.get("duration_s")
    start = parse_time(info.get("start_utc") or "")
    t0 = start.timestamp() if start else (s["times"][0] if s["times"] else None)
    early_samples = []
    if t0 is not None and duration:
        for f in s["flags"]:
            if f["t"] < t0 + duration and (f["upload_stopped"] or f["ended"]):
                early_samples.append({"utc": f["utc"], "upload_stopped": f["upload_stopped"],
                                      "ended": f["ended"]})
    if markers or early_samples:
        st = "fail"
    elif running:
        st = "running"
    elif info.get("interrupted"):
        st = "fail"
    elif elapsed is None or duration is None:
        st = INSUFFICIENT
    else:
        st = "pass" if elapsed >= duration else "fail"
    lines.append({"name": "no_early_stop", "value": elapsed, "unit": "s",
                  "threshold": f">= {duration} s; no stop line, and no sample with "
                               "upload_stopped=1 or ended=1, before start + duration",
                  "status": st, "early_stop_lines": markers[:10],
                  "early_stop_samples": early_samples[:10],
                  "detail": "a stop caused by a drill is still a FAIL; start is the cell's "
                            "start (before the run's own start, so never late)"})
    # 3. RSS slope; 4. open files.
    lines.append(slope_line("rss_slope", s["rss"], RSS_SLOPE_MIB_PER_H, "MiB/h", 1024 ** 2))
    lines.append(slope_line("open_files_slope", s["fd"], FD_SLOPE_PER_H, "files/h"))
    # 5. Local chain bytes (chain_bytes: every run's parts on disk).
    base = {"name": "chain_bytes_max", "unit": "bytes",
            "threshold": f"< {CHAIN_BYTES_MAX} bytes (2 GiB)",
            "detail": "chain_bytes: all runs' chain parts on disk; " + RESERVE_NOTE}
    if s["chain"]:
        mx = max(v for _, v in s["chain"])
        lines.append(dict(base, value=int(mx), status="pass" if mx < CHAIN_BYTES_MAX else "fail",
                          entry_bytes_max=int(max(v for _, v in s["entry"])) if s["entry"] else None,
                          min_free_disk_bytes=int(min(v for _, v in s["free"])) if s["free"] else None))
    else:
        lines.append(dict(base, value=None, status=INSUFFICIENT))
    # 6. ACK lag recovers after every network cut.
    lines.append(ack_line(info.get("drills") or [], s["ack"], s["times"]))
    for line in lines:
        line.setdefault("samples", note)
    return lines


ACK_RULE = (f"recovered at the first sample at or after the cut's end where ack_lag_seq == 0 "
            f"(every record on disk acknowledged), or where ack_age_s <= {ACK_LAG_OK_S} with "
            "ack_lag_seq not above the previous sample's (ACKs arriving and the backlog not "
            f"growing); it must come within {ACK_RECOVERY_WINDOW_S // 60} min of the cut's end. "
            "ack_lag_seq counts records, not seconds")


def upload_facts_for(samples_path):
    return upload_facts(series_from_samples(read_samples(samples_path) if samples_path else []))


def ack_recovered(prev, cur):
    _, lag, age = cur
    if lag is not None and lag == 0:
        return True
    if age is None or age > ACK_LAG_OK_S or lag is None:
        return False
    prev_lag = prev[1] if prev is not None else None
    return prev_lag is not None and lag <= prev_lag


def ack_line(drills, ack, times):
    cuts = [d for d in drills if d.get("kind") == "network_cut"
            and d.get("status") not in ("skipped", "pending")]
    base = {"name": "ack_recovery", "unit": "s", "rule": ACK_RULE,
            "threshold": f"recovered within {ACK_RECOVERY_WINDOW_S // 60} min after each "
                         "network cut ends"}
    if not cuts:
        return dict(base, value=None, status="not applicable", per_cut=[],
                    detail="no network_cut drill ran")
    last = times[-1] if times else None
    per = []
    for d in cuts:
        end = parse_time(d.get("ended_utc") or "")
        if end is None:
            start = parse_time(d.get("started_utc") or d.get("planned_utc") or "")
            end = start + timedelta(seconds=d.get("seconds", 0)) if start else None
        if end is None:
            per.append({"cut_end_utc": None, "status": INSUFFICIENT})
            continue
        e = end.timestamp()
        found, prev, in_window = None, None, 0
        for cur in ack:
            if cur[0] >= e:
                if cur[0] > e + ACK_RECOVERY_WINDOW_S:
                    break
                in_window += 1
                if ack_recovered(prev, cur):
                    found = cur
                    break
            prev = cur
        rec = {"cut_end_utc": utc(end), "samples_in_window": in_window}
        if found:
            rec.update(status="pass", recovered_after_s=round(found[0] - e, 1),
                       ack_lag_seq=found[1], ack_age_s=found[2])
        elif last is None or last < e + ACK_RECOVERY_WINDOW_S:
            rec.update(status=INSUFFICIENT)
        else:
            rec.update(status="fail")
        if d.get("status") == "manual":
            rec["note"] = "host drill done by hand: the end time is the instruction's, not measured"
        per.append(rec)
    statuses = [p["status"] for p in per]
    status = "fail" if "fail" in statuses else (INSUFFICIENT if INSUFFICIENT in statuses else "pass")
    worst = max((p.get("recovered_after_s", 0) for p in per if p["status"] == "pass"), default=None)
    return dict(base, value=worst, status=status, per_cut=per,
                detail="value: the slowest recovery, seconds after the cut ended")


def overall(lines):
    sts = [l["status"] for l in lines]
    if "fail" in sts:
        return "FAIL"
    if "running" in sts:
        return "RUNNING"
    if INSUFFICIENT in sts:
        return "INCOMPLETE"
    return "PASS"


# --------------------------------------------------------------------------- the campaign

class CellRun:
    def __init__(self, camp, cell):
        self.camp, self.cell = camp, cell
        self.dir = Path(camp.out) / cell["id"]
        self.dir.mkdir(parents=True, exist_ok=True)
        self.stop = threading.Event()
        self.done = threading.Event()
        self.proc = None
        self.sampler = None
        self.sampler_info = {"path": None, "status": "not started"}
        self.drills = []
        self.state = {"cell": cell["id"], "where": cell["where"], "connector": cell["connector"],
                      "duration_s": cell["duration_s"], "state": "pending", "drills": [],
                      "active_drill": None, "last_line": None, "last_line_utc": None}
        self.lock = threading.Lock()
        self.last_state_write = None
        self.start_mono = None
        self.start_utc = None
        self.final_row = None
        self.result = None
        self.write_state(force=True)

    # ---- files
    def event(self, name, /, **fields):
        doc = {"utc": utc(self.camp.clock.utcnow()), "event": name}
        doc.update(fields)
        with self.lock, open(self.dir / "events.jsonl", "a") as f:
            f.write(json.dumps(doc, sort_keys=True) + "\n")

    def set_state(self, **fields):
        with self.lock:
            self.state.update(fields)

    def write_state(self, force=False):
        now = self.camp.clock.monotonic()
        with self.lock:
            if not force and self.last_state_write is not None and now - self.last_state_write < 5:
                return
            self.last_state_write = now
            snapshot = copy.deepcopy(self.state)
            write_json(self.dir / "state.json", snapshot)

    def logline(self, text, own=False):
        """One line into run.log; the campaign's own lines (drills) are tagged [campaign]."""
        stamp = utc(self.camp.clock.utcnow())
        if own:
            text = f"{CAMPAIGN_TAG} {text}"
        with self.lock:
            with open(self.dir / "run.log", "a", encoding="utf-8", errors="replace") as f:
                f.write(f"{stamp} | {text}\n")

    # ---- the thread
    def run(self):
        try:
            if not self.camp.acquire_slot(self.stop):
                self.set_state(state="not started")
                self.write_result(exit_code=None, interrupted=True, started=False)
                return
            try:
                self._run()
            finally:
                self.camp.release_slot()
        except Exception as e:  # one cell's failure never stops the others
            self.event("error", error=repr(e))
            self.camp.say(f"campaign: cell {self.cell['id']} error: {e}")
            if self.result is None:
                self.write_result(exit_code=None, error=repr(e))
        finally:
            self.done.set()
            self.camp.cell_finished()

    def _run(self):
        camp, cell = self.camp, self.cell
        argv, extra = run_command(camp.cfg, cell, self.dir)
        if cell["where"] == "docker":
            (self.dir / "logs").mkdir(exist_ok=True)
        self.start_mono = camp.clock.monotonic()
        self.start_utc = camp.clock.utcnow()
        self.set_state(state="running", start_utc=utc(self.start_utc))
        self.proc = camp.runner.popen(argv, env=child_env(extra), cwd=str(REPO))
        self.set_state(pid=getattr(self.proc, "pid", None))
        self.event("start", argv=argv)
        self.write_state(force=True)
        camp.say(f"campaign: cell {cell['id']} started ({cell['where']} {cell['connector']}, "
                 f"{cell['duration_s']}s)")
        reader = threading.Thread(target=self._read, name=f"read-{cell['id']}", daemon=True)
        reader.start()
        drill_thread = threading.Thread(target=self._drills, name=f"drill-{cell['id']}", daemon=True)
        drill_thread.start()
        sampler_thread = threading.Thread(target=self._start_sampler, name=f"sample-{cell['id']}",
                                          daemon=True)
        sampler_thread.start()
        rc = self.proc.wait()
        reader.join(timeout=10)
        self.stop.set()
        drill_thread.join(timeout=120)
        sampler_thread.join(timeout=30)
        self.stop_sampler()
        self.write_result(exit_code=rc, interrupted=camp.stopping.is_set())

    def _read(self):
        for raw in self.proc.stdout:
            # One bad line (a full disk, a state write that failed) never stops the draining:
            # a child whose pipe is not read blocks.
            try:
                text = raw.decode("utf-8", "replace") if isinstance(raw, bytes) else raw
                text = text.rstrip("\r\n")
                row = row_of(text)
                if row:
                    self.final_row = row
                self.logline(text)
                if text.strip():
                    self.set_state(last_line=text[-300:],
                                   last_line_utc=utc(self.camp.clock.utcnow()))
                    self.write_state()
            except Exception as e:  # noqa: BLE001 - keep reading
                try:
                    self.event("reader-error", error=repr(e))
                except Exception:  # noqa: BLE001
                    pass

    def _alive(self):
        return self.proc is not None and self.proc.poll() is None

    def _drills(self):
        cell = self.cell
        if cell["where"] == "docker":
            ex = DockerDrills(self.camp.runner, container_name(cell), self.camp.clock,
                              self._say_drill, cell["id"])
        else:
            ex = HostDrills(self._say_drill, cell["id"], self.camp.clock, self.camp.platform)

        def record(rec):
            with self.lock:
                if rec.get("event") == "drill-start":
                    self.state["active_drill"] = {k: rec[k]
                                                  for k in ("kind", "seconds", "started_utc")}
                else:
                    self.state["active_drill"] = None
                    self.state["drills"].append({k: v for k, v in rec.items() if k != "event"})
            self.event(rec.get("event", "drill-skipped"), **{k: v for k, v in rec.items()
                                                             if k != "event"})
            self.write_state(force=True)

        sched = DrillScheduler(cell["id"], cell["drills"], ex, self.camp.clock, self.start_mono,
                               self.start_utc, self.stop, self._alive, record,
                               self.camp.active_drills)
        self.drills = sched.run()

    def _say_drill(self, line):
        self.logline(line, own=True)
        self.camp.say(line)

    def _start_sampler(self):
        camp, cell = self.camp, self.cell
        argv, env, path = sampler_command(camp.cfg, cell, self.dir)
        self.sampler_info["path"] = path
        self.set_state(sampler_path=path)
        if cell["where"] == "docker":
            name = container_name(cell)
            while not self.stop.is_set():
                rc, out = camp.runner.run(["docker", "inspect", "-f", "{{.State.Running}}", name],
                                          timeout=30)
                if rc == 0 and out.strip() == "true":
                    break
                if camp.clock.wait(self.stop, 10):
                    break
            if self.stop.is_set():
                self.sampler_info["status"] = "not started: the container never ran"
                return
        try:
            with open(self.dir / "sampler.log", "ab") as log:
                self.sampler = camp.runner.popen(argv, env=env, cwd=str(REPO), stdout=log)
        except OSError as e:
            self.sampler_info["status"] = f"failed to start: {e}"
            self.event("sampler", status=self.sampler_info["status"])
            return
        self.sampler_info.update(status="running", started_utc=utc(camp.clock.utcnow()))
        self.event("sampler", status="started", argv=argv, path=path)
        self.write_state(force=True)

    def stop_sampler(self):
        if self.sampler is not None and self.sampler.poll() is None:
            self.camp.terminate(self.sampler, grace=15)
            self.sampler_info["status"] = "stopped"
        elif self.sampler is not None:
            self.sampler_info["status"] = f"exited {self.sampler.poll()}"

    def write_result(self, exit_code, interrupted=False, started=True, error=None):
        clock = self.camp.clock
        end_utc = clock.utcnow()
        cell = self.cell
        # The last matrix row the child printed (matrix.sh prints `log: <path>` after it).
        row = self.final_row
        m = MATRIX_ROW.match(row or "")
        ok_row = bool(m) and m.group(1) == "PASS"
        verdict = "PASS" if exit_code == 0 and ok_row and not interrupted and not error else "FAIL"
        info = {
            "cell": cell["id"], "connector": cell["connector"], "where": cell["where"],
            "test_member_id": cell["test_member_id"], "duration_s": cell["duration_s"],
            "start_utc": utc(self.start_utc) if started else None,
            "end_utc": utc(end_utc) if started else None,
            "elapsed_s": round(clock.monotonic() - self.start_mono, 1)
            if started and self.start_mono is not None else None,
            "exit_code": exit_code, "final_row": row, "verdict": verdict,
            "interrupted": bool(interrupted), "error": error,
            "drills": self.drills or list(self.state["drills"]),
            "sampler": dict(self.sampler_info),
            "run_log": str(self.dir / "run.log"),
            "container": container_name(cell) if cell["where"] == "docker" else None,
            "config_dir": cell.get("config_dir"),
        }
        info["pass_lines"] = evaluate_lines(info, self.sampler_info.get("path"), info["run_log"])
        info["upload"] = upload_facts_for(self.sampler_info.get("path"))
        info["overall"] = overall(info["pass_lines"])
        self.result = info
        write_json(self.dir / "result.json", info)
        self.set_state(state="interrupted" if interrupted else "done", verdict=verdict,
                       overall=info["overall"], end_utc=info["end_utc"], active_drill=None)
        self.write_state(force=True)
        self.event("end", exit_code=exit_code, verdict=verdict, overall=info["overall"])
        self.camp.say(f"campaign: cell {cell['id']} {verdict} (overall {info['overall']}) "
                      f"row: {row}")
        self.camp.write_summary()


class Campaign:
    def __init__(self, cfg, cells, out, parallel, runner=None, clock=None, say=None,
                 platform=None):
        self.cfg, self.cells, self.out = cfg, cells, str(out)
        self.parallel = max(1, parallel)
        self.runner = runner or RealRunner()
        self.clock = clock or RealClock()
        self.platform = platform
        self._say = say or (lambda line: print(line, flush=True))
        self.say_lock = threading.Lock()
        self.slots = threading.Semaphore(self.parallel)
        self.stopping = threading.Event()
        self.all_done = threading.Event()
        self.active_drills = {}
        self.summary_lock = threading.Lock()
        self.remaining = len(cells)
        self.remaining_lock = threading.Lock()
        Path(self.out).mkdir(parents=True, exist_ok=True)
        self.runs = [CellRun(self, c) for c in cells]

    def say(self, line):
        with self.say_lock:
            self._say(line)

    def acquire_slot(self, stop):
        while not self.stopping.is_set() and not stop.is_set():
            if self.slots.acquire(timeout=0.2):
                if self.stopping.is_set():
                    self.slots.release()
                    return False
                return True
        return False

    def release_slot(self):
        self.slots.release()

    def cell_finished(self):
        with self.remaining_lock:
            self.remaining -= 1
            if self.remaining <= 0:
                self.all_done.set()

    def terminate(self, proc, grace):
        self.runner.signal_group(proc, signal.SIGTERM)
        try:
            proc.wait(timeout=grace)
        except subprocess.TimeoutExpired:
            self.runner.signal_group(proc, signal.SIGKILL)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass

    def write_summary(self):
        with self.summary_lock:
            cells = []
            for r in self.runs:
                res = r.result
                if res:
                    entry = {k: res.get(k) for k in (
                        "cell", "where", "connector", "verdict", "overall", "final_row",
                        "start_utc", "end_utc", "exit_code", "interrupted")}
                    entry["lines"] = {l["name"]: {"status": l["status"], "value": l["value"]}
                                      for l in res["pass_lines"]}
                    entry["result"] = str(r.dir / "result.json")
                    cells.append(entry)
                else:
                    cells.append({"cell": r.cell["id"], "where": r.cell["where"],
                                  "connector": r.cell["connector"], "verdict": None,
                                  "overall": r.state["state"].upper(),
                                  "state": str(r.dir / "state.json")})
            write_summary_files(self.out, cells, extra={"campaign": self.cfg["name"],
                                                         "config": self.cfg["source"]})

    def run(self):
        write_json(Path(self.out) / "campaign.json", {
            "name": self.cfg["name"], "config": self.cfg["source"], "pid": os.getpid(),
            "started_utc": utc(self.clock.utcnow()), "parallel": self.parallel,
            "server": self.cfg["server"], "cells": [c["id"] for c in self.cells]})
        self.write_summary()
        self.say(f"campaign: out {self.out}")
        threads = [threading.Thread(target=r.run, name=f"cell-{r.cell['id']}", daemon=True)
                   for r in self.runs]
        for t in threads:
            t.start()
        if not self.runs:
            self.all_done.set()
        interrupted = False
        saved = None
        try:
            try:
                while not self.all_done.is_set():
                    self.clock.wait(self.all_done, 1.0)
            except KeyboardInterrupt:
                interrupted = True
                # A second Ctrl-C (or SIGTERM) must not cut the shutdown short: drills are undone
                # and children reaped first.
                saved = ignore_stop_signals()
                self.say("campaign: stopping: undoing drills, then stopping every cell")
                self.shutdown()
            for t in threads:
                t.join(timeout=180)
            self.write_summary()
        finally:
            restore_signals(saved)
        self.say(f"campaign: summary {Path(self.out) / 'summary.json'}")
        if interrupted:
            return 130
        ok = all(r.result and r.result["verdict"] == "PASS" and r.result["overall"] != "FAIL"
                 for r in self.runs)
        return 0 if ok else 1

    def shutdown(self, grace=200):
        # grace: linux-matrix.sh waits up to 150 s for matrix.sh to seal its test run, and
        # matrix.sh up to 120 s for a seal already asked for.
        saved = ignore_stop_signals()
        try:
            self._shutdown(grace)
        finally:
            restore_signals(saved)

    def _shutdown(self, grace):
        self.stopping.set()
        for r in self.runs:
            r.stop.set()
        # First every drill is undone (a paused container cannot even be stopped cleanly). The
        # drill threads wake on their stop event and undo in their finally; whatever is still
        # active after a bounded wait (real time, not the injected clock) is undone from here.
        deadline = time.monotonic() + 60
        while self.active_drills and time.monotonic() < deadline:
            time.sleep(0.05)
        for key, (ex, drill, ctx, rec) in list(self.active_drills.items()):
            undo_drill(ex, drill, ctx, rec)
            self.say(f"campaign: undid {drill['kind']} on cell {key[0]}"
                     + (f" ({rec['undo_error']})" if rec.get("undo_error") else ""))
        # Then every child, all at once: SIGTERM (linux-matrix.sh removes its container on it),
        # a bounded wait, SIGKILL for what is left.
        procs = []
        for r in self.runs:
            for proc in (r.proc, r.sampler):
                if proc is not None and proc.poll() is None:
                    self.runner.signal_group(proc, signal.SIGTERM)
                    procs.append(proc)
        end = time.monotonic() + grace
        for proc in procs:
            try:
                proc.wait(timeout=max(0.1, end - time.monotonic()))
            except subprocess.TimeoutExpired:
                self.runner.signal_group(proc, signal.SIGKILL)
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    pass


def ignore_stop_signals():
    """Ignore SIGINT and SIGTERM (main thread only); what restore_signals needs, or None."""
    if threading.current_thread() is not threading.main_thread():
        return None
    saved = {}
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            saved[sig] = signal.signal(sig, signal.SIG_IGN)
        except (ValueError, OSError):
            pass
    return saved


def restore_signals(saved):
    for sig, handler in (saved or {}).items():
        try:
            signal.signal(sig, handler if handler is not None else signal.SIG_DFL)
        except (ValueError, OSError, TypeError):
            pass


def write_summary_files(out, cells, extra=None):
    doc = dict(extra or {})
    doc["updated_utc"] = utc(datetime.now(timezone.utc))
    doc["cells"] = cells
    write_json(Path(out) / "summary.json", doc)

    def val(c, name):
        l = (c.get("lines") or {}).get(name)
        if not l:
            return ""
        v = l["value"]
        if isinstance(v, float):
            v = round(v, 3)
        if name == "chain_bytes_max" and isinstance(v, (int, float)):
            v = f"{v / 1024 ** 2:.1f} MiB"
        return f"{l['status']}" + (f" ({v})" if v not in (None, "") else "")

    md = ["| cell | where | connector | verdict | overall | RSS MiB/h | files/h | chain max | ACK recovery |",
          "|---|---|---|---|---|---|---|---|---|"]
    for c in cells:
        md.append("| " + " | ".join(str(x) for x in (
            c.get("cell"), c.get("where"), c.get("connector"), c.get("verdict") or "",
            c.get("overall") or "", val(c, "rss_slope"), val(c, "open_files_slope"),
            val(c, "chain_bytes_max"), val(c, "ack_recovery"))) + " |")
    (Path(out) / "summary.md").write_text("\n".join(md) + "\n")


# --------------------------------------------------------------------------- status / evaluate

def cell_dirs(out):
    out = Path(out)
    camp = read_json(out / "campaign.json") or {}
    names = camp.get("cells") or sorted(p.name for p in out.iterdir()
                                        if p.is_dir() and (p / "state.json").exists())
    return camp, [out / n for n in names]


def pid_alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


def status_rows(out):
    camp, dirs = cell_dirs(out)
    rows = []
    for d in dirs:
        st = read_json(d / "state.json") or {"cell": d.name, "state": "unknown"}
        res = read_json(d / "result.json")
        row = {"cell": st.get("cell", d.name), "where": st.get("where"),
               "connector": st.get("connector"), "state": st.get("state"),
               "start_utc": st.get("start_utc"), "duration_s": st.get("duration_s"),
               "active_drill": st.get("active_drill"),
               "drills_done": len(st.get("drills") or []), "last_line": st.get("last_line"),
               "last_line_utc": st.get("last_line_utc"),
               "verdict": (res or {}).get("verdict"), "overall": (res or {}).get("overall")}
        start = parse_time(st.get("start_utc") or "")
        if start and st.get("state") == "running":
            row["elapsed_s"] = int((datetime.now(timezone.utc) - start).total_seconds())
        rows.append(row)
    alive = pid_alive(camp.get("pid")) if camp.get("pid") else None
    return {"campaign": camp.get("name"), "campaign_pid_alive": alive, "cells": rows}


def evaluate_dir(out):
    _, dirs = cell_dirs(out)
    results = []
    for d in dirs:
        res = read_json(d / "result.json")
        if res:
            path = (res.get("sampler") or {}).get("path")
            res["pass_lines"] = evaluate_lines(res, path, res.get("run_log"))
            res["upload"] = upload_facts_for(path)
            res["overall"] = overall(res["pass_lines"])
            write_json(d / "result.json", res)
        else:
            st = read_json(d / "state.json") or {"cell": d.name}
            start = parse_time(st.get("start_utc") or "")
            res = {"cell": st.get("cell", d.name), "where": st.get("where"),
                   "connector": st.get("connector"), "state": "running",
                   "duration_s": st.get("duration_s"), "drills": st.get("drills") or [],
                   "verdict": None, "final_row": None, "provisional": True,
                   "start_utc": st.get("start_utc"),
                   "elapsed_s": int((datetime.now(timezone.utc) - start).total_seconds())
                   if start else None}
            res["pass_lines"] = evaluate_lines(res, st.get("sampler_path"), str(d / "run.log"))
            res["upload"] = upload_facts_for(st.get("sampler_path"))
            res["overall"] = overall(res["pass_lines"])
        results.append(res)
    cells = [{"cell": r.get("cell"), "where": r.get("where"), "connector": r.get("connector"),
              "verdict": r.get("verdict"), "overall": r.get("overall"),
              "final_row": r.get("final_row"),
              "lines": {l["name"]: {"status": l["status"], "value": l["value"]}
                        for l in r["pass_lines"]}} for r in results]
    camp = read_json(Path(out) / "campaign.json") or {}
    write_summary_files(out, cells, extra={"campaign": camp.get("name"),
                                           "config": camp.get("config")})
    return results


# --------------------------------------------------------------------------- CLI

def cmd_check(args, runner, say=print):
    cfg = load_config(args.config)
    cells = select_cells(cfg, args.cells)
    errors, warnings = validate(cfg, cells, args.parallel)
    info = []
    if not errors and not args.static:
        e2, w2, info = live_checks(cfg, cells, runner)
        errors += e2
        warnings += w2
    if args.json:
        say(json.dumps({"ok": not errors, "errors": errors, "warnings": warnings, "info": info},
                       indent=2))
    else:
        for m in info:
            say(f"ok    {m}")
        for m in warnings:
            say(f"warn  {m}")
        for m in errors:
            say(f"error {m}")
        say(f"check: {'FAIL' if errors else 'ok'} ({len(cells)} cells, parallel {args.parallel})")
    return 1 if errors else 0


def cmd_build(args, runner):
    cfg = load_config(args.config)
    errors, _ = validate(cfg, cfg["cells"], 1)
    if errors:
        for m in errors:
            print(f"error {m}", file=sys.stderr)
        return 2
    # A build needs no approval key, so none is passed.
    env = child_env({"LINUX_MATRIX_SERVER": cfg["server"]})
    connectors = sorted({c["connector"] for c in cfg["cells"] if c["where"] == "docker"})
    if not connectors:
        print("campaign: no docker cell; nothing to build")
        return 0
    return runner.interactive(["sh", str(HERE / "linux-matrix.sh"), "build"] + connectors,
                              env=env, cwd=str(REPO))


def cmd_login(args, runner):
    cfg = load_config(args.config)
    cell = select_cells(cfg, args.cell)[0]
    errors, _ = validate(cfg, [cell], 1)
    # A host cell's configuration directory is made by this very login.
    errors = [e for e in errors if "does not exist" not in e]
    if errors:
        for m in errors:
            print(f"error {m}", file=sys.stderr)
        return 2
    if cell["where"] == "docker":
        env = child_env({KEY_ENV: cfg["approval_key_file"], "LINUX_MATRIX_CELL": cell["id"],
                         "LINUX_MATRIX_PREBUILT": "1", "LINUX_MATRIX_SERVER": cfg["server"],
                         "GOALCUP_TEST_MEMBER_ID": cell["test_member_id"],
                         "LINUX_MATRIX_CODEX_EFFORT": cell.get("codex_effort")})
        return runner.interactive(["sh", str(HERE / "linux-matrix.sh"), "login",
                                   cell["connector"]], env=env, cwd=str(REPO))
    os.makedirs(cell["config_dir"], mode=0o700, exist_ok=True)
    env = child_env({"GOALCUP_CONFIG_DIR": cell["config_dir"]})
    goalcup = cfg["goalcup"]
    if cfg.get("bin_dir"):
        goalcup = os.path.join(cfg["bin_dir"], "goalcup")
    rc = runner.interactive([goalcup, "login", "--server", cfg["server"]], env=env)
    if rc != 0:
        return rc
    return runner.interactive([goalcup, "prep", "--harness", cell["connector"]], env=env)


def default_out(cfg, clock):
    stamp = clock.utcnow().strftime("%Y%m%dT%H%M%SZ")
    return os.path.join(tempfile.gettempdir(), "goalcup-campaign", f"{cfg['name']}-{stamp}")


def cmd_run(args, runner, clock, say=None):
    cfg = load_config(args.config)
    cells = select_cells(cfg, args.cells)
    errors, warnings = validate(cfg, cells, args.parallel)
    out_say = say or (lambda l: print(l, flush=True))
    for m in warnings:
        out_say(f"warn  {m}")
    if errors:
        for m in errors:
            out_say(f"error {m}")
        return 2
    out = args.out or default_out(cfg, clock)
    camp = Campaign(cfg, cells, out, args.parallel, runner=runner, clock=clock, say=say)
    return camp.run()


def print_status(doc):
    print(f"campaign {doc['campaign']}: orchestrator "
          f"{'running' if doc['campaign_pid_alive'] else 'not running'}")
    for c in doc["cells"]:
        drill = c["active_drill"]
        bits = [c["cell"], c.get("where") or "", c.get("state") or ""]
        if c.get("elapsed_s") is not None:
            bits.append(f"{c['elapsed_s']}/{c.get('duration_s')}s")
        if c.get("verdict"):
            bits.append(f"{c['verdict']}/{c.get('overall')}")
        if drill:
            bits.append(f"DRILL {drill['kind']} since {drill['started_utc']}")
        bits.append(f"drills {c['drills_done']}")
        if c.get("last_line"):
            bits.append(f"last: {c['last_line'][:120]}")
        print("  " + "  ".join(bits))


def print_evaluation(results):
    for r in results:
        print(f"{r.get('cell')}: {r.get('verdict') or 'running'} overall {r.get('overall')}")
        for l in r["pass_lines"]:
            print(f"  {l['name']:<18} {l['status']:<18} value {l['value']}  ({l['threshold']})")


def build_parser():
    p = argparse.ArgumentParser(prog="campaign.py", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="command", required=True)
    c = sub.add_parser("check", help="validate a config (and probe docker and sign-ins)")
    c.add_argument("config", help="the campaign config (JSON)")
    c.add_argument("--cells", help="comma-separated cell ids (default: every cell)")
    c.add_argument("--parallel", type=int, default=4, help="cells at once (default 4)")
    c.add_argument("--json", action="store_true", help="print the result as JSON")
    c.add_argument("--static", action="store_true", help="no docker or file probes")
    b = sub.add_parser("build", help="build the images and binaries for the docker cells")
    b.add_argument("config", help="the campaign config (JSON)")
    l = sub.add_parser("login", help="sign one cell's harness in (interactive)")
    l.add_argument("config", help="the campaign config (JSON)")
    l.add_argument("cell", help="the cell id")
    r = sub.add_parser("run", help="run the cells with their drills and samplers")
    r.add_argument("config", help="the campaign config (JSON)")
    r.add_argument("--cells", help="comma-separated cell ids (default: every cell)")
    r.add_argument("--parallel", type=int, default=4, help="cells at once (default 4)")
    r.add_argument("--out", help="output directory (default under the temporary directory)")
    s = sub.add_parser("status", help="show a campaign's live state")
    s.add_argument("dir", help="the campaign's output directory")
    s.add_argument("--json", action="store_true", help="print the state as JSON")
    e = sub.add_parser("evaluate", help="recompute the pass lines from the files")
    e.add_argument("dir", help="the campaign's output directory")
    e.add_argument("--json", action="store_true", help="print the results as JSON")
    return p


def main(argv=None, runner=None, clock=None):
    args = build_parser().parse_args(argv)
    runner = runner or RealRunner()
    clock = clock or RealClock()
    try:
        if args.command == "check":
            return cmd_check(args, runner)
        if args.command == "build":
            return cmd_build(args, runner)
        if args.command == "login":
            return cmd_login(args, runner)
        if args.command == "run":
            if threading.current_thread() is threading.main_thread():
                def on_term(signum, frame):
                    raise KeyboardInterrupt
                signal.signal(signal.SIGTERM, on_term)
            return cmd_run(args, runner, clock)
        if args.command == "status":
            doc = status_rows(args.dir)
            if args.json:
                print(json.dumps(doc, indent=2))
            else:
                print_status(doc)
            return 0
        if args.command == "evaluate":
            results = evaluate_dir(args.dir)
            if args.json:
                print(json.dumps(results, indent=2))
            else:
                print_evaluation(results)
            return 0 if all(r["overall"] != "FAIL" for r in results) else 1
    except ConfigError as e:
        print(f"campaign: {e}", file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main())
