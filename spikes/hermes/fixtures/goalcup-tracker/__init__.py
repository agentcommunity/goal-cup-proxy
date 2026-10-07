"""goal-cup capture spike 7 — Hermes folder-scoped tracker shim.

Placement : <workspace>/.hermes/plugins/goalcup-tracker/{plugin.yaml,__init__.py}
Loading   : TWO gates, both verified in hermes-agent 0.19.0 source
            (hermes_cli/plugins.py, ``PluginManager._discover_and_load_inner``):

              1. ``HERMES_ENABLE_PROJECT_PLUGINS`` must be truthy, or
                 ``Path.cwd() / ".hermes" / "plugins"`` is never scanned
                 (plugins.py:1357).  Process-scoped env pin -> folder-scoped.
              2. the plugin key must appear in ``plugins.enabled`` in
                 ``$HERMES_HOME/config.yaml`` (plugins.py:1454-1467).  There is
                 NO project-level config.yaml: ``hermes_cli.config.get_config_path()``
                 is ``get_hermes_home() / "config.yaml"``, full stop.  So the
                 allow-list entry is the one thing this design cannot keep inside
                 the workspace.  See NOTES.md "Isolation gap".

            ``--safe-mode`` sets ``HERMES_SAFE_MODE=1`` and
            ``discover_and_load()`` returns before scanning anything
            (plugins.py:1288).  A ``--safe-mode`` start therefore loads NO
            plugins and registers NO shell hooks.  That is a capture bypass,
            not a bug in this shim.

Contract: record raw, interpret later (AGENTS.md).  Every hook payload is
forwarded verbatim.  The only shaping done here:

  * values that are not JSON-serialisable (provider SDK objects on
    ``post_api_request.assistant_message`` / ``response``) fall back to
    ``repr()`` through ``_enc``; nothing is dropped.
  * ``request_messages`` is posted whole and is NEVER mutated.  It is the
    shallow-copied ``api_kwargs["messages"]`` (or ``["input"]``) that Hermes is
    about to hand the provider client — the byte-exact wire context.  Its
    sibling ``request`` is the *sanitised* view: ``_sanitize_hook_payload``
    truncates every string at 8000 chars, every sequence at 200 items, and the
    whole payload at ``HERMES_PLUGIN_PAYLOAD_MAX_CHARS`` (default 50000).  Use
    ``request_messages`` for fidelity and ``request["body"]`` for the non-message
    parameters (``system``, ``instructions``, ``tools``, ``max_tokens``, ...).

Threading: ``PluginManager.invoke_hook`` calls every callback SYNCHRONOUSLY on
the agent's own thread and 0.19.0 enforces no per-callback timeout (the
``plugins.hook_callback_timeout`` key documented upstream is not read here).  A
blocking HTTP POST would stall the agent loop, so every post goes through a
bounded queue drained by one daemon thread.  The two hooks with decision
semantics (``pre_llm_call``, ``pre_tool_call``) post SYNCHRONOUSLY with a short
timeout so the receiver's answer can be forwarded; everything else is async.

Blocking (assertion A7): **Hermes has no user-message rejection surface.**
``pre_llm_call`` return values are only ever concatenated into
``plugin_user_context`` and appended to the user message
(``agent/turn_context.py:693-741``); no return shape aborts the turn.
``pre_gateway_dispatch`` can ``{"action": "skip"}`` but fires only on the
gateway path, never in the CLI.  So **A7 = recorded, not blocked.**  This shim
records the receiver's ``{"decision": "block"}`` verdict on the post and, only
when ``GOALCUP_BLOCK_MODE=nudge``, appends a violation note as ephemeral
``context`` (which changes the wire bytes — off by default).

Env (all process-scoped, set by run.sh):
  GOALCUP_RECV            receiver base URL      (default http://127.0.0.1:7777)
  GOALCUP_RUN             run id, echoed in every post
  GOALCUP_BLOCK_MODE      record | nudge         (default record)
  GOALCUP_POST_TIMEOUT    async post timeout, s  (default 5)
  GOALCUP_SYNC_TIMEOUT    sync  post timeout, s  (default 3)
  GOALCUP_QUEUE_MAX       queue depth            (default 10000)
"""

from __future__ import annotations

import json
import os
import queue
import threading
import time
import urllib.error
import urllib.request
import uuid

RECV = os.environ.get("GOALCUP_RECV") or "http://127.0.0.1:7777"
RUN = os.environ.get("GOALCUP_RUN") or "unknown-run"
BLOCK_MODE = (os.environ.get("GOALCUP_BLOCK_MODE") or "record").strip().lower()


def _f(name: str, default: float) -> float:
    try:
        return float(os.environ.get(name) or default)
    except (TypeError, ValueError):
        return default


