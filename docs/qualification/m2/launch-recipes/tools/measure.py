#!/usr/bin/env python3
"""T02 launch-recipe measurement runner. Throwaway measurement tooling, not production code.

One bounded live launch of an installed harness with an explicit allowlisted process
environment (nothing inherited from the caller except HOME/USER/LOGNAME/SHELL/TMPDIR),
the goal delivered once as a single argv element after `--`, stdin /dev/null unless the
variant says otherwise. Raw stdout/stderr, native-store copies and probe recordings go to
an ignored spikes/<harness>/out/ directory; meta.json beside them holds only counts,
names, paths, versions and our own goal text (no header values, no fingerprints).

usage:
  measure.py run --harness codex|claude --variant V --run-id ID --goal-file F
                 --out DIR --ws-root DIR [--harnesses DIR] [--claude-bin PATH]
                 [--proxy --wprobe PATH] [--fake-home] [--scoped] [--codex-trust-override]
                 [--allow-tool NAME] [--appwide] [--timeout S]
                 [--expect-file NAME --expect-content S]
  measure.py argmax --out FILE [--harnesses DIR] [--claude-bin PATH]
--harnesses defaults to $GOALCUP_HARNESSES (exported by harnesses/env.sh).
--appwide lists names, mtimes and sizes (never contents) under the app-wide roots below
before and after the run and reports every created/modified/deleted path.
variants: positive | big | approval | stdinpipe | bash | missing | empty | dash_nosep
"""
import argparse, errno, glob, hashlib, json, os, re, stat, subprocess, sys, threading, time, tomllib, uuid

CODEX_BIN = CLAUDE_BIN = CODEX_HOME = CLAUDE_HOME = WPROBE = None  # set by configure()
APPWIDE_ROOTS = ("~/.claude", "~/.claude.json", "~/Library/Caches/claude-cli-nodejs", "~/.local/state/claude",
                 "~/.cache/claude", "~/.local/share/claude", "~/.config/claude",
                 "~/Library/Application Support/Claude", "~/.codex", "~/.agents")
NO_SIZE_ROOTS = ("~/Library/Application Support/Claude",)  # names and mtimes only
LOCK_DIRS = ("~/.local/state/claude/locks",)
SHALLOW_ROOTS = ("~",)  # immediate children only (e.g. ~/.zcompdump*, ~/.zsh_history)
PATH = "/usr/bin:/bin:/usr/sbin:/sbin"
BASE_KEYS = ("HOME", "USER", "LOGNAME", "SHELL", "TMPDIR")
TIMEOUT = 600
WITH_GOAL = ("positive", "big", "approval", "stdinpipe", "bash")


def sha(b):
    return hashlib.sha256(b).hexdigest()


def now_iso(t):
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + ".%03dZ" % int((t % 1) * 1000)


def configure(a):
    global CODEX_BIN, CLAUDE_BIN, CODEX_HOME, CLAUDE_HOME, WPROBE
    h = a.harnesses or os.environ.get("GOALCUP_HARNESSES")
    if not h:
        sys.exit("--harnesses or $GOALCUP_HARNESSES (source harnesses/env.sh) is required")
    CODEX_BIN = (h + "/node/lib/node_modules/@openai/codex/node_modules/"
                 "@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex")
    CODEX_HOME = h + "/home/codex"
    CLAUDE_HOME = h + "/home/claude"
    CLAUDE_BIN = a.claude_bin or os.path.expanduser("~/.local/bin/claude")
    WPROBE = getattr(a, "wprobe", None)
    if getattr(a, "proxy", False) and not (WPROBE and os.access(WPROBE, os.X_OK)):
        sys.exit("--proxy needs --wprobe PATH to an executable probe binary")


def base_env():
    env = {k: os.environ[k] for k in BASE_KEYS if k in os.environ}
    env["PATH"] = PATH
    return env


