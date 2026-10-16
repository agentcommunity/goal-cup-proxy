#!/usr/bin/env python3
"""Behavioral tests for the matrix, scripts/matrix.sh, against stand-ins for everything it drives.

`goalcup`, `goalcup-verify`, `goalcup-daemon` and `curl` are stubs in a scratch directory (the real
ones are covered by running the script, docs/OPERATING.md "The matrix"). The `goalcup` stub records
what it was started with: its arguments, the parts of its environment that matter, the goal file's
text and what arrived on its standard input. Every key here is synthetic.
"""

from __future__ import annotations

import concurrent.futures
import json
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

MATRIX = Path(__file__).with_name("matrix.sh")

KEY = "sk-synthetic0000111122223333"
PROMPT = "Goal Cup test prompt: say ok."

GOALCUP = r'''#!/usr/bin/env python3
import json, os, select, sys, termios, time, tty
from pathlib import Path

out = Path(os.environ["STUB_OUT"])
args = sys.argv[1:]
note = lambda name, text: (out / name).write_text(text)

if args == ["--version"]:
    print("goalcup 0.0.0-stub")
    sys.exit(0)

if args[0] == "login":
    note("login.no_browser", os.environ.get("GOALCUP_NO_BROWSER", "unset"))
    print("Logged in as member.")
    sys.exit(0)

assert args[0] == "start", args
note("start.argv", "\n".join(args))
if os.environ.get("STUB_REAL_REFUSAL") == "1" and "--practice" in args:
    # What the real goalcup says to --practice for a keyed harness (start.rs, `uploading`).
    sys.exit(print("goalcup: `--practice` cannot be used with this harness: a harness handed a provider key "
                   "is never uploaded, so it cannot make a practice run. Nothing was created.", file=sys.stderr) or 1)
note("start.no_browser", os.environ.get("GOALCUP_NO_BROWSER", "unset"))
note("start.stdin_is_tty", str(os.isatty(0)))
if "DEEPSEEK_API_KEY" in os.environ:
    note("start.key_length", str(len(os.environ["DEEPSEEK_API_KEY"])))
note("start.has_key_env_file_var", str("MATRIX_KEY_ENV_FILE" in os.environ))
if "--goal-file" in args:
    note("start.goal", Path(args[args.index("--goal-file") + 1]).read_text())

chat = os.environ["STUB_MODE"] == "chat"
if chat:
    print("Recording in chat mode: stub starts now in this terminal.", flush=True)
    tty.setraw(0)
    chunks, t0 = [], time.time()
    while time.time() - t0 < 30:
        if select.select([0], [], [], 0.2)[0]:
            data = os.read(0, 4096)
            chunks.append([round(time.time() - t0, 2), data.decode("latin1")])
            if b"\r" in data and chunks and any(c[1] == os.environ["STUB_PROMPT"] for c in chunks):
                break
    note("start.stdin_chunks", json.dumps(chunks))

cfg = Path(os.environ["GOALCUP_CONFIG_DIR"])
run_dir = cfg / "runs" / "r1"
trace = cfg / "artifacts" / "e0123456789abcdef" / "trace"
trace.mkdir(parents=True)
run_dir.mkdir(parents=True)
chain = b'{"body":{"harness_version":"stub-harness 9.9.9"}}\n'
(trace / "chain.jsonl").write_bytes(chain)
(run_dir / "credential").write_text(json.dumps({"credential": "c"}))
(run_dir / "run.json").write_text(json.dumps({
    "run_id": "r1", "server_url": "http://localhost:8787",
    "entries": [{"entry_id": "e0123456789abcdef", "trace_dir": str(trace)}],
}))
note("chain.size", str(len(chain)))

exit_line = "harness exited: exit code %s." % os.environ.get("STUB_EXIT", "0")
if chat:
    # A chat-mode run's `deadline:` line goes to the run's host.log, not the terminal.
    (run_dir / "host.log").write_text("deadline: the run's time is up. The agent is stopped.\n")
print(exit_line)
if os.environ.get("STUB_NO_SEAL") != "1":
    print("uploaded: %d bytes, acknowledged, and the seal." % len(chain))
'''

VERIFY = r'''#!/usr/bin/env python3
import json, sys
from pathlib import Path
size = (Path(sys.argv[2]) / "chain.jsonl").stat().st_size
print(json.dumps({"integrity": "valid", "anchor": "signed", "validated_records": 1,
                  "validated_bytes": size, "validated_head": "a" * 64, "sealed": True}))
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
            "final": {"seq": 0, "head": "a" * 64}, "status": "sealed", "stop": True}
elif url.endswith("/v1/health"):
    body = {"status": "ok"}
else:
    body = {"status": "approved"}
Path(target).write_text(json.dumps(body))
sys.stdout.write("200")
'''

