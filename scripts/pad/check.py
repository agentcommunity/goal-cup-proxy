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
    "s4_off": "written.off_ms", "s4_seal": "seal.reason", "s4_pid": "pid", "s4_spans": "off.spans", "s4_down_ms": "off.ms",
    "s4_since": "off.since", "s4_last": "last_request_at", "s9_final": "final", "s9_keep": "keep", "s9_state": "state",
    "s6_hosts": "counts.hosts", "s6_tunnel_hosts": "counts.tunnel_hosts", "c_row": "k", "c_host": "host",
    "c_kind": "host_kind", "c_thread": "thread"}.items()}  # c_: one S7 calls.jsonl line
S7_TOKENS = ("input", "cached", "cache_write", "output", "reasoning")  # S7 models[].tokens keys
# PRD-08's `secrets` JSON (S12). A key below makes the scan fail by itself: a known value, an unglued shape, or a
# non-fail_closed MASK span over 16 KiB (`hits` is the pad's own conductor scan). Else hits fall back to
# verify/exits.json (keyed by the JSON file's stem; 0 and 1 are no hit, 2 a hit, anything else no scan). MASK spans
# by rule are read from the `masks` key.
SECRET_HITS = ("hits", "known_hits", "shape_hits", "big_masks")
# Section 5's planted shaped values, each as the S12 rule kinds (PRD-05 section 10) whose MASK covers it. The
# Google line is masked under the shell rule, row 11, which wins over `google_key`, row 13.
PLANTED = (("openai_key",), ("github_token",), ("google_key", "generic_secret_assignment"), ("jwt",), ("private_key",),
           ("url_password",))
DOWN = ("recorder_down", "null_mode", "watchdog_lost")
DOWN_ERR = re.compile(r"(?<!\d)5\d\d(?!\d)|overloaded|server_error|api_error|internal", re.I)  # S6 keys are
REFUSED_ERR = re.compile(r"(?<!\d)40[13](?!\d)|auth|permission|api_key|quota|billing", re.I)  # kinds or codes
RECOVER = ("nothing to recover", "Recording is on again")  # PRD-10 acceptance 11: first, second recover
STATUS_RECOVER = "Run: goalcup recover"  # PRD-10 acceptance 11: the status line after the recorder kill
BUILT_FAULTS = ("kill-recorder", "kill-watchdog-recover")
C1_IDS = ("gap_has_end", "history_complete", "streams_closed", "seen_matches_written", "recorder_alive", "listed_hosts_decrypted")
RANK = {"pass": 0, "warn": 1, "skip": 2, "named": 3, "fail": 4}
ORDER = {"PASS": 0, "INCONCLUSIVE": 1, "FAIL": 2}
CHECKS = ("C1", "C2", "C3", "C4", "C5", "C6")
SEAL_SLACK = datetime.timedelta(seconds=120)  # pad's seal-now limit: a seal may begin that long before its line
WATCHER = ("exec", "pre")  # section 3.1 watcher steps: the owning PRD's pass line decides, not `faults`
EXPECTS_LOSS = ("refusal-curl.sh",)  # exec scripts whose owner expects a named loss (PRD-03 acceptance 7: a refused call)
LOSS_GAP = "tls_refused"  # the only loss such a row may name: the curl's own refused connections
REFUSES = "proxy-setting.sh"  # the pre script after which goalcup start must refuse: no run is recorded
STALE = "stale-gateway.sh"  # the pre script that leaves a daemon running for goalcup start to stop
JUDGED = (REFUSES, STALE)  # pre scripts whose row the record decides (section 3.1), as a fault row is
# C3's scans (section 5, 7): the verifier's over the record, the box's output folder, the conductor's own folder.
SCANS = (("secrets", "the local record and the run's files"), ("secrets-out", "the box's output folder"),
         ("secrets-conductor", "the conductor's run folder (its own secrets, section 7)"))
GATEWAY = "ai-gateway.vercel.sh"
COPY = "connector copy"  # pad's variant mark for a connector file from disk (P13): a check row, not a fault

def fault_kinds(meta):
    """The variant's fault kinds: the connector copy and the watcher steps (watchers() reads those) left out."""
    kinds = [str(v).split("@")[0].split(" ")[0] for v in meta.get("variant") or [] if v != COPY]
    return [k for k in kinds if k not in WATCHER]

