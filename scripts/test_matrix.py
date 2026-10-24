#!/usr/bin/env python3
"""Behavioral tests for the matrix, scripts/matrix.sh, against stand-ins for everything it drives.

`goalcup`, `goalcup-verify`, `goalcup-daemon`, `gh` and `curl` are stubs in a scratch directory (the
real ones are covered by running the script, docs/OPERATING.md "The matrix"). The `goalcup` stub
records what it was started with: its arguments, the parts of its environment that matter (the git
login it was given among them) and what arrived on its standard input. Every connector is chat mode
(D57), so every run is on a pseudo-terminal (script(1)); the refusals run with no terminal. Every
key here is synthetic.
"""

from __future__ import annotations

import concurrent.futures
import json
import os
import shutil
import stat
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path

MATRIX = Path(__file__).with_name("matrix.sh")

PROMPT = "Goal Cup test prompt: say ok."
GIVEN_REPO = "goalcup-kit/kit-given"
HOST_SSH = "/host/agent.sock"

GOALCUP = r'''#!/usr/bin/env python3
import json, os, select, subprocess, sys, termios, time, tty
from pathlib import Path

out = Path(os.environ["STUB_OUT"])
args = sys.argv[1:]
note = lambda name, text: (out / name).write_text(text)
ENTRY = "e0123456789abcdef"
cfg = Path(os.environ.get("GOALCUP_CONFIG_DIR", "/nonexistent"))

if args == ["--version"]:
    print("goalcup 0.0.0-stub")
    sys.exit(0)

def wait_approved(phase, count):
    """With STUB_CODE=1 a login and a start show a device code and wait for the matrix's approval
    (the curl stub counts them); then say whether the approval key's file is still there, a second
    after the approval so the matrix has had time to act on it."""
    if os.environ.get("STUB_CODE") != "1":
        return
    print("Open the page; it shows the code ABCD-EFGH.", flush=True)
    t0 = time.time()
    while len(list(out.glob("approved.*"))) < count and time.time() - t0 < 30:
        time.sleep(0.2)
    time.sleep(1)
    key = os.environ.get("STUB_KEYFILE")
    note("%s.keyfile_exists" % phase, str(bool(key) and os.path.exists(key)))

if args[0] == "login":
    wait_approved("login", 1)
    note("login.no_browser", os.environ.get("GOALCUP_NO_BROWSER", "unset"))
    print("Logged in as member.")
    sys.exit(0)

if args[0] == "entries":
    # What the daemon lists: every entry of every run record (none with STUB_ENTRIES_EMPTY).
    note("entries.argv", "\n".join(args))
    items = []
    if os.environ.get("STUB_ENTRIES_EMPTY") != "1":
        for rj in (cfg / "runs").glob("*/run.json"):
            r = json.loads(rj.read_text())
            for e in r["entries"]:
                items.append({"entry_id": e["entry_id"], "connector_id": r["connector_id"],
                              "workspace": "/w", "arming": "started", "health": "online"})
    sys.stderr.write("goalcup: a line on stderr that is not JSON\n")
    print(json.dumps(items, indent=2))
    sys.exit(0)

if args[0] == "seal":
    time.sleep(float(os.environ.get("STUB_SEAL_SLEEP", "0")))
    note("seal.argv", "\n".join(args))
    note("seal.at", repr(time.time()))
    note("sealed", args[1])
    print("harness termination: Confirmed, the harness was asked to stop and seen stopped")
    sys.exit(0)

if args[0] == "upload":
    note("upload.argv", "\n".join(args))
    size = int((out / "chain.size").read_text())
    print("uploaded: %d bytes of entry %s, acknowledged, and its seal, on stub." % (size, args[1]))
    sys.exit(0)

assert args[0] == "start", args

def git_login():
    """The git login this start was given: the variables goalcup reads it from, and the git config
    the kit named."""
    names = ("GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM", "GH_CONFIG_DIR", "SSH_AUTH_SOCK",
             "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "GH_TOKEN", "GITHUB_TOKEN")
    seen = {k: os.environ.get(k) for k in names}
    gc = Path(os.environ.get("GIT_CONFIG_GLOBAL", "/nonexistent"))
    seen["gitconfig"] = subprocess.run(["git", "config", "--file", str(gc), "--list"],
                                       capture_output=True, text=True).stdout.splitlines() if gc.is_file() else None
    return seen

if os.environ.get("GOALCUP_TEST_CHAT_WITHOUT_TERMINAL") == "1":
    # A refusal cell: the start's checks before the approval (run_repo.rs), as a test build makes
    # them through its doors, with no terminal.
    import urllib.error, urllib.request
    repo = args[args.index("--repo") + 1] if "--repo" in args else None
    kind = {None: "no-login"}.get(repo) or ("bare-name" if "/" not in repo else repo.split("/")[1][4:])
    doors = {k: os.environ.get(k) for k in ("GOALCUP_TEST_GITHUB_API", "GOALCUP_TEST_GITHUB_LOGIN",
                                            "GOALCUP_TEST_PUSH_REMOTE")}
    note("refuse.%s" % kind, json.dumps({"argv": args, "doors": doors, "tty": os.isatty(0),
                                         "git": git_login()}))
    def refuse(why):
        sys.stderr.write("goalcup: %s Nothing was created.\n" % why)
        sys.exit(1)
    if os.environ.get("STUB_SHIPPED") == "1":
        refuse("stub needs a terminal of its own.")
    if os.environ.get("STUB_NEVER_REFUSES") == "1":
        print("Approve this run in your browser: the page shows the code ABCD-EFGH.", flush=True)
        sys.exit(1)
    login = doors["GOALCUP_TEST_GITHUB_LOGIN"] or None
    if login is None:
        print("No GitHub login found on this computer. To set one up (about 5 minutes):")
    if repo is None:
        refuse("A real run needs a new, empty, public GitHub repository. Run goalcup start in a "
               "terminal and name it, or add --repo <owner>/<name>.")
    if "/" not in repo and login is None:
        refuse("No GitHub login was found on this computer. Give --repo <your user name>/%s." % repo)
    api = doors["GOALCUP_TEST_GITHUB_API"]
    if api:
        try:
            doc = json.load(urllib.request.urlopen(api + "/repos/" + repo, timeout=10))
        except urllib.error.HTTPError:
            doc = None
        if doc is None or doc["private"]:
            refuse("GitHub does not show %s: it is not created yet, or it is private." % repo)
        if doc["size"]:
            refuse("The repository %s is not empty (%d KB), and a new run needs a new empty one."
                   % (repo, doc["size"]))
    remote = doors["GOALCUP_TEST_PUSH_REMOTE"]
    if remote and not os.path.exists(remote):
        refuse("The agent's git cannot push to %s (no such remote)." % repo)
    print("Approve this run in your browser: the page shows the code ABCD-EFGH.", flush=True)
    sys.exit(1)

note("start.argv", "\n".join(args))
note("start.no_browser", os.environ.get("GOALCUP_NO_BROWSER", "unset"))
note("start.stdin_is_tty", str(os.isatty(0)))
note("start.git", json.dumps(git_login()))
test = "--test" in args
if os.environ.get("STUB_ASK") == "1":
    # A real run's start asking for its repository, which nobody in the matrix can answer.
    sys.stdout.write("Your run needs a new, empty, public GitHub repo. Each real run gets its own.\n  Name it: ")
    sys.stdout.flush()
    time.sleep(60)
    sys.exit(1)

run_dir = cfg / "runs" / "r1"
trace = cfg / "artifacts" / ENTRY / "trace"
chain = (json.dumps({"body": {"harness_version":
                             os.environ.get("STUB_HV", "stub-harness 9.9.9")}}) + "\n").encode()

def make_workspace(ws):
    # The clone the agent works in, and what the agent left in it (STUB_WS): file (an untracked
    # file, the default), commit (a commit, the tree clean), none (nothing), old (only a commit
    # dated before the run: the clone's own history).
    ws.mkdir(parents=True)
    git = lambda *a, **env: subprocess.run(
        ["git", "-C", str(ws), "-c", "user.name=stub", "-c", "user.email=stub@example.invalid", *a],
        check=True, capture_output=True, env=dict(os.environ, **env))
    git("init", "-q")
    mode = os.environ.get("STUB_WS", "file")
    if mode == "old":
        then = "2020-01-01T00:00:00Z"
        git("commit", "-q", "--allow-empty", "-m", "seed", GIT_AUTHOR_DATE=then,
            GIT_COMMITTER_DATE=then)
    elif mode in ("file", "commit"):
        (ws / "matrix-ok.txt").write_text("ok\n")
        if mode == "commit":
            git("add", "matrix-ok.txt")
            git("commit", "-q", "-m", "ok")

def write_run():
    # A test run's record is written before the harness starts, as goalcup start does it.
    trace.mkdir(parents=True)
    run_dir.mkdir(parents=True)
    note("start.env", json.dumps(sorted(k for k in os.environ
                                        if k in ("DATABASE_URL", "CLOUDFLARE_ACCOUNT_ID",
                                                 "CLOUDFLARE_API_TOKEN"))))
    if os.environ.get("STUB_PRUNED") == "1":
        # A long run: the acknowledged first part was pruned here; only the tail part is left.
        full = chain + b'{"body":{"n":1}}\n' + b'{"body":{"n":2}}\n'
        (out / "full_chain.jsonl").write_bytes(full)
        tail = len(chain)
        (trace / ("chain.%020d.jsonl" % tail)).write_bytes(full[tail:])
        note("chain.size", str(len(full)))
    else:
        (trace / "chain.jsonl").write_bytes(chain)
    (run_dir / "credential").write_text(json.dumps({"credential": "c"}))
    started = int(time.time())
    workspace = cfg / "work" / "goalcup-stub"
    make_workspace(workspace)
    (run_dir / "run.json").write_text(json.dumps({
        "run_id": "r1", "server_url": "http://localhost:8787", "test": test,
        "connector_id": args[args.index("--harness") + 1], "workspace": str(workspace),
        "started_at_unix": started,
        "entries": [{"entry_id": ENTRY, "trace_dir": str(trace)}],
    }))
    if os.environ.get("STUB_PRUNED") != "1":
        note("chain.size", str(len(chain)))

if test:
    write_run()

wait_approved("start", 2)
# setraw flushes pending input: finish that transition before the feeder sees ready.
tty.setraw(0)
if os.environ.get("STUB_BRACKETED") == "1":
    # A TUI that switches bracketed paste on, as Codex, Claude Code and Amp do.
    sys.stdout.write("\x1b[?2004h")
print("Recording in chat mode: stub starts now in this terminal.", flush=True)
chunks, t0 = [], time.time()
while time.time() - t0 < 30:
    if select.select([0], [], [], 0.2)[0]:
        data = os.read(0, 4096)
        chunks.append([round(time.time() - t0, 2), data.decode("latin1")])
        if b"\r" in data and chunks and any(c[1] == os.environ["STUB_PROMPT"] for c in chunks):
            note("start.submitted_at", repr(time.time()))
            break
note("start.stdin_chunks", json.dumps(chunks))

if not test:
    write_run()

exit_line = "harness exited: exit code %s." % os.environ.get("STUB_EXIT", "0")
if test and os.environ.get("STUB_EXIT_EARLY") != "1":
    # The harness runs until the seal stops it.
    t0 = time.time()
    while not (out / "sealed").exists() and time.time() - t0 < 60:
        time.sleep(0.2)
    exit_line = "harness exited: ended by signal 15."
elif test:
    # Ending first, but a while after the Enter: the matrix polls every two seconds, so it has
    # seen the prompt submitted before the harness ends.
    time.sleep(5)
if not test:
    # A chat-mode run's `deadline:` line goes to the run's host.log, not the terminal.
    (run_dir / "host.log").write_text("deadline: the run's time is up. The agent is stopped.\n")
print(exit_line)
if os.environ.get("STUB_NO_SEAL") != "1":
    print("uploaded: %d bytes, acknowledged, and the seal." % len(chain))
else:
    print("uploaded: %d bytes, acknowledged." % len(chain))
'''

