#!/usr/bin/env python3
"""Behavioral tests for the matrix, scripts/matrix.sh, against stand-ins for everything it drives.

`goalcup`, `goalcup-verify`, `goalcup-daemon`, `gh` and `curl` are stubs in a scratch directory (the
real ones are covered by running the script, docs/OPERATING.md "The matrix"). The `goalcup` stub
records what it was started with: its arguments, the parts of its environment that matter (the git
git variables it was given among them), the folder it was started in and what that held, and what
arrived on its standard input. Every connector is chat mode (D57), so every run is on a
pseudo-terminal (script(1)); the refusals run with no terminal. Every key and token here is
synthetic.
"""

from __future__ import annotations

import base64
import concurrent.futures
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

MATRIX = Path(__file__).with_name("matrix.sh")

PROMPT = "Goal Cup test prompt: say ok."
GIVEN_REPO = "goalcup-kit/kit-given"
TOKEN = "stub-gh-token-0123456789abcdef"
STUB_TOKEN_VALUE = TOKEN
FILE_TOKEN = "github_pat_fine-grained-0123456789"
CLASSIC_TOKEN = "ghp_classic0123456789abcdefABCDEF0123"

GOALCUP = r'''#!/usr/bin/env python3
import base64, hashlib, json, os, select, subprocess, sys, termios, time, tty
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
    note("%s.approved" % phase, "yes")

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

STUB_TOKEN = "stub-gh-token-0123456789abcdef"  # what the gh stub prints for `gh auth token`

def git_env():
    """What the kit gave this start of a git login: nothing (D71)."""
    names = ("GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM", "GH_CONFIG_DIR", "SSH_AUTH_SOCK", "GH_TOKEN",
             "GITHUB_TOKEN", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "SSH_AGENT_PID", "GIT_CONFIG_COUNT",
             "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0", "XDG_CONFIG_HOME")
    return {k: os.environ.get(k) for k in names}

def project_facts():
    """The folder this start was run from, and what the starting kit in it looks like."""
    here = Path.cwd()
    env_local = here / ".env.local"
    return {"cwd": str(here), "names": sorted(os.listdir(here)),
            "gitignore": (here / ".gitignore").read_text() if (here / ".gitignore").is_file() else None,
            "env_local_mode": oct(env_local.stat().st_mode & 0o777) if env_local.is_file() else None,
            "env_local_sha256": hashlib.sha256(env_local.read_bytes()).hexdigest() if env_local.is_file() else None,
            "token_in_argv": STUB_TOKEN in " ".join(args),
            "token_in_env": any(STUB_TOKEN in v for v in os.environ.values())}

if os.environ.get("GOALCUP_TEST_CHAT_WITHOUT_TERMINAL") == "1":
    # A refusal cell: the start's checks before the approval, as a test build makes them through its
    # doors, with no terminal. The folder is judged first, then the repository, as goalcup does.
    # (The .gitignore and commits checks are the task 2b-CLI ones; the phrases are the short ones.)
    import urllib.error, urllib.request
    kind = Path.cwd().name.removeprefix("refuse-")
    doors = {k: os.environ.get(k) for k in ("GOALCUP_TEST_GITHUB_API", "GOALCUP_TEST_GITHUB_LOGIN",
                                            "GOALCUP_TEST_PUSH_REMOTE")}
    note("refuse.%s" % kind, json.dumps({"argv": args, "doors": doors, "tty": os.isatty(0),
                                         "git": git_env(), "project": project_facts()}))
    def refuse(why):
        sys.stderr.write("goalcup: %s Nothing was created.\n" % why)
        sys.exit(1)
    if os.environ.get("STUB_SHIPPED") == "1":
        refuse("stub needs a terminal of its own.")
    if os.environ.get("STUB_NEVER_REFUSES") == "1":
        print("Approve this run in your browser: the page shows the code ABCD-EFGH.", flush=True)
        sys.exit(1)
    names = sorted(os.listdir("."))
    extra = [n for n in names if not (n == ".gitignore" or n.startswith(".env"))]
    if extra:
        refuse("Make a new, empty folder for this run. This folder (%s) also holds %s." % (os.getcwd(), ", ".join(extra)))
    if any(n.startswith(".env") for n in names) and ".gitignore" not in names:
        refuse("Add these two lines to the .gitignore in this folder. .gitignore does not cover .env.local.")
    repo = args[args.index("--repo") + 1] if "--repo" in args else None
    if repo is None:
        refuse("A real run needs a new, empty, public GitHub repository.")
    if "/" not in repo:
        refuse("Give the repository with its owner: <your-user>/%s." % repo)
    api = doors["GOALCUP_TEST_GITHUB_API"]
    if api:
        def get(path):
            try:
                resp = urllib.request.urlopen(api + path, timeout=10)
                return resp.status, json.load(resp)
            except urllib.error.HTTPError as e:
                return e.code, None
        _, doc = get("/repos/" + repo)
        if doc is None or doc["private"]:
            refuse("GitHub does not show %s: it is not created yet, or it is private." % repo)
        if doc["size"]:
            refuse("The repository must be empty when the run opens. %s already has commits (%d KB); name the new one."
                   % (repo, doc["size"]))
        if get("/repos/%s/commits?per_page=1" % repo)[0] == 200:
            refuse("The repository must be empty when the run opens. %s already has commits; name the new one." % repo)
    print("Approve this run in your browser: the page shows the code ABCD-EFGH.", flush=True)
    sys.exit(1)

note("start.argv", "\n".join(args))
note("start.no_browser", os.environ.get("GOALCUP_NO_BROWSER", "unset"))
note("start.stdin_is_tty", str(os.isatty(0)))
note("start.git", json.dumps(git_env()))
note("start.project", json.dumps(project_facts()))
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
    # What the agent left in the project folder it was started in (STUB_WS): file (a new file,
    # the default), commit (an empty commit in a repository of its own, no new file), git (only
    # a .git), none (nothing).
    mode = os.environ.get("STUB_WS", "file")
    git = lambda *a: subprocess.run(
        ["git", "-C", str(ws), "-c", "user.name=stub", "-c", "user.email=stub@example.invalid", *a],
        check=True, capture_output=True)
    if mode == "file":
        (ws / "matrix-ok.txt").write_text("ok\n")
    elif mode == "commit":
        git("init", "-q")
        git("commit", "-q", "--allow-empty", "-m", "ok")
    elif mode == "git":
        git("init", "-q")
    elif mode == "leak":
        # The agent commits the token (as the base64 of an Authorization-style pair).
        (ws / "leak.txt").write_text(base64.b64encode(("x-access-token:" + STUB_TOKEN).encode()).decode() + "\n")
        git("init", "-q")
        git("add", "leak.txt")
        git("commit", "-q", "-m", "oops")
    elif mode == "leak-binary":
        # ...in a binary file its .gitattributes keeps out of any diff (`git log -p` shows no bytes).
        (ws / ".gitattributes").write_text("*.bin -diff\n")
        (ws / "blob.bin").write_bytes(b"\0\1\2" + STUB_TOKEN.encode() + b"\0\xff")
        git("init", "-q")
        git("add", ".gitattributes", "blob.bin")
        git("commit", "-q", "-m", "data")
    elif mode == "leak-submodule":
        # ...only in a submodule's repository under .git/modules, with no checkout pointing at it.
        git("init", "-q")
        sub = ws / ".git" / "modules" / "sub"
        subprocess.run(["git", "init", "-q", "--bare", str(sub)], check=True)
        subprocess.run(["git", "--git-dir", str(sub), "hash-object", "-w", "--stdin"],
                       input=("GH_TOKEN=" + STUB_TOKEN + "\n").encode(), check=True, capture_output=True)
    elif mode == "leak-objects-link":
        # ...in a repository whose .git/objects is a link to a folder outside the project folder.
        (ws / "leak.txt").write_text("GH_TOKEN=" + STUB_TOKEN + "\n")
        git("init", "-q")
        git("add", "leak.txt")
        git("commit", "-q", "-m", "oops")
        away = ws.with_name(ws.name + "-objects")
        (ws / ".git" / "objects").rename(away)
        (ws / ".git" / "objects").symlink_to(away)
    elif mode == "leak-newline":
        # ...in a bare repository whose folder name holds a line break: "x<newline>y", beside a
        # clean bare repository "x", so a search split on lines reads only the clean one.
        for name in ("x", "x\ny"):
            subprocess.run(["git", "init", "-q", "--bare", str(ws / name)], check=True)
        subprocess.run(["git", "--git-dir", str(ws / "x\ny"), "hash-object", "-w", "--stdin"],
                       input=("GH_TOKEN=" + STUB_TOKEN + "\n").encode(), check=True, capture_output=True)
    elif mode == "lfs-submodule":
        # A clean submodule repository with a Git LFS store inside it (not a git repository).
        (ws / "matrix-ok.txt").write_text("ok\n")
        git("init", "-q")
        sub = ws / ".git" / "modules" / "sub"
        subprocess.run(["git", "init", "-q", "--bare", str(sub)], check=True)
        lfs = sub / "lfs" / "objects" / "ab" / "cd"
        lfs.mkdir(parents=True)
        (lfs / ("abcd" + "0" * 60)).write_bytes(b"large file\n")
    elif mode == "project-link":
        # The agent moves its folder away and leaves a link in its place.
        (ws / "matrix-ok.txt").write_text("ok\n")
        moved = ws.with_name(ws.name + "-moved")
        ws.rename(moved)
        ws.symlink_to(moved)
    victim = os.environ.get("STUB_PLANT_LINKS")
    if victim:
        # The agent plants links where the kit writes its own files, pointing at a file of the
        # owner's: none of the kit's writes may go through them.
        Path(victim).write_text("the owner's file\n")
        # (Not the files whose presence the kit waits for: a link there only ends the cell early.)
        for name in ("seal.out", "seal.err", "seal.pid", "entries.out", "start.txt", "ended.txt",
                     "verify.out", "verify.err", "research.err", "compare.out"):
            target = ws.parent.parent / name
            if target.exists() or target.is_symlink():
                target.unlink()
            target.symlink_to(victim)
    tamper = os.environ.get("STUB_TAMPER_COPY")
    if tamper:
        # The agent reaches the kit's private copy of its token (../../private from its folder):
        # it deletes it, or rewrites it so the push check would look for something else.
        copy = ws.parent.parent / "private" / "gh-token"
        if tamper == "delete":
            copy.unlink()
        else:
            copy.write_text("GH_TOKEN=not-the-token-at-all\n")

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
    workspace = Path.cwd()
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
if os.environ.get("STUB_PRINT_TOKEN") in ("1", "wrapped"):
    # An agent that prints its own token to the terminal the matrix logs.
    said = (Path.cwd() / ".env.local").read_text().strip()
    if os.environ["STUB_PRINT_TOKEN"] == "wrapped":
        # ...with escape sequences (a character set, as `tput sgr0` prints, and bold) and a line
        # wrap inside it, and another token of the same shape.
        value = said.split("=", 1)[1]
        said = "GH_TOKEN=" + value[:12] + "\x1b(B\x1b[1m" + value[12:20] + "\r\n" + value[20:]
        said += " and ghp_" + "a1B2c3D4e5" * 4
    print("agent said: " + said, flush=True)
    if os.environ.get("STUB_DELETE_ENV") == "1":
        # ...and removes the file, so only the kit's own record of the token can still scrub the log.
        (Path.cwd() / ".env.local").unlink()
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
            "final": {"seq": 0, "head": "a" * 64},
            # A run is sealed on the server only once it is over (its deadline has passed): a test
            # run sealed after its duration is still `open` there (F2).
            "status": "sealed" if os.environ.get("STUB_STOP", "1") == "1" else "open",
            "stop": os.environ.get("STUB_STOP", "1") == "1"}
elif url.endswith("/v1/health"):
    body = {"status": "ok"}
else:
    body = {"status": "approved"}
Path(target).write_text(json.dumps(body))
sys.stdout.write("200")
'''

