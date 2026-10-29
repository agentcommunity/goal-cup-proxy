import json,sys
L=[json.loads(l) for l in open("/vercel/obs/log.jsonl")] if len(sys.argv)<2 else [json.load(open("/vercel/obs/current.json"))]
s=L[-1]; c=s["counts"]; u=s["usage"]
unc=u["input_tokens"]-u["cached_tokens"]
cost=unc*0.10/1e6+u["cached_tokens"]*0.01/1e6+u["output_tokens"]*0.50/1e6
pr={k:[(p["rss_mb"],p["pcpu"],p["cpu_s"],p["fds"]) for p in v] for k,v in s["procs"].items()}
r=s.get("rollouts") or {}
print(f'{s["at"]} t+{s["min_since_prompt"]}m ws_calls={sum(v for k,v in s["models"].items() if "gen" in k and "nogen" not in k)} http_posts={c.get("http_post_responses",0)} nogen={sum(v for k,v in s["models"].items() if "nogen" in k)} completed={c.get("server:response.completed",0)} failed={c.get("server:response.failed",0)} err={c.get("server:error",0)} last_call={s["last_call"]} trace={s["trace_bytes"]/1e6:.0f}MB up_chunks={s["upload_chunks"]} up_comp={s["upload_compressed_bytes"]/1e6:.1f}MB acks={s["acks_lines"]} tok_in={u["input_tokens"]} cached={u["cached_tokens"]} out={u["output_tokens"]} cost=${cost:.2f} disk_avail={s["disk_avail"]/1e9:.1f}GB open_streams={s["open_streams"]} other={s["other"]} ends={s["end_reasons"]} procs={pr} tail_s={c.get("tail_seconds_last")}')
print("models:",s["models"])
print("rollouts:",{k:v for k,v in r.items() if k!="sessions"})
print("wire threads:",len(s["create_by_session"]),"hosts:",s["hosts"])