VERIFY = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
if sys.argv[1] == "--research-json":
    # The research report: what the agent did (STUB_WORK: usage, assistant or none).
    (Path(os.environ["STUB_OUT"]) / "research.argv").write_text("\n".join(sys.argv[1:]))
    work = os.environ.get("STUB_WORK", "usage")
    obs = [{"kind": "model_request", "fields": {"endpoint": "/v1/messages"}},
           {"kind": "usage", "fields": {"input_tokens": 900, "output_tokens": 0}}]
    if work == "usage":
        obs.append({"kind": "usage", "fields": {"last_token_usage": {"output_tokens": 12}}})
    elif work == "assistant":
        obs.append({"kind": "model_response", "fields": {"role": "assistant", "content": "ok"}})
    with open(sys.argv[sys.argv.index("--output") + 1], "x") as f:
        for ob in obs:
            f.write(json.dumps({"type": "observation", "decoder": "stub", "observation": ob}) + "\n")
        f.write(json.dumps({"type": "summary", "exit_code": 1}) + "\n")
    sys.exit(1)
size = (Path(sys.argv[2]) / "chain.jsonl").stat().st_size
print(json.dumps({"integrity": "valid", "anchor": "signed", "validated_records": 1,
                  "validated_bytes": size, "validated_head": "a" * 64, "sealed": True}))
'''

# fetch-run.sh's arguments; rebuilds the entry's copy from what the goalcup stub wrote (STUB_FETCH_BAD:
# one byte of the tail differs) and notes which credentials reached its environment.
FETCH = r'''#!/bin/sh
exec python3 - "$@" <<'PY'
import json, os, sys
from pathlib import Path
out = Path(os.environ["STUB_OUT"])
args = sys.argv[1:]
(out / "fetch.argv").write_text("\n".join(args))
(out / "fetch.env").write_text(json.dumps({k: os.environ.get(k) for k in
    ("DATABASE_URL", "CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN")}))
entry, outdir = args[-2], Path(args[-1])
full = (out / "full_chain.jsonl").read_bytes()
if os.environ.get("STUB_FETCH_BAD") == "1":
    full = full[:-3] + b"X" + full[-2:]