def judged(meta):
    """Whether the faults slot of the row holds a verdict: a fault kind, or a pre script the record decides."""
    return bool(fault_kinds(meta)) or any(s in JUDGED for _, s in watchers(meta))

def watchers(meta):
    """The (kind, script) of each exec and pre variant: ("exec", "h2-parallel.sh") for "exec h2-parallel.sh@1"."""
    words = [str(v).split() for v in meta.get("variant") or []]
    return [(w[0], w[1].split("@")[0]) for w in words if w[0] in WATCHER and w[1:]]

def dig(doc, name):
    for key in SEAM[name]:
        doc = doc.get(key) if isinstance(doc, dict) else None
    return doc

def items(d):
    return [(str(k), v) for k, v in d.items()] if isinstance(d, dict) else []

def load(folder, rel, lines=False):
    """A JSON file, or the dicts of a JSONL file (a watch line may wrap its file as {"at", "status"} or "upload": then
    the file's dict, with the line's time as pad_at)."""
    try:
        with open(os.path.join(folder, rel)) as f:
            got = [json.loads(ln) for ln in f if re.match(r"\s*\{.*\}\s*$", ln)] if lines else json.load(f)  # no torn line
    except (OSError, ValueError):
        return [] if lines else None
    return [dict(v[k], pad_at=v.get("at")) if k else v for v in got
            for k in [next((k for k in ("status", "upload") if isinstance(v.get(k), dict)), None)]] if lines else got

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

def models(doc):
    """S7 `models` as [{model, tokens}], each token count an int or null: what the spend check and the ledger price."""
    got, out = dig(doc, "s7_models"), []
    for x in got if isinstance(got, list) else []:
        tok = x.get("tokens") if isinstance(x, dict) and isinstance(x.get("tokens"), dict) else {}
        if isinstance(x, dict) and isinstance(x.get("model"), str):
            out.append({"model": x["model"], "tokens": {k: tok[k] if type(tok.get(k)) is int else None for k in S7_TOKENS}})
    return out

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
        names = sorted(n for n in os.listdir(usage) if n.endswith(".json")) if os.path.isdir(usage) else []
        name = "%s.json" % entry if "%s.json" % entry in names else names[0] if len(names) == 1 else None
        got = load(usage, name) if name else None
        self.usage = got if isinstance(got, dict) else None
        calls = name and name[:-5] + ".calls.jsonl"
        self.calls = ([c for c in load(usage, calls, 1) if dig(c, "c_row") == "call"]
                      if calls and os.path.isfile(os.path.join(usage, calls)) else None)
        self.server, self.status = load(folder, "verify/check-server.json"), load(folder, "status.json")
        self.fetch = load(folder, "fetched/%s/fetch.json" % entry) if entry else None
        self.samples, self.faults = load(folder, "watch/status.jsonl", 1), load(folder, "watch/faults.jsonl", 1)
        self.exits = dict(items(load(folder, "verify/exits.json")))
        self.failed = {c.get("id"): c.get("why") for e in self.entries for c in dig(e, "s6_checks") or []
                       if isinstance(c, dict) and c.get("result") == "fail" and isinstance(c.get("id"), str)}
        self.v = dict.fromkeys(CHECKS + ("faults",), "PASS")
        seals = [when(f.get("at")) for f in self.faults if f.get("kind") == "seal-now" and f.get("exit") == 0]  # section 7,
        self.cap_at = min([t for t in seals if t], default=None) if meta.get("over_cap") is True else None  # as pad-run
        got = [dig(e, "s6_requests") for e in self.entries if isinstance(dig(e, "s6_requests"), int)]
        self.reqs, self.native = sum(got) if got else None, sum(count(dig(e, "s6_native")) for e in self.entries)

    def mark(self, check, result, why):
        self.v[check] = max(self.v[check], result, key=ORDER.get)
        self.why.append("%s %s: %s" % (check, result, why))

    def flag(self, cond, check, result, why):
        if cond:
            self.mark(check, result, why)

    def check(self, cid):
        """One S6 check's worst result; None when absent or `skip` "not built" (not in the report)."""
        c = worst(self.entries, cid)
        return None if c.get("result") == "skip" and "not built" in str(c.get("why")) else c.get("result")

    def hosts(self, name):
        """S6 counts.hosts (a count) or tunnel_hosts ({conns, up, down, listed}) as {host: count}; None: absent."""
        got, out = [dig(e, name) for e in self.entries if isinstance(dig(e, name), dict)], {}
        for h, v in (kv for d in got for kv in items(d)):
            n = v.get("conns") if isinstance(v, dict) else v
            out[h] = out.get(h, 0) + (n if type(n) is int else 0)
        return out if got else None

    def gaps(self):
        """recorder_down gaps from inv6's facts (None when no entry lists them), and counts.gaps summed."""
        listed, seen, counts = [], False, {}
        for e in self.entries:
            for k, n in items(dig(e, "s6_gaps")):
                counts[k] = counts.get(k, 0) + count(n)
            got = (worst([e], "inv6").get("facts") or {}).get("gaps")  # inv6 facts.gaps: [{cause, start, end, ms}]
            seen, listed = seen or isinstance(got, list), listed + (got if isinstance(got, list) else [])
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
    pin = r.meta.get("model_host") if r.meta.get("case") == "dodger" else None  # meta.json, written by pad
    tunnels = (r.hosts("s6_tunnel_hosts") or {}).get(pin, 0)  # section 2: a dodger's calls ride `pinned` tunnels
    if ("pre", REFUSES) in watchers(r.meta):
        return refused(r)
    secrets(r)  # C3 first: a hit is a FAIL whatever else happened
    if ("pre", STALE) in watchers(r.meta):
        stale(r)  # whatever else happened, as C3
    work = (seen or tunnels or r.native or dig(r.usage, "s7_native")) and (r.meta.get("commits_since_start") or 0) >= 1
    stop = cause(r) or (None if work else "the harness did no work (commits since start %s, model requests %s)"
                        % (r.meta.get("commits_since_start"), seen))
    if stop:
        for c in ("C1", "C2", "C4", "C5", "C6", "faults"):
            r.mark(c, "INCONCLUSIVE", stop)
        return
    few = (seen or 0) + tunnels < 2  # pinned tunnels stand in for the calls they carry, at least one each
    local(r, few)
    usage_checks(r, few)
    c6(r)
    faults(r)

