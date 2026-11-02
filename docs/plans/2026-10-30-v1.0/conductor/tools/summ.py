# Summarize pad-out run folders: checks, why, status, upload, threads, gaps. Names/numbers only.
import json, sys, glob, os
def g(d, *p):
    for k in p:
        d = d.get(k) if isinstance(d, dict) else None
    return d
for d in sys.argv[1:]:
    print("##", d)
    try: row = json.load(open(d + "/row.json"))
    except Exception as e: print("no row", e); continue
    print("result", row.get("result"), "checks", row.get("checks"))
    for w in row.get("why", []) if isinstance(row.get("why"), list) else [row.get("why")]: print("  why", w)
    for k in ("usd", "spend", "threads", "gaps", "exec", "variant", "faults"):
        if k in row: print(" ", k, json.dumps(row[k])[:400])
    try:
        s = json.load(open(d + "/status.json")); print("status mode", s.get("mode"), "model_requests", g(s, "written", "model_requests"), "seal", g(s, "seal", "reason"), "off", json.dumps(s.get("off"))[:200])
    except Exception as e: print("status", e)
    for f in sorted(glob.glob(d + "/verify/*.json")): print("  verify", os.path.basename(f), os.path.getsize(f))
