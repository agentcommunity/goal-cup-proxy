#!/usr/bin/env python3
"""goal-cup capture spike 9 -- Cursor CLI hook shim (leg 2).

Cursor hooks are command-type only: the CLI spawns this process, writes one JSON object on
stdin, and reads one JSON object from stdout. There is no in-process plugin API, so every
hook in <ws>/.cursor/hooks.json points at this one script and the event name is taken from
the payload's `hook_event_name` field (the `command` string is passed with no argv, because
whether Cursor shell-splits it is undocumented -- see NOTES.md).

What it does, per invocation:
  1. read the raw stdin bytes (kept byte-exact for the hash chain);
  2. attach a monotonic per-conversation counter and a SHA-256 chain over the raw bytes
     (research file B9: the only defence against a hook being removed mid-run is a gap in
     the sequence, so the counter has to live outside the harness);
  3. attach the SHA-256 of the live <ws>/.cursor/hooks.json and compare it to the hash
     pinned by run.sh -- a mismatch means the config hot-reloaded into something else;
  4. append the whole thing to $GOALCUP_R/hooks-shim.ndjson (local witness that survives
     the receiver being down);
  5. POST it to $GOALCUP_RECV/<hook_event_name>;
  6. translate the receiver's answer into Cursor's hook-output shape.

Blocking (A7). The stub receiver answers `{"decision":"block","reason":...}` when a
top-level "prompt" field exceeds --block-over chars. The payload is posted unmodified, so
`prompt` stays top-level and the receiver's existing rule fires on beforeSubmitPrompt.
Cursor documents two refusal routes; GOALCUP_BLOCK_MODE picks which one the spike exercises:
  json  (default) exit 0 and print {"continue": false, "permission": "deny", ...}
  exit2            print the same object and exit 2 ("Exit code 2 from command hooks blocks
                   the action (equivalent to returning permission: "deny")")
Both are recorded in the post body under _goalcup.block_mode so check.py can tell which
route produced the observed behaviour.

Fail-open by design: any error here prints "{}" and exits 0, so a broken tracker never
wedges the participant's session. The exception is beforeSubmitPrompt, which run.sh marks
"failClosed": true in hooks.json -- Cursor then treats a *hook failure* as a block. That is
the tracker's tamper stance, not this script's.

Environment (set process-scoped by run.sh):
  GOALCUP_RECV   receiver base URL, default http://127.0.0.1:7777
  GOALCUP_R      artifact root; state/ and hooks-shim.ndjson live here
  GOALCUP_RUN    run id, echoed into every post
  GOALCUP_HOOKS_SHA  pinned sha256 of hooks.json
  GOALCUP_BLOCK_MODE json | exit2
  GOALCUP_TIMEOUT    receiver timeout in seconds (default 8)
"""
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request

RECV = os.environ.get("GOALCUP_RECV", "http://127.0.0.1:7777").rstrip("/")
R = os.environ.get("GOALCUP_R", "")
RUN = os.environ.get("GOALCUP_RUN", "")
PINNED = os.environ.get("GOALCUP_HOOKS_SHA", "")
BLOCK_MODE = os.environ.get("GOALCUP_BLOCK_MODE", "json")
TIMEOUT = float(os.environ.get("GOALCUP_TIMEOUT", "8"))
HOOKS_JSON = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "hooks.json")


def sha256_file(path):
    try:
        with open(path, "rb") as fh:
            return hashlib.sha256(fh.read()).hexdigest()
    except OSError:
        return ""


def state_dir():
    d = os.path.join(R, "state") if R else os.path.join(os.path.dirname(os.path.abspath(__file__)), ".state")
    os.makedirs(d, exist_ok=True)
    return d