def refused(r):
    """Section 3.1, proxy-setting.sh: goalcup start must refuse and name the file and key, so no run is recorded and the
    six checks have nothing to read. The row rests on start's exit and the screen's words, recorded by pad-run (no value)."""
    for name, where in SCANS[1:]:  # no run, so no verifier report: the other two scans, and no scan is never a PASS
        scan(r, name, where)
    r.flag(r.meta.get("known_box") is not True, "C3", "INCONCLUSIVE", "the run certificate key is not in the known values")
    said = [f for f in r.faults if f.get("kind") == "start-refused"]
    if not said:
        pre = [f for f in r.faults if f.get("kind") == "pre" and f.get("script") == REFUSES]
        began = r.entries or isinstance(r.status, dict) or r.meta.get("entry")
        if began and pre and pre[-1].get("exit") == 0:  # the setting was written and start went on: PRD-11 acceptance 4
            return r.mark("faults", "FAIL", "goalcup start did not refuse the proxy setting: a run was recorded")
        return r.mark("faults", "INCONCLUSIVE", "goalcup start did not end: no refusal recorded")
    f = said[-1]
    r.flag(f.get("value_shown"), "faults", "FAIL", "start printed the proxy value")
    r.flag(f.get("exit") in (None, 0), "faults", "FAIL", "start ended with exit %s, not a refusal" % f.get("exit"))
    r.flag(not (f.get("named_file") and f.get("named_key")), "faults", "FAIL",
           "start refused but the screen did not name settings.json and proxy")
    r.why.append("no run was recorded (start refused): C1, C2, C4 to C6 have nothing to read")

