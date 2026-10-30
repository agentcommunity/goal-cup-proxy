#!/usr/bin/env python3
"""PRD-12 section 8: `scripts/pad/check.py <run folder>` maps the JSON the box's commands wrote there to C1 to C6,
the fault expectations of section 3.1 and the INCONCLUSIVE causes, and writes row.json and report.txt. It never
runs goalcup-verify. Standard library, Python 3.9 or later. Last stdout line: PADCHECK {"result", "checks",
"why"}. Exit 0 for any mapped result, 2 without a readable meta.json."""
import datetime
import json
import os
import re
import sys

# Every key read from another PRD's seam, so a seam change is a one-line edit here. S6: PRD-08 section 4
# (s6_ keys per entry). S7: PRD-09 section 7 (t_ keys per thread). S13: PRD-07 section 7 fetch.json.
# S4: PRD-02 section 6 status.json, also each watch/status.jsonl sample. S9: PRD-06 section 11 upload.json,
# also each watch/upload.jsonl line.
SEAM = {k: tuple(v.split(".")) for k, v in {
    "s6_exit": "exit", "s6_entries": "entries", "s6_checks": "checks", "s6_requests": "counts.model_requests",
    "s6_gaps": "counts.gaps", "s6_off": "counts.off_ms", "s6_threads": "counts.threads", "s6_seal": "seal_reason",
    "s6_native": "counts.native_files", "s6_errors": "provider_errors.wire", "s6_history": "history",
    "s6_log": "harness_log", "s6_bytes": "bytes", "s7_readable": "readable", "s7_reason": "reason",
    "s7_wire": "coverage.wire_calls", "s7_native": "coverage.native_calls", "s7_wire_only": "coverage.wire_only",
    "s7_unattributed": "coverage.unattributed", "s7_tokens": "tokens.merged", "s7_models": "models",
    "s7_threads": "threads", "t_parent": "parent", "t_models": "models", "t_wire": "wire_calls",
    "t_native": "native_calls", "s13_requests": "model_requests", "s13_final": "final", "s13_complete": "complete",
    "s4_mode": "mode", "s4_port": "port", "s4_harness_pid": "harness.pid", "s4_requests": "written.model_requests",
    "s4_off": "written.off_ms", "s4_seal": "seal.reason", "s9_final": "final", "s9_keep": "keep", "s9_state": "state"}.items()}
# Not fixed yet (asked of PRD-08 in the night log): the `secrets` JSON, and how a recorder_down gap's
# cause and length appear. Read when present; else hits fall back to verify/exits.json (keyed by the
# JSON file's stem; PRD-08 section 2: 0 and 1 are no hit, 2 a hit, anything else no scan) and causes to counts.gaps keys ("null_mode",
# "recorder_down:null_mode"). MASK spans by rule are read from the `masks` key of verify/secrets.json.
SECRET_HITS = ("hits", "known_hits", "shape_hits")
# Section 5's planted shaped values, each as the S12 rule kinds (PRD-05 section 10) whose MASK covers it. The
# Google line is masked under the shell rule, row 11, which wins over `google_key`, row 13.
PLANTED = (("openai_key",), ("github_token",), ("google_key", "generic_secret_assignment"), ("jwt",), ("private_key",),
           ("url_password",))
INV6_GAPS = ("gaps", "recorder_down")  # inv6 facts: a list of {cause, start, end, ms}
DOWN = ("recorder_down", "null_mode", "watchdog_lost")
DOWN_ERR = re.compile(r"(?<!\d)5\d\d(?!\d)|overloaded|server_error|api_error|internal", re.I)  # S6 keys are
REFUSED_ERR = re.compile(r"(?<!\d)40[13](?!\d)|auth|permission|api_key|quota|billing", re.I)  # kinds or codes
RECOVER = ("nothing to recover", "Recording is on again")  # PRD-10 acceptance 11: first, second recover
BUILT_FAULTS = ("kill-recorder", "kill-watchdog-recover")
C1_IDS = ("gap_has_end", "history_complete", "streams_closed", "seen_matches_written", "recorder_alive", "listed_hosts_decrypted")
RANK = {"pass": 0, "warn": 1, "skip": 2, "named": 3, "fail": 4}
ORDER = {"PASS": 0, "INCONCLUSIVE": 1, "FAIL": 2}
CHECKS = ("C1", "C2", "C3", "C4", "C5", "C6")
COPY = "connector copy"  # pad's variant mark for a connector file from disk (P13): a check row, not a fault

