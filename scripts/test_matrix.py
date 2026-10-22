#!/usr/bin/env python3
"""Behavioral tests for the matrix, scripts/matrix.sh, against stand-ins for everything it drives.

`goalcup`, `goalcup-verify`, `goalcup-daemon` and `curl` are stubs in a scratch directory (the real
ones are covered by running the script, docs/OPERATING.md "The matrix"). The `goalcup` stub records
what it was started with: its arguments, the parts of its environment that matter and what
arrived on its standard input. Every connector is chat mode (D57), so every run is on a
pseudo-terminal (script(1)). Every key here is synthetic.
"""

from __future__ import annotations

import concurrent.futures
import json
import os
import shutil
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

MATRIX = Path(__file__).with_name("matrix.sh")

PROMPT = "Goal Cup test prompt: say ok."

GOALCUP = r'''#!/usr/bin/env python3
import json, os, select, sys, termios, time, tty
from pathlib import Path

out = Path(os.environ["STUB_OUT"])
args = sys.argv[1:]
note = lambda name, text: (out / name).write_text(text)
ENTRY = "e0123456789abcdef"
cfg = Path(os.environ.get("GOALCUP_CONFIG_DIR", "/nonexistent"))

if args == ["--version"]:
    print("goalcup 0.0.0-stub")
    sys.exit(0)

if args[0] == "login":
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
note("start.argv", "\n".join(args))
note("start.no_browser", os.environ.get("GOALCUP_NO_BROWSER", "unset"))
note("start.stdin_is_tty", str(os.isatty(0)))
test = "--test" in args

run_dir = cfg / "runs" / "r1"
trace = cfg / "artifacts" / ENTRY / "trace"
chain = (json.dumps({"body": {"harness_version":
                             os.environ.get("STUB_HV", "stub-harness 9.9.9")}}) + "\n").encode()

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
    (run_dir / "run.json").write_text(json.dumps({
        "run_id": "r1", "server_url": "http://localhost:8787", "test": test,
        "connector_id": args[args.index("--harness") + 1],
        "entries": [{"entry_id": ENTRY, "trace_dir": str(trace)}],
    }))
    if os.environ.get("STUB_PRUNED") != "1":
        note("chain.size", str(len(chain)))

if test:
    write_run()

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
    sys.stdin.read()
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

CHAT = """schema_version = 1
id = "{id}"

[capture]
interactive = true
args = []
"""


def executable(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class World:
    """One matrix run against the stubs, in a scratch tree of its own. Runs are slow (the script
    polls every two seconds), so the tests start them all at once and look at the results."""

    def __init__(self, connector: str, env: dict[str, str | None] | None = None,
                 args: tuple[str, ...] = (), prepared: bool = False,
                 prompt_file: str | None = None) -> None:
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
        }
        if prepared:
            # A configuration directory as `goalcup prep` leaves it (MATRIX_CONFIG_DIR).
            self.cfg = self.root / "prepared-cfg"
            self.cfg.mkdir()
            (self.cfg / "prep.json").write_text("{}")
            (self.cfg / "github.json").write_text("{}")
            environment["MATRIX_CONFIG_DIR"] = str(self.cfg)
        if prompt_file is not None:
            path = self.root / "prompt.txt"
            path.write_text(prompt_file)
            environment["MATRIX_PROMPT_FILE"] = str(path)
            del environment["MATRIX_PROMPT"]
        for key, value in (env or {}).items():
            if value is None:
                environment.pop(key, None)
            else:
                environment[key] = value.replace("{bin}", str(self.bin))
        log = self.root / "matrix.log"
        # A test run is left to its default budget (the duration plus 1800 s) unless it names one.
        budget = () if "--test" in args else ("--timeout", "60")
        self.started = time.time()
        self.run = subprocess.run(
            ["sh", str(MATRIX), connector, "http://localhost:8787", "--log", str(log), "--keep",
             *budget, *args],
            env=environment, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=120,
        )
        self.log = log.read_text() if log.exists() else ""

    def seen(self, name: str) -> str:
        return (self.out / name).read_text()

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
    "test_chat_early": ("stubchat",
                        {"STUB_STOP": "0", "STUB_EXIT_EARLY": "1", "STUB_NO_SEAL": "1",
                         "STUB_PROMPT": TYPED},
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
    "bracketed": ("stubchat", {"STUB_BRACKETED": "1",
                               "STUB_PROMPT": "\x1b[200~" + PROMPT + "\x1b[201~"}),
    # Long runs: the trace holds only the tail part (pruned), and the server copy is fetched.
    "pruned_unfetched": ("stubchat", {"STUB_PRUNED": "1", "DATABASE_URL": None,
                                      "CLOUDFLARE_API_TOKEN": None}),
    "pruned_files": ("stubchat", {"STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                  "MATRIX_FETCH_INDEX": "/index.json",
                                  "MATRIX_FETCH_STORE": "/bucket-mirror"}),
    "pruned_env": ("stubchat", {"STUB_PRUNED": "1", "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                "DATABASE_URL": DB_URL, "CLOUDFLARE_ACCOUNT_ID": "acct",
                                "CLOUDFLARE_API_TOKEN": CF_TOKEN}),
    "pruned_bad_tail": ("stubchat", {"STUB_PRUNED": "1", "STUB_FETCH_BAD": "1",
                                     "MATRIX_FETCH_SCRIPT": "{bin}/fake-fetch.sh",
                                     "MATRIX_FETCH_INDEX": "/index.json",
                                     "MATRIX_FETCH_STORE": "/bucket-mirror"}),
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
        self.assertEqual(w.seen("start.argv"), "start\n--harness\nstubchat\n--test")
        # The exact entry of this run, sealed no earlier than the duration after the Enter.
        self.assertEqual(w.seen("seal.argv"), "seal\n--yes\ne0123456789abcdef")
        waited = float(w.seen("seal.at")) - float(w.seen("start.submitted_at"))
        self.assertGreaterEqual(waited, DURATION)
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

    def test_a_test_run_whose_harness_ends_first_is_sealed_then_uploaded(self) -> None:
        w = self.worlds["test_chat_early"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout, r"stubchat \S+ stub-harness_9\.9\.9 PASS ")
        self.assertEqual(w.seen("start.argv"), "start\n--harness\nstubchat\n--test")
        # Sealed when the harness ended, long before the 30 s duration.
        self.assertEqual(w.seen("seal.argv"), "seal\n--yes\ne0123456789abcdef")
        self.assertLess(float(w.seen("seal.at")) - w.started, 25)
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
        self.assertEqual(argv[:6], ["--bucket", "goalcup-traces-staging", "--index", "/index.json",
                                    "--store", "/bucket-mirror"])
        self.assertEqual(argv[6], "e0123456789abcdef")
        # The verifier and the agent-work check judged the copy, and the tail matched it.
        self.assertIn("servercopy/e0123456789abcdef", w.seen("research.argv"))
        self.assertRegex(w.log, r"ok +local chain\.\d{20}\.jsonl")
        self.assertIn("ends where the server copy ends", w.log)

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

    def test_test_and_duration_go_together(self) -> None:
        cfg = {"MATRIX_CONFIG_DIR": str(self.cfg)}
        self.assertIn("go together", self.refuse("codex-cli", "https://x.example", "--test", **cfg))
        self.assertIn("go together",
                      self.refuse("codex-cli", "https://x.example", "--duration", "60", **cfg))

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