# gh as the kit uses it: this machine's login makes a test run's repository and gives its token (no
# login with STUB_GH_NO_LOGIN).
GH = r'''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
out = Path(os.environ["STUB_OUT"])
if os.environ.get("STUB_GH_NO_LOGIN") == "1" and args[:1] in (["api"], ["auth"]):
    sys.exit("To get started with GitHub CLI, please run:  gh auth login")
if args[:2] == ["api", "user"]:
    print("kit-tester")
elif args[:2] == ["auth", "token"]:
    if os.environ.get("STUB_GH_TOKEN_ONCE") == "1" and (out / "gh.auth_token").exists():
        # The login is gone by the end of the run (signed out, or another machine's).
        sys.exit("no oauth token found for github.com")
    (out / "gh.auth_token").write_text("asked")
    print("stub-gh-token-0123456789abcdef")
elif args[:2] == ["repo", "create"]:
    (out / "gh.repo_create").write_text("\n".join(args))
    print("https://github.com/" + args[2])
else:
    sys.exit("gh stub: " + " ".join(args))
'''

# git as the kit's push check uses it: `clone --mirror` makes a local bare repository
# (STUB_CLONE_LEAK=1: one whose commit holds the token; =tag: one whose only trace of it is an
# annotated tag's message; STUB_CLONE_FAIL: none), everything else is the real git.
def git_stub(real: str) -> str:
    return f'''#!/bin/sh
for a in "$@"; do
  if [ "$a" = clone ]; then
    for last; do :; done
    echo "$*" >"$STUB_OUT/git.clone"
    if [ "${{STUB_CLONE_FAIL:-}}" = 1 ]; then echo "fatal: unable to access" >&2; exit 128; fi
    if [ "${{STUB_CLONE_SWAP_COPY:-}}" = 1 ]; then
      # A process of the agent's still running swaps the kit's token copy while the clone runs
      # (after the copy was checked): the scan must not search for the swapped value.
      echo "GH_TOKEN=a-value-the-agent-chose" >"$(dirname "$last")/private/gh-token"
    fi
    case " $* " in *" --mirror "*) {real} init -q --bare "$last" || exit 1 ;; *) {real} init -q "$last" || exit 1 ;; esac
    if [ -n "${{STUB_CLONE_LEAK:-}}" ]; then
      g="{real} --git-dir=$last -c user.name=stub -c user.email=stub@example.invalid"
      [ -d "$last/.git" ] && g="{real} --git-dir=$last/.git -c user.name=stub -c user.email=stub@example.invalid"
      if [ "$STUB_CLONE_LEAK" = tag ]; then text="ok"; else text="GH_TOKEN={STUB_TOKEN_VALUE}"; fi
      blob=$(echo "$text" | $g hash-object -w --stdin) || exit 1
      tree=$(printf '100644 blob %s\\tpushed.txt\\n' "$blob" | $g mktree) || exit 1
      commit=$($g commit-tree -m pushed "$tree") || exit 1
      $g update-ref refs/heads/main "$commit" || exit 1
      if [ "$STUB_CLONE_LEAK" = tag ]; then
        $g tag -a v1 -m "release notes: GH_TOKEN={STUB_TOKEN_VALUE}" "$commit" || exit 1
      fi
    fi
    exit 0
  fi
done
exec {real} "$@"
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
        token_file = (env or {}).get("WORLD_TOKEN_FILE")
        self.stray: list[Path] = []  # files outside the scratch tree to remove afterwards
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
        executable(self.bin / "git", git_stub(shutil.which("git") or "git"))
        # ps, failing for the one process STUB_PS_FAIL_PID names.
        executable(self.bin / "ps", '#!/bin/sh\nfor a; do [ "$a" != "${STUB_PS_FAIL_PID:-none}" ] || exit 1; done\n'
                   f'exec {shutil.which("ps") or "/bin/ps"} "$@"\n')
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
            # This machine's own git login: the kit must not hand any of it to goalcup.
            "SSH_AUTH_SOCK": "/host/agent.sock",
            "GH_TOKEN": "gh-token-of-this-machine",
            "GITHUB_TOKEN": "github-token-of-this-machine",
            # A --test cell is refused without a token source: these worlds allow the login's.
            "MATRIX_ALLOW_LOGIN_TOKEN": "1",
            "SSH_AGENT_PID": "4242",
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": "credential.helper",
            "GIT_CONFIG_VALUE_0": "osxkeychain",
            "XDG_CONFIG_HOME": "/host/config",
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
        self.stale = None
        if (env or {}).get("WORLD_STALE"):
            self.stale = self.plant_stale()
            environment["STUB_PS_FAIL_PID"] = str(self.unsure.pid)
        if token_file is not None:
            # A token file for MATRIX_GH_TOKEN_FILE: "mode:text"; mode "link" makes the path a
            # symlink to a private file, "inside" a private file inside this checkout.
            mode, text = token_file.split(":", 1)
            path = self.root / "token-file"
            if mode == "inside":
                fd, name = tempfile.mkstemp(dir=MATRIX.parent.parent, prefix="scratch-token-")
                os.close(fd)
                path = Path(name)
                self.stray.append(path)
            real = self.root / "real-token" if mode == "link" else path
            real.write_text(text)
            real.chmod(0o600 if mode in ("link", "inside") else int(mode, 8))
            if mode == "link":
                path.symlink_to(real)
            environment["MATRIX_GH_TOKEN_FILE"] = str(path)
        for key, value in (env or {}).items():
            if key in ("WORLD_SERVER", "WORLD_TOKEN_FILE", "WORLD_STALE"):
                continue
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = value.replace("{bin}", str(self.bin)).replace("{root}", str(self.root))
        # The host half of a Docker cell's approvals (STUB_HOST_APPROVES=1): approve each code that
        # matrix.sh writes to its codes file, the way linux-matrix.sh does with the key.
        stop = threading.Event()
        if environment.get("STUB_HOST_APPROVES") == "1":
            codes = Path(environment["MATRIX_CODES_FILE"])

            def approve() -> None:
                done = 0
                while not stop.is_set():
                    lines = codes.read_text().splitlines() if codes.exists() else []
                    while done < len(lines):
                        done += 1
                        (self.out / f"approved.host.{done}").write_text(lines[done - 1])
                    time.sleep(0.1)

            threading.Thread(target=approve, daemon=True).start()
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
        if self.stale is not None:
            for proc in (self.live, self.unsure):
                proc.kill()
                proc.wait()
        self.elapsed = time.time() - self.started
        self.log = log.read_text() if log.exists() else ""

    def plant_stale(self) -> dict[str, Path]:
        """Directories in the cell's TMPDIR as earlier kits left them: a dead run's (its kit.pid names
        no live matrix.sh), a live run's (a process whose command line holds matrix.sh), an unrelated
        directory, and a link to a directory outside."""
        dead = subprocess.Popen(["true"])
        dead.wait()
        live = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)", "matrix.sh"])
        self.live = live
        planted = {}
        for name, pid in (("goalcup-matrix.STALE1", dead.pid), ("goalcup-matrix.LIVE01", live.pid)):
            d = self.tmpdir / name
            (d / "private").mkdir(parents=True)
            (d / "private" / "gh-token").write_text("GH_TOKEN=whatever-token-value\n")
            (d / "projects" / "run").mkdir(parents=True)
            (d / "projects" / "run" / ".env.local").write_text("GH_TOKEN=whatever-token-value\n")
            (d / "projects" / "run" / "keep.txt").write_text("not the kit's to delete\n")
            (d / "kit.pid").write_text(f"{pid}\n")
            planted[name] = d
        # A live process that ps cannot describe (the ps stub fails for it): counted as live.
        self.unsure = subprocess.Popen(["sleep", "120"])
        d = self.tmpdir / "goalcup-matrix.PSFAIL"
        (d / "projects" / "run").mkdir(parents=True)
        (d / "projects" / "run" / ".env.local").write_text("GH_TOKEN=whatever-token-value\n")
        (d / "kit.pid").write_text(f"{self.unsure.pid}\n")
        planted["unsure"] = d
        # Dead runs whose `projects`, or a project in it, is a link to a real folder elsewhere: the
        # sweep must not reach through either.
        for name, link_at in (("goalcup-matrix.SLINK1", "projects"), ("goalcup-matrix.SLINK2", "projects/run")):
            d = self.tmpdir / name
            target = self.root / ("real-" + name[-6:])
            (target / "run").mkdir(parents=True)
            (target / "run" / ".env.local").write_text("GH_TOKEN=a-real-project-of-the-owner\n")
            (target / ".env.local").write_text("GH_TOKEN=a-real-project-of-the-owner\n")
            (d / link_at).parent.mkdir(parents=True, exist_ok=True)
            (d / link_at).symlink_to(target if link_at == "projects" else target / "run")
            (d / "kit.pid").write_text(f"{dead.pid}\n")
            planted[name] = target
        other = self.tmpdir / "not-a-kit-dir"
        (other / "projects" / "run").mkdir(parents=True)
        (other / "projects" / "run" / ".env.local").write_text("GH_TOKEN=somebody-elses\n")
        planted["other"] = other
        outside = self.root / "outside"
        (outside / "projects" / "run").mkdir(parents=True)
        (outside / "projects" / "run" / ".env.local").write_text("GH_TOKEN=elsewhere\n")
        (outside / "kit.pid").write_text(f"{dead.pid}\n")
        (self.tmpdir / "goalcup-matrix.LINK01").symlink_to(outside)
        planted["outside"] = outside
        return planted

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
    # The same test run in a Docker cell (its device codes go to a codes file the host approves).
    "test_chat_cell": ("stubchat", {"STUB_STOP": "0", "STUB_PROMPT": TYPED, "STUB_CODE": "1",
                                    "MATRIX_CODES_FILE": "{root}/codes", "STUB_HOST_APPROVES": "1"},
                       TEST, True, MULTILINE),
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
    "ws_git_only": ("stubchat", {"STUB_WS": "git"}),
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
    # A Docker cell has no key: matrix.sh writes each device code to MATRIX_CODES_FILE and the
    # host (the responder below) approves it. A forged extra line changes nothing in matrix.sh.
    "codes_file": ("stubchat", {"MATRIX_CODES_FILE": "{root}/codes", "STUB_CODE": "1",
                                "STUB_HOST_APPROVES": "1"}),
    # A test run whose start asks for its repository: nobody can answer it.
    "test_asks": ("stubchat", {"STUB_ASK": "1"}, ("--test", "--duration", "30"), True),
    # A test run on a machine with no GitHub login.
    "test_no_gh_login": ("stubchat", {"STUB_GH_NO_LOGIN": "1"}, ("--test", "--duration", "30"), True),
    # The refusals, through a test build's doors; then a build without them; then a start that
    # gets past them to the approval.
    "refusals": ("m2-fake-tui", {}, ("--refusals",)),
    "refusals_shipped": ("m2-fake-tui", {"STUB_SHIPPED": "1"}, ("--refusals",)),
    "refusals_pass_through": ("m2-fake-tui", {"STUB_NEVER_REFUSES": "1"}, ("--refusals",)),
    # An agent that prints the token it was given to the terminal the matrix logs.
    "test_prints_token": ("stubchat", {"STUB_STOP": "0", "STUB_PRINT_TOKEN": "1", "STUB_DELETE_ENV": "1"},
                          TEST, True),
    # A --test cell is refused when it is given no token source.
    "test_no_token_source": ("stubchat", {"MATRIX_ALLOW_LOGIN_TOKEN": None}, TEST, True),
    # The agent prints its token with an escape sequence and a line wrap inside it, and another
    # token of the same shape.
    "test_prints_wrapped": ("stubchat", {"STUB_STOP": "0", "STUB_PRINT_TOKEN": "wrapped"}, TEST, True),
    # The token reaches the agent's commit, or the pushed repository; or the repository cannot be read.
    "test_leak_project": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "leak"}, TEST, True),
    "test_leak_clone": ("stubchat", {"STUB_STOP": "0", "STUB_CLONE_LEAK": "1"}, TEST, True),
    "test_clone_fails": ("stubchat", {"STUB_STOP": "0", "STUB_CLONE_FAIL": "1"}, TEST, True),
    # ...in a binary file no diff shows, or only in an annotated tag's message.
    "test_leak_binary": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "leak-binary"}, TEST, True),
    "test_leak_tag": ("stubchat", {"STUB_STOP": "0", "STUB_CLONE_LEAK": "tag"}, TEST, True),
    # The agent rewrites the kit's private copy of the token and pushes the real one; or deletes
    # the copy when the token cannot be had again from its source.
    "test_copy_rewritten": ("stubchat", {"STUB_STOP": "0", "STUB_TAMPER_COPY": "rewrite",
                                         "STUB_CLONE_LEAK": "1"}, TEST, True),
    "test_copy_gone": ("stubchat", {"STUB_STOP": "0", "STUB_TAMPER_COPY": "delete",
                                    "STUB_GH_TOKEN_ONCE": "1"}, TEST, True),
    # ...or swaps the copy during the check, after it was found good, and the token was pushed.
    "test_copy_swapped": ("stubchat", {"STUB_STOP": "0", "STUB_CLONE_SWAP_COPY": "1",
                                       "STUB_CLONE_LEAK": "1"}, TEST, True),
    # The token only in a submodule repository; a project folder replaced by a link; links planted
    # where the kit writes its own files.
    "test_leak_submodule": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "leak-submodule"}, TEST, True),
    "test_project_link": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "project-link"}, TEST, True),
    # The token in a repository whose objects folder is a link; or in one hidden behind a folder
    # name with a line break. A Git LFS store in a clean submodule repository.
    "test_leak_objects_link": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "leak-objects-link"}, TEST, True),
    "test_leak_newline": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "leak-newline"}, TEST, True),
    "test_lfs_submodule": ("stubchat", {"STUB_STOP": "0", "STUB_WS": "lfs-submodule"}, TEST, True),
    "test_planted_links": ("stubchat", {"STUB_STOP": "0", "STUB_PLANT_LINKS": "{root}/victim.txt"},
                           TEST, True),
    # Leftovers of dead runs in the temporary directory.
    "test_sweep": ("stubchat", {"STUB_STOP": "0", "WORLD_STALE": "1"}, TEST, True),
    # The token from a file (a fine-grained one-repository token) instead of `gh auth token`.
    "test_token_file": ("stubchat", {"STUB_STOP": "0", "MATRIX_REPO": GIVEN_REPO,
                                     "WORLD_TOKEN_FILE": "600:GH_TOKEN=" + FILE_TOKEN + "\n"}, TEST, True),
    # ...which needs the cell's repository named: the kit would make one the token cannot reach.
    "test_token_file_no_repo": ("stubchat", {"STUB_STOP": "0", "WORLD_TOKEN_FILE": "600:" + FILE_TOKEN + "\n"},
                                TEST, True),
    "test_token_file_link": ("stubchat", {"STUB_STOP": "0", "MATRIX_REPO": GIVEN_REPO, "WORLD_TOKEN_FILE": "link:" + FILE_TOKEN + "\n"},
                             TEST, True),
    "test_token_file_inside": ("stubchat", {"STUB_STOP": "0", "MATRIX_REPO": GIVEN_REPO, "WORLD_TOKEN_FILE": "inside:" + FILE_TOKEN + "\n"},
                               TEST, True),
    "test_token_file_classic": ("stubchat", {"STUB_STOP": "0", "MATRIX_REPO": GIVEN_REPO,
                                             "WORLD_TOKEN_FILE": "600:GH_TOKEN=" + CLASSIC_TOKEN + "\n"},
                                TEST, True),
    "test_token_file_open": ("stubchat", {"STUB_STOP": "0", "MATRIX_REPO": GIVEN_REPO, "WORLD_TOKEN_FILE": "644:" + FILE_TOKEN + "\n"},
                             TEST, True),
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
            for stray in world.stray:
                cls.addClassCleanup(stray.unlink, missing_ok=True)

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
        # A model answer (a title, a stalled turn) with the project folder as it began; and one
        # where the agent only made a repository.
        for name in ("ws_none", "ws_git_only"):
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
        # F2: the server keeps a test run `open` until its deadline; the committed end is the proof.
        self.assertIn("ok    committed end (final).head", w.log)
        self.assertNotIn("status is sealed", w.log)

    def test_a_multi_line_prompt_file_is_typed_on_one_line(self) -> None:
        w = self.worlds["test_chat"]
        chunks = json.loads(w.seen("start.stdin_chunks"))
        self.assertEqual(chunks[0][1], TYPED)
        self.assertEqual(chunks[1][1], "\r")

    def test_a_practice_run_starts_in_an_empty_project_folder_with_no_git_environment(self) -> None:
        w = self.worlds["chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        project = w.seen_json("start.project")
        self.assertEqual(project["names"], [])
        # The folder is the run's workspace, under the cell's temporary directory and no sibling of
        # its configuration directory (the daemon refuses a workspace beside it).
        cwd = Path(project["cwd"])
        self.assertTrue(cwd.is_relative_to(w.tmpdir.resolve()), cwd)
        [cfg] = [p for p in w.tmpdir.glob("goalcup-matrix.*/cfg")]
        self.assertNotEqual(cwd.parent, cfg.resolve().parent)
        self.assertEqual(set(w.seen_json("start.git").values()), {None})
        self.assertNotIn("--repo", w.seen("start.argv"))
        self.assertFalse(w.has("gh.repo_create"))
        self.assertFalse(w.has("gh.auth_token"))

    def test_a_test_run_starts_in_a_project_folder_with_its_kit_and_a_private_token(self) -> None:
        w = self.worlds["test_chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        project = w.seen_json("start.project")
        # What a player makes: a .gitignore covering .env*, and a .env.local holding the token of
        # `gh auth token`, readable by its owner alone.
        self.assertEqual(project["names"], [".env.local", ".gitignore"])
        self.assertEqual(project["gitignore"], ".env*\n")
        self.assertEqual(project["env_local_mode"], "0o600")
        self.assertEqual(project["env_local_sha256"],
                         hashlib.sha256(f"GH_TOKEN={TOKEN}\n".encode()).hexdigest())
        # It is in no argument and no environment of goalcup's, and the kit passes no git login.
        self.assertFalse(project["token_in_argv"])
        self.assertFalse(project["token_in_env"])
        self.assertEqual(set(w.seen_json("start.git").values()), {None})
        # The token is never printed or logged, and the file does not outlive the run (--keep).
        self.assertNotIn(TOKEN, w.everything_matrix_wrote())
        self.assertFalse((Path(project["cwd"]) / ".env.local").exists())

    def test_a_token_the_agent_printed_is_taken_out_of_the_log_even_if_it_deleted_the_file(self) -> None:
        w = self.worlds["test_prints_token"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertIn("agent said: ", (next(w.tmpdir.rglob("start.out"))).read_text())
        self.assertNotIn(TOKEN, w.everything_matrix_wrote())
        self.assertIn("agent said: GH_TOKEN=", w.everything_matrix_wrote())

    def test_a_token_file_replaces_gh_auth_token_and_must_be_private(self) -> None:
        w = self.worlds["test_token_file"]
        self.assertEqual(w.run.returncode, 0, w.said())
        # The file's token is the agent's, gh was not asked for one, and the source is named.
        self.assertEqual(w.seen_json("start.project")["env_local_sha256"],
                         hashlib.sha256(f"GH_TOKEN={FILE_TOKEN}\n".encode()).hexdigest())
        self.assertFalse(w.has("gh.auth_token"))
        self.assertFalse(w.has("gh.repo_create"))
        self.assertTrue(w.seen("start.argv").endswith("--repo\n" + GIVEN_REPO))
        self.assertIn("from MATRIX_GH_TOKEN_FILE", w.run.stderr)
        self.assertNotIn(FILE_TOKEN, w.everything_matrix_wrote())
        self.assertTrue((w.root / "token-file").exists())
        # The file's token needs the cell's repository named, or nothing starts.
        w = self.worlds["test_token_file_no_repo"]
        self.assertEqual(w.run.returncode, 2, w.said())
        self.assertIn("MATRIX_GH_TOKEN_FILE needs MATRIX_REPO", w.run.stderr)
        self.assertFalse(w.has("gh.repo_create"))
        # A file others can read is refused before anything starts.
        w = self.worlds["test_token_file_open"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("MATRIX_GH_TOKEN_FILE must be mode 0600", w.run.stderr)
        self.assertFalse(w.has("start.argv"))
        self.assertNotIn(FILE_TOKEN, w.everything_matrix_wrote())

    def test_a_token_file_that_is_a_link_or_inside_the_checkout_is_refused(self) -> None:
        for name, why, value in (("test_token_file_link", "must name a regular file", FILE_TOKEN),
                                 ("test_token_file_inside", "is inside the checkout", FILE_TOKEN),
                                 # A classic or gh login token reaches every repository: refused.
                                 ("test_token_file_classic", "gh login or classic token", CLASSIC_TOKEN)):
            w = self.worlds[name]
            self.assertNotEqual(w.run.returncode, 0, name)
            self.assertIn(why, w.run.stderr)
            self.assertFalse(w.has("start.argv"), name)
            self.assertNotIn(value, w.everything_matrix_wrote())

    def test_a_test_cell_refuses_to_start_without_a_token_source_and_says_so_loudly_when_given_the_login(self) -> None:
        w = self.worlds["test_no_token_source"]
        self.assertEqual(w.run.returncode, 2, w.said())
        self.assertIn("MATRIX_GH_TOKEN_FILE", w.run.stderr)
        self.assertIn("MATRIX_ALLOW_LOGIN_TOKEN=1", w.run.stderr)
        for name in ("start.argv", "gh.repo_create", "gh.auth_token"):
            self.assertFalse(w.has(name), name)
        # With the explicit say-so the login token is given, and one loud line says so; with a
        # token file it is not given.
        self.assertIn("ACCOUNT-WIDE gh login token", self.worlds["test_chat"].run.stderr)
        self.assertNotIn("ACCOUNT-WIDE", self.worlds["test_token_file"].run.stderr)

    def test_a_token_in_the_agents_commits_or_the_pushed_repository_fails_the_cell(self) -> None:
        for name, where in (("test_leak_project", "the project folder"),
                            ("test_leak_clone", "the pushed repository"),
                            # Every object is read raw: a binary blob no diff shows, a tag message.
                            ("test_leak_binary", "the project folder"),
                            ("test_leak_tag", "the pushed repository"),
                            # A rewritten private copy is made again from the token's source.
                            ("test_copy_rewritten", "the pushed repository"),
                            # A submodule's repository under .git/modules is read too.
                            ("test_leak_submodule", "the project folder"),
                            # ...and one whose .git/objects is a link to a folder elsewhere.
                            ("test_leak_objects_link", "the project folder")):
            w = self.worlds[name]
            self.assertEqual(w.run.returncode, 1, (name, w.said()))
            self.assertIn("the token was pushed: revoke it now", w.run.stderr)
            self.assertIn(where, w.run.stderr)
            self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")
            # The value is never printed, in any of its forms.
            said = w.said()  # (the agent's own files, kept with --keep, hold what it pushed)
            self.assertNotIn(TOKEN, said)
            self.assertNotIn(base64.b64encode(("x-access-token:" + TOKEN).encode()).decode(), said)
            # The clone is never kept, even with --keep.
            self.assertEqual(list(w.tmpdir.rglob("clone")), [], name)
        # A clean run was checked too (the repository was cloned), and a repository that cannot
        # be cloned fails the check instead of passing it.
        clean = self.worlds["test_chat"]
        self.assertIn("kit-tester/goalcup-kit-stubchat-", clean.seen("git.clone"))
        self.assertIn("searched for the token: not found", clean.log)
        w = self.worlds["test_clone_fails"]
        self.assertEqual(w.run.returncode, 1, w.said())
        self.assertIn("could not be cloned", w.run.stderr)

    def test_a_push_check_without_the_token_fails_the_cell_instead_of_passing_it(self) -> None:
        # The agent deleted the kit's copy of its token, and the token cannot be had again.
        w = self.worlds["test_copy_gone"]
        self.assertEqual(w.run.returncode, 1, w.said())
        self.assertIn("push check not run: the token copy is missing", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")
        self.assertNotIn(TOKEN, w.said())
        # The log scrub then knows the token only by its shape, and says so.
        self.assertIn("scrub by shape only", w.log)
        # A copy swapped after it was found good (while the clone ran) is never searched with:
        # the scan says it could not check, it never says "not found".
        w = self.worlds["test_copy_swapped"]
        self.assertEqual(w.run.returncode, 1, w.said())
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")
        self.assertIn("could not be checked for the token", w.run.stderr)
        self.assertIn("private copy changed during the check", w.log)
        self.assertNotIn("searched for the token: not found", w.log)
        # A project folder the agent replaced by a link is not checked, and says so.
        w = self.worlds["test_project_link"]
        self.assertEqual(w.run.returncode, 1, w.said())
        self.assertIn("the project folder (it is gone or a link)", w.run.stderr)
        # A folder name with a line break could hide a repository from the search: not checked.
        w = self.worlds["test_leak_newline"]
        self.assertEqual(w.run.returncode, 1, w.said())
        self.assertIn("could not be checked for the token", w.run.stderr)
        self.assertIn("line break", w.run.stderr)
        # A Git LFS store inside a clean submodule repository is no repository: the cell passes.
        w = self.worlds["test_lfs_submodule"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertIn("searched for the token: not found", w.log)

    def test_the_kit_never_writes_through_a_link_the_agent_planted_in_its_folder(self) -> None:
        w = self.worlds["test_planted_links"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertEqual((w.root / "victim.txt").read_text(), "the owner's file\n")

    def test_leftovers_of_dead_runs_are_swept_and_nothing_else(self) -> None:
        w = self.worlds["test_sweep"]
        self.assertEqual(w.run.returncode, 0, w.said())
        dead, live = w.stale["goalcup-matrix.STALE1"], w.stale["goalcup-matrix.LIVE01"]
        self.assertFalse((dead / "private").exists())
        self.assertFalse((dead / "projects/run/.env.local").exists())
        self.assertTrue((dead / "projects/run/keep.txt").exists())
        # A live run's token, another directory, and what a link points to are not touched.
        self.assertTrue((live / "private/gh-token").exists())
        self.assertTrue((live / "projects/run/.env.local").exists())
        self.assertTrue((w.stale["other"] / "projects/run/.env.local").exists())
        self.assertTrue((w.stale["outside"] / "projects/run/.env.local").exists())
        # Nor a real folder a dead run's `projects` or project links to, nor a run whose process
        # ps cannot describe (in doubt, live).
        for name in ("goalcup-matrix.SLINK1", "goalcup-matrix.SLINK2"):
            self.assertTrue((w.stale[name] / "run/.env.local").exists(), name)
        self.assertTrue((w.stale["unsure"] / "projects/run/.env.local").exists())

    def test_a_token_is_scrubbed_by_shape_through_escape_sequences_and_wraps_and_the_log_is_private(self) -> None:
        w = self.worlds["test_prints_wrapped"]
        self.assertEqual(w.run.returncode, 0, w.said())
        said = w.everything_matrix_wrote()
        self.assertIn("agent said: ", said)
        self.assertNotIn(TOKEN, said)
        self.assertNotIn(TOKEN[:12], said)
        self.assertNotIn("a1B2c3D4e5", said)
        self.assertEqual(stat.S_IMODE((w.root / "matrix.log").stat().st_mode), 0o600)

    def test_a_test_run_makes_its_repository_and_passes_it_to_the_start(self) -> None:
        w = self.worlds["test_chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        made = w.seen("gh.repo_create").split("\n")
        # Public and with no README, so it stays empty; passed to the start; named at the end.
        self.assertEqual(made[:2], ["repo", "create"])
        self.assertEqual(made[3:], ["--public"])
        self.assertTrue(w.seen("start.argv").endswith("--repo\n" + made[2]))
        # The default `gh auth login` lacks the delete_repo scope: the advice adds it first.
        self.assertIn(f"test repository made by the kit (delete it when done: first `gh auth refresh -s "
                      f"delete_repo`, which the default gh login lacks, then `gh repo delete {made[2]} --yes`, "
                      "then `gh auth refresh -r delete_repo` to drop the scope again: ", w.run.stderr)

    def test_in_a_docker_cell_the_scope_advice_names_the_cells_own_login(self) -> None:
        w = self.worlds["test_chat_cell"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertIn("then `gh auth refresh -r delete_repo` to drop the scope again", w.run.stderr)
        self.assertIn("the login stored in this cell", w.run.stderr)
        self.assertNotIn("this machine's gh token", w.run.stderr)

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
        # No token file is left behind for a run that did not start.
        self.assertEqual([p for p in w.tmpdir.rglob(".env.local")], [])

    def test_a_question_before_the_approval_fails_the_row_at_once(self) -> None:
        w = self.worlds["test_asks"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("asked a question before the run's approval", w.run.stderr)
        self.assertRegex(w.run.stdout, r"(?m)^stubchat \S+ \S+ FAIL ")
        self.assertLess(w.elapsed, 50)

    def test_every_refusal_is_checked_in_a_project_folder_through_a_test_builds_doors(self) -> None:
        w = self.worlds["refusals"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout, r"(?m)^m2-fake-tui \S+ refusals PASS \d{4}-\d\d-\d\d$")
        kinds = ("folder-not-empty", "git-present", "env-no-gitignore", "repo-has-size",
                 "readme-repo", "private", "bare-name")
        for kind in kinds:
            self.assertIn(f"refusal {kind}: PASS", w.run.stderr)
            seen = w.seen_json(f"refuse.{kind}")
            self.assertFalse(seen["tty"], kind)
            self.assertEqual(seen["argv"][:5], ["start", "--harness", "m2-fake-tui", "--dev", "--test"])
            # Each row ran in a project folder of its own, with no git login of any kind.
            self.assertEqual(Path(seen["project"]["cwd"]).name, f"refuse-{kind}")
            self.assertEqual(set(seen["git"].values()), {None})
            # The retired doors are not set.
            self.assertIsNone(seen["doors"]["GOALCUP_TEST_GITHUB_LOGIN"], kind)
            self.assertIsNone(seen["doors"]["GOALCUP_TEST_PUSH_REMOTE"], kind)
        # The folders the rows are about.
        names = {k: w.seen_json(f"refuse.{k}")["project"]["names"] for k in kinds}
        self.assertEqual(names["folder-not-empty"], [".env.local", ".gitignore", "README.md"])
        self.assertEqual(names["git-present"], [".env.local", ".git", ".gitignore"])
        self.assertEqual(names["env-no-gitignore"], [".env.local"])
        self.assertEqual(names["private"], [".env.local", ".gitignore"])
        # GitHub is asked only where the refusal is GitHub's answer.
        for kind in ("repo-has-size", "readme-repo", "private"):
            api = w.seen_json(f"refuse.{kind}")["doors"]["GOALCUP_TEST_GITHUB_API"]
            self.assertTrue(api.startswith("http://127.0.0.1:"), kind)
        self.assertIsNone(w.seen_json("refuse.bare-name")["doors"]["GOALCUP_TEST_GITHUB_API"])
        # The README repository reads size 0: only the commits call refuses it.
        self.assertIn("/repos/goalcup-kit/kit-readme/commits", w.log)
        # No refusal left a token anywhere: the rows hold a placeholder.
        self.assertFalse(w.has("gh.auth_token"))

    def test_a_build_without_the_test_doors_fails_the_refusals(self) -> None:
        w = self.worlds["refusals_shipped"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertRegex(w.run.stdout, r"(?m)^m2-fake-tui \S+ refusals FAIL ")
        self.assertIn("refusal folder-not-empty: FAIL: it refused for another reason", w.run.stderr)
        self.assertIn("no test doors", w.run.stderr)

    def test_a_start_that_reaches_the_approval_fails_its_refusal(self) -> None:
        w = self.worlds["refusals_pass_through"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("refusal private: FAIL: the run's approval was asked for", w.run.stderr)
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

    def test_a_device_code_is_written_for_the_host_and_the_matrix_approves_nothing(self) -> None:
        w = self.worlds["codes_file"]
        self.assertEqual(w.run.returncode, 0, w.said())
        # One code per phase, in order, as `<phase> <code>` lines; no key, no approval call.
        self.assertEqual((w.root / "codes").read_text(), "login ABCD-EFGH\nstart ABCD-EFGH\n")
        self.assertEqual(sorted(p.name for p in w.out.glob("approved.*")), ["approved.host.1", "approved.host.2"])
        self.assertEqual(w.seen("login.approved"), "yes")
        self.assertEqual(w.seen("start.approved"), "yes")
        self.assertIn("written for the host to approve", w.log)
        self.assertIn("key file supplied: no", w.log)

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
                       "https://localhost:8787", "http://localhost.evil.example:8787",
                       # Userinfo: the host is what follows the @.
                       "http://localhost:1@evil.example:80", "http://127.0.0.1@evil.example",
                       "http://127.0.0.1:1@evil.example", "http://[::1]:1@evil.example:80",
                       "http://localhost:80/path", "http://localhost:8787?x=1", "http://localhost:"):
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

    def test_a_key_file_inside_the_checkout_is_refused(self) -> None:
        inside = MATRIX.parent.parent / "scratch-key-for-test"
        inside.write_text("k\n")
        self.addCleanup(lambda: inside.unlink(missing_ok=True))
        link = self.root / "link-to-key"
        link.symlink_to(inside)
        for path in (inside, link, MATRIX.parent / ".." / "scratch-key-for-test"):
            with self.subTest(path=str(path)):
                said = self.refuse("m2-fake-tui", "http://localhost:8787",
                                   GOALCUP_TEST_APPROVAL_KEY_FILE=str(path))
                self.assertIn("inside the checkout", said)
                self.assertEqual(said.count("\n"), 1, "one clear line")

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