def fault_kinds(meta):
    """The variant's fault, exec and pre kinds, a connector copy left out."""
    return [str(v).split("@")[0].split(" ")[0] for v in meta.get("variant") or [] if v != COPY]

def dig(doc, name):
    for key in SEAM[name]:
        doc = doc.get(key) if isinstance(doc, dict) else None
    return doc

def items(d):
    return [(str(k), v) for k, v in d.items()] if isinstance(d, dict) else []

def load(folder, rel, lines=False):
    """A JSON file, or the dicts of a JSONL file (a watch line may wrap its file as {"at", "status"} or "upload")."""
    try:
        with open(os.path.join(folder, rel)) as f:
            got = [json.loads(ln) for ln in f if re.match(r"\s*\{.*\}\s*$", ln)] if lines else json.load(f)  # no torn line
    except (OSError, ValueError):
        return [] if lines else None
    return [next((v[k] for k in ("status", "upload") if isinstance(v.get(k), dict)), v) for v in got] if lines else got

def when(s):
    try:
        s = re.sub(r"\.(\d+)", lambda m: "." + (m.group(1) + "00000")[:6], str(s).replace("Z", "+00:00"))
        return datetime.datetime.fromisoformat(s)  # Python 3.9 takes 6 fraction digits and no Z
    except ValueError:
        return None

def count(v):
    if isinstance(v, (bool, int)):
        return int(v)
    return len(v) if isinstance(v, list) else sum(count(n) for _, n in items(v))

def worst(entries, cid):
    """The worst result of one S6 check id across the entries, as the check's dict; {} when absent."""
    best = {}
    for c in (c for e in entries for c in dig(e, "s6_checks") or [] if isinstance(c, dict)):
        if c.get("id") == cid and RANK.get(c.get("result"), 4) >= RANK.get(best.get("result"), -1):
            best = c
    return best

class Run:
    def __init__(self, folder, meta):
        self.folder, self.meta, self.why = folder, meta, []
        self.local = load(folder, "verify/check-local.json")
        self.entries = [e for e in dig(self.local, "s6_entries") or [] if isinstance(e, dict)]
        entry = meta.get("entry") or next((e["entry"] for e in self.entries[::-1] if e.get("entry")), None)
        self.entry = next((e for e in self.entries if e.get("entry") == entry), (self.entries or [{}])[-1])
        usage = os.path.join(folder, "verify/usage")
        names = sorted(os.listdir(usage)) if os.path.isdir(usage) else []
        name = "%s.json" % entry if "%s.json" % entry in names else names[0] if len(names) == 1 else None
        got = load(usage, name) if name else None
        self.usage = got if isinstance(got, dict) else None
        self.server, self.status = load(folder, "verify/check-server.json"), load(folder, "status.json")
        self.fetch = load(folder, "fetched/%s/fetch.json" % entry) if entry else None
        self.samples, self.faults = load(folder, "watch/status.jsonl", 1), load(folder, "watch/faults.jsonl", 1)
        self.exits = dict(items(load(folder, "verify/exits.json")))
        self.failed = {c.get("id"): c.get("why") for e in self.entries for c in dig(e, "s6_checks") or []
                       if isinstance(c, dict) and c.get("result") == "fail" and isinstance(c.get("id"), str)}
        self.v = dict.fromkeys(CHECKS + ("faults",), "PASS")
        got = [dig(e, "s6_requests") for e in self.entries if isinstance(dig(e, "s6_requests"), int)]
        self.reqs, self.native = sum(got) if got else None, sum(count(dig(e, "s6_native")) for e in self.entries)

    def mark(self, check, result, why):
        self.v[check] = max(self.v[check], result, key=ORDER.get)
        self.why.append("%s %s: %s" % (check, result, why))

    def flag(self, cond, check, result, why):
        if cond:
            self.mark(check, result, why)

    def check(self, cid):
        return worst(self.entries, cid).get("result")

    def gaps(self):
        """recorder_down gaps from inv6's facts (None when no entry lists them), and counts.gaps summed."""
        listed, seen, counts = [], False, {}
        for e in self.entries:
            for k, n in items(dig(e, "s6_gaps")):
                counts[k] = counts.get(k, 0) + count(n)
            facts = worst([e], "inv6").get("facts") or {}
            key = next((k for k in INV6_GAPS if isinstance(facts.get(k), list)), None)
            seen, listed = seen or key is not None, listed + (facts[key] if key else [])
        return ([g for g in listed if isinstance(g, dict)] if seen else None), counts

    def hits(self, name):
        doc = load(self.folder, "verify/%s.json" % name)
        if isinstance(doc, dict) and any(k in doc for k in SECRET_HITS):
            return sum(count(doc.get(k)) for k in SECRET_HITS)
        code = self.exits.get(name)
        return {0: 0, 1: 0, 2: 1}.get(code) if isinstance(code, int) else None  # 1: incomplete, never a hit

    def masks(self):
        """MASK spans by rule kind from the record scan, or None when the report has none."""
        doc = load(self.folder, "verify/secrets.json")
        got = doc.get("masks") if isinstance(doc, dict) else None
        return {k: count(n) for k, n in items(got)} if isinstance(got, dict) else None