POST_TIMEOUT = _f("GOALCUP_POST_TIMEOUT", 5.0)
SYNC_TIMEOUT = _f("GOALCUP_SYNC_TIMEOUT", 3.0)
QUEUE_MAX = int(_f("GOALCUP_QUEUE_MAX", 10000))

SHIM = uuid.uuid4().hex[:12]

# Every hook name in hermes_cli.plugins.VALID_HOOKS as of hermes-agent 0.19.0.
# Verified by reading the set at plugins.py:135-222 — 23 names, NOT the 26+
# the upstream docs list.  on_stream_start / on_stream_delta / on_stream_end /
# on_interim_message / pre_command / on_skill_lifecycle /
# transform_api_error_classification do NOT exist in this build.
ALL_HOOKS = [
    "pre_api_request",
    "post_api_request",
    "api_request_error",
    "pre_llm_call",
    "post_llm_call",
    "transform_llm_output",
    "pre_tool_call",
    "post_tool_call",
    "transform_tool_result",
    "transform_terminal_output",
    "pre_verify",
    "on_session_start",
    "on_session_end",
    "on_session_finalize",
    "on_session_reset",
    "subagent_start",
    "subagent_stop",
    "pre_gateway_dispatch",
    "pre_approval_request",
    "post_approval_response",
    "kanban_task_claimed",
    "kanban_task_completed",
    "kanban_task_blocked",
]

# Hooks whose return value Hermes acts on.  This shim must return None for all
# of them except where explicitly noted, or it would change agent behaviour.
#   transform_llm_output      first non-None str replaces the response text
#   transform_tool_result     first non-None str replaces the tool result
#   transform_terminal_output first non-None str replaces terminal output
#   pre_verify                {"action": "continue"} keeps the agent going
#   pre_tool_call             {"action": "block"|"approve"} gates the tool
#   pre_llm_call              {"context": str} is appended to the user message
#   pre_gateway_dispatch      {"action": "skip"|"rewrite"} (gateway only)
SYNC_HOOKS = {"pre_llm_call", "pre_tool_call"}

_seq_lock = threading.Lock()
_seq = 0


def _next_seq() -> int:
    global _seq
    with _seq_lock:
        _seq += 1
        return _seq