def stale(r):
    """Section 3.1, stale-gateway.sh: the daemon must have been running before start, and neither it nor any process
    with the daemon command may live after goalcup start or after the harness exited (PRD-11 acceptance 5)."""
    looks = [f for f in r.faults if f.get("kind") == "daemon-check"]
    pre = [f for f in r.faults if f.get("kind") == "pre" and f.get("script") == STALE]
    r.flag(any(f.get("alive") for f in looks), "faults", "FAIL", "the stale daemon was alive after goalcup start")
    r.flag(any(f.get("left") for f in looks), "faults", "FAIL", "a daemon process was left after the harness exited")
    r.flag(not pre or pre[-1].get("exit") != 0, "faults", "INCONCLUSIVE",
           "the stale daemon was not running before start (pre %s exit %s)" % (STALE, pre[-1].get("exit") if pre else "not run"))
    r.flag(pre and pre[-1].get("exit") == 0 and not looks, "faults", "INCONCLUSIVE",
           "the stale daemon was not running before start (no pid file, no daemon-check line)")
    r.flag(looks and not any("left" in f for f in looks), "faults", "INCONCLUSIVE", "no look for left daemons after exit")

def local(r, few):
    """C1 (no missed call) and C4 (record whole, server copy whole and readable)."""
    if not r.entries:
        for c in ("C1", "C4"):
            r.mark(c, "INCONCLUSIVE", "no local verifier report")
        return
    r.flag(few, "C1", "INCONCLUSIVE", "the record holds fewer than 2 model calls and pinned tunnels")
    for cid in ("gap_has_end", "history_complete"):
        r.flag(not r.check(cid), "C1", "INCONCLUSIVE", "%s not in the report" % cid)
    for cid in sorted(i for i in r.failed if i in C1_IDS or i.startswith("inv")):
        r.mark("C1", "FAIL", "%s fail: %s" % (cid, r.failed[cid]))
    if dig(r.usage, "s7_readable") is True:  # the local log check counts only where that log was read
        tm, wk = r.check("threads_match"), r.check("wire_keeps_up")
        totals = dig(r.entry, "s6_log") == "totals" and wk == "skip"
        r.flag(tm not in ("pass", "named") or not (wk in ("pass", "named") or totals), "C1", "INCONCLUSIVE" if None in
               (tm, wk) else "FAIL", "local log check: threads_match %s, wire_keeps_up %s" % (tm, wk))  # None: not in the report
    case(r)
    # A failed check is already filed under its own C; with none, exit 0, or exit 1 whose only named losses are an
    # after_stop GAP (PRD-08 R2: a harness still sending after the seal at `minutes`) and, for a row whose script
    # expects a refused call, tls_refused GAPs (PRD-03 acceptance 7).
    code, named = dig(r.local, "s6_exit"), {k.split(":")[0] for k, n in r.gaps()[1].items() if n}
    allowed = {"after_stop"} | ({LOSS_GAP} if any(s in EXPECTS_LOSS for _, s in watchers(r.meta)) else set())
    explained = code == 1 and named and named <= allowed and not any(dig(e, "s6_off") for e in r.entries)
    r.flag(r.meta.get("case") in ("direct", "gateway") and not fault_kinds(r.meta) and not r.failed and code != 0 and not explained,
           "C1", "FAIL", "verifier exit %s, the %s case expects 0 or only the GAPs %s" % (
               code, r.meta.get("case"), ", ".join(sorted(allowed))))
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