def cause(r):
    """Section 8: the INCONCLUSIVE causes that are never goalcup's failure, from the record first."""
    errs = [(k, n) for e in r.entries for k, n in items(dig(e, "s6_errors")) if isinstance(n, int)]
    if r.reqs and 2 * sum(n for k, n in errs if DOWN_ERR.search(k)) > r.reqs:
        return "provider_down: most recorded model responses are 5xx"
    if any(n and REFUSED_ERR.search(k) for k, n in errs):
        return "provider_refused: 401 or 403 from the model host"
    return None

def judge(r):
    seen = dig(r.status, "s4_requests") if r.reqs is None else r.reqs  # None: no count in the folder
    reqs = seen or 0
    secrets(r)  # C3 first: a hit is a FAIL whatever else happened
    work = (reqs or r.native or dig(r.usage, "s7_native")) and (r.meta.get("commits_since_start") or 0) >= 1
    stop = cause(r) or (None if work else "the harness did no work (commits since start %s, model requests %s)"
                        % (r.meta.get("commits_since_start"), seen))
    if stop:
        for c in ("C1", "C2", "C4", "C5", "C6", "faults"):
            r.mark(c, "INCONCLUSIVE", stop)
        return
    local(r, reqs)
    usage_checks(r, reqs)
    c6(r)
    faults(r)

def local(r, reqs):
    """C1 (no missed call) and C4 (record whole, server copy whole and readable)."""
    if not r.entries:
        for c in ("C1", "C4"):
            r.mark(c, "INCONCLUSIVE", "no local verifier report")
        return
    r.flag(reqs < 2, "C1", "INCONCLUSIVE", "the record holds fewer than 2 model calls")
    for cid in ("gap_has_end", "history_complete"):
        r.flag(not r.check(cid), "C1", "INCONCLUSIVE", "%s not in the report" % cid)
    for cid in sorted(i for i in r.failed if i in C1_IDS or i.startswith("inv")):
        r.mark("C1", "FAIL", "%s fail: %s" % (cid, r.failed[cid]))
    if dig(r.usage, "s7_readable") is True:  # the local log check counts only where that log was read
        tm, wk = r.check("threads_match"), r.check("wire_keeps_up")
        totals = dig(r.entry, "s6_log") == "totals" and wk == "skip"
        r.flag(tm not in ("pass", "named") or not (wk in ("pass", "named") or totals), "C1", "FAIL",
               "local log check: threads_match %s, wire_keeps_up %s" % (tm, wk))
    if r.meta.get("case") != "direct":
        r.mark("C1", "INCONCLUSIVE", "case %s expectations not built yet" % r.meta.get("case"))
    # A failed check is already filed under its own C; with none, exit 0, or exit 1 whose only named loss
    # is an after_stop GAP (PRD-08 R2: a harness still sending after the seal at `minutes`).
    code, named = dig(r.local, "s6_exit"), {k for k, n in r.gaps()[1].items() if n}
    after_stop = code == 1 and named == {"after_stop"} and not any(dig(e, "s6_off") for e in r.entries)
    r.flag(r.meta.get("case") == "direct" and not fault_kinds(r.meta) and not r.failed and code != 0 and not after_stop,
           "C1", "FAIL", "verifier exit %s, the direct case expects 0 or an after_stop GAP only" % code)
    own = sorted(i for i in r.failed if i != "secrets")  # `secrets` is C3's
    half = "local checks failed: %s" % ", ".join(own) if own else "no local check failed (keep from the settings)"
    r.flag(own, "C4", "FAIL", half)
    r.flag(r.meta.get("keep") is False, "C4", "FAIL", "parts pruned (keep false)")
    if r.meta.get("server", "off") == "off":
        return r.mark("C4", "INCONCLUSIVE", "server off: local pass at best; " + half)
    up = load(r.folder, "watch/upload.jsonl", 1)
    r.flag(up and dig(up[-1], "s9_keep") is False, "C4", "FAIL", "parts pruned (upload.json keep false)")
    if not isinstance(r.fetch, dict):
        return r.mark("C4", "FAIL", "server copy not fetched")
    srv = [e for e in dig(r.server, "s6_entries") or [] if isinstance(e, dict)]
    for cid in ("server_chunks_whole", "server_equals_local", "server_counts_match"):
        got = worst(srv, cid).get("result")  # a skip on server_equals_local (pruned parts) is FAIL too
        r.flag(got != "pass", "C4", "FAIL" if got else "INCONCLUSIVE", "%s %s" % (cid, got or "not in a server report"))
    r.flag(not dig(r.fetch, "s13_final") or dig(r.fetch, "s13_complete") is not True, "C4", "FAIL",
           "fetch.json is not final and complete")

