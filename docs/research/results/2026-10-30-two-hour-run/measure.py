# Observer for the 2-hour run. Tails the chain incrementally; every 60 s writes ~/obs/current.json,
# every 15 min after the prompt appends a line to ~/obs/log.jsonl. Never prints secrets.
import json, base64, os, glob, time, re, collections, subprocess, sys, datetime
HOME = os.path.expanduser("~")
OBS = HOME + "/obs"
T = sys.argv[1]
SESS = HOME + "/.config/goalcup/capture-homes/codex-cli/sessions"
RUNSTART = sys.argv[2]           # e.g. 2026-10-29T14:54:18Z
PROMPT = datetime.datetime.fromisoformat(open(OBS + "/prompt-sent-at").read().strip().replace("Z", "+00:00"))
src = {}            # (producer,stream) -> (source, host, path, label)
buf = {}            # stream -> bytearray for ws_client / interesting ws_server
st = collections.Counter()
models = collections.Counter()
hosts = collections.Counter()
endreasons = collections.Counter()
other_kinds = collections.Counter()
usage = collections.Counter()
calls_by_min = collections.Counter()
completed_by_session = collections.Counter()
create_by_session = collections.Counter()
first_call = None
last_call = [None]
offsets = {}
def handle_msg(source, data, wall):
    global first_call
    try:
        j = json.loads(data)
    except Exception:
        st[source + ":unparsed"] += 1
        return
    t = j.get("type")
    if source == "ws_client":
        st["client:" + str(t)] += 1
        if t == "response.create":
            gen = j.get("generate", True)
            eff = (j.get("reasoning") or {}).get("effort")
            models[("create", j.get("model"), eff, "gen" if gen is not False else "nogen")] += 1
            m = re.search(r'thread_id\\?":\\?"([0-9a-f-]+)', json.dumps(j.get("client_metadata") or {}))
            sid = m.group(1) if m else (j.get("client_metadata") or {}).get("session_id")
            if gen is not False:
                create_by_session[sid] += 1
                calls_by_min[wall[:15] + "0"] += 1
                if first_call is None:
                    first_call = wall
                st["last_call_wall"] = 0
                last_call[0] = max(last_call[0] or "", wall)
    else:
        if t in ("response.completed", "response.failed", "response.incomplete", "error"):
            st["server:" + t] += 1
            r = j.get("response") or {}
            u = r.get("usage") or {}
            models[("server", r.get("model"), (r.get("reasoning") or {}).get("effort"), t)] += 1
            if t == "response.completed":
                completed_by_session[r.get("prompt_cache_key")] += 1
            for k in ("input_tokens", "output_tokens", "total_tokens"):
                usage[k] += u.get(k) or 0
            usage["cached_tokens"] += (u.get("input_tokens_details") or {}).get("cached_tokens") or 0
            usage["reasoning_tokens"] += (u.get("output_tokens_details") or {}).get("reasoning_tokens") or 0
            if t == "error":
                st["server_error_sample"] = 0
                os.makedirs(OBS, exist_ok=True)
                with open(OBS + "/errors.jsonl", "a") as g:
                    g.write(json.dumps({"wall": wall, "error": j.get("error"), "status": j.get("status")})[:600] + "\n")
def chain_files():
    return sorted(glob.glob(T + "/chain*.jsonl"))