def case(r):
    """Section 2's rows beyond the six checks, from S6 counts.hosts and tunnel_hosts and S7's call rows. A fact not in
    the report is INCONCLUSIVE, never PASS or FAIL. Gateway and direct exit 0 is local()'s rule."""
    kind, code, own, hs = r.meta.get("case"), dig(r.local, "s6_exit"), r.meta.get("model_host"), r.hosts("s6_hosts")

    def need(fact, got):
        r.flag(got is None, "C1", "INCONCLUSIVE", "%s not in the report" % fact)
        return got is not None
    if kind == "direct" and hs is not None:
        r.flag(not (hs.get(own) if own else sum(hs.values())), "C1", "FAIL",
               "%s not decrypted in counts.hosts" % (own or "no model host"))
    if kind == "gateway":
        got = r.check("listed_hosts_decrypted")
        r.flag(need("listed_hosts_decrypted", got) and got != "pass", "C1", "FAIL", "listed_hosts_decrypted %s" % got)
        r.flag(need("counts.hosts", hs) and not hs.get(GATEWAY), "C1", "FAIL", GATEWAY + " not decrypted in counts.hosts")
        wire = [c for c in (r.calls if need("usage call rows", r.calls) else []) if dig(c, "c_host")]  # native rows have no host
        bad = sorted({str(dig(c, "c_host")) for c in wire if dig(c, "c_kind") != "gateway"})  # the vendor's host is kind provider
        r.flag(bad, "C1", "FAIL", "model calls not through the gateway: " + ", ".join(bad))
    if kind != "dodger":
        return
    r.flag(code in (0, 2), "C1", "FAIL", "verifier exit %s%s" % (code, ": the hosts copy was not used" if code == 0 else ""))
    r.flag(code not in (0, 1, 2), "C1", "INCONCLUSIVE", "verifier exit %s, the dodger case expects 1" % code)
    for cid, ok in (("listed_hosts_decrypted", ("named",)), ("wire_keeps_up", ("named", "skip"))):
        got = r.check(cid)
        r.flag(need(cid, got) and got not in ok, "C1", "FAIL", "%s %s, the dodger expects %s" % (cid, got, " or ".join(ok)))
    hist = r.check("history_complete")  # not in the report: local() files that
    r.flag(hist not in (None, "skip"), "C1", "FAIL", "history_complete %s, the dodger expects skip" % hist)
    th = r.hosts("s6_tunnel_hosts")
    r.flag(need("counts.tunnel_hosts", th) and not th.get(own), "C1", "FAIL", "no tunnel to the pinned host %s" % own)
    tun = [(h, v) for e in r.entries for h, v in items(dig(e, "s6_tunnel_hosts")) if isinstance(v, dict)]
    why = sorted({str(v.get("listed")) for h, v in tun if h == own} - {"pinned"})  # section 2: every tunnel there has GAP `pinned`
    r.flag(why, "C1", "FAIL", "tunnels to the pinned host %s carry GAP %s, not pinned" % (own, ", ".join(why)))
    r.flag(need("counts.hosts", hs) and hs.get(own), "C1", "FAIL", "the pinned host %s was decrypted" % own)
    expected = {"pinned", "after_stop"} | (set(DOWN) if fault_kinds(r.meta) else set())  # a fault row's own gaps: faults() judges them
    loss = sorted(k for k, n in r.gaps()[1].items() if n and k.split(":")[0] not in expected)
    r.flag(loss, "C1", "FAIL", "the dodger row names a loss other than pinned: " + ", ".join(loss))
    other = sorted({h for h, v in tun if h != own and v.get("listed") is not None})  # a listed host not decrypted
    r.flag(other, "C1", "FAIL", "tunnels to other listed hosts: " + ", ".join(other))
    if r.usage and r.calls is None:
        for c in ("C2", "C5"):
            r.mark(c, "INCONCLUSIVE", "usage call rows not in the report")
    mine = [c for c in r.calls or [] if dig(c, "c_host") == own]
    r.flag(any(dig(c, "c_thread") for c in mine), "C2", "FAIL", "a thread claimed from the pinned host %s" % own)
    r.flag(any(c.get(k) for c in mine for k in S7_TOKENS), "C5", "FAIL", "wire tokens claimed for the pinned host %s" % own)

def usage_checks(r, few):
    """C2 (every subagent recorded) and C5 (usage stats present), both from the S7 summary."""
    if not r.usage:
        for c in ("C2", "C5"):
            r.mark(c, "INCONCLUSIVE", "no usage summary")
        return
    threads = [t for t in dig(r.usage, "s7_threads") or [] if isinstance(t, dict)]
    unread = r.meta.get("case") == "dodger" and dig(r.usage, "s7_readable") is not True  # section 2: it claims no thread
    r.flag(not unread and not any(dig(t, "t_parent") for t in threads), "C2", "INCONCLUSIVE", "no thread with a parent")
    r.flag(any(not (dig(t, "t_wire") or dig(t, "t_native")) for t in threads), "C2", "FAIL",
           "a thread with no wire calls and no native stream")
    sub = r.check("subagents_present")
    r.flag(dig(r.usage, "s7_readable") is True and sub not in ("pass", "named"), "C2", "FAIL" if sub else "INCONCLUSIVE",
           "subagents_present %s" % (sub or "not in the report"))
    if few:
        return r.mark("C5", "INCONCLUSIVE", "the record holds fewer than 2 model calls and pinned tunnels")
    tok = dict(items(dig(r.usage, "s7_tokens")))
    native = r.meta.get("case") != "dodger" or dig(r.usage, "s7_readable") is True  # a dodger's tokens are native only
    r.flag(native and (tok.get("input") or 0) + (tok.get("output") or 0) <= 0, "C5", "FAIL", "merged input plus output tokens are 0")
    r.flag(any(not dig(t, "t_models") or not (dig(t, "t_wire") or dig(t, "t_native")) for t in threads), "C5", "FAIL",
           "a thread with no model or no calls")
    r.flag(dig(r.usage, "s7_unattributed") != 0, "C5", "FAIL", "coverage.unattributed %s" % dig(r.usage, "s7_unattributed"))