def usage_checks(r, reqs):
    """C2 (every subagent recorded) and C5 (usage stats present), both from the S7 summary."""
    if not r.usage:
        for c in ("C2", "C5"):
            r.mark(c, "INCONCLUSIVE", "no usage summary")
        return
    threads = [t for t in dig(r.usage, "s7_threads") or [] if isinstance(t, dict)]
    r.flag(not any(dig(t, "t_parent") for t in threads), "C2", "INCONCLUSIVE", "no thread with a parent")
    r.flag(any(not (dig(t, "t_wire") or dig(t, "t_native")) for t in threads), "C2", "FAIL",
           "a thread with no wire calls and no native stream")
    sub = r.check("subagents_present")
    r.flag(dig(r.usage, "s7_readable") is True and sub not in ("pass", "named"), "C2", "FAIL", "subagents_present %s" % sub)
    if reqs < 2:
        return r.mark("C5", "INCONCLUSIVE", "the record holds fewer than 2 model calls")
    tok = dict(items(dig(r.usage, "s7_tokens")))
    r.flag((tok.get("input") or 0) + (tok.get("output") or 0) <= 0, "C5", "FAIL", "merged input plus output tokens are 0")
    r.flag(any(not dig(t, "t_models") or not (dig(t, "t_wire") or dig(t, "t_native")) for t in threads), "C5", "FAIL",
           "a thread with no model or no calls")
    r.flag(dig(r.usage, "s7_unattributed") != 0, "C5", "FAIL", "coverage.unattributed %s" % dig(r.usage, "s7_unattributed"))

def secrets(r):
    """C3. The known-value and shape scans ran in the box; only their counts reach this folder."""
    scans = [("secrets", "the local record and the run's files"), ("secrets-out", "the box's output folder"),
             ("secrets-conductor", "the conductor's run folder (its own secrets, section 7)")]
    for name, where in scans + [("secrets-git", "the project's git objects")] * (r.meta.get("push") == "tested"):
        n = r.hits(name)
        r.flag(n != 0, "C3", "FAIL" if n else "INCONCLUSIVE", "%s hits in %s" % (n, where) if n else "no scan of " + where)
    r.flag("secrets" in r.failed, "C3", "FAIL", "secrets fail in the verifier report: %s" % r.failed.get("secrets"))
    r.flag(r.meta.get("known_box") is not True, "C3", "INCONCLUSIVE", "the run certificate key is not in the known values")
    # Section 5: the setup-check script printed .env.local. Its unmasked no-shape value (plain hits) or a
    # MASK of a planted value's own shape shows that output reached the model. Never any MASK: the
    # harness's auth header is masked on every call.
    masks = r.masks()
    if masks is None:
        return r.mark("C3", "INCONCLUSIVE", "no MASK counts by rule in verify/secrets.json")
    if not r.hits("secrets-plain") and not any(masks.get(kinds[0]) for kinds in PLANTED):
        return r.mark("C3", "INCONCLUSIVE", "secret exercise not done: no plain-value hit and no MASK of a planted shape")
    for kinds in PLANTED:
        r.flag(not any(masks.get(k) for k in kinds), "C3", "FAIL", "no MASK of %s for a planted value" % " or ".join(kinds))