def tail():
    for fn in chain_files():
        try:
            f = open(fn, "rb")
        except FileNotFoundError:
            continue
        ino = os.fstat(f.fileno()).st_ino   # a rotated chain file keeps its inode under a new name
        off = offsets.get(ino, 0)
        f.seek(off)
        while True:
            line = f.readline()
            if not line or not line.endswith(b"\n"):
                break
            off += len(line)
            st["records"] += 1
            r = json.loads(line)
            b = r["body"]
            if b.get("kind") != "capture":
                other_kinds["body:" + str(b.get("kind"))] += 1
                continue
            rec = b["content"]["record"]
            k = rec["kind"]
            if k == "open":
                md = rec.get("metadata") or {}
                key = (rec["stream"]["producer"], rec["stream"]["stream"])
                s = rec.get("source")
                src[key] = s
                st["open:" + str(s)] += 1
                if md.get("host"):
                    hosts[(md.get("host"), (md.get("label") or "")[:40] if s in ("wire_request",) else s)] += 1
                if s == "wire_request" and md.get("protocol_hint") == "openai-responses" and (md.get("label") or "").startswith("POST /v1/responses"):
                    st["http_post_responses"] += 1
                    w = rec["observed"]["wall"]
                    calls_by_min[w[:15] + "0"] += 1
                    last_call[0] = max(last_call[0] or "", w)
                if s == "native":
                    st["native_streams"] += 1
            elif k == "segment":
                key = (rec["stream"]["producer"], rec["stream"]["stream"])
                s = src.get(key)
                d = rec.get("data_b64")
                n = rec.get("source_len") or 0
                st["seg_bytes:" + str(s)] += n
                if s in ("ws_client", "ws_server") and d is not None:
                    raw = base64.b64decode(d)
                    if key not in buf:
                        if s == "ws_server" and not re.match(rb'\{"type":"(response\.(completed|failed|incomplete)|error)"', raw):
                            buf[key] = None
                        else:
                            buf[key] = bytearray()
                    if buf[key] is not None:
                        buf[key] += raw
            elif k == "end":
                key = (rec["stream"]["producer"], rec["stream"]["stream"])
                endreasons[rec.get("reason")] += 1
                s = src.pop(key, None)
                bb = buf.pop(key, None)
                if bb:
                    handle_msg(s, bytes(bb), rec["observed"]["wall"])
            else:
                other_kinds[k] += 1
                if k not in ("connection", "producer_closed", "launch"):
                    with open(OBS + "/odd-records.jsonl", "a") as g:
                        g.write(json.dumps(rec)[:800] + "\n")
        offsets[ino] = off
        f.close()
def rollouts():
    out = {"files": 0, "subagent_files": 0, "token_count_events": 0, "spawn_agent_calls": 0, "turn_models": collections.Counter(), "with_parent": 0, "bytes": 0, "sessions": {}}
    for fn in glob.glob(SESS + "/*/*/*/rollout-*.jsonl"):
        if os.path.getmtime(fn) < datetime.datetime.fromisoformat(RUNSTART.replace("Z", "+00:00")).timestamp():
            continue
        out["files"] += 1
        out["bytes"] += os.path.getsize(fn)
        sid = None; parent = None; tc = 0; first = last = None; sp = 0
        for line in open(fn, errors="replace"):
            try:
                r = json.loads(line)
            except Exception:
                continue
            ts = r.get("timestamp"); first = first or ts; last = ts or last
            p = r.get("payload") or {}
            ty = r.get("type")
            if ty == "session_meta" and sid is None:
                sid = p.get("id")
                srcp = p.get("source")
                if isinstance(srcp, dict):
                    parent = json.dumps(srcp)[:300]
            elif ty == "turn_context":
                out["turn_models"][(p.get("model"), p.get("effort") or (p.get("reasoning") or {}).get("effort"))] += 1
            elif ty == "event_msg" and p.get("type") == "token_count" and p.get("info"):
                tc += 1
            elif ty == "response_item" and p.get("type") in ("function_call", "custom_tool_call"):
                if parent is None:
                    sp += (p.get("name") == "spawn_agent") + str(p.get("input") or "").count("spawn_agent(") + str(p.get("arguments") or "").count("spawn_agent(")
        out["token_count_events"] += tc
        out["spawn_agent_calls"] += sp
        if parent:
            out["subagent_files"] += 1
            pm = re.search(r'parent_thread_id": "([0-9a-f-]+)', parent)
            if pm: out.setdefault("children_by_parent", collections.Counter())[pm.group(1)] += 1
        out["sessions"][sid] = {"file": os.path.basename(fn), "parent": parent, "token_counts": tc, "first": first, "last": last, "spawn": sp}
    out["turn_models"] = {str(k): v for k, v in out["turn_models"].items()}
    return out