HEADLESS = """schema_version = 1
id = "{id}"

[capture]
args = ["run", "--"]
goal_flag = "--query"
{extra}"""
CHAT = """schema_version = 1
id = "{id}"

[capture]
interactive = true
args = []
"""
CREDENTIAL = """
[capture.credential]
env = "DEEPSEEK_API_KEY"
"""


def executable(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class World:
    """One matrix run against the stubs, in a scratch tree of its own. Runs are slow (the script
    polls every two seconds), so the tests start them all at once and look at the results."""

    def __init__(self, connector: str, mode: str, env: dict[str, str] | None = None) -> None:
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
        self.connectors = self.root / "connectors"
        for name, text in (
            ("stubhead", HEADLESS.format(id="stubhead", extra="")),
            ("hermes-cli", HEADLESS.format(id="hermes-cli", extra=CREDENTIAL)),
            ("stubchat", CHAT.format(id="stubchat")),
        ):
            (self.connectors / name).mkdir(parents=True)
            (self.connectors / name / "harness.toml").write_text(text)
        self.keyfile = self.root / "secret-place" / "deepseek.env"
        self.keyfile.parent.mkdir()
        self.keyfile.write_text(f"OTHER=1\nDEEPSEEK_API_KEY={KEY}\n")
        self.keyfile.chmod(0o600)
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
            "STUB_MODE": mode,
            "STUB_PROMPT": PROMPT,
        }
        for key, value in (env or {}).items():
            environment[key] = value.replace("KEYFILE", str(self.keyfile))
        log = self.root / "matrix.log"
        self.run = subprocess.run(
            ["sh", str(MATRIX), connector, "http://localhost:8787", "--log", str(log), "--keep",
             "--timeout", "60"],
            env=environment, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=120,
        )
        self.log = log.read_text() if log.exists() else ""

    def seen(self, name: str) -> str:
        return (self.out / name).read_text()

    def everything_matrix_wrote(self) -> str:
        """The terminal output, the log and every file in the kept directory."""
        texts = [self.run.stdout, self.run.stderr, self.log]
        for found in self.tmpdir.rglob("*"):
            if found.is_file():
                texts.append(found.read_text(errors="replace"))
        return "\n".join(texts)

    def said(self) -> str:
        return self.run.stdout + self.run.stderr + self.log


SCENARIOS = {
    "headless": ("stubhead", "headless", {}),
    "exits_non_zero": ("stubhead", "headless", {"STUB_EXIT": "3"}),
    "no_seal": ("stubhead", "headless", {"STUB_NO_SEAL": "1"}),
    # Today a keyed connector is refused up front, before any login or start.
    "keyed_refused": ("hermes-cli", "headless", {"MATRIX_KEY_ENV_FILE": "KEYFILE"}),
    # The plumbing behind MATRIX_KEYED_UPLOADED=1. The stub accepts --practice here; the real
    # goalcup does not (next scenario), so this proves the key handling and nothing more.
    "keyed": ("hermes-cli", "headless", {"MATRIX_KEY_ENV_FILE": "KEYFILE", "MATRIX_KEYED_UPLOADED": "1"}),
    "keyed_real_refusal": ("hermes-cli", "headless", {"MATRIX_KEY_ENV_FILE": "KEYFILE", "MATRIX_KEYED_UPLOADED": "1", "STUB_REAL_REFUSAL": "1"}),
    "no_manifest": ("nosuchconnector", "headless", {}),
    "key_ignored": ("stubhead", "headless", {"MATRIX_KEY_ENV_FILE": "KEYFILE"}),
    "bad_key_file": ("hermes-cli", "headless", {"MATRIX_KEY_ENV_FILE": "/nonexistent/secret-place/x.env", "MATRIX_KEYED_UPLOADED": "1"}),
    "chat": ("stubchat", "chat", {"MATRIX_PRE_PROMPT_KEYS": r"\033[B\r", "MATRIX_PRE_PROMPT_DELAY": "1"}),
}


