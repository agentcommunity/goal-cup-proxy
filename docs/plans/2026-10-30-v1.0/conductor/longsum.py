#!/usr/bin/env python3
"""Cost per hour and the PRD-16 X-line numbers for one long pad run (long-runs plan Task 4).

Reads a pad run folder (meta.json, row.json, watch/spend.jsonl, verify/usage/<entry>.json) and check.py's Run.gaps().
Standard library only, Python 3.9. Prints no value from calls.log or panes, and no model name.

  python3 longsum.py RUN_DIR [--live MINUTES] [--json] [--min-subagents N] [--gap-limit-ms N] [--repo DIR]

Exit 2 on a folder without row.json (or without watch/spend.jsonl for --live)."""
import argparse
import datetime
import importlib.machinery
import importlib.util
import json
import os
import re
import sys

RUN1 = {"minutes": 49, "input": 117_400_000, "output": 410_600, "uploaded_mb": 405}  # PRD-16 X8, X9


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def read_lines(path):
    out = []
    try:
        with open(path) as f:
            for ln in f:
                try:
                    v = json.loads(ln)
                except ValueError:
                    continue
                if isinstance(v, dict):
                    out.append(v)
    except OSError:
        pass
    return out


def when(s):
    try:
        s = re.sub(r"\.(\d+)", lambda m: "." + (m.group(1) + "00000")[:6], str(s).replace("Z", "+00:00"))
        got = datetime.datetime.fromisoformat(s)
    except ValueError:
        return None
    return got if got.tzinfo else got.replace(tzinfo=datetime.timezone.utc)


def step_min(meta, prefix):
    """Minutes of the first step line whose name starts with prefix, or None."""
    for ln in meta.get("steps") or []:
        if isinstance(ln, (list, tuple)) and len(ln) >= 3 and str(ln[0]).startswith(prefix):
            m = re.fullmatch(r"(\d+):(\d\d)", str(ln[2]))
            if m:
                return int(m.group(1)) * 60 + int(m.group(2))
    return None


def hours(meta):
    a, b = step_min(meta, "start"), step_min(meta, "seal")
    if a is not None and b is not None and b > a:
        return (b - a) / 60
    return float(meta.get("minutes") or 0) / 60


def max_parallel(threads):
    """Most threads with a parent whose [first, last] overlap at one instant (a start counts before an end)."""
    ev = []
    for t in threads:
        if isinstance(t, dict) and t.get("parent"):
            a, b = when(t.get("first")), when(t.get("last"))
            if a and b and a <= b:
                ev += [(a, 0), (b, 1)]
    best = cur = 0
    for _, kind in sorted(ev):
        cur += 1 if kind == 0 else -1
        best = max(best, cur)
    return best


def load_check(repo):
    path = os.path.join(repo, "scripts", "pad", "check.py")
    loader = importlib.machinery.SourceFileLoader("pad_check", path)
    spec = importlib.util.spec_from_loader("pad_check", loader)
    mod = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    loader.exec_module(mod)
    return mod


def num(v):
    return v if isinstance(v, (int, float)) and not isinstance(v, bool) else None


def per_hour(v, h):
    return round(v / h, 1) if num(v) is not None and h else None


def x4(gaps, meta, why, gap_limit_ms):
    steps = [str(ln[0]) for ln in meta.get("steps") or [] if isinstance(ln, (list, tuple)) and ln]
    k = sum(1 for s in steps if s.startswith("kill-recorder@"))
    w = 1 if any(s.startswith("kill-watchdog-recover@") for s in steps) else 0
    down = sorted((g for g in gaps if g.get("cause") == "recorder_down"), key=lambda g: str(g.get("start")))
    if len(down) != k + w:
        return "FAIL: %d recorder_down gaps for %d kills and %d recovers" % (len(down), k, w)
    for g in down:
        a, b = when(g.get("start")), when(g.get("end"))
        if a is None or b is None or a >= b:
            return "FAIL: a recorder_down gap without start before end"
    for g in down[:k]:
        ms = num(g.get("ms"))
        if ms is None or ms >= gap_limit_ms:
            return "FAIL: a kill gap of %s ms, not under %d ms" % (ms, gap_limit_ms)
    if any(g.get("cause") == "null_mode" for g in gaps):
        return "FAIL: a null_mode gap"
    for line in why:
        if line.startswith("faults") and "not under 2 s" not in line and "(limit 2 s)" not in line:
            return "FAIL: faults line not about the 2 s bound: " + line[:120]
    return "PASS"