def procs():
    res = {}
    for line in subprocess.run(["ps", "-eo", "pid,rss,pcpu,cputimes,etimes,comm,args", "--no-headers"], capture_output=True, text=True).stdout.splitlines():
        p = line.split(None, 6)
        if len(p) < 7: continue
        a = p[6]
        name = None
        if "goalcup-daemon" in a: name = "daemon"
        elif "goalcup start" in a: name = "goalcup_start"
        elif "goalcup mcp" in a: name = "goalcup_mcp"
        elif a.startswith("/usr/local/lib/node_modules/@openai/codex/node_modules") and "code-mode-host" not in a: name = "codex"
        if name:
            try: fds = len(os.listdir("/proc/%s/fd" % p[0]))
            except Exception: fds = -1
            try: thr = int(open("/proc/%s/status" % p[0]).read().split("Threads:")[1].split()[0])
            except Exception: thr = -1
            res.setdefault(name, []).append({"pid": int(p[0]), "rss_mb": round(int(p[1]) / 1024, 1), "pcpu": float(p[2]), "cpu_s": int(p[3]), "fds": fds, "threads": thr})
    return res
def summary(full):
    tb = sum(os.path.getsize(f) for f in chain_files() if os.path.exists(f))
    acks = 0
    try: acks = sum(1 for _ in open(T + "/acks.jsonl"))
    except Exception: pass
    up = [l for l in open(HOME + "/.config/goalcup/runs/" + os.environ.get("RUN", "") + "/host.log") if "upload" in l] if os.environ.get("RUN") else []
    comp = 0
    for l in up:
        m = re.search(r"(\d+) bytes compressed", l)
        if m: comp += int(m.group(1))
    du = subprocess.run(["df", "-B1", "--output=used,avail", HOME], capture_output=True, text=True).stdout.split()
    s = {"at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
         "min_since_prompt": round((datetime.datetime.now(datetime.timezone.utc) - PROMPT).total_seconds() / 60, 1),
         "first_call": first_call, "last_call": last_call[0], "trace_bytes": tb, "acks_lines": acks, "upload_chunks": len(up), "upload_compressed_bytes": comp,
         "disk_used": int(du[2]) if len(du) > 3 else None, "disk_avail": int(du[3]) if len(du) > 3 else None,
         "counts": dict(st), "models": {str(k): v for k, v in models.items()}, "usage": dict(usage),
         "end_reasons": dict(endreasons), "other": dict(other_kinds), "hosts": {str(k): v for k, v in hosts.items()},
         "calls_by_10min": dict(calls_by_min), "open_streams": len(src), "procs": procs(),
         "create_by_session": dict(create_by_session), "completed_by_session": dict(completed_by_session)}
    if full:
        s["rollouts"] = rollouts()
    return s
next_tick = PROMPT + datetime.timedelta(minutes=15)
while True:
    t0 = time.time()
    tail()
    st["tail_seconds_last"] = round(time.time() - t0, 2)
    now = datetime.datetime.now(datetime.timezone.utc)
    full = now >= next_tick or os.path.exists(OBS + "/want-full")
    s = summary(full)
    with open(OBS + "/current.json.tmp", "w") as f: json.dump(s, f)
    os.replace(OBS + "/current.json.tmp", OBS + "/current.json")
    if full:
        if os.path.exists(OBS + "/want-full"): os.remove(OBS + "/want-full")
        else:
            with open(OBS + "/log.jsonl", "a") as f: f.write(json.dumps(s) + "\n")
            next_tick += datetime.timedelta(minutes=15)
        with open(OBS + "/full.json", "w") as f: json.dump(s, f)
    time.sleep(max(5, 60 - (time.time() - t0)))
