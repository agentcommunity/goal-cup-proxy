import json,glob,collections,os,re
F=json.load(open("/vercel/obs/final.json"))
SESS="/vercel/.config/goalcup/capture-homes/codex-cli/sessions"
rb=collections.Counter(); per=[]; tot=0; spawn_root=0
for fn in sorted(glob.glob(SESS+"/*/*/*/rollout-*.jsonl")):
    if os.path.getmtime(fn) < 1793285658: continue   # 14:54:18Z
    sid=parent=None; tc=0; first=last=None; models=collections.Counter(); tcb=collections.Counter()
    for l in open(fn,errors="replace"):
        try: r=json.loads(l)
        except: continue
        p=r.get("payload") or {}; ts=r.get("timestamp")
        if r.get("type")=="session_meta" and sid is None:
            sid=p.get("id"); s=p.get("source"); first=ts
            if isinstance(s,dict): parent=((s.get("subagent") or {}).get("thread_spawn") or {})
        last=ts or last
        if r.get("type")=="turn_context": models[(p.get("model"),p.get("effort"))]+=1
        if r.get("type")=="event_msg" and p.get("type")=="token_count" and p.get("info"):
            tc+=1; rb[ts[:15]+"0"]+=1; tcb[ts[:15]+"0"]+=1
    tot+=tc
    per.append((sid, (parent or {}).get("parent_thread_id"), (parent or {}).get("depth"), (parent or {}).get("agent_path"), first, last, tc, dict(models), F["create_by_session"].get(sid,0)))
print("rollout token_count total",tot,"files",len(per))
wb=F["calls_by_10min"]
print("10-min bucket: harness token_counts vs wire calls")
for k in sorted(set(rb)|set(wb)): print(" ",k, rb.get(k,0), wb.get(k,0))
print("per thread: sid parent depth path first last harness_tc wire_ws_gen models")
for x in per: print(" ",x[0][-12:], (x[1] or "-")[-12:], x[2], x[3], x[4], x[5], x[6], x[8], x[7])
print("wire-only threads:", [k for k in F["create_by_session"] if k not in {p[0] for p in per}])
print("native streams opened in trace:", F["counts"].get("native_streams"))
