#!/usr/bin/env python3
"""Build committed, whitelisted per-run metadata from raw meta.json files (raw stays ignored).

Run from a checkout: paths resolve from this file (docs/qualification/m2/launch-recipes/tools/).
Side-effect path lists keep names only; UUIDs and long hex ids that are not this run's own
identities (account/org ids appear in some app-wide directory names) become <uuid>/<hex>."""
import glob, json, os, re, sys

DST = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
W = os.path.abspath(os.path.join(DST, "..", "..", "..", ".."))
RAW = [W + "/spikes/codex-cli/out/m2-t02", W + "/spikes/claude-code-cli/out/m2-t02"]
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
HEX = re.compile(r"(?<![0-9a-f])[0-9a-f]{24,}(?![0-9a-f])")


def scrub_path(p, keep):
    p = UUID.sub(lambda m: m.group(0) if m.group(0) in keep else "<uuid>", p)
    return HEX.sub(lambda m: m.group(0) if m.group(0) in keep else "<hex>", p)


def side_effects_view(se, keep):
    out = {}
    for k, v in se.items():
        if k in ("appwide_changes", "isolated_home_changes"):
            out[k] = [dict(r, path=scrub_path(r["path"], keep)) for r in v]
        else:
            out[k] = v
    return out


def cut(s, n):
    if s is None:
        return None
    s = s if isinstance(s, str) else json.dumps(s, ensure_ascii=False)
    return s if len(s) <= n else s[:n] + "...(+%d)" % (len(s) - n)


def native_view(n, variant):
    a = n["analysis"]
    keep_outputs = variant in ("positive", "approval", "big")
    v = {"path_rel_home": n["path_rel_home"], "first_seen_after_spawn_s": n["first_seen_after_spawn_s"],
         "size": n["size"], "sha256": n["sha256"], "record_types": a["record_types"],
         "user_messages": a["user_messages"], "goal_exact_by_record": a["goal_exact_by_record"],
         "goal_exact_total": a["goal_exact_total"], "goal_substring_total": a["goal_substring_total"],
         "real_home_refs": a.get("real_home_refs")}
    if "session_meta" in a:
        v["session_meta"] = a["session_meta"]
        v["turn_context"] = [{k: t.get(k) for k in ("model", "effort", "approval_policy", "sandbox_policy", "cwd")} for t in a["turn_context"]]
        v["tool_calls"] = [dict(c, args=cut(c["args"], 300)) for c in a["tool_calls"]]
        v["tool_outputs"] = [dict(o, output=cut(o["output"], 200)) for o in a["tool_outputs"]] if keep_outputs else len(a["tool_outputs"])
    else:
        v.update({k: a[k] for k in ("models", "session_ids", "cwds", "versions")})
        for k in ("mcp_servers", "mcp_tool_count", "attachment_types"):
            if k in a:
                v[k] = a[k]
        v["tool_calls"] = [dict(c, input=cut(c["input"], 300)) for c in a["tool_calls"]]
        v["tool_results"] = [dict(r, content=cut(r["content"], 240)) for r in a["tool_results"]] if keep_outputs else len(a["tool_results"])
    return v


for d in RAW:
    for mf in sorted(glob.glob(d + "/*/meta.json")):
        m = json.load(open(mf))
        o = {k: m.get(k) for k in ("run_id", "harness", "variant", "proxy", "started_utc", "duration_s", "exit_code",
                                   "timed_out", "timeout_s", "spawn_error", "executable", "executable_realpath",
                                   "executable_sha256", "argv_template", "argc", "cwd", "stdin", "env_names", "home",
                                   "env_values_nonsecret", "exec_bytes", "session_id_preassigned", "oracle", "appwide_scope",
                                   "scoped", "home_link", "codex_trust_override", "allow_tools")}
        for k in ("scoped", "home_link", "codex_trust_override", "allow_tools"):
            if k not in m:
                o.pop(k)
        keep = set(UUID.findall(m.get("cwd") or ""))  # ids that are part of the run's own workspace path
        keep.update(x for x in (m.get("session_id_preassigned"), (m.get("stdout") or {}).get("thread_id"),
                                (m.get("stdout") or {}).get("session_id")) if x)
        for n in m["native"]:
            keep.update(UUID.findall(n["path_rel_home"]))
        if m.get("side_effects"):
            o["side_effects"] = side_effects_view(m["side_effects"], keep)
        if m.get("home") is None:
            o["home"] = "real user HOME (run predates the runner's home field)"
        if m.get("timeout_s") is None:
            o["timeout_s"] = 600
        for n in m["native"]:
            if n["analysis"].get("real_home_refs") is None:
                raw = open(os.path.join(os.path.dirname(mf), "native-" + os.path.basename(n["path_rel_home"])), encoding="utf-8", errors="replace").read()
                n["analysis"]["real_home_refs"] = {k: raw.count(k) for k in (os.path.expanduser("~/.agents"), os.path.expanduser("~/.claude/"),
                                                                             os.path.expanduser("~/.codex/"), os.path.expanduser("~/.zshrc"))}
        g = dict(m["goal"])
        if m["variant"] == "big":
            g["text"] = "<65536 bytes; regenerate with tools/mkgoal.py big; sha256 above>"
        o["goal"] = g
        st = dict(m["stdout"])
        o["stdout_summary"] = st
        o["stderr_bytes"] = m.get("stderr_bytes")
        o["native"] = [native_view(n, m["variant"]) for n in m["native"]]
        if m.get("wire") is not None:
            o["wire_summary"] = m["wire"]
            o["probe"] = m.get("probe")
        os.makedirs(DST + "/runs", exist_ok=True)
        json.dump(o, open(DST + "/runs/%s.json" % m["run_id"], "w"), indent=1, ensure_ascii=False)
        print(m["run_id"])