def scan(r, name, where):
    """One C3 scan: hits fail; no scan (None) is INCONCLUSIVE, never a PASS."""
    n = r.hits(name)
    r.flag(n != 0, "C3", "FAIL" if n else "INCONCLUSIVE", "%s hits in %s" % (n, where) if n else "no scan of " + where)

def secrets(r):
    """C3. The known-value and shape scans ran in the box; only their counts reach this folder."""
    for name, where in SCANS + ((("secrets-git", "the project's git objects"),) if r.meta.get("push") == "tested" else ()):
        scan(r, name, where)
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
    if isinstance(r.fetch, dict):
        want.append(("fetch.json model_requests", dig(r.fetch, "s13_requests"), final))
    for what, theirs, ours in want:
        r.flag(theirs is None or ours is None or theirs != ours, "C6", "INCONCLUSIVE" if None in (theirs, ours)
               else "FAIL", "%s %s, status.json %s" % (what, theirs, ours))
    high = max([dig(s, "s4_requests") for s in r.samples if isinstance(dig(s, "s4_requests"), int)] or [0])
    r.flag(isinstance(final, int) and high > final, "C6", "FAIL", "a status sample showed %s requests, final %s" % (high, final))

def capped(r, start, end):
    """Section 7: the pad sealed the run at the spend cap between start and end (None: open), so what a fault
    expectation in that window waits for may be cut short by the rig, not by goalcup."""
    s = r.cap_at
    return bool(s and start and s >= start and (end is None or s - SEAL_SLACK <= end))

def fail(r, cond, start, end, why):
    """A faults FAIL, or INCONCLUSIVE when the pad's seal at the spend cap falls in the expectation's window."""
    if cond:
        r.mark("faults", *(("INCONCLUSIVE", "sealed at the spend cap: " + why) if capped(r, start, end) else ("FAIL", why)))

def fired(r, *kinds):
    """The fault lines of a kill of one of these kinds that fired (a pid was hit)."""
    return [f for f in r.faults if f.get("kind") in kinds and f.get("pid") is not None]