class MatrixTests(unittest.TestCase):
    worlds: dict[str, World] = {}

    @classmethod
    def setUpClass(cls) -> None:
        names = [n for n in SCENARIOS if n != "chat" or shutil.which("script")]
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(names)) as pool:
            futures = {n: pool.submit(World, *SCENARIOS[n]) for n in names}
            cls.worlds = {n: f.result() for n, f in futures.items()}
        for world in cls.worlds.values():
            cls.addClassCleanup(world.tmp.cleanup)

    def test_a_headless_connector_gets_a_goal_file_and_no_typing(self) -> None:
        w = self.worlds["headless"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertRegex(w.run.stdout, r"stubhead \S+ stub-harness_9\.9\.9 PASS ")
        argv = w.seen("start.argv").split("\n")
        self.assertEqual(argv[:4], ["start", "--harness", "stubhead", "--practice"])
        self.assertEqual(argv[4], "--goal-file")
        self.assertEqual(len(argv), 6)
        self.assertEqual(w.seen("start.goal"), PROMPT + "\n")
        self.assertEqual(w.seen("start.stdin_is_tty"), "False")
        # No browser page per approval, for the login and for the run.
        self.assertEqual(w.seen("login.no_browser"), "1")
        self.assertEqual(w.seen("start.no_browser"), "1")

    def test_a_headless_harness_that_exits_non_zero_fails_the_row(self) -> None:
        w = self.worlds["exits_non_zero"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("FAIL", w.run.stdout)
        self.assertIn("harness exited: exit code 3.", w.run.stderr)

    def test_a_run_without_the_seal_line_fails_the_row(self) -> None:
        w = self.worlds["no_seal"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("and the seal", w.run.stderr)

    def test_a_keyed_connector_is_refused_before_anything_starts(self) -> None:
        w = self.worlds["keyed_refused"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("keyed harnesses are not uploaded today (RULES); matrix cannot test them", w.run.stderr)
        self.assertFalse((w.out / "start.argv").exists())
        self.assertFalse((w.out / "login.no_browser").exists())
        self.assertNotIn(KEY, w.everything_matrix_wrote())

    def test_the_real_refusal_of_practice_for_a_keyed_harness_fails_the_row(self) -> None:
        w = self.worlds["keyed_real_refusal"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("is never uploaded, so it cannot make a practice run", w.run.stderr)
        self.assertIn("FAIL", w.run.stdout)

    def test_a_connector_with_no_manifest_fails_and_says_where_to_look(self) -> None:
        w = self.worlds["no_manifest"]
        self.assertNotEqual(w.run.returncode, 0)
        self.assertIn("GOALCUP_CONNECTORS_DIR", w.run.stderr)
        self.assertFalse((w.out / "start.argv").exists())

    def test_the_key_goes_through_the_loader_and_leaves_no_trace(self) -> None:
        # Plumbing only (MATRIX_KEYED_UPLOADED=1): see the scenario's comment.
        w = self.worlds["keyed"]
        self.assertEqual(w.run.returncode, 0, w.said())
        argv = w.seen("start.argv").split("\n")
        self.assertEqual(argv[:4], ["start", "--harness", "hermes-cli", "--practice"])
        self.assertEqual(argv[4], "--goal-file")
        # The loader read the key and exported it; the path was dropped from the environment.
        self.assertEqual(w.seen("start.key_length"), str(len(KEY)))
        self.assertEqual(w.seen("start.has_key_env_file_var"), "False")
        everything = w.everything_matrix_wrote()
        self.assertNotIn(KEY, everything)
        self.assertNotIn(str(w.keyfile), everything)
        self.assertNotIn("secret-place", everything)

    def test_a_key_file_is_ignored_for_a_connector_with_no_such_credential(self) -> None:
        w = self.worlds["key_ignored"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertFalse((w.out / "start.key_length").exists())
        self.assertEqual(w.seen("start.argv").split("\n")[2], "stubhead")
        self.assertNotIn(KEY, w.everything_matrix_wrote())

    def test_a_bad_key_file_is_refused_without_naming_its_path(self) -> None:
        w = self.worlds["bad_key_file"]
        self.assertEqual(w.run.returncode, 2)
        self.assertNotIn("secret-place", w.run.stdout + w.run.stderr)

    @unittest.skipUnless(shutil.which("script"), "needs script(1)")
    def test_a_chat_connector_types_the_prompt_and_submits_it_apart(self) -> None:
        w = self.worlds["chat"]
        self.assertEqual(w.run.returncode, 0, w.said())
        self.assertEqual(w.seen("start.argv"), "start\n--harness\nstubchat\n--practice")
        # The `deadline:` line is in the run's host.log only, as the tracer has it in chat mode;
        # the row passes on it.
        terminal = next(w.tmpdir.rglob("start.out")).read_text()
        self.assertNotIn("deadline:", terminal)
        chunks = json.loads(w.seen("start.stdin_chunks"))
        keys, text, enter = chunks[0], chunks[1], chunks[2]
        self.assertEqual(keys[1], "\x1b[B\r")
        # The text arrives without its Enter, and the Enter comes on its own (a read of its own).
        self.assertEqual(text[1], PROMPT)
        self.assertEqual(enter[1], "\r")


if __name__ == "__main__":
    unittest.main()