def c6(r):
    st, final = r.status, dig(r.status, "s4_requests")
    if not isinstance(st, dict):
        return r.mark("C6", "INCONCLUSIVE", "no status.json")
    r.flag(dig(st, "s4_mode") != "sealed", "C6", "FAIL", "status.json mode %s, not sealed" % dig(st, "s4_mode"))
    want = [("verifier model_requests", dig(r.entry, "s6_requests"), final),
            ("verifier off_ms", dig(r.entry, "s6_off"), dig(st, "s4_off")),
            ("end screen requests", r.meta.get("end_screen_requests"), final)]
    want += [("fetch.json model_requests", dig(r.fetch, "s13_requests"), final)] * isinstance(r.fetch, dict)
    for what, theirs, ours in want:
        r.flag(theirs is None or ours is None or theirs != ours, "C6", "INCONCLUSIVE" if None in (theirs, ours)
               else "FAIL", "%s %s, status.json %s" % (what, theirs, ours))
    high = max([dig(s, "s4_requests") for s in r.samples if isinstance(dig(s, "s4_requests"), int)] or [0])
    r.flag(isinstance(final, int) and high > final, "C6", "FAIL", "a status sample showed %s requests, final %s" % (high, final))

def faults(r):
    """Section 3.1, for the two faults built so far; any other kind makes the row INCONCLUSIVE."""
    kinds = fault_kinds(r.meta)
    for k in sorted(set(kinds) - set(BUILT_FAULTS)):
        r.mark("faults", "INCONCLUSIVE", "expectations for %s not built yet" % k)
    if not set(kinds) & set(BUILT_FAULTS):
        return
    gaps, counts = r.gaps()
    other = sorted(k for k, n in counts.items() if n and k.split(":")[0] not in DOWN + ("after_stop",))  # R2: seal
    r.flag(other, "faults", "FAIL", "named loss other than the expected gaps: %s" % ", ".join(other))  # C1, C2
    if gaps is None:
        return r.mark("faults", "INCONCLUSIVE", "recorder_down gaps not readable from inv6")
    span = [(when(g.get("start")), when(g.get("end")), g.get("ms")) for g in gaps]
    r.flag(any(a is None or b is None or a >= b for a, b, _ in span), "faults", "FAIL",
           "a recorder_down gap without start before end")
    if "kill-watchdog-recover" in kinds:
        return recover(r, gaps)
    r.flag(any(g.get("cause") == "null_mode" for g in gaps) or any(n and "null_mode" in k for k, n in counts.items()),
           "faults", "FAIL", "a null_mode gap")
    r.flag(any("mode" in s and dig(s, "s4_mode") in (None, "null") for s in r.samples), "faults", "FAIL",
           "a status.jsonl line with mode null")
    kills = kinds.count("kill-recorder")
    r.flag(len(gaps) != kills, "faults", "FAIL", "%d recorder_down gaps for %d kills" % (len(gaps), kills))
    for a, b, ms in span:
        ms = ms if isinstance(ms, (int, float)) else (b - a).total_seconds() * 1000 if a and b else None
        r.flag(ms is None or ms >= 2000, "faults", "FAIL", "a recorder_down gap of %s ms, not under 2 s" % ms)
    r.flag(dig(r.local, "s6_exit") != 1, "faults", "FAIL", "verifier exit %s, a kill row expects 1" % dig(r.local, "s6_exit"))

def recover(r, gaps):
    if any("goalcup continue" in str(f.get("said")) for f in r.faults):
        return r.mark("faults", "INCONCLUSIVE", "harness lost its terminal")
    done = [f for f in r.faults if f.get("kind") == "recover"]
    r.flag(len(done) < 2, "faults", "INCONCLUSIVE", "%d of 2 recover outcomes logged" % len(done))
    for f, words in zip(done, RECOVER):
        r.flag(f.get("exit") != 0 or words not in str(f.get("said")), "faults", "FAIL",
               "recover at %s: exit %s, not the expected outcome" % (f.get("at"), f.get("exit")))
    for what, name in (("harness pid", "s4_harness_pid"), ("port", "s4_port")):
        r.flag(len({dig(s, name) for s in r.samples} - {None}) > 1, "faults", "FAIL", "the %s changed" % what)
    r.flag(len(gaps) != 1, "faults", "FAIL", "%d recorder_down gaps, the recover row expects 1" % len(gaps))
    seal = dig(r.entry, "s6_seal") or dig(r.status, "s4_seal")
    r.flag(seal not in ("harness_exit", "deadline"), "faults", "FAIL", "seal reason %s" % seal)