def faults(r):
    """Section 3.1, for the two faults built so far; any other kind makes the row INCONCLUSIVE."""
    kinds = fault_kinds(r.meta)
    for k in sorted(set(kinds) - set(BUILT_FAULTS)):
        r.mark("faults", "INCONCLUSIVE", "expectations for %s not built yet" % k)
    if not set(kinds) & set(BUILT_FAULTS):
        return
    kills = fired(r, "kill-recorder")
    missed = [when(f.get("at")) for f in r.faults if f.get("kind") == "kill-recorder" and f.get("pid") is None]
    short = "fault did not fire: %d of %d kills" % (len(kills), kinds.count("kill-recorder"))
    if r.cap_at and any(t and t + SEAL_SLACK >= r.cap_at for t in missed):  # section 7: the pad's seal left nothing to kill
        short = "sealed at the spend cap: " + short
    r.flag(len(kills) < kinds.count("kill-recorder"), "faults", "INCONCLUSIVE", short)
    # The record's evidence around each kill: a status sample before and after it (the watch ran), and after a
    # recorder kill, written model requests up by at least 1 (section 1a).
    pairs = [(t, n) for t, n in ((when(s.get("pad_at")), dig(s, "s4_requests")) for s in r.samples) if t]
    times, reqs = [t for t, _ in pairs], [(t, n) for t, n in pairs if isinstance(n, int)]
    rec = [(f.get("result"), when(f.get("at"))) for f in r.faults if f.get("kind") == "wait-recovery"]
    killed = [t for t in (when(f.get("at")) for f in kills) if t]
    # Each kill's window runs to the end of the wait-recovery after it (open when unknown); a wait-recovery's from
    # the last kill before it. A record-only check (gaps, null_mode) is judged whatever the pad did after.
    for f in fired(r, "kill-recorder", "kill-watchdog"):
        k = when(f.get("at"))
        if not (k and any(t < k for t in times) and any(t > k for t in times)):
            r.mark("faults", "INCONCLUSIVE", "no status sample on both sides of the %s at %s" % (f["kind"], f.get("at")))
        elif f["kind"] == "kill-recorder" and "kill-recorder" in kinds:
            old, new = [n for t, n in reqs if t < k], [n for t, n in reqs if t > k]
            fail(r, old and new and max(new) <= max(old), k, next((e for _, e in rec if e is None or e >= k), None),
                 "no model request written after the kill at %s (status samples)" % f.get("at"))
    for got, end in rec:
        fail(r, got == "FAIL", max([k for k in killed if end is None or k <= end], default=None), end,
             "did not stand back up (wait-recovery)")
    res = [x for x, _ in rec]
    r.flag(kills and "kill-recorder" in kinds and (not res or set(res) - {"ok", "FAIL"}), "faults",
           "INCONCLUSIVE", "recovery not observed: wait-recovery %s" % (", ".join(map(str, res)) or "not run"))
    gaps, counts = r.gaps()
    ok = DOWN + ("after_stop",) + (("pinned",) if r.meta.get("case") == "dodger" else ())  # R2: seal; section 2: the dodger
    other = sorted(k for k, n in counts.items() if n and k.split(":")[0] not in ok)
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
    r.flag(len(gaps) != len(kills), "faults", "FAIL", "%d recorder_down gaps for %d kills" % (len(gaps), len(kills)))
    for a, b, ms in span:
        ms = ms if isinstance(ms, (int, float)) else (b - a).total_seconds() * 1000 if a and b else None
        r.flag(ms is None or ms >= 2000, "faults", "FAIL", "a recorder_down gap of %s ms, not under 2 s" % ms)
    r.flag(kills and dig(r.local, "s6_exit") != 1, "faults", "FAIL", "verifier exit %s, a kill row expects 1" % dig(r.local, "s6_exit"))

def recover(r, gaps):
    for kind in ("kill-watchdog", "kill-recorder"):  # PRD-10 acceptance 11 needs both kills
        if not fired(r, kind):
            return r.mark("faults", "INCONCLUSIVE", "fault did not fire: %s" % kind)
    if any("goalcup continue" in str(f.get("said")) for f in r.faults):
        return r.mark("faults", "INCONCLUSIVE", "harness lost its terminal")
    for what, name in (("harness pid", "s4_harness_pid"), ("port", "s4_port")):  # record-only: judged whatever came after
        r.flag(len({dig(s, name) for s in r.samples} - {None}) > 1, "faults", "FAIL", "the %s changed" % what)
    done = [f for f in r.faults if f.get("kind") == "recover"]
    if capped(r, when(fired(r, "kill-watchdog")[0].get("at")), when(done[1].get("at")) if len(done) > 1 else None):
        return r.mark("faults", "INCONCLUSIVE", "sealed at the spend cap between the watchdog kill and the second recover")
    r.flag(not any(f.get("kind") == "status" and STATUS_RECOVER in str(f.get("said")) for f in r.faults), "faults",
           "FAIL", "status did not show " + STATUS_RECOVER)
    r.flag(len(done) < 2, "faults", "INCONCLUSIVE", "%d of 2 recover outcomes logged" % len(done))
    for f, words in zip(done, RECOVER):
        r.flag(f.get("exit") != 0 or words not in str(f.get("said")), "faults", "FAIL",
               "recover at %s: exit %s, not the expected outcome" % (f.get("at"), f.get("exit")))
    r.flag(len(gaps) != 1, "faults", "FAIL", "%d recorder_down gaps, the recover row expects 1" % len(gaps))
    seal = dig(r.entry, "s6_seal") or dig(r.status, "s4_seal")
    if seal not in ("harness_exit", "deadline") and r.meta.get("over_cap") is True:  # section 7: pad sealed it
        return r.mark("faults", "INCONCLUSIVE", "sealed at the spend cap (seal reason %s)" % seal)
    r.flag(seal not in ("harness_exit", "deadline"), "faults", "FAIL", "seal reason %s" % seal)

