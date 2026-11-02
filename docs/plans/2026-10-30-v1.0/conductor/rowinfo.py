# Names/numbers only from one run folder: checks, why, facts, verifier non-pass invariants, hosts, upload.
import json, sys
d = sys.argv[1]; r = json.load(open(d + "/row.json"))
print("result", r["result"], r["checks"], "faults", r.get("faults"), "minutes", r.get("minutes"), "harness", r.get("harness_version"))
for w in (r["why"] if isinstance(r["why"], list) else [r["why"]]): print(" why", str(w)[:500])
f = r["facts"]
for k in ("models", "spend_usd_est", "priced_high", "provider_errors", "subagents", "threads", "wire_requests", "harness_calls", "gaps", "secret_hits", "tokens", "over_cap", "history"):
    print(" ", k, json.dumps(f.get(k))[:300])
s = json.load(open(d + "/status.json"))
print("status", s["mode"], "model_requests", s["written"]["model_requests"], "seal", s["seal"]["reason"], "hosts", s["written"].get("hosts"), "gaps", s["written"]["gaps"], "lost", s["written"]["lost_bytes"])
print("tunnels", {h: v.get("conns") for h, v in (s["written"].get("tunnel_hosts") or {}).items()})
print("exits", open(d + "/verify/exits.json").read().strip())
for fn in ("check-local", "check-server"):
    v = json.load(open(d + "/verify/%s.json" % fn))
    for e in v["entries"]:
        print(fn, "exit", v["exit"], "result", v["result"], "hosts", e["counts"]["hosts"], "threads", len(e["threads"]), "prov", e["provider_errors"])
        for c in e["checks"]:
            if c["result"] != "pass": print("  ", c["id"], c["result"], c["why"][:300])
last = [l for l in open(d + "/watch/upload.jsonl")][-1]; u = json.loads(last)["upload"]
print("upload", u["state"], "confirmed", u["confirmed"]["raw"], "seq", u["confirmed"]["seq"])