def build(h, variant, goal, ws, sid, scoped=None, trust_override=False, allow_tools=()):
    """scoped: None, or a dict of per-run dirs {"zdotdir","xdg_state","xdg_cache"} (Claude only)."""
    if h == "codex":
        argv = [CODEX_BIN, "exec", "--json", "-m", "gpt-5.6-luna",
                "-c", 'model_reasoning_effort="low"', "-c", 'approval_policy="never"',
                "-c", "check_for_update_on_startup=false"]
        if trust_override:
            argv += ["-c", 'projects={ "%s" = { trust_level = "trusted" } }' % ws]
        argv += ["-s", "workspace-write", "--skip-git-repo-check", "-C", ws]
        env = {"CODEX_HOME": CODEX_HOME}
    else:
        argv = [CLAUDE_BIN, "-p", "--model", "haiku", "--effort", "low",
                "--permission-mode", "acceptEdits", "--permission-prompts", "none",
                "--session-id", sid, "--output-format", "json"]
        env = {"CLAUDE_CONFIG_DIR": CLAUDE_HOME, "DISABLE_AUTOUPDATER": "1"}
        scoped = scoped or {}
        if scoped.get("mcp_off"):
            argv += ["--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}']
            env.update({"ENABLE_CLAUDEAI_MCP_SERVERS": "false", "CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS": "1"})
        for key, var in (("zdotdir", "ZDOTDIR"), ("xdg_state", "XDG_STATE_HOME"), ("xdg_cache", "XDG_CACHE_HOME")):
            if scoped.get(key):
                env[var] = scoped[key]
        for t in allow_tools:
            argv += ["--allowedTools", t]
    if variant in WITH_GOAL:
        argv += ["--", goal]
    elif variant == "empty":
        argv += ["--", ""]
    elif variant == "dash_nosep":
        argv += [goal]
    elif variant == "missing":
        pass
    else:
        sys.exit("unknown variant " + variant)
    return argv, env


def exec_bytes(argv, env):
    """Bytes the kernel copies for execve: strings + NULs + pointer array (8-byte)."""
    s = sum(len(a.encode()) + 1 for a in argv) + sum(len(k.encode()) + len(v.encode()) + 2 for k, v in env.items())
    return s, s + 8 * (len(argv) + len(env) + 2)


# ------------------------------------------------------------------ native discovery
def native_candidates(h, sid):
    if h == "codex":
        return set(glob.glob(CODEX_HOME + "/sessions/**/rollout-*.jsonl", recursive=True))
    return set(glob.glob(CLAUDE_HOME + "/projects/*/" + sid + ".jsonl"))


def walk_strings(o):
    if isinstance(o, str):
        yield o
    elif isinstance(o, dict):
        for v in o.values():
            yield from walk_strings(v)
    elif isinstance(o, list):
        for v in o:
            yield from walk_strings(v)


def goal_counts(obj, goal):
    exact = sub = 0
    for s in walk_strings(obj):
        if goal and s == goal:
            exact += 1
        if goal and goal in s:
            sub += 1
    return exact, sub


def excerpt(s, n=240):
    s = s if isinstance(s, str) else json.dumps(s)
    return s if len(s) <= n else s[:n] + "...(+%d)" % (len(s) - n)


def analyse_codex(path, goal, ws):
    recs = [json.loads(l) for l in open(path, encoding="utf-8") if l.strip()]
    out = {"records": len(recs), "record_types": {}, "user_messages": [], "tool_calls": [],
           "tool_outputs": [], "goal_exact_by_record": [], "session_meta": None, "turn_context": []}
    for i, r in enumerate(recs):
        t = r.get("type")
        p = r.get("payload") or {}
        key = t + ":" + str(p.get("type"))
        out["record_types"][key] = out["record_types"].get(key, 0) + 1
        ex, sb = goal_counts(r, goal)
        if ex or sb:
            out["goal_exact_by_record"].append({"line": i + 1, "kind": key, "role": p.get("role"), "exact": ex, "substring": sb})
        if t == "session_meta":
            out["session_meta"] = {k: p.get(k) for k in ("id", "cwd", "cli_version", "originator", "source", "model_provider", "timestamp")}
        elif t == "turn_context":
            tc = {k: p.get(k) for k in ("model", "effort", "reasoning_effort", "approval_policy", "sandbox_policy", "cwd")}
            out["turn_context"].append(tc)
        elif t == "response_item" and p.get("type") == "message" and p.get("role") == "user":
            texts = [c.get("text") for c in p.get("content", []) if isinstance(c, dict)]
            for tx in texts:
                out["user_messages"].append({"line": i + 1, "source": "response_item", "is_goal": tx == goal,
                                             "len": len(tx.encode()) if tx else 0,
                                             "head": excerpt(tx, 80) if tx != goal else "<GOAL>"})
        elif t == "event_msg" and p.get("type") == "user_message":
            tx = p.get("message")
            out["user_messages"].append({"line": i + 1, "source": "event_msg", "is_goal": tx == goal,
                                         "len": len(tx.encode()) if tx else 0,
                                         "head": excerpt(tx, 80) if tx != goal else "<GOAL>"})
        elif t == "response_item" and p.get("type") in ("function_call", "custom_tool_call", "local_shell_call"):
            out["tool_calls"].append({"line": i + 1, "type": p.get("type"), "name": p.get("name"),
                                      "call_id": p.get("call_id"),
                                      "args": excerpt(p.get("arguments") or p.get("input") or p.get("action"), 400)})
        elif t == "response_item" and p.get("type") in ("function_call_output", "custom_tool_call_output"):
            out["tool_outputs"].append({"line": i + 1, "call_id": p.get("call_id"), "output": excerpt(p.get("output"), 300)})
    ex, sb = goal_counts(recs, goal)
    out["goal_exact_total"], out["goal_substring_total"] = ex, sb
    return out


def analyse_claude(path, goal, ws):
    recs = [json.loads(l) for l in open(path, encoding="utf-8") if l.strip()]
    out = {"records": len(recs), "record_types": {}, "user_messages": [], "tool_calls": [],
           "tool_results": [], "models": [], "goal_exact_by_record": [], "session_ids": [], "cwds": [], "versions": []}
    for i, r in enumerate(recs):
        t = r.get("type")
        out["record_types"][t] = out["record_types"].get(t, 0) + 1
        for k, dst in (("sessionId", "session_ids"), ("cwd", "cwds"), ("version", "versions")):
            v = r.get(k)
            if v and v not in out[dst]:
                out[dst].append(v)
        ex, sb = goal_counts(r, goal)
        if ex or sb:
            out["goal_exact_by_record"].append({"line": i + 1, "type": t, "exact": ex, "substring": sb,
                                                "isMeta": r.get("isMeta"), "promptSource": r.get("promptSource"),
                                                "origin": r.get("origin")})
        att = r.get("attachment") or {}
        if t == "attachment" and att.get("type") in ("deferred_tools_delta", "mcp_instructions_delta"):
            for nm in att.get("addedNames") or []:
                out.setdefault("mcp_related_names", [])
                if (nm.startswith("mcp__") or att.get("type") == "mcp_instructions_delta") and nm not in out["mcp_related_names"]:
                    out["mcp_related_names"].append(nm)
        if t == "attachment":
            out.setdefault("attachment_types", {})
            out["attachment_types"][att.get("type")] = out["attachment_types"].get(att.get("type"), 0) + 1
        m = r.get("message") or {}
        if t == "assistant":
            if m.get("model") and m.get("model") not in out["models"]:
                out["models"].append(m.get("model"))
            for b in m.get("content") or []:
                if isinstance(b, dict) and b.get("type") == "tool_use":
                    out["tool_calls"].append({"line": i + 1, "name": b.get("name"), "id": b.get("id"),
                                              "input": excerpt(b.get("input"), 400)})
        if t == "user":
            c = m.get("content")
            if isinstance(c, str):
                out["user_messages"].append({"line": i + 1, "is_goal": c == goal, "len": len(c.encode()),
                                             "isMeta": r.get("isMeta"), "head": excerpt(c, 80) if c != goal else "<GOAL>"})
            elif isinstance(c, list):
                for b in c:
                    if not isinstance(b, dict):
                        continue
                    if b.get("type") == "text":
                        tx = b.get("text")
                        out["user_messages"].append({"line": i + 1, "is_goal": tx == goal, "len": len(tx.encode()),
                                                     "isMeta": r.get("isMeta"), "head": excerpt(tx, 80) if tx != goal else "<GOAL>"})
                    elif b.get("type") == "tool_result":
                        out["tool_results"].append({"line": i + 1, "tool_use_id": b.get("tool_use_id"),
                                                    "is_error": b.get("is_error"), "content": excerpt(b.get("content"), 300)})
    ex, sb = goal_counts(recs, goal)
    out["goal_exact_total"], out["goal_substring_total"] = ex, sb
    names = out.get("mcp_related_names", [])
    out["mcp_servers"] = sorted({n.split("__")[1] for n in names if n.startswith("mcp__") and n.count("__") >= 2}
                                | {n for n in names if not n.startswith("mcp__")})
    out["mcp_tool_count"] = sum(1 for n in names if n.startswith("mcp__"))
    return out


# ------------------------------------------------------------------ metadata-only listings
def listing(roots, shallow=SHALLOW_ROOTS):
    """{abs path: (mtime_ns, size or -1, is_dir)} for every entry under roots; no contents read."""
    out = {}
    for r in shallow:
        p = os.path.expanduser(r)
        for n in os.listdir(p):
            q = os.path.join(p, n)
            try:
                st = os.lstat(q)
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(st.st_mode):
                out[q] = (st.st_mtime_ns, st.st_size, False)
    for r in roots:
        p = os.path.expanduser(r)
        nosize = r in NO_SIZE_ROOTS
        if not os.path.lexists(p):
            continue
        def add(path):
            try:
                st = os.lstat(path)
            except FileNotFoundError:
                return
            out[path] = (st.st_mtime_ns, -1 if nosize else st.st_size, stat.S_ISDIR(st.st_mode))
        add(p)
        if os.path.isdir(p) and not os.path.islink(p):
            for d, ds, fs in os.walk(p):
                for n in ds + fs:
                    add(os.path.join(d, n))
    return out


def diff_listing(before, after, tags):
    home = os.path.expanduser("~")
    def rel(x):
        return "~" + x[len(home):] if x.startswith(home) else x
    rows = []
    for k in sorted(set(before) | set(after)):
        b, a = before.get(k), after.get(k)
        if b == a:
            continue
        kind = "created" if b is None else "deleted" if a is None else "modified"
        who = "run" if any(t and t in k for t in tags) else "unattributed"
        rows.append({"path": rel(k), "change": kind, "dir": (a or b)[2], "size": (a or b)[1], "attributed_to": who})
    return rows


def claude_snapshot_check(before_names, seen=None):
    """Counts only (size, alias lines, function definitions); snapshot text is never stored."""
    d = os.path.join(CLAUDE_HOME, "shell-snapshots")
    res = dict(seen or {})
    new = sorted(set(os.listdir(d)) - before_names) if os.path.isdir(d) else []
    for n in new:
        try:
            b = open(os.path.join(d, n), "rb").read()
        except FileNotFoundError:
            continue
        txt = b.decode("utf-8", "replace")
        res[n] = {"name": n, "size": len(b), "alias_lines": len(re.findall(r"^alias ", txt, re.M)),
                  "function_defs": len(re.findall(r"^(?:function |[A-Za-z_][\w:.-]* \(\) )", txt, re.M)),
                  "still_present_after_exit": None}
    return res


def codex_config_state():
    p = os.path.join(CODEX_HOME, "config.toml")
    b = open(p, "rb").read()
    try:
        projects = sorted((tomllib.loads(b.decode()).get("projects") or {}).keys())
    except Exception:
        projects = None
    return sha(b), projects


def analyse_wire(path, goal, harness):
    out = {"tunnels": {}, "mitm_hosts": {}, "client_tls_fail": 0, "errors": 0, "http": [],
           "ws_open": [], "model_requests": [], "plain_http_refused": 0}
    if not os.path.exists(path):
        return out
    for l in open(path, encoding="utf-8"):
        try:
            r = json.loads(l)
        except Exception:
            out["errors"] += 1
            continue
        k = r.get("kind")
        if k == "tunnel":
            h = "%s:%s" % (r.get("host"), r.get("port"))
            out["tunnels"][h] = out["tunnels"].get(h, 0) + 1
        elif k == "mitm_open":
            out["mitm_hosts"][r.get("host")] = out["mitm_hosts"].get(r.get("host"), 0) + 1
        elif k == "client_tls_fail":
            out["client_tls_fail"] += 1
        elif k == "error":
            out["errors"] += 1
        elif k == "plain_http_refused":
            out["plain_http_refused"] += 1
        elif k == "ws_open":
            out["ws_open"].append({"host": r.get("host"), "path": (r.get("path") or "").split("?")[0],
                                   "permessage_deflate": "permessage-deflate" in (r.get("extensions") or "")})
        elif k == "http":
            path_noq = (r.get("path") or "").split("?")[0]
            out["http"].append({"host": r.get("host"), "method": r.get("method"), "path": path_noq,
                                "status": r.get("status"), "ws": bool(r.get("ws"))})
            if harness == "claude" and r.get("method") == "POST" and path_noq.startswith("/v1/messages"):
                body = (r.get("req_body") or {}).get("text")
                try:
                    j = json.loads(body) if body else None
                except Exception:
                    j = None
                if j is not None:
                    ex_user = 0
                    for msg in j.get("messages", []):
                        if msg.get("role") != "user":
                            continue
                        c = msg.get("content")
                        blocks = [c] if isinstance(c, str) else [b.get("text") for b in c if isinstance(b, dict) and b.get("type") == "text"]
                        ex_user += sum(1 for b in blocks if b == goal)
                    out["model_requests"].append({"transport": "http", "path": path_noq, "status": r.get("status"),
                                                  "model": j.get("model"), "n_messages": len(j.get("messages", [])),
                                                  "goal_exact_user_blocks": ex_user,
                                                  "goal_substring_in_body": body.count(goal) if goal else 0,
                                                  "tools": len(j.get("tools") or [])})
        elif k == "ws_msg" and harness == "codex" and r.get("dir") == "c2s" and r.get("text"):
            try:
                j = json.loads(r["text"])
            except Exception:
                continue
            if j.get("type") == "response.create":
                ex_user = 0
                for it in j.get("input", []) or []:
                    if isinstance(it, dict) and it.get("role") == "user":
                        for c in it.get("content", []) or []:
                            if isinstance(c, dict) and c.get("text") == goal:
                                ex_user += 1
                out["model_requests"].append({"transport": "ws", "type": "response.create", "model": j.get("model"),
                                              "n_input_items": len(j.get("input", []) or []),
                                              "has_previous_response_id": bool(j.get("previous_response_id")),
                                              "goal_exact_user_items": ex_user,
                                              "goal_substring_in_msg": r["text"].count(goal) if goal else 0})
    return out


def stdout_summary(h, path):
    raw = open(path, encoding="utf-8", errors="replace").read()
    if h == "claude":
        try:
            j = json.loads(raw)
        except Exception:
            return {"parse": "not-json", "len": len(raw), "head": excerpt(raw, 300)}
        return {"type": j.get("type"), "subtype": j.get("subtype"), "is_error": j.get("is_error"),
                "num_turns": j.get("num_turns"), "session_id": j.get("session_id"),
                "result": excerpt(j.get("result") or "", 200),
                "model_usage_keys": sorted((j.get("modelUsage") or {}).keys()),
                "permission_denials": [{"tool": d.get("tool_name"), "input": excerpt(d.get("tool_input"), 200)}
                                       for d in (j.get("permission_denials") or [])],
                "stop_reason": j.get("stop_reason"), "duration_ms": j.get("duration_ms")}
    events, thread, items, final, errs = [], None, [], None, []
    for l in raw.splitlines():
        try:
            e = json.loads(l)
        except Exception:
            continue
        events.append(e.get("type"))
        if e.get("type") == "thread.started":
            thread = e.get("thread_id")
        if e.get("type") in ("item.completed",):
            it = e.get("item") or {}
            d = {"type": it.get("type")}
            if it.get("type") == "command_execution":
                d.update(command=excerpt(it.get("command"), 300), exit_code=it.get("exit_code"), status=it.get("status"))
            elif it.get("type") == "file_change":
                d.update(changes=it.get("changes"), status=it.get("status"))
            elif it.get("type") == "agent_message":
                final = excerpt(it.get("text") or "", 200)
            items.append(d)
        if e.get("type") in ("error", "turn.failed"):
            errs.append(excerpt(e, 300))
    return {"events": events, "thread_id": thread, "items": items, "final_agent_message": final,
            "errors": errs, "len": len(raw), "head_if_not_json": excerpt(raw, 300) if not events else None}


def run(a):
    goal = open(a.goal_file, "rb").read().decode("utf-8") if a.goal_file else ""
    run_dir = os.path.join(a.out, a.run_id)
    os.makedirs(run_dir, exist_ok=False)
    ws = os.path.join(a.ws_root, a.run_id, "ws")
    outside = os.path.join(a.ws_root, a.run_id, "outside")
    os.makedirs(ws)
    os.makedirs(outside)
    assert os.listdir(ws) == [] and os.listdir(outside) == []
    sid = str(uuid.uuid4())
    scoped = None
    if (a.scoped or a.mcp_off or a.home_link) and a.harness != "claude":
        sys.exit("--scoped/--mcp-off/--home-link are Claude measurements")
    if a.scoped or a.mcp_off:
        scoped = {"mcp_off": True}
    if a.scoped:
        for k, d in (("zdotdir", "zdotdir"), ("xdg_state", "xdg-state"), ("xdg_cache", "xdg-cache")):
            scoped[k] = os.path.join(a.ws_root, a.run_id, d)
            os.makedirs(scoped[k])
    argv, henv = build(a.harness, a.variant, goal, ws, sid, scoped=scoped,
                       trust_override=a.codex_trust_override, allow_tools=a.allow_tool or ())
    env = base_env()
    env.update(henv)
    fake_home = None
    if a.fake_home or a.home_link:
        fake_home = os.path.join(a.ws_root, a.run_id, "home")
        os.makedirs(fake_home)
        env["HOME"] = fake_home
    if a.home_link:
        # Claude's subscription login lives in the login keychain, which it finds through
        # $HOME/Library/Keychains. Link only that directory into the per-run home.
        os.makedirs(os.path.join(fake_home, "Library"))
        os.symlink(os.path.expanduser("~/Library/Keychains"), os.path.join(fake_home, "Library", "Keychains"))
    probe = None
    probe_dir = os.path.join(run_dir, "probe")
    if a.proxy:
        os.makedirs(probe_dir)
        probe = subprocess.Popen([WPROBE, probe_dir], stdout=subprocess.PIPE, stderr=open(os.path.join(probe_dir, "proxy.err"), "wb"))
        line = probe.stdout.readline().decode().strip()
        port = int(line.split()[1])
        purl = "http://127.0.0.1:%d" % port
        env.update({"HTTPS_PROXY": purl, "https_proxy": purl, "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"})
        env["CODEX_CA_CERTIFICATE" if a.harness == "codex" else "NODE_EXTRA_CA_CERTS"] = os.path.join(probe_dir, "ca.pem")
    iso_home = CODEX_HOME if a.harness == "codex" else CLAUDE_HOME
    snap_before = listing(APPWIDE_ROOTS) if a.appwide else None
    iso_before = listing((iso_home,)) if a.appwide else None
    lock_dirs = [os.path.expanduser(d) for d in LOCK_DIRS] + ([os.path.join(scoped["xdg_state"], "claude", "locks")] if scoped and scoped.get("xdg_state") else []) \
        + ([os.path.join(fake_home, ".local", "state", "claude", "locks")] if fake_home else [])
    lock_before = {d: set(os.listdir(d)) if os.path.isdir(d) else set() for d in lock_dirs}
    lock_seen = {d: set() for d in lock_dirs}
    snaps_dir = os.path.join(CLAUDE_HOME, "shell-snapshots")
    snaps_before = set(os.listdir(snaps_dir)) if a.harness == "claude" and os.path.isdir(snaps_dir) else set()
    cfg_before = codex_config_state() if a.harness == "codex" else None
    snaps_seen = {}
    before = native_candidates(a.harness, sid)
    first_seen = {}
    stop = threading.Event()

    def poll():
        while not stop.is_set():
            for f in native_candidates(a.harness, sid) - before:
                first_seen.setdefault(f, time.time())
            for d in lock_dirs:
                if os.path.isdir(d):
                    lock_seen[d].update(set(os.listdir(d)) - lock_before[d])
            if a.harness == "claude":
                snaps_seen.update(claude_snapshot_check(snaps_before, snaps_seen))
            time.sleep(0.05)

    stdin = subprocess.DEVNULL
    stdin_bytes = None
    if a.variant == "stdinpipe":
        stdin = subprocess.PIPE
        stdin_bytes = ("T02-STDIN-SENTINEL " + a.run_id + "\n").encode()
    so, se = open(os.path.join(run_dir, "stdout.txt"), "wb"), open(os.path.join(run_dir, "stderr.txt"), "wb")
    th = threading.Thread(target=poll, daemon=True)
    th.start()
    t0, m0 = time.time(), time.monotonic()
    spawn_error = None
    rc = None
    timed_out = False
    pid = None
    try:
        p = subprocess.Popen(argv, cwd=ws, env=env, stdin=stdin, stdout=so, stderr=se)
        pid = p.pid
        if stdin_bytes is not None:
            p.stdin.write(stdin_bytes)
            p.stdin.close()
        try:
            rc = p.wait(timeout=a.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            p.terminate()
            try:
                rc = p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
                rc = p.wait()
    except (OSError, ValueError) as e:
        spawn_error = "%s: %s" % (type(e).__name__, getattr(e, "strerror", None) or str(e)[:120])
    dur = time.monotonic() - m0
    time.sleep(0.5)
    stop.set()
    th.join()
    so.close()
    se.close()
    if probe:
        time.sleep(1.0)
        probe.terminate()
        probe.wait(timeout=10)
    tags = [a.run_id, sid]
    for f in native_candidates(a.harness, sid) - before:
        m_ = re.search(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$", f)
        if m_:
            tags.append(m_.group(1))
    side_effects = {}
    if a.appwide:
        side_effects["appwide_changes"] = diff_listing(snap_before, listing(APPWIDE_ROOTS), tags)
        side_effects["isolated_home_changes"] = diff_listing(iso_before, listing((iso_home,)), tags)
    side_effects["lock_names_seen_during_run"] = {("~" + d[len(os.path.expanduser("~")):] if d.startswith(os.path.expanduser("~")) else d): sorted(v)
                                                  for d, v in lock_seen.items()}
    if scoped and len(scoped) > 1:
        side_effects["scoped_dirs_after"] = {k: sorted(os.path.relpath(os.path.join(dd, n), v) for dd, ds, fs in os.walk(v) for n in ds + fs)
                                             for k, v in scoped.items() if k != "mcp_off"}
    if a.harness == "claude":
        final = claude_snapshot_check(snaps_before, snaps_seen)
        present = set(os.listdir(snaps_dir)) if os.path.isdir(snaps_dir) else set()
        for n, v in final.items():
            v["still_present_after_exit"] = n in present
        side_effects["new_shell_snapshots"] = sorted(final.values(), key=lambda v: v["name"])
    else:
        cfg_after = codex_config_state()
        side_effects["codex_config_toml"] = {"sha256_before": cfg_before[0], "sha256_after": cfg_after[0],
                                             "projects_added": sorted(set(cfg_after[1] or []) - set(cfg_before[1] or [])),
                                             "projects_removed": sorted(set(cfg_before[1] or []) - set(cfg_after[1] or []))}
    # ---------------------------------------------------------------- output oracle
    ws_entries = sorted(os.listdir(ws))
    outside_entries = sorted(os.listdir(outside))
    oracle = {"ws_entries": ws_entries, "outside_entries": outside_entries}
    if fake_home:
        oracle["fake_home_entries_after"] = sorted(
            os.path.relpath(os.path.join(d, f), fake_home) for d, ds, fs in os.walk(fake_home) for f in fs + ds)
        if len(oracle["fake_home_entries_after"]) > 200:
            oracle["fake_home_entries_after"] = oracle["fake_home_entries_after"][:200] + ["...(+%d)" % (len(oracle["fake_home_entries_after"]) - 200)]
    if a.expect_file:
        fp = os.path.join(ws, a.expect_file)
        if os.path.exists(fp):
            st = os.stat(fp)
            b = open(fp, "rb").read()
            exp = a.expect_content.encode()
            oracle.update(exists=True, size=len(b), sha256=sha(b), bytes_repr=repr(b[:200]),
                          exact=(b == exp), exact_plus_lf=(b == exp + b"\n"),
                          birth_after_spawn=getattr(st, "st_birthtime", st.st_mtime) >= t0,
                          birth_iso=now_iso(getattr(st, "st_birthtime", st.st_mtime)))
        else:
            oracle.update(exists=False)
    # ---------------------------------------------------------------- native store
    new = sorted(native_candidates(a.harness, sid) - before)
    native = []
    for f in new:
        rel = os.path.relpath(f, CODEX_HOME if a.harness == "codex" else CLAUDE_HOME)
        dst = os.path.join(run_dir, "native-" + os.path.basename(f))
        with open(f, "rb") as src, open(dst, "wb") as d:
            d.write(src.read())
        an = analyse_codex(f, goal, ws) if a.harness == "codex" else analyse_claude(f, goal, ws)
        raw = open(f, encoding="utf-8", errors="replace").read()
        an["real_home_refs"] = {k: raw.count(k) for k in (os.path.expanduser("~/.agents"), os.path.expanduser("~/.claude/"),
                                                          os.path.expanduser("~/.codex/"), os.path.expanduser("~/.zshrc"))}
        native.append({"path_rel_home": rel, "first_seen_after_spawn_s": round(first_seen.get(f, time.time()) - t0, 3),
                       "size": os.path.getsize(f), "sha256": sha(open(f, "rb").read()), "analysis": an})
    # scope check: the session id must not appear as a file under the app-wide default store
    appwide = {}
    if a.harness == "claude":
        appwide["claude_default_projects_hits"] = len(glob.glob(os.path.expanduser("~/.claude/projects/*/" + sid + "*")))
    else:
        tids = [n["analysis"]["session_meta"]["id"] for n in native if n["analysis"].get("session_meta")]
        appwide["codex_default_sessions_hits"] = sum(len(glob.glob(os.path.expanduser("~/.codex/sessions/**/*" + t + "*"), recursive=True)) for t in tids)
    meta = {
        "run_id": a.run_id, "harness": a.harness, "variant": a.variant, "proxy": bool(a.proxy),
        "started_utc": now_iso(t0), "duration_s": round(dur, 3), "exit_code": rc, "timed_out": timed_out, "timeout_s": a.timeout,
        "spawn_error": spawn_error, "pid": pid,
        "executable": argv[0], "executable_realpath": os.path.realpath(argv[0]),
        "executable_sha256": sha(open(os.path.realpath(argv[0]), "rb").read()),
        "argv_template": [("<GOAL>" if (x == goal and goal) else x) for x in argv],
        "argc": len(argv), "cwd": ws,
        "stdin": "pipe:" + repr(stdin_bytes.decode()) if stdin_bytes else "/dev/null",
        "env_names": sorted(env.keys()),
        "home": ("per-run dir with Library/Keychains linked to the real one" if a.home_link else "per-run empty dir") if fake_home else "real user HOME",
        "env_values_nonsecret": {k: env[k] for k in sorted(env) if k in (
            "CODEX_HOME", "CLAUDE_CONFIG_DIR", "PATH", "DISABLE_AUTOUPDATER", "NO_PROXY", "no_proxy",
            "HTTPS_PROXY", "https_proxy", "CODEX_CA_CERTIFICATE", "NODE_EXTRA_CA_CERTS", "SHELL",
            "ENABLE_CLAUDEAI_MCP_SERVERS", "CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS", "ZDOTDIR", "XDG_STATE_HOME", "XDG_CACHE_HOME")
            or (k == "HOME" and fake_home)},
        "exec_bytes": dict(zip(("strings", "strings_plus_pointers"), exec_bytes(argv, env))),
        "goal": {"bytes": len(goal.encode()), "sha256": sha(goal.encode()), "lines": goal.count("\n") + 1 if goal else 0,
                 "starts_with_dash": goal.startswith("-"), "has_tab": "\t" in goal, "has_double_space": "  " in goal,
                 "non_ascii": any(ord(c) > 127 for c in goal),
                 "text": goal if len(goal) <= 4096 else None},
        "session_id_preassigned": sid if a.harness == "claude" else None,
        "stdout": stdout_summary(a.harness, os.path.join(run_dir, "stdout.txt")),
        "stderr_bytes": os.path.getsize(os.path.join(run_dir, "stderr.txt")),
        "oracle": oracle, "native": native, "appwide_scope": appwide, "side_effects": side_effects,
        "scoped": sorted(k for k in (scoped or {})), "home_link": bool(a.home_link), "codex_trust_override": bool(a.codex_trust_override), "allow_tools": a.allow_tool or [],
    }
    if a.proxy:
        meta["wire"] = analyse_wire(os.path.join(probe_dir, "wire.jsonl"), goal, a.harness)
        meta["probe"] = {"binary_sha256": sha(open(WPROBE, "rb").read()), "listen": "127.0.0.1 (ephemeral port)"}
    json.dump(meta, open(os.path.join(run_dir, "meta.json"), "w"), indent=1, ensure_ascii=False)
    print(json.dumps({k: meta[k] for k in ("run_id", "exit_code", "duration_s", "timed_out", "spawn_error")}))


def argmax(a):
    """Find the largest single argv element that execve accepts with a recipe-shaped env."""
    res = {"getconf_ARG_MAX": int(subprocess.check_output(["getconf", "ARG_MAX"]).strip())}
    ws = "/private/tmp/x" * 10
    for h in ("codex", "claude"):
        argv, henv = build(h, "positive", "", ws, str(uuid.uuid4()))
        env = base_env()
        env.update(henv)
        env.update({"HTTPS_PROXY": "http://127.0.0.1:65535", "https_proxy": "http://127.0.0.1:65535",
                    "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
                    ("CODEX_CA_CERTIFICATE" if h == "codex" else "NODE_EXTRA_CA_CERTS"): "/" + "p" * 200 + "/ca.pem"})

        def ok(n):
            v = argv[:-1] + ["G" * n]
            try:
                subprocess.run(v, executable="/usr/bin/true", env=env, check=True)
                return True
            except OSError as e:
                if e.errno == errno.E2BIG:
                    return False
                raise
        lo, hi = 0, res["getconf_ARG_MAX"] + 4096
        assert ok(lo) and not ok(hi)
        while hi - lo > 1:
            mid = (lo + hi) // 2
            (lo, hi) = (mid, hi) if ok(mid) else (lo, mid)
        s, sp = exec_bytes(argv[:-1] + ["G" * 65536], env)
        s0, sp0 = exec_bytes(argv[:-1] + [""], env)
        res[h] = {"recipe_env_names": sorted(env.keys()), "env_plus_argv_without_goal_bytes": sp0,
                  "with_64KiB_goal_strings": s, "with_64KiB_goal_strings_plus_pointers": sp,
                  "exec_64KiB_ok": ok(65536), "max_single_goal_arg_accepted": lo,
                  "headroom_after_64KiB_goal": lo - 65536}
    try:
        subprocess.run(["/usr/bin/true", "a\0b"])
        res["nul_in_arg"] = "accepted?!"
    except ValueError as e:
        res["nul_in_arg"] = "refused before spawn by the launcher: ValueError: " + str(e)
    json.dump(res, open(a.out, "w"), indent=1)
    print(json.dumps(res, indent=1))


def main():
    ap = argparse.ArgumentParser()
    sp = ap.add_subparsers(dest="cmd", required=True)
    r = sp.add_parser("run")
    r.add_argument("--harness", choices=("codex", "claude"), required=True)
    r.add_argument("--variant", required=True)
    r.add_argument("--run-id", required=True)
    r.add_argument("--goal-file")
    r.add_argument("--out", required=True)
    r.add_argument("--ws-root", required=True)
    r.add_argument("--proxy", action="store_true")
    r.add_argument("--fake-home", action="store_true")
    r.add_argument("--wprobe")
    r.add_argument("--scoped", action="store_true", help="Claude: --mcp-off plus ZDOTDIR/XDG_STATE_HOME/XDG_CACHE_HOME per-run dirs")
    r.add_argument("--mcp-off", action="store_true", help="Claude: strict empty MCP config and the two MCP env switches")
    r.add_argument("--home-link", action="store_true", help="Claude: per-run HOME holding only Library/Keychains -> real")
    r.add_argument("--codex-trust-override", action="store_true")
    r.add_argument("--allow-tool", action="append")
    r.add_argument("--appwide", action="store_true")
    r.add_argument("--timeout", type=int, default=TIMEOUT)
    r.add_argument("--expect-file")
    r.add_argument("--expect-content")
    m = sp.add_parser("argmax")
    m.add_argument("--out", required=True)
    for x in (r, m):
        x.add_argument("--harnesses")
        x.add_argument("--claude-bin")
    a = ap.parse_args()
    configure(a)
    run(a) if a.cmd == "run" else argmax(a)


if __name__ == "__main__":
    main()