def summary(run_dir, check_mod, min_subagents=15, gap_limit_ms=5000):
    meta = read_json(os.path.join(run_dir, "meta.json")) or {}
    row = read_json(os.path.join(run_dir, "row.json")) or {}
    facts = row.get("facts") or {}
    why = [w for w in row.get("why") or [] if isinstance(w, str)]
    h = hours(meta)
    entry = meta.get("entry")
    usage = read_json(os.path.join(run_dir, "verify", "usage", "%s.json" % entry)) or {}
    threads = [t for t in usage.get("threads") or [] if isinstance(t, dict)]
    got = check_mod.Run(run_dir, meta).gaps()[0] or []
    gaps = [{"cause": g.get("cause"), "ms": g.get("ms")} for g in got if isinstance(g, dict)]
    tok = facts.get("tokens") if isinstance(facts.get("tokens"), dict) else {}
    usd = num(facts.get("spend_usd_est"))
    over_cap = facts.get("over_cap") is True or meta.get("over_cap") is True
    subs, par = facts.get("subagents"), max_parallel(threads)
    subs = subs if isinstance(subs, int) else sum(1 for t in threads if t.get("parent"))
    if subs >= min_subagents and par >= 10:
        x2 = "PASS"
    elif over_cap:
        x2 = "INCONCLUSIVE: sealed at the spend cap"
    else:
        x2 = "INCONCLUSIVE: %d subagents, %d at once" % (subs, par)
    raw, comp = num(facts.get("raw_bytes")), num(facts.get("compressed_bytes"))
    return {
        "hours": round(h, 2), "usd": usd, "over_cap": over_cap, "usd_per_hour": per_hour(usd, h),
        "tokens_per_hour": {k: per_hour(tok.get(k), h) for k in ("input", "cached", "output")},
        "raw_mb_per_hour": per_hour(raw / 1e6 if raw is not None else None, h),
        "compressed_mb_per_hour": per_hour(comp / 1e6 if comp is not None else None, h),
        "rss_peak_mb": facts.get("recorder_rss_peak_mb"), "wire_requests": facts.get("wire_requests"),
        "subagents": subs, "threads": facts.get("threads"), "max_parallel": par, "gaps": gaps, "x2": x2,
        "x3": "FAIL" if any(g["cause"] == "memory_guard" for g in gaps) else "PASS",
        "x4": x4(got, meta, why, gap_limit_ms),
    }


def live(run_dir, planned_minutes):
    m = re.search(r"(\d{8}T\d{6}Z)", os.path.basename(os.path.normpath(run_dir)))
    if not m:
        raise ValueError("longsum: no stamp in folder name")
    start = datetime.datetime.strptime(m.group(1), "%Y%m%dT%H%M%SZ").replace(tzinfo=datetime.timezone.utc)
    lines = [x for x in read_lines(os.path.join(run_dir, "watch", "spend.jsonl")) if num(x.get("usd")) is not None]
    if not lines:
        raise ValueError("longsum: no priced spend line yet")
    last = lines[-1]
    at = when(last.get("at"))
    if at is None:
        raise ValueError("longsum: no readable time in spend line")
    minutes = (at - start).total_seconds() / 60
    usd = last["usd"]
    rate = usd / (minutes / 60) if minutes > 0 else 0.0
    return {"minutes": round(minutes, 1), "usd": usd, "usd_per_hour": round(rate, 1),
            "projected_usd": round(rate * planned_minutes / 60, 1)}


def markdown(s, baseline=RUN1):
    b = baseline or {}
    bh = b["minutes"] / 60 if b.get("minutes") else None

    def one(v):
        return "-" if v is None else str(v)

    def b_rate(v):
        return one(round(v / bh, 1)) if bh and v is not None else "-"

    t = s["tokens_per_hour"]
    gaps = ", ".join("%s %s ms" % (g["cause"], g["ms"]) for g in s["gaps"]) or "none"
    rows = [
        ("Hours", s["hours"], one(round(bh, 1)) if bh else "-"),
        ("USD", s["usd"], "-"),
        ("USD per hour", s["usd_per_hour"], "-"),
        ("Input tokens per hour", t["input"], b_rate(b.get("input"))),
        ("Output tokens per hour", t["output"], b_rate(b.get("output"))),
        ("Raw MB per hour", s["raw_mb_per_hour"], b_rate(b.get("uploaded_mb"))),
        ("Compressed MB per hour", s["compressed_mb_per_hour"], "-"),
        ("Recorder RSS peak MB", s["rss_peak_mb"], "-"),
        ("Subagents", s["subagents"], "-"),
        ("Most at once", s["max_parallel"], "-"),
        ("Gaps", gaps, "-"),
        ("X2", s["x2"], "-"),
        ("X3", s["x3"], "-"),
        ("X4 (kill gaps under 5 s under load)", s["x4"], "-"),
    ]
    return "\n".join(["| Measure | This run | Run 1 |", "|---|---|---|"] +
                     ["| %s | %s | %s |" % (n, one(v), r) for n, v, r in rows])


def find_repo(run_dir):
    d = os.path.abspath(run_dir)
    while True:
        if os.path.isfile(os.path.join(d, "scripts", "pad", "check.py")):
            return d
        up = os.path.dirname(d)
        if up == d:
            return None
        d = up


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dir")
    ap.add_argument("--live", type=float, metavar="MINUTES")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--min-subagents", type=int, default=15)
    ap.add_argument("--gap-limit-ms", type=int, default=5000)
    ap.add_argument("--repo")
    a = ap.parse_args(argv)
    need = os.path.join("watch", "spend.jsonl") if a.live is not None else "row.json"
    if not os.path.isfile(os.path.join(a.run_dir, need)):
        print("longsum: %s not found in %s" % (need, a.run_dir), file=sys.stderr)
        return 2
    if a.live is not None:
        try:
            out = live(a.run_dir, a.live)
        except ValueError as e:
            print(e, file=sys.stderr)
            return 2
        print(json.dumps(out, sort_keys=True))
        return 0
    repo = a.repo or find_repo(a.run_dir)
    if not repo:
        print("longsum: no scripts/pad/check.py above the run folder; pass --repo", file=sys.stderr)
        return 2
    s = summary(a.run_dir, load_check(repo), a.min_subagents, a.gap_limit_ms)
    print(json.dumps(s, sort_keys=True, indent=1) if a.json else markdown(s))
    return 0


if __name__ == "__main__":
    sys.exit(main())