def facts(r):
    u, e, m = r.usage or {}, r.entry, r.meta
    read, hist, size = dig(u, "s7_readable") is True, dig(e, "s6_history") or {}, dig(e, "s6_bytes") or {}
    threads = [t for t in dig(u, "s7_threads") or [] if isinstance(t, dict)]
    rss = [p.get("rss_kb") for x in load(r.folder, "watch/procs.jsonl", 1) for p in x.get("procs") or []
           if isinstance(p, dict) and p.get("role") == "recorder" and isinstance(p.get("rss_kb"), (int, float))]
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
            "raw_bytes": size.get("raw"), "compressed_bytes": size.get("compressed"), "over_cap": m.get("over_cap") is True,
            "recorder_rss_peak_mb": round(max(rss) / 1024, 1) if rss else None, "spend_usd_est": m.get("spend_usd_est"),
            "priced_high": m.get("priced_high") or [], "first_live_alarm": (alarm or [None])[0],
            "push": m.get("push"), "wire_only": dig(u, "s7_wire_only"), "local_log_reason": dig(u, "s7_reason")}

def watcher(r):
    """Section 3.1: owner lines that need a human read, for report.txt only, never a check."""
    out = ["%s %s exit %s: %s" % (f.get("kind"), f.get("script"), f.get("exit"), "output withheld (known values unreadable)"
                                  if f.get("withheld") else "read exec/%s.txt" % f.get("script"))
           for f in r.faults if f.get("kind") in WATCHER and f.get("exit") != 0]
    if r.meta.get("case") != "dodger":
        return out
    return out + ["the start event's hosts_sha256 must equal meta.json's %s" % r.meta.get("hosts_sha256"),
                  "the start event's hosts must show %s pinned" % r.meta.get("model_host"),
                  "the launch block must show: Host list from a file: <path> (not verified).",
                  "the end screen must name %s under 'Not recorded'" % r.meta.get("model_host"),
                  "usage must say %s was not recorded on the wire" % r.meta.get("model_host")]

def verdict(folder, script):
    """Section 3.1: the first word of the watcher's exec/<script>.verdict when PASS or FAIL, else pending."""
    try:
        with open(os.path.join(folder, "exec", script + ".verdict")) as f:
            words = f.read().split()
    except OSError:
        return "pending"
    return words[0] if words and words[0] in ("PASS", "FAIL") else "pending"

def evaluate(folder):
    meta = load(folder, "meta.json")
    if not isinstance(meta, dict):
        return None
    r = Run(folder, meta)
    judge(r)
    fault = r.v["faults"] if judged(meta) else None
    got = [r.v[c] for c in CHECKS] + ([fault] if fault else [])
    row = {k: meta.get(k) for k in ("connector", "harness_version", "case", "mode", "prompt", "variant", "where",
                                    "server", "image", "run", "entry", "started", "minutes", "pad_trusted_at")}
    row.update(os=None, goalcup=None, checks={c: r.v[c] for c in CHECKS}, faults=fault, why=r.why,
               exec={s: verdict(folder, s) for _, s in watchers(meta)}, notes=watcher(r),
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
    if fault_kinds(row):
        lines.append("for the watcher: read goalcup.log and the panes for a failed call the harness did not retry")
    lines += ["for the watcher: " + n for n in row["notes"]]
    for name, text in (("row.json", json.dumps(row, sort_keys=True, indent=1)), ("report.txt", "\n".join(lines))):
        with open(os.path.join(argv[1], name), "w") as f:
            f.write(text + "\n")
    print("\n".join(lines + ["PADCHECK " + json.dumps({k: row[k] for k in ("result", "checks", "why")}, sort_keys=True)]))
    return 0

if __name__ == "__main__":
    sys.exit(main(sys.argv))