def chain(conv_id, raw):
    """Monotonic counter + hash chain per conversation_id, kept outside the harness.

    Not locked: Cursor may run hooks concurrently, so a torn read shows up as a repeated
    seq or a broken link, which is exactly the signal check.py looks for (A11).
    """
    safe = "".join(c if (c.isalnum() or c in "-_") else "_" for c in (conv_id or "no-conversation"))[:120]
    p = os.path.join(state_dir(), f"chain-{safe}.json")
    try:
        with open(p) as fh:
            st = json.load(fh)
    except (OSError, ValueError):
        st = {"seq": 0, "prev": "0" * 64}
    seq = int(st.get("seq", 0)) + 1
    prev = str(st.get("prev", "0" * 64))
    h = hashlib.sha256(prev.encode() + raw).hexdigest()
    try:
        with open(p, "w") as fh:
            json.dump({"seq": seq, "prev": h}, fh)
    except OSError:
        pass
    return seq, prev, h


def local_mirror(rec):
    if not R:
        return
    try:
        with open(os.path.join(R, "hooks-shim.ndjson"), "a") as fh:
            fh.write(json.dumps(rec, ensure_ascii=False) + "\n")
    except OSError:
        pass


def note(msg):
    if not R:
        return
    try:
        with open(os.path.join(R, "hook-shim-errors.log"), "a") as fh:
            fh.write(f"{time.time()} {msg}\n")
    except OSError:
        pass


def main():
    raw = sys.stdin.buffer.read()
    try:
        payload = json.loads(raw.decode("utf-8", "replace")) if raw.strip() else {}
    except ValueError:
        payload = {}
    if not isinstance(payload, dict):
        payload = {"_non_object_payload": payload}

    event = payload.get("hook_event_name") or (sys.argv[1] if len(sys.argv) > 1 else "unknown")
    conv = payload.get("conversation_id") or ""
    seq, prev, h = chain(conv, raw)
    live_sha = sha256_file(HOOKS_JSON)

    post = dict(payload)
    post["_goalcup"] = {
        "run": RUN,
        "event": event,
        "seq": seq,
        "prev": prev,
        "hash": h,
        "raw_len": len(raw),
        "raw_sha256": hashlib.sha256(raw).hexdigest(),
        "t_shim": time.time(),
        "pid": os.getpid(),
        "cwd": os.getcwd(),
        "hooks_json_sha256": live_sha,
        "hooks_json_pinned": PINNED,
        "config_mismatch": bool(PINNED) and live_sha != PINNED,
        "block_mode": BLOCK_MODE,
    }
    local_mirror(post)

    answer = {}
    try:
        body = json.dumps(post, ensure_ascii=False).encode("utf-8")
        req = urllib.request.Request(
            f"{RECV}/{event}", data=body,
            headers={"Content-Type": "application/json", "X-Goalcup-Run": RUN or "-",
                     "X-Goalcup-Seq": str(seq)},
            method="POST")
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            txt = resp.read().decode("utf-8", "replace")
        answer = json.loads(txt) if txt.strip() else {}
        if not isinstance(answer, dict):
            answer = {}
    except (urllib.error.URLError, OSError, ValueError) as e:
        note(f"{event} seq={seq} post failed: {e!r}")
        answer = {}

    blocked = answer.get("decision") == "block" or answer.get("permission") == "deny"
    if blocked and event in ("beforeSubmitPrompt", "preToolUse", "beforeShellExecution",
                             "beforeMCPExecution", "subagentStart"):
        reason = answer.get("reason") or answer.get("user_message") or "blocked by goal-cup tracker"
        out = {
            "continue": False,              # documented beforeSubmitPrompt output field
            "permission": "deny",           # documented permission-hook output field
            "user_message": reason,
            "agent_message": reason,
        }
        sys.stdout.write(json.dumps(out))
        sys.stdout.flush()
        sys.exit(2 if BLOCK_MODE == "exit2" else 0)

    # Pass-through: never rewrite tool output, never inject followup_message.
    sys.stdout.write("{}")
    sys.stdout.flush()
    sys.exit(0)


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except BaseException as e:  # fail open, always
        note(f"fatal {e!r}")
        sys.stdout.write("{}")
        sys.exit(0)