def _jsonable(value, _depth: int = 0):
    """Make a payload JSON-serialisable without losing anything silently.

    Deliberately NOT a truncator: goal-cup records raw.  Unknown objects become
    a tagged repr so the verifier can see that a non-plain value was present
    and what it looked like.
    """
    if _depth > 12:
        return {"_goalcup_depth_limit": type(value).__name__}
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (bytes, bytearray)):
        return {"_goalcup_bytes": len(value)}
    if isinstance(value, dict):
        return {str(k): _jsonable(v, _depth + 1) for k, v in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [_jsonable(v, _depth + 1) for v in value]
    dump = getattr(value, "model_dump", None)
    if callable(dump):
        try:
            return _jsonable(dump(), _depth + 1)
        except Exception:
            pass
    d = getattr(value, "__dict__", None)
    if isinstance(d, dict):
        try:
            return {
                "_goalcup_type": type(value).__name__,
                "_goalcup_attrs": {
                    str(k): _jsonable(v, _depth + 1)
                    for k, v in d.items()
                    if not str(k).startswith("_")
                },
            }
        except Exception:
            pass
    return {"_goalcup_type": type(value).__name__, "_goalcup_repr": repr(value)}


def _enc(obj) -> bytes:
    try:
        return json.dumps(obj, ensure_ascii=False).encode("utf-8")
    except Exception as exc:  # pragma: no cover - _jsonable should prevent this
        return json.dumps(
            {"_goalcup_unserializable": True, "error": str(exc)}
        ).encode("utf-8")


def _http(hook: str, body: bytes, timeout: float):
    req = urllib.request.Request(
        "%s/%s" % (RECV.rstrip("/"), hook),
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-GoalCup-Hook": hook,
            "X-GoalCup-Shim": SHIM,
            "X-GoalCup-Run": RUN,
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read().decode("utf-8", "replace")
    try:
        out = json.loads(raw) if raw.strip() else {}
    except json.JSONDecodeError:
        return {}
    return out if isinstance(out, dict) else {}


_q: "queue.Queue" = queue.Queue(maxsize=QUEUE_MAX)
_dropped = 0
_worker_started = False
_worker_lock = threading.Lock()


def _drain() -> None:
    while True:
        item = _q.get()
        if item is None:
            return
        hook, body = item
        try:
            _http(hook, body, POST_TIMEOUT)
        except Exception:
            pass
        finally:
            _q.task_done()


def _start_worker() -> None:
    global _worker_started
    with _worker_lock:
        if _worker_started:
            return
        t = threading.Thread(target=_drain, name="goalcup-tracker", daemon=True)
        t.start()
        _worker_started = True


def _envelope(hook: str, kwargs: dict) -> dict:
    env = {
        "shim": SHIM,
        "run": RUN,
        "seq": _next_seq(),
        "hook": hook,
        "t": time.time(),
        "pid": os.getpid(),
        "thread": threading.current_thread().name,
        "cwd": os.getcwd(),
        "hermes_home": os.environ.get("HERMES_HOME"),
        "kwargs": _jsonable(kwargs),
    }
    # The stub receiver's --block-over gate reads a top-level "prompt" string.
    um = kwargs.get("user_message")
    if isinstance(um, str):
        env["prompt"] = um
    return env


def _post_async(hook: str, kwargs: dict) -> None:
    global _dropped
    body = _enc(_envelope(hook, kwargs))
    try:
        _q.put_nowait((hook, body))
    except queue.Full:
        _dropped += 1  # never block the agent loop; check.py reports the gap


def _post_sync(hook: str, kwargs: dict) -> dict:
    body = _enc(_envelope(hook, kwargs))
    try:
        return _http(hook, body, SYNC_TIMEOUT)
    except Exception:
        return {}


def _make_observer(hook: str):
    def _cb(**kwargs):
        _post_async(hook, kwargs)
        return None  # never influence the agent

    _cb.__name__ = "goalcup_%s" % hook
    return _cb


def _on_pre_llm_call(**kwargs):
    """Turn-level hook.  Posts synchronously so the receiver's A7 verdict is
    recorded next to the message it applies to.  Returns None (= no injection)
    unless GOALCUP_BLOCK_MODE=nudge, because Hermes cannot reject a user
    message and a silent context injection would change the wire bytes."""
    decision = _post_sync("pre_llm_call", kwargs)
    if BLOCK_MODE == "nudge" and decision.get("decision") == "block":
        reason = decision.get("reason") or "rejected by goal-cup receiver"
        _post_async(
            "goalcup.violation",
            {
                "kind": "over_length_user_message",
                "reason": reason,
                "user_message_len": len(kwargs.get("user_message") or ""),
                "session_id": kwargs.get("session_id"),
                "turn_id": kwargs.get("turn_id"),
                "action": "nudge_injected",
            },
        )
        return {"context": "[goal-cup] rule violation: %s" % reason}
    if decision.get("decision") == "block":
        _post_async(
            "goalcup.violation",
            {
                "kind": "over_length_user_message",
                "reason": decision.get("reason"),
                "user_message_len": len(kwargs.get("user_message") or ""),
                "session_id": kwargs.get("session_id"),
                "turn_id": kwargs.get("turn_id"),
                "action": "recorded_only",
                "note": "Hermes has no user-message rejection surface; A7 = recorded",
            },
        )
    return None


def _on_pre_tool_call(**kwargs):
    """The one hook that CAN block (``{"action": "block", "message": str}``,
    resolved in plugins.py:2228 ``resolve_pre_tool_block``).  Recorded only —
    the spike must not change what the agent is allowed to do."""
    _post_sync("pre_tool_call", kwargs)
    return None


def register(ctx) -> None:
    _start_worker()
    for hook in ALL_HOOKS:
        if hook == "pre_llm_call":
            ctx.register_hook(hook, _on_pre_llm_call)
        elif hook == "pre_tool_call":
            ctx.register_hook(hook, _on_pre_tool_call)
        else:
            ctx.register_hook(hook, _make_observer(hook))

    # One startup post so run.sh / check.py can prove the shim actually loaded
    # (the "did project plugins load?" unknown).  Sent async; the receiver
    # records the arrival time.
    _post_async(
        "goalcup.loaded",
        {
            "plugin_name": getattr(getattr(ctx, "manifest", None), "name", None),
            "plugin_key": getattr(getattr(ctx, "manifest", None), "key", None),
            "plugin_source": getattr(getattr(ctx, "manifest", None), "source", None),
            "plugin_path": getattr(getattr(ctx, "manifest", None), "path", None),
            "hooks_registered": ALL_HOOKS,
            "block_mode": BLOCK_MODE,
            "argv": list(__import__("sys").argv),
            "env": {
                k: os.environ.get(k)
                for k in (
                    "HERMES_HOME",
                    "HERMES_ENABLE_PROJECT_PLUGINS",
                    "HERMES_SAFE_MODE",
                    "HERMES_IGNORE_USER_CONFIG",
                    "HERMES_IGNORE_RULES",
                    "HERMES_PLUGIN_PAYLOAD_MAX_CHARS",
                    "HERMES_ACCEPT_HOOKS",
                    "HERMES_YOLO_MODE",
                    "GOALCUP_RUN",
                )
            },
        },
    )