(outdir / entry).mkdir(parents=True)
(outdir / entry / "chain.jsonl").write_bytes(full)
print(outdir / entry)
PY
'''

CURL = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = sys.argv[1:]
out = Path(os.environ["STUB_OUT"])
if "@-" in args:
    header = sys.stdin.read()
    if args[-1].endswith("/v1/device/approve"):
        n = len(list(out.glob("approved.*")))
        (out / ("approved.%d" % (n + 1))).write_text(header)
target = args[args.index("-o") + 1]
url = args[-1]
if "/v1/runs/" in url:
    size = int((out / "chain.size").read_text())
    body = {"offset": size, "next_seq": 1, "head": "a" * 64, "last_ack": {"hash": "a" * 64},
            "final": {"seq": 0, "head": "a" * 64}, "status": "sealed",
            "stop": os.environ.get("STUB_STOP", "1") == "1"}
elif url.endswith("/v1/health"):
    body = {"status": "ok"}
else:
    body = {"status": "approved"}
Path(target).write_text(json.dumps(body))
sys.stdout.write("200")
'''

# gh as the kit uses it to make a test run's repository with this machine's login (none with
# STUB_GH_NO_LOGIN).
GH = r'''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
out = Path(os.environ["STUB_OUT"])
if args[:2] == ["api", "user"]:
    if os.environ.get("STUB_GH_NO_LOGIN") == "1":
        sys.exit("To get started with GitHub CLI, please run:  gh auth login")
    print("kit-tester")
elif args[:2] == ["repo", "create"]:
    (out / "gh.repo_create").write_text("\n".join(args))
    print("https://github.com/" + args[2])
else:
    sys.exit("gh stub: " + " ".join(args))
'''

CHAT = """schema_version = 1
id = "{id}"

[capture]
interactive = true
args = []
"""


def refused_for_server(run: subprocess.CompletedProcess) -> bool:
    return run.returncode == 2 and "MATRIX_ALLOW_SERVER" in run.stderr