def facts(r):
    u, e, m = r.usage or {}, r.entry, r.meta
    read, hist, size = dig(u, "s7_readable") is True, dig(e, "s6_history") or {}, dig(e, "s6_bytes") or {}
    threads = [t for t in dig(u, "s7_threads") or [] if isinstance(t, dict)]
    alarm = [x.get("at") for x in load(r.folder, "watch/live.jsonl", 1) if x.get("result") == "fail" or x.get("fail")]
    return {"wire_requests": dig(u, "s7_wire"), "harness_calls": dig(u, "s7_native") if read else None,
            "harness_log": dig(e, "s6_log") if read else None, "local_log": "read" if read else "none",
            "history": {"state": r.check("history_complete"), "answers": hist.get("checked"),
                        "orphans": hist.get("missing_call"), "orphans_in_gaps": None},
            "threads": len(threads) if r.usage else dig(e, "s6_threads"), "native_files": dig(e, "s6_native"),
            "subagents": sum(1 for t in threads if dig(t, "t_parent")), "gaps": r.gaps()[1], "off_ms": dig(e, "s6_off"),
            "provider_errors": dict(kn for en in r.entries for kn in items(dig(en, "s6_errors"))),
            "secret_hits": r.hits("secrets"), "masks": r.masks() or {}, "plain_hits": r.hits("secrets-plain"),
            "screen_known": m.get("screen_known"), "canary": "not planted", "tokens": dig(u, "s7_tokens"),
            "models": [x.get("model") for x in dig(u, "s7_models") or [] if isinstance(x, dict)],
            "raw_bytes": size.get("raw"), "compressed_bytes": size.get("compressed"), "recorder_rss_peak_mb": None,
            "spend_usd_est": None, "priced_high": [], "first_live_alarm": (alarm or [None])[0], "over_cap": False,
            "push": m.get("push"), "wire_only": dig(u, "s7_wire_only"), "local_log_reason": dig(u, "s7_reason")}

def evaluate(folder):
    meta = load(folder, "meta.json")
    if not isinstance(meta, dict):
        return None
    r = Run(folder, meta)
    judge(r)
    fault = r.v["faults"] if fault_kinds(meta) else None
    got = [r.v[c] for c in CHECKS] + [fault] * bool(fault)
    row = {k: meta.get(k) for k in ("connector", "harness_version", "case", "mode", "prompt", "variant", "where",
                                    "server", "image", "run", "entry", "started", "minutes", "pad_trusted_at")}
    row.update(os=None, goalcup=None, checks={c: r.v[c] for c in CHECKS}, faults=fault, exec={}, why=r.why,
               result="FAIL" if "FAIL" in got else "PASS" if set(got) == {"PASS"} else "INCONCLUSIVE", facts=facts(r))
    return row

def main(argv):
    row = evaluate(argv[1]) if len(argv) == 2 else None
    if row is None:
        print("usage: scripts/pad/check.py <run folder with a readable meta.json>", file=sys.stderr)
        return 2
    lines = ["pad check %(connector)s %(case)s %(mode)s: result %(result)s, faults %(faults)s" % row,
             "checks: " + ", ".join("%s %s" % kv for kv in sorted(row["checks"].items()))]
    lines += ["why: " + w for w in row["why"]] + ["facts: " + json.dumps(row["facts"], sort_keys=True)]
    lines += ["for the watcher: read goalcup.log and the panes for a failed call the harness did not retry"] * bool(fault_kinds(row))
    for name, text in (("row.json", json.dumps(row, sort_keys=True, indent=1)), ("report.txt", "\n".join(lines))):
        with open(os.path.join(argv[1], name), "w") as f:
            f.write(text + "\n")
    print("\n".join(lines + ["PADCHECK " + json.dumps({k: row[k] for k in ("result", "checks", "why")}, sort_keys=True)]))
    return 0

if __name__ == "__main__":
    sys.exit(main(sys.argv))