def executable(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class World:
    """One matrix run against the stubs, in a scratch tree of its own. Runs are slow (the script
    polls every two seconds), so the tests start them all at once and look at the results."""

    def __init__(self, connector: str, env: dict[str, str | None] | None = None,
                 args: tuple[str, ...] = (), prepared: bool = False,
                 prompt_file: str | None = None) -> None:
        server = (env or {}).get("WORLD_SERVER") or "http://localhost:8787"
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.out = self.root / "out"
        self.out.mkdir()
        self.tmpdir = self.root / "tmp"
        self.tmpdir.mkdir()
        executable(self.bin / "goalcup", GOALCUP)
        executable(self.bin / "goalcup-verify", VERIFY)
        executable(self.bin / "goalcup-daemon", "#!/bin/sh\nexit 0\n")
        executable(self.bin / "curl", CURL)
        executable(self.bin / "gh", GH)
        executable(self.bin / "fake-fetch.sh", FETCH)
        self.connectors = self.root / "connectors"
        (self.connectors / "stubchat").mkdir(parents=True)
        (self.connectors / "stubchat" / "harness.toml").write_text(CHAT.format(id="stubchat"))
        environment = {
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "HOME": str(self.root),
            "TMPDIR": str(self.tmpdir),
            "MATRIX_BIN_DIR": str(self.bin),
            "GOALCUP_CONNECTORS_DIR": str(self.connectors),
            "MATRIX_PROMPT": PROMPT,
            "MATRIX_PROMPT_DELAY": "1",
            "MATRIX_SUBMIT_DELAY": "1",
            "STUB_OUT": str(self.out),
            "STUB_PROMPT": PROMPT,
            # This machine's own git login: a test run's goalcup sees it, no other cell's does.
            "SSH_AUTH_SOCK": HOST_SSH,
            "GH_TOKEN": "gh-token-of-this-machine",
        }
        if prepared:
            # A configuration directory as `goalcup setup` leaves it (MATRIX_CONFIG_DIR).
            self.cfg = self.root / "prepared-cfg"
            self.cfg.mkdir()
            (self.cfg / "prep.json").write_text("{}")
            environment["MATRIX_CONFIG_DIR"] = str(self.cfg)
        if prompt_file is not None:
            path = self.root / "prompt.txt"
            path.write_text(prompt_file)
            environment["MATRIX_PROMPT_FILE"] = str(path)
            del environment["MATRIX_PROMPT"]
        for key, value in (env or {}).items():
            if key == "WORLD_SERVER":
                continue
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = value.replace("{bin}", str(self.bin)).replace("{root}", str(self.root))
        if "{root}/approval-key" in (env or {}).get("GOALCUP_TEST_APPROVAL_KEY_FILE", ""):
            (self.root / "approval-key").write_text(KEY_VALUE + "\n")
        # The host half of a Docker cell's fetch credential handover (STUB_HOST_ANSWERS=1): when
        # matrix.sh makes its request file, put the credentials in the private directory it made.
        self.request_seen: dict[str, str] = {}
        stop = threading.Event()
        responder = None
        if environment.get("STUB_HOST_ANSWERS") == "1":
            request = Path(environment["MATRIX_FETCH_REQUEST"])
            secrets = Path(environment["MATRIX_FETCH_SECRETS_DIR"])

            def answer() -> None:
                while not stop.is_set():
                    if request.exists() and secrets.is_dir():
                        self.request_seen["mode"] = oct(stat.S_IMODE(secrets.stat().st_mode))
                        (secrets / "db-url").write_text(DB_URL)
                        (secrets / "cf-token").write_text(CF_TOKEN)
                        (secrets / "ready").write_text("")
                        return
                    time.sleep(0.1)

            responder = threading.Thread(target=answer, daemon=True)
            responder.start()
        log = self.root / "matrix.log"
        # A test run is left to its default budget (the duration plus 1800 s) unless it names one.
        budget = () if "--test" in args or "--refusals" in args else ("--timeout", "60")
        self.started = time.time()
        self.run = subprocess.run(
            ["sh", str(MATRIX), connector, server, "--log", str(log), "--keep",
             *budget, *args],
            env=environment, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=120,
        )
        stop.set()
        self.elapsed = time.time() - self.started
        self.log = log.read_text() if log.exists() else ""

    def seen(self, name: str) -> str:
        return (self.out / name).read_text()

    def seen_json(self, name: str):
        return json.loads(self.seen(name))

    def has(self, name: str) -> bool:
        return (self.out / name).exists()

    def everything_matrix_wrote(self) -> str:
        """The terminal output, the log and every file in the kept directory."""
        texts = [self.run.stdout, self.run.stderr, self.log]
        for found in self.tmpdir.rglob("*"):
            if found.is_file():
                texts.append(found.read_text(errors="replace"))
        return "\n".join(texts)

    def said(self) -> str:
        return self.run.stdout + self.run.stderr + self.log


DURATION = 4
KEY_VALUE = "SENTINEL-KEY-VALUE-never-printed-0123456789"
DB_URL = "postgres://reader:db-secret-never-shown@db.invalid/postgres"
CF_TOKEN = "cf-token-never-shown"
AMP_VERSION = "0.0.1792209635-g24a9fe (released 2026-10-17T04:00:35.000Z, 4d ago)"
MULTILINE = "Build the thing.\n\n  Then\ttest it.\n"
TYPED = "Build the thing. Then test it."
TEST = ("--test", "--duration", str(DURATION))

SCENARIOS = {
    "no_seal": ("stubchat", {"STUB_NO_SEAL": "1"}),
    "no_manifest": ("nosuchconnector", {}),
    "chat": ("stubchat", {"MATRIX_PRE_PROMPT_KEYS": r"\033[B\r", "MATRIX_PRE_PROMPT_DELAY": "1"}),
    # Test runs: sealed by the script, the server's copy `sealed` with no `stop`.
    "test_chat": ("stubchat", {"STUB_STOP": "0", "STUB_PROMPT": TYPED}, TEST, True, MULTILINE),
    "test_chat_early": ("stubchat",
                        {"STUB_STOP": "0", "STUB_EXIT_EARLY": "1", "STUB_NO_SEAL": "1",
                         "STUB_PROMPT": TYPED, "MATRIX_REPO": GIVEN_REPO},
                        ("--test", "--duration", "30"), True, MULTILINE),
    "test_unlisted": ("stubchat", {"STUB_ENTRIES_EMPTY": "1", "STUB_STOP": "0"},
                      ("--test", "--duration", "30"), True),
    "test_timeout": ("stubchat", {"STUB_STOP": "0"},
                     ("--test", "--duration", "60", "--timeout", "12"), True),
    # The seal is asked for, then the budget runs out while it is still running.
    "test_seal_slow": ("stubchat", {"STUB_STOP": "0", "STUB_SEAL_SLEEP": "10"},
                       ("--test", "--duration", "4", "--timeout", "14"), True),
    "prompt_control": ("stubchat", {}, (), False, "go\x1b[Bnow\n"),
    "amp_version": ("stubchat", {"STUB_HV": AMP_VERSION, "STUB_WORK": "assistant"}),
    "silent": ("stubchat", {"STUB_WORK": "none"}),
    # The model answered; what the agent left in its workspace decides.
    "ws_none": ("stubchat", {"STUB_WS": "none"}),
    "ws_old": ("stubchat", {"STUB_WS": "old"}),
    "ws_commit": ("stubchat", {"STUB_WS": "commit"}),
    "ws_none_opt_out": ("stubchat", {"STUB_WS": "none", "MATRIX_EXPECT_FILES": "0"}),
    "bracketed": ("stubchat", {"STUB_BRACKETED": "1",
                               "STUB_PROMPT": "\x1b[200~" + PROMPT + "\x1b[201~"}),
    # Long runs: the trace holds only the tail part (pruned), and the server copy is fetched.
    "pruned_unfetched": ("stubchat", {"STUB_PRUNED": "1", "DATABASE_URL": None,
                                      "CLOUDFLARE_API_TOKEN": None}),
    "pruned_files": ("stubchat", {"STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                  "MATRIX_FETCH_INDEX": "/index.json",
                                  "MATRIX_FETCH_STORE": "/bucket-mirror",
                                  "MATRIX_FETCH_BUCKET": "goalcup-traces-local"}),
    # The bucket is known for the staging Worker; for any other server it must be named.
    "pruned_staging": ("stubchat", {"STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                    "MATRIX_FETCH_INDEX": "/index.json",
                                    "MATRIX_FETCH_STORE": "/bucket-mirror",
                                    "WORLD_SERVER": "https://goalcup-ingest-staging.b-9a5.workers.dev"}),
    "pruned_no_bucket": ("stubchat", {"STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                      "MATRIX_FETCH_INDEX": "/index.json",
                                      "MATRIX_FETCH_STORE": "/bucket-mirror"}),
    "pruned_env": ("stubchat", {"STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                "MATRIX_FETCH_BUCKET": "goalcup-traces-local", "DATABASE_URL": DB_URL, "CLOUDFLARE_ACCOUNT_ID": "acct",
                                "CLOUDFLARE_API_TOKEN": CF_TOKEN}),
    "pruned_bad_tail": ("stubchat", {"STUB_PRUNED": "1", "STUB_FETCH_BAD": "1",
                                     "MATRIX_FETCH_BUCKET": "goalcup-traces-local",
                                     "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                     "MATRIX_FETCH_INDEX": "/index.json",
                                     "MATRIX_FETCH_STORE": "/bucket-mirror"}),
    # A Docker cell's key: a private copy, removed once the start is approved (MATRIX_FORGET_KEY);
    # the control keeps it. The stubs show a device code and say whether the file is still there.
    "key_forgotten": ("stubchat", {"GOALCUP_TEST_APPROVAL_KEY_FILE": "{root}/approval-key",
                                   "MATRIX_FORGET_KEY": "1", "STUB_CODE": "1",
                                   "STUB_KEYFILE": "{root}/approval-key"}),
    "key_kept": ("stubchat", {"GOALCUP_TEST_APPROVAL_KEY_FILE": "{root}/approval-key",
                              "STUB_CODE": "1", "STUB_KEYFILE": "{root}/approval-key"}),
    # A Docker cell's fetch credentials: not in the container; matrix.sh asks the host when the
    # fetch needs them (the host answers, or does not).
    "pruned_handoff": ("stubchat", {
        "STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh", "CLOUDFLARE_ACCOUNT_ID": "acct",
        "MATRIX_FETCH_BUCKET": "goalcup-traces-staging", "MATRIX_FETCH_REQUEST": "{root}/fetch-request",
        "MATRIX_FETCH_WILL_GIVE": "db cf", "MATRIX_FETCH_SECRETS_DIR": "{root}/fetch-secrets",
        "STUB_HOST_ANSWERS": "1"}),
    "pruned_handoff_unanswered": ("stubchat", {
        "STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh", "CLOUDFLARE_ACCOUNT_ID": "acct",
        "MATRIX_FETCH_BUCKET": "goalcup-traces-staging", "MATRIX_FETCH_REQUEST": "{root}/fetch-request",
        "MATRIX_FETCH_WILL_GIVE": "db cf", "MATRIX_FETCH_SECRETS_DIR": "{root}/fetch-secrets",
        "MATRIX_FETCH_SECRETS_WAIT": "2"}),
    # A test run whose start asks for its repository: nobody can answer it.
    "test_asks": ("stubchat", {"STUB_ASK": "1"}, ("--test", "--duration", "30"), True),
    # A test run on a machine with no GitHub login.
    "test_no_gh_login": ("stubchat", {"STUB_GH_NO_LOGIN": "1"}, ("--test", "--duration", "30"), True),
    # The refusals, through a test build's doors; then a build without them; then a start that
    # gets past them to the approval.
    "refusals": ("m2-fake-tui", {}, ("--refusals",)),
    "refusals_shipped": ("m2-fake-tui", {"STUB_SHIPPED": "1"}, ("--refusals",)),
    "refusals_pass_through": ("m2-fake-tui", {"STUB_NEVER_REFUSES": "1"}, ("--refusals",)),
}


@unittest.skipUnless(shutil.which("script"), "needs script(1)")
class MatrixTests(unittest.TestCase):
    worlds: dict[str, World] = {}

    @classmethod
    def setUpClass(cls) -> None:
        names = list(SCENARIOS)
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(names)) as pool:
            futures = {n: pool.submit(World, *SCENARIOS[n]) for n in names}
            cls.worlds = {n: f.result() for n, f in futures.items()}
        for world in cls.worlds.values():
            cls.addClassCleanup(world.tmp.cleanup)

    def test_a_run_without_the_seal_line_fails_the_row(self) -> None:
        w = self.worlds["no_seal"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("and the seal", w.run.stderr)

    def test_a_connector_with_no_manifest_fails_and_says_where_to_look(self) -> None:
        w = self.worlds["no_manifest"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("GOALCUP_CONNECTORS_DIR", w.run.stderr)
        self.assertFalse((w.out / "start.argv").exists())

    def test_a_chat_connector_types_the_prompt_and_submits_it_apart(self) -> None:
        w = self.worlds["chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertEqual(w.seen("start.argv"), "start\n--harness\nstubchat\n--practice")
        # The `deadline:` line is in the run's host.log only, as goalcup has it in chat mode;
        # the row passes on it.
        terminal = next(w.tmpdir.rglob("start.out")).read_text()
        self.assertNotIn("deadline:", terminal)
        chunks = json.loads(w.seen("start.stdin_chunks"))
        keys, text, enter = chunks[0], chunks[1], chunks[2]
        self.assertEqual(keys[1], "\x1b[B\r")
        # The text arrives without its Enter, and the Enter comes on its own (a read of its own).
        self.assertEqual(text[1], PROMPT)
        self.assertEqual(enter[1], "\r")

    def test_a_tui_with_bracketed_paste_on_gets_the_prompt_as_one_paste(self) -> None:
        w = self.worlds["bracketed"]
        self.assertEqual(w.run.returncode, 0, w.said())
        chunks = json.loads(w.seen("start.stdin_chunks"))
        self.assertEqual(chunks[0][1], "\x1b[200~" + PROMPT + "\x1b[201~")
        self.assertEqual(chunks[1][1], "\r")

    def test_a_run_whose_agent_never_answered_fails_the_row(self) -> None:
        w = self.worlds["silent"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("the agent never answered the prompt", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")
        self.assertNotRegex(w.run.stdout, r"(?m) PASS ")

    def test_output_tokens_or_an_assistant_message_count_as_agent_work(self) -> None:
        # chat: a usage observation with output tokens; amp_version: an assistant message only.
        for name in ("chat", "amp_version"):
            w = self.worlds[name]
            self.assertEqual(w.run.returncode, 0, w.said())
            self.assertTrue(w.seen("research.argv").startswith("--research-json\n"))
            self.assertIn("agent work in the trace", w.log)

    def test_a_run_that_changed_nothing_in_its_workspace_fails_the_row(self) -> None:
        # A model answer (a title, a stalled turn) with a clean clone; and a clone whose only
        # commit predates the run.
        for name in ("ws_none", "ws_old"):
            w = self.worlds[name]
            self.assertNotEqual(w.run.returncode, 0, name)
            self.assertIn("the agent changed nothing in its workspace", w.run.stderr)
            self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")

    def test_a_new_file_or_a_new_commit_counts_as_a_changed_workspace(self) -> None:
        for name, seen in (("chat", "1 changed paths, 0 new commits"),
                           ("ws_commit", "0 changed paths, 1 new commits")):
            w = self.worlds[name]
            self.assertEqual(w.run.returncode, 0, w.said())
            self.assertIn(f"workspace: {seen} in ", w.log)

    def test_a_prompt_that_writes_nothing_can_opt_out_of_the_workspace_check(self) -> None:
        w = self.worlds["ws_none_opt_out"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertIn("MATRIX_EXPECT_FILES=0", w.log)

    def test_the_row_keeps_the_version_line_up_to_its_version_number(self) -> None:
        w = self.worlds["amp_version"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout,
                         r"(?m)^stubchat \S+ 0\.0\.1792209635-g24a9fe PASS \d{4}-\d\d-\d\d$")

    def test_a_tui_without_bracketed_paste_gets_the_raw_keys(self) -> None:
        w = self.worlds["chat"]
        chunks = json.loads(w.seen("start.stdin_chunks"))
        self.assertEqual(chunks[1][1], PROMPT)
        self.assertNotIn("\x1b[200~", "".join(c[1] for c in chunks))

    def test_a_test_run_is_sealed_after_its_duration_from_the_submitted_prompt(self) -> None:
        w = self.worlds["test_chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        # The row keeps its shape.
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ stub-harness_9\.9\.9 PASS \d{4}-\d\d-\d\d$")
        self.assertRegex(w.seen("start.argv"),
                         r"^start\n--harness\nstubchat\n--test\n--repo\nkit-tester/goalcup-kit-stubchat-[0-9a-f]{6}$")
        # The exact entry of this run, sealed no earlier than the duration after the Enter.
        self.assertEqual(w.seen("seal.argv"), "seal\n--yes\ne0123456789abcdef")
        waited = float(w.seen("seal.at")) - float(w.seen("start.submitted_at"))
        # matrix.sh counts whole seconds (date +%s) from when it sees the Enter's marker, so the
        # seal can come up to a second short of DURATION by the stub's own stamp.
        self.assertGreaterEqual(waited, DURATION - 1)
        self.assertLess(waited, DURATION + 20)
        # `goalcup start` reported the seal uploaded, so no upload was needed; the server's copy
        # passed without `stop`.
        self.assertFalse(w.has("upload.argv"))
        self.assertIn("mode=test duration=4s", w.log)
        self.assertIn(f"timeout={DURATION + 1800}s", w.log)
        self.assertNotIn("the run is over (stop)", w.log)
        self.assertIn("ok    status is sealed", w.log)

    def test_a_multi_line_prompt_file_is_typed_on_one_line(self) -> None:
        w = self.worlds["test_chat"]
        chunks = json.loads(w.seen("start.stdin_chunks"))
        self.assertEqual(chunks[0][1], TYPED)
        self.assertEqual(chunks[1][1], "\r")

    def test_a_practice_run_has_no_git_login_and_never_the_machines(self) -> None:
        w = self.worlds["chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        git = w.seen_json("start.git")
        # A git config of the run's own that resets every credential helper, the system config
        # (macOS's names the keychain) off, and no SSH agent, keyring or token.
        self.assertEqual(git["gitconfig"], ["credential.helper="])
        # The script works in `pwd -P` paths (/private/var/... on macOS): compare resolved paths.
        self.assertTrue(Path(git["GIT_CONFIG_GLOBAL"]).resolve().is_relative_to(w.tmpdir.resolve()))
        self.assertEqual(git["GIT_CONFIG_NOSYSTEM"], "1")
        self.assertTrue(Path(git["GH_CONFIG_DIR"]).resolve().is_relative_to(w.tmpdir.resolve()))
        for name in ("SSH_AUTH_SOCK", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "GH_TOKEN"):
            self.assertIsNone(git[name], name)
        self.assertNotIn("--repo", w.seen("start.argv"))
        self.assertFalse(w.has("gh.repo_create"))

    def test_a_test_run_makes_its_repository_and_uses_the_machines_own_login(self) -> None:
        w = self.worlds["test_chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        made = w.seen("gh.repo_create").split("\n")
        # Public and with no README, so it stays empty; passed to the start; named at the end.
        self.assertEqual(made[:2], ["repo", "create"])
        self.assertEqual(made[3:], ["--public"])
        self.assertTrue(w.seen("start.argv").endswith("--repo\n" + made[2]))
        # The default `gh auth login` lacks the delete_repo scope: the advice adds it first.
        self.assertIn(f"test repository made by the kit (delete it when done: first `gh auth refresh -s "
                      f"delete_repo`, which the default gh login lacks, then `gh repo delete {made[2]} --yes`)",
                      w.run.stderr)
        git = w.seen_json("start.git")
        self.assertEqual(git["SSH_AUTH_SOCK"], HOST_SSH)
        self.assertEqual(git["GH_TOKEN"], "gh-token-of-this-machine")
        for name in ("GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM", "GH_CONFIG_DIR"):
            self.assertIsNone(git[name], name)

    def test_a_test_run_takes_the_repository_matrix_repo_names(self) -> None:
        w = self.worlds["test_chat_early"]
        self.assertTrue(w.seen("start.argv").endswith("--repo\n" + GIVEN_REPO))
        self.assertFalse(w.has("gh.repo_create"))
        self.assertNotIn("made by the kit", w.run.stderr)

    def test_a_test_run_with_no_github_login_says_to_sign_in_and_stops(self) -> None:
        w = self.worlds["test_no_gh_login"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("no GitHub login found here: run `gh auth login`", w.run.stderr)
        self.assertFalse(w.has("start.argv"))
        self.assertFalse(w.has("gh.repo_create"))

    def test_a_question_before_the_approval_fails_the_row_at_once(self) -> None:
        w = self.worlds["test_asks"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("asked a question before the run's approval", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")
        self.assertLess(w.elapsed, 50)

    def test_every_refusal_is_checked_through_a_test_builds_doors(self) -> None:
        w = self.worlds["refusals"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout, r"(?m)^m2-fake-tui \S+ refusals PASS \d{4}-\d\d-\d\d$")
        for kind in ("not-empty", "private", "missing", "push", "no-login", "bare-name"):
            self.assertIn(f"refusal {kind}: PASS", w.run.stderr)
            seen = w.seen_json(f"refuse.{kind}")
            self.assertFalse(seen["tty"], kind)
            self.assertEqual(seen["argv"][:5], ["start", "--harness", "m2-fake-tui", "--dev", "--test"])
            # No git login of any kind reaches a refusal cell.
            self.assertEqual(seen["git"]["gitconfig"], ["credential.helper="], kind)
            self.assertEqual(seen["git"]["GIT_CONFIG_NOSYSTEM"], "1", kind)
            self.assertIsNone(seen["git"]["SSH_AUTH_SOCK"], kind)
        # GitHub is asked only where the refusal is GitHub's answer; the login door is open only
        # where a login is found.
        for kind in ("not-empty", "private", "missing", "push"):
            doors = w.seen_json(f"refuse.{kind}")["doors"]
            self.assertTrue(doors["GOALCUP_TEST_GITHUB_API"].startswith("http://127.0.0.1:"))
            self.assertEqual(doors["GOALCUP_TEST_GITHUB_LOGIN"], "goalcup-kit")
        # Only the push refusal points the dry-run push at a remote that is not there.
        self.assertFalse(Path(w.seen_json("refuse.push")["doors"]["GOALCUP_TEST_PUSH_REMOTE"]).exists())
        self.assertIsNone(w.seen_json("refuse.missing")["doors"]["GOALCUP_TEST_PUSH_REMOTE"])
        for kind in ("no-login", "bare-name"):
            self.assertEqual(w.seen_json(f"refuse.{kind}")["doors"],
                             {"GOALCUP_TEST_GITHUB_API": None, "GOALCUP_TEST_GITHUB_LOGIN": None,
                              "GOALCUP_TEST_PUSH_REMOTE": None})

    def test_a_build_without_the_test_doors_fails_the_refusals(self) -> None:
        w = self.worlds["refusals_shipped"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertRegex(w.run.stdout, r"(?m)^m2-fake-tui \S+ refusals FAIL ")
        self.assertIn("refusal not-empty: FAIL: it refused for another reason", w.run.stderr)
        self.assertIn("no test doors", w.run.stderr)

    def test_a_start_that_reaches_the_approval_fails_its_refusal(self) -> None:
        w = self.worlds["refusals_pass_through"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("refusal missing: FAIL: the run's approval was asked for", w.run.stderr)
        self.assertNotIn(": PASS", w.run.stderr)

    def test_a_test_run_whose_harness_ends_first_is_sealed_then_uploaded(self) -> None:
        w = self.worlds["test_chat_early"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout, r"stubchat \S+ stub-harness_9\.9\.9 PASS ")
        self.assertEqual(w.seen("start.argv"),
                         f"start\n--harness\nstubchat\n--test\n--repo\n{GIVEN_REPO}")
        # Sealed when the harness ended, before the 30 s duration: counted from the Enter, as the
        # duration is, so a slow start on a loaded machine does not count against it.
        self.assertEqual(w.seen("seal.argv"), "seal\n--yes\ne0123456789abcdef")
        self.assertIn("(the harness ended first)", w.log)
        self.assertLess(float(w.seen("seal.at")) - float(w.seen("start.submitted_at")), 30)
        # start did not upload the seal, so the script ran `goalcup upload` for the same entry.
        self.assertEqual(w.seen("upload.argv"), "upload\ne0123456789abcdef")
        self.assertIn("timeout=1830s", w.log)

    def test_an_entry_goalcup_entries_does_not_list_is_never_sealed(self) -> None:
        w = self.worlds["test_unlisted"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn(" FAIL ", w.run.stdout)
        self.assertIn("does not list entry e0123456789abcdef", w.run.stderr)
        self.assertTrue(w.has("entries.argv"))
        self.assertEqual(w.seen("entries.argv"), "entries\n--json")
        self.assertFalse(w.has("seal.argv"), "an entry was sealed on a guess")
        self.assertIn("may still be open on the server", w.log)

    def test_a_test_run_that_fails_after_its_entry_was_found_is_sealed_on_the_way_out(self) -> None:
        w = self.worlds["test_timeout"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("did not finish within 12s", w.run.stderr)
        self.assertEqual(w.seen("seal.argv"), "seal\n--yes\ne0123456789abcdef")
        self.assertIn("sealing it now", w.log)

    def test_a_seal_still_running_at_the_end_is_waited_for_not_killed(self) -> None:
        w = self.worlds["test_seal_slow"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("is still running; waiting for it", w.log)
        self.assertIn("on the way out: exit 0", w.log)
        self.assertEqual(w.seen("seal.argv"), "seal\n--yes\ne0123456789abcdef")

    def test_a_long_run_without_its_server_copy_is_not_verified_and_not_a_pass(self) -> None:
        w = self.worlds["pruned_unfetched"]
        self.assertEqual(w.run.returncode, 4, w.said())
        self.assertIn("NOT VERIFIED: long run, server copy not fetched", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ NOT-VERIFIED \d{4}-")
        self.assertNotRegex(w.run.stdout, r"(?m) (PASS|FAIL) ")
        self.assertFalse(w.has("fetch.argv"))

    def test_a_long_run_is_verified_against_its_fetched_server_copy(self) -> None:
        w = self.worlds["pruned_files"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ stub-harness_9\.9\.9 PASS ")
        argv = w.seen("fetch.argv").split("\n")
        self.assertEqual(argv[:6], ["--bucket", "goalcup-traces-local", "--index", "/index.json",
                                    "--store", "/bucket-mirror"])
        self.assertEqual(argv[6], "e0123456789abcdef")
        # The verifier and the agent-work check judged the copy, and the tail matched it.
        self.assertIn("servercopy/e0123456789abcdef", w.seen("research.argv"))
        self.assertRegex(w.log, r"ok +local chain\.\d{20}\.jsonl")
        self.assertIn("ends where the server copy ends", w.log)

    def test_the_staging_bucket_is_assumed_for_the_staging_worker_only(self) -> None:
        w = self.worlds["pruned_staging"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertEqual(w.seen("fetch.argv").split("\n")[:2], ["--bucket", "goalcup-traces-staging"])

    def test_a_long_run_on_another_server_needs_its_bucket_named(self) -> None:
        w = self.worlds["pruned_no_bucket"]
        self.assertEqual(w.run.returncode, 4, w.said())
        self.assertIn("MATRIX_FETCH_BUCKET", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ NOT-VERIFIED \d{4}-")
        self.assertFalse(w.has("fetch.argv"), "the staging bucket was assumed for an unknown server")

    def test_fetch_credentials_reach_only_the_fetch(self) -> None:
        w = self.worlds["pruned_env"]
        self.assertEqual(w.run.returncode, 0, w.said())
        got = json.loads(w.seen("fetch.env"))
        self.assertEqual(got, {"DATABASE_URL": DB_URL, "CLOUDFLARE_ACCOUNT_ID": "acct",
                               "CLOUDFLARE_API_TOKEN": CF_TOKEN})
        self.assertEqual(json.loads(w.seen("start.env")), [])
        self.assertNotIn("--index", w.seen("fetch.argv"))
        for secret in ("db-secret-never-shown", CF_TOKEN):
            self.assertNotIn(secret, w.everything_matrix_wrote())
            self.assertNotIn(secret, w.seen("fetch.argv"))

    def test_the_key_file_is_removed_once_the_start_is_approved_and_not_before(self) -> None:
        w = self.worlds["key_forgotten"]
        self.assertEqual(w.run.returncode, 0, w.said())
        # Both approvals used the key, the login's and the start's.
        self.assertIn(f"Bearer {KEY_VALUE}", w.seen("approved.1"))
        self.assertIn(f"Bearer {KEY_VALUE}", w.seen("approved.2"))
        # After the login's approval the file is still there; after the start's it is gone, before
        # the harness (the stub's start) goes on.
        self.assertEqual(w.seen("login.keyfile_exists"), "True")
        self.assertEqual(w.seen("start.keyfile_exists"), "False")
        self.assertFalse((w.root / "approval-key").exists())
        self.assertIn("the approval key's file is removed", w.log)
        self.assertNotIn(KEY_VALUE, w.everything_matrix_wrote())

    def test_without_the_forget_setting_the_key_file_is_left_alone(self) -> None:
        w = self.worlds["key_kept"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertEqual(w.seen("start.keyfile_exists"), "True")
        self.assertTrue((w.root / "approval-key").exists())

    def test_fetch_credentials_are_asked_of_the_host_and_removed_at_once(self) -> None:
        w = self.worlds["pruned_handoff"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertEqual(json.loads(w.seen("fetch.env")), {
            "DATABASE_URL": DB_URL, "CLOUDFLARE_ACCOUNT_ID": "acct", "CLOUDFLARE_API_TOKEN": CF_TOKEN})
        # The harness (the stub's start) never had them, and the private directory was 0700.
        self.assertEqual(json.loads(w.seen("start.env")), [])
        self.assertEqual(w.request_seen["mode"], "0o700")
        self.assertFalse((w.root / "fetch-secrets").exists(), "the credentials were left on disk")
        self.assertFalse((w.root / "fetch-request").exists())
        for secret in ("db-secret-never-shown", CF_TOKEN):
            self.assertNotIn(secret, w.everything_matrix_wrote())

    def test_credentials_the_host_never_hands_over_make_the_row_not_verified(self) -> None:
        w = self.worlds["pruned_handoff_unanswered"]
        self.assertEqual(w.run.returncode, 4, w.said())
        self.assertIn("did not hand over the fetch credentials within 2s", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ NOT-VERIFIED \d{4}-")
        self.assertFalse(w.has("fetch.argv"))
        self.assertFalse((w.root / "fetch-secrets").exists())
        self.assertFalse((w.root / "fetch-request").exists())

    def test_a_local_tail_that_differs_from_the_server_copy_fails(self) -> None:
        w = self.worlds["pruned_bad_tail"]
        self.assertEqual(w.run.returncode, 1, w.said())
        self.assertIn("not the server copy's tail", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")

    def test_a_bad_command_line_names_the_problem_in_one_line(self) -> None:
        for args in (("stubchat",), ("--frob", "a", "b"), ("a", "b", "c"), ("a", "b", "--log")):
            with self.subTest(args=args):
                run = subprocess.run(["sh", str(MATRIX), *args], stdin=subprocess.DEVNULL,
                                     capture_output=True, text=True, timeout=30)
                self.assertEqual(run.returncode, 2)
                self.assertEqual(len(run.stderr.strip().splitlines()), 1, run.stderr)
                self.assertIn("see --help", run.stderr)

    def test_a_prompt_file_with_a_control_character_is_refused(self) -> None:
        w = self.worlds["prompt_control"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("control character", w.run.stderr)
        self.assertFalse(w.has("start.argv"))


class MatrixArgumentTests(unittest.TestCase):
    """Refusals that come before anything runs: exit 2, nothing started."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.cfg = self.root / "cfg"
        self.cfg.mkdir()
        self.env = {"PATH": os.environ["PATH"], "HOME": str(self.root), "TMPDIR": str(self.root)}

    def refuse(self, *args: str, **env: str) -> str:
        run = subprocess.run(["sh", str(MATRIX), *args], env={**self.env, **env},
                             stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30)
        self.assertEqual(run.returncode, 2, run.stdout + run.stderr)
        self.assertEqual(run.stdout, "")
        return run.stderr

    def verdict(self, server: str, *args: str, **env: str) -> tuple[bool, str]:
        """Whether the matrix refused the server for not being on the allowlist, and what it said.
        A server that is allowed goes on to the health check, which fails here on a curl stub that
        reaches no network (it must never touch a real server)."""
        stubs = self.root / "stub-bin"
        stubs.mkdir(exist_ok=True)
        executable(stubs / "curl", "#!/bin/sh\nprintf 000\nexit 7\n")
        run = subprocess.run(["sh", str(MATRIX), "m2-fake-tui", server, "--timeout", "5", *args],
                             env={**self.env, "PATH": f"{stubs}:{self.env['PATH']}", **env},
                             stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30)
        if run.returncode != 2:
            self.assertIn("did not answer /v1/health", run.stderr, run.stdout + run.stderr)
        return refused_for_server(run), run.stderr

    def test_only_staging_and_this_machine_are_servers_unless_one_is_named_exactly(self) -> None:
        for server in ("https://goalcup-ingest-staging.b-9a5.workers.dev",
                       "https://goalcup-ingest-staging.b-9a5.workers.dev/",
                       "https://GOALCUP-INGEST-STAGING.b-9a5.workers.dev",
                       "http://localhost:8787", "http://127.0.0.1:8787", "http://[::1]:8787",
                       "http://localhost"):
            with self.subTest(allowed=server):
                refused, said = self.verdict(server)
                self.assertFalse(refused, said)
        for server in ("https://example.com", "https://staging.example.com",
                       "https://goalcup-ingest-staging.b-9a5.workers.dev.evil.example",
                       "https://evil.example/goalcup-ingest-staging.b-9a5.workers.dev",
                       "https://goalcup-ingest-staging.b-9a5.workers.dev:8443",
                       "https://localhost:8787", "http://localhost.evil.example:8787"):
            with self.subTest(refused=server):
                refused, said = self.verdict(server)
                self.assertTrue(refused, said)

    def test_a_server_off_the_list_is_allowed_only_when_named_exactly(self) -> None:
        server = "https://staging.example.com"
        for named in (server, server + "/", server.upper()):
            with self.subTest(named=named):
                refused, said = self.verdict(server, MATRIX_ALLOW_SERVER=named)
                self.assertFalse(refused, said)
        for named in ("https://other.example.com", "*", "staging.example.com", "https://example.com",
                      server + ".evil.example", ""):
            with self.subTest(named=named):
                refused, said = self.verdict(server, MATRIX_ALLOW_SERVER=named)
                self.assertTrue(refused, said)

    def test_the_allowlist_never_lets_production_through(self) -> None:
        prod = "https://ingest.agentcommunity.org"
        for server in (prod, prod + "/", "https://INGEST.agentcommunity.org", "https://x.ingest.agentcommunity.org"):
            with self.subTest(server=server):
                refused, said = self.verdict(server, MATRIX_ALLOW_SERVER=server)
                self.assertIn("refusing ingest.agentcommunity.org", said)

    def test_the_production_flag_lets_only_the_production_address_through(self) -> None:
        prod = "https://ingest.agentcommunity.org"
        refused, said = self.verdict(prod, "--allow-production")
        self.assertNotIn("refusing", said)
        for server in (prod + ".evil.example", "https://evil.example/ingest.agentcommunity.org"):
            with self.subTest(server=server):
                refused, said = self.verdict(server, "--allow-production")
                self.assertIn("refusing ingest.agentcommunity.org", said)

    def test_the_allow_variable_does_not_reach_the_harness(self) -> None:
        # It is read and removed like the key's path: nothing goalcup starts inherits it.
        text = MATRIX.read_text()
        self.assertIn("unset MATRIX_ALLOW_SERVER", text)

    def test_test_and_duration_go_together(self) -> None:
        cfg = {"MATRIX_CONFIG_DIR": str(self.cfg)}
        self.assertIn("go together", self.refuse("codex-cli", "https://x.example", "--test", **cfg))
        self.assertIn("go together",
                      self.refuse("codex-cli", "https://x.example", "--duration", "60", **cfg))

    def test_expect_files_is_zero_or_one(self) -> None:
        self.assertIn("MATRIX_EXPECT_FILES is 0 or 1",
                      self.refuse("codex-cli", "https://x.example", MATRIX_EXPECT_FILES="no"))

    def test_duration_is_a_whole_number_of_seconds_in_range(self) -> None:
        for bad in ("0", "abc", "-5", "1.5", "604801", "0100"):
            with self.subTest(bad=bad):
                self.assertIn("--duration", self.refuse(
                    "codex-cli", "https://x.example", "--test", "--duration", bad,
                    MATRIX_CONFIG_DIR=str(self.cfg)))

    def test_a_test_run_needs_a_real_connector(self) -> None:
        for fake in ("m2-fake-tui",):
            with self.subTest(fake=fake):
                self.assertIn("real connector", self.refuse(
                    fake, "https://x.example", "--test", "--duration", "60",
                    MATRIX_CONFIG_DIR=str(self.cfg)))

    def test_a_test_run_needs_a_prepared_configuration_directory(self) -> None:
        self.assertIn("MATRIX_CONFIG_DIR", self.refuse(
            "codex-cli", "https://x.example", "--test", "--duration", "60"))

    def test_a_named_test_repository_is_owner_slash_name_and_never_the_practice_one(self) -> None:
        test = ("codex-cli", "https://x.example", "--test", "--duration", "60")
        cfg = {"MATRIX_CONFIG_DIR": str(self.cfg)}
        for bad in ("just-a-name", "a/b/c", "a/b c"):
            with self.subTest(bad=bad):
                self.assertIn("<owner>/<name>", self.refuse(*test, **cfg, MATRIX_REPO=bad))
        # The same repository however it is written: case, a trailing .git, a trailing /.
        for same in ("agentcommunity/goalcup-matrix-empty", "AgentCommunity/goalcup-matrix-empty",
                     "agentcommunity/goalcup-matrix-empty.git", "AgentCommunity/GoalCup-Matrix-Empty.GIT",
                     "agentcommunity/goalcup-matrix-empty/", "agentcommunity/goalcup-matrix-empty.git/",
                     "agentcommunity/goalcup-matrix-empty//"):
            with self.subTest(same=same):
                self.assertIn("practice repository", self.refuse(*test, **cfg, MATRIX_REPO=same))
        # A different repository is not it.
        self.assertNotIn("practice repository", self.refuse(
            *test, **cfg, MATRIX_REPO="agentcommunity/goalcup-matrix-empty-2/x"))
        self.assertIn("MATRIX_CELL", self.refuse(*test, **cfg, MATRIX_CELL="Bad_Cell"))

    def test_refusals_go_alone_with_the_fake(self) -> None:
        self.assertIn("m2-fake-tui only", self.refuse("codex-cli", "https://x.example", "--refusals"))
        self.assertIn("goes alone", self.refuse("m2-fake-tui", "https://x.example", "--refusals",
                                                "--test", "--duration", "60",
                                                MATRIX_CONFIG_DIR=str(self.cfg)))

    def test_check_copy_takes_no_test_options(self) -> None:
        self.refuse("--check-copy", str(self.cfg), "--test", "--duration", "60")

    def test_prompt_and_prompt_file_are_not_both_given(self) -> None:
        f = self.root / "p.txt"
        f.write_text("x")
        self.assertIn("not both", self.refuse("codex-cli", "https://x.example",
                                              MATRIX_PROMPT="y", MATRIX_PROMPT_FILE=str(f)))
        self.assertIn("MATRIX_PROMPT_FILE", self.refuse(
            "codex-cli", "https://x.example", MATRIX_PROMPT_FILE=str(self.root / "missing")))


if __name__ == "__main__":
    unittest.main()
