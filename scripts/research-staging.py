#!/usr/bin/env python3
"""Read-only measurement of one event's uploads, as aggregates: chunk sizes and the compression
ratio, ACK lag, heartbeat health. For WP-0 on staging; also an event dashboard on production.

    python3 scripts/research-staging.py [--event ID] [--since WHEN] [--json] [--top N]
                                        [--bucket NAME] [--allow-production] [--no-r2]

  --event ID          the event (default test-staging; the staging dry run is goalcup-dryrun-staging)
  --since WHEN        only runs created at or after WHEN: an RFC 3339 time, a date, or 24h / 7d
  --json              one JSON document instead of the text report
  --top N             how many worst entries to list for ACK lag (default 5)
  --bucket NAME       the R2 bucket (default goalcup-traces-staging)
  --allow-production  allow the production bucket goalcup-traces and a production event (still
                      read-only: a SELECT in a READ ONLY transaction, and R2 list requests)
  --no-r2             skip the R2 listing (no compressed sizes; needs no Cloudflare credentials)

Needs psql and, in the environment, DATABASE_URL (a Postgres URL; give it a read-only role) and,
unless --no-r2, CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN (an R2 token with read access).
The queries are scripts/research-staging.sql, sent on psql's stdin; the database connection
reaches psql through a private service file, never its command line. The Cloudflare token goes
only into the Authorization header of the list request. Neither is ever printed.

The database stores each chunk's logical (uncompressed chain) length but not its compressed size;
the compressed size is the R2 object's size, from listing traces/<event>/ and joining on
chunks.r2_key. Output is aggregates only; run and entry ids are cut to 8 characters, and no
member id, email, IP address, repo URL or R2 key is printed.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

SQL_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "research-staging.sql")
STAGING_BUCKET = "goalcup-traces-staging"
PRODUCTION_BUCKET = "goalcup-traces"
DEFAULT_EVENT = "test-staging"
EVENT_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")  # events.id CHECK, upload SQL line 44
BUCKET_RE = re.compile(r"^[a-z0-9][a-z0-9-]{1,62}$")
ACCOUNT_RE = re.compile(r"^[A-Za-z0-9]{1,32}$")
SILENT_SECONDS = 600
MAX_PAGES = 100000

NOT_STORED = [
    "Refused chunks: not stored. commit_chunk answers not_found, closed, beyond_final or conflict "
    "without writing a row (upload SQL 1353-1393); only an R2 object stored before a refused commit "
    "remains (counted below as chunk objects without an index row).",
    "Quotas: no per-run size, quota or stored-bytes column exists (WP-D would add one).",
    "HTTP error codes: not in the database; they are only in Worker logs.",
    "Heartbeat stop answers (stop, reason) are computed per request and not stored; runs.status, "
    "runs.flag_reason, heartbeats.recording and run emails (notices) are.",
]


class Fail(Exception):
    pass


# ---- secrets -------------------------------------------------------------------------------


def secrets_of(env):
    """Every value that must never be printed or passed on a command line."""
    out = []
    for name in ("DATABASE_URL", "CLOUDFLARE_API_TOKEN"):
        value = env.get(name)
        if value:
            out.append(value)
    url = env.get("DATABASE_URL") or ""
    if url.startswith(("postgres://", "postgresql://")):
        try:
            password = urllib.parse.urlsplit(url).password
        except ValueError:
            password = None
        if password:
            out += [password, urllib.parse.unquote(password)]
    return sorted({s for s in out if len(s) >= 4}, key=len, reverse=True)


def scrub(text, secrets):
    for s in secrets:
        text = text.replace(s, "[redacted]")
    return text


# ---- the queries ---------------------------------------------------------------------------

WRITE_WORDS = re.compile(
    r"\b(insert|update|delete|merge|truncate|alter|create|drop|grant|revoke|copy|into|call|lock|"
    r"execute|prepare|vacuum|reindex|cluster|refresh|listen|notify|commit|savepoint|release)\b",
    re.IGNORECASE,
)


def statements(sql):
    """The statements of the SQL text, without comments, lower case, whitespace collapsed."""
    body = re.sub(r"--[^\n]*", "", sql)
    return [" ".join(s.split()).lower() for s in body.split(";") if s.strip()]


def check_read_only(sql):
    """Refuse any SQL that is not one READ ONLY transaction of SELECTs that ends in ROLLBACK."""
    stmts = statements(sql)
    if not stmts or not (stmts[0].startswith("begin") and "read only" in stmts[0]):
        raise Fail("the SQL must begin with BEGIN ... READ ONLY")
    if stmts[-1] != "rollback":
        raise Fail("the SQL must end with ROLLBACK")
    for s in stmts[1:-1]:
        if s.startswith(("select", "with")) or s == "set transaction read only" or s.startswith("set local statement_timeout"):
            continue
        raise Fail(f"the SQL holds a statement that is not a query: {s[:40]!r}")
    if any(line.lstrip().startswith("\\") for line in re.sub(r"--[^\n]*", "", sql).splitlines()):
        raise Fail("the SQL holds a psql meta-command")
    m = WRITE_WORDS.search(re.sub(r"--[^\n]*", "", sql))
    if m:
        raise Fail(f"the SQL holds the word {m.group(1)!r}")
    return sql


def service_file(url, path):
    """Write the connection as a libpq service file readable by the owner only (as fetch-run.sh)."""
    kept = ("sslmode", "sslrootcert", "sslcert", "sslkey", "connect_timeout", "application_name")
    pairs = {}
    if url.startswith(("postgres://", "postgresql://")):
        u = urllib.parse.urlsplit(url)
        for key, value in (("host", u.hostname), ("port", u.port), ("user", u.username),
                           ("password", u.password), ("dbname", u.path.lstrip("/"))):
            if value not in (None, ""):
                pairs[key] = urllib.parse.unquote(str(value))
        for key, value in urllib.parse.parse_qsl(u.query):
            if key in kept:
                pairs[key] = value
    else:  # a "host=... user=..." connection string
        for word in shlex.split(url):
            key, _, value = word.partition("=")
            pairs[key] = value
    if any("\n" in v for v in pairs.values()):
        raise Fail("DATABASE_URL has a newline in it")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write("[researchstaging]\n" + "".join(f"{k}={v}\n" for k, v in pairs.items()))


def run_psql(argv, sql, env):
    """The real psql: the SQL on stdin, the answer on stdout."""
    p = subprocess.run(argv, input=sql, env=env, capture_output=True, text=True)
    if p.returncode != 0:
        raise Fail("psql failed: " + (p.stderr.strip() or f"exit {p.returncode}")[-800:])
    return p.stdout


def query_database(env, event, since, sql, runner=run_psql):
    """Run the queries; return their rows, grouped by section."""
    url = env.get("DATABASE_URL")
    if not url:
        raise Fail("set DATABASE_URL (a read-only Postgres URL)")
    tmp = tempfile.mkdtemp(prefix="research-staging.")
    try:
        conf = os.path.join(tmp, "pg_service.conf")
        service_file(url, conf)
        child = {k: v for k, v in env.items() if k not in ("DATABASE_URL", "CLOUDFLARE_API_TOKEN")}
        child.update(PGSERVICEFILE=conf, PGSERVICE="researchstaging")
        argv = ["psql", "-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1",
                "-v", f"event={event}", "-v", f"since={since}"]
        out = runner(argv, sql, child)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    rows = {}
    for line in out.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        row = json.loads(line)
        rows.setdefault(row.pop("section"), []).append(row)
    return rows


# ---- R2 ------------------------------------------------------------------------------------


def r2_list_url(account, bucket, prefix, cursor):
    """Cloudflare's R2 list-objects endpoint, the one place its shape is assumed:
    GET /client/v4/accounts/{account}/r2/buckets/{bucket}/objects?prefix=&per_page=&cursor=
    answering {success, errors, result: [{key, size, ...}], result_info: {cursor, is_truncated}}
    (as server/worker/scripts/delete-test-runs.sh reads it)."""
    query = {"prefix": prefix, "per_page": "1000"}
    if cursor:
        query["cursor"] = cursor
    return (f"https://api.cloudflare.com/client/v4/accounts/{account}/r2/buckets/{bucket}/objects?"
            + urllib.parse.urlencode(query))


def fetch_json(url, token):
    """GET with the token in the Authorization header only."""
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}", "Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        raise Fail(f"R2 list answered HTTP {e.code}") from None
    except urllib.error.URLError as e:
        raise Fail(f"R2 list failed: {e.reason}") from None


def list_objects(account, bucket, prefix, token, fetch=fetch_json):
    """Every object under prefix, as {key: size in bytes}, following the cursor to the end."""
    objects, cursor, seen = {}, None, set()
    for _ in range(MAX_PAGES):
        page = fetch(r2_list_url(account, bucket, prefix, cursor), token)
        if not isinstance(page, dict) or not page.get("success"):
            codes = [e.get("code") for e in (page.get("errors") or []) if isinstance(e, dict)] if isinstance(page, dict) else []
            raise Fail(f"R2 list did not succeed (error codes {codes or 'none'})")
        for o in page.get("result") or []:
            key, size = o.get("key"), o.get("size")
            if not isinstance(key, str) or not key.startswith(prefix):
                raise Fail("R2 list answered a key outside the prefix")
            try:
                objects[key] = int(size)
            except (TypeError, ValueError):
                raise Fail("R2 list answered an object without a size") from None
        info = page.get("result_info") or {}
        cursor = info.get("cursor")
        if not info.get("is_truncated") or not cursor:
            return objects
        if cursor in seen:
            raise Fail("R2 list repeated a cursor")
        seen.add(cursor)
    raise Fail("R2 list did not end")


# ---- arithmetic ----------------------------------------------------------------------------


def pct(values, p):
    """Nearest-rank percentile (as percentile_disc): None for no values."""
    if not values:
        return None
    s = sorted(values)
    return s[max(0, math.ceil(p * len(s)) - 1)]


def dist(values):
    return {"n": len(values), "p50": pct(values, 0.5), "p90": pct(values, 0.9),
            "max": max(values) if values else None}


def ratio(logical, compressed):
    return round(logical / compressed, 3) if compressed else None


def short(ident):
    return ident[:8] if isinstance(ident, str) else ident


def num(v):
    return None if v is None else float(v)


def analyze(rows, objects, prefix, top=5):
    """Every aggregate from the database rows and the R2 listing (None: no listing)."""
    event = (rows.get("event") or [None])[0]
    now = num(event["now"]) if event else None
    runs = rows.get("run", [])
    entries = rows.get("entry", [])
    chunks = rows.get("chunk", [])
    run_ids = {r["run"] for r in runs}
    have_r2 = objects is not None

    per_entry, per_run = {}, {}
    logical_all, compressed_all, ratio_all = [], [], []
    missing = 0
    keys = set()
    for c in chunks:
        keys.add(c["key"])
        size = objects.get(c["key"]) if have_r2 else None
        logical_all.append(c["length"])
        for bucket_, ident in ((per_entry, c["entry"]), (per_run, c["run"])):
            a = bucket_.setdefault(ident, {"chunks": 0, "logical": 0, "joined_logical": 0, "compressed": 0,
                                           "missing": 0, "first_at": None, "last_at": None})
            a["chunks"] += 1
            a["logical"] += c["length"]
            at = num(c["at"])
            a["first_at"] = at if a["first_at"] is None else min(a["first_at"], at)
            a["last_at"] = at if a["last_at"] is None else max(a["last_at"], at)
            if size is not None:
                a["joined_logical"] += c["length"]
                a["compressed"] += size
            elif have_r2:
                a["missing"] += 1
        if size is not None:
            compressed_all.append(size)
            if size > 0:
                ratio_all.append(c["length"] / size)
        elif have_r2:
            missing += 1

    def sizes(a):
        d = {"chunks": a["chunks"], "logical_bytes": a["logical"]}
        if have_r2:
            d.update(compressed_bytes=a["compressed"], ratio=ratio(a["joined_logical"], a["compressed"]),
                     chunks_missing_object=a["missing"])
        return d

    empty = {"chunks": 0, "logical": 0, "joined_logical": 0, "compressed": 0, "missing": 0, "first_at": None, "last_at": None}

    run_out = []
    for r in runs:
        a = per_run.get(r["run"], empty)
        d = {"run": short(r["run"]), "mode": r["mode"], "status": r["status"]}
        d.update(sizes(a))
        start = num(r["official_start"])
        hours = (a["last_at"] - start) / 3600 if a["last_at"] is not None and start is not None else None
        if hours is not None and hours < 1 / 60:
            hours = None
        d["active_hours"] = round(hours, 3) if hours else None
        d["logical_bytes_per_hour"] = round(a["logical"] / hours) if hours else None
        if have_r2:
            d["compressed_bytes_per_hour"] = round(a["compressed"] / hours) if hours else None
        last = num(r["last_beat_at"])
        d.update(heartbeats=r["heartbeats"], max_gap_s=num(r["max_gap_s"]), p50_gap_s=num(r["p50_gap_s"]),
                 p90_gap_s=num(r["p90_gap_s"]),
                 silent_s=round(now - last) if last is not None and now is not None else None,
                 flag_reason=r["flag_reason"])
        run_out.append(d)

    entry_out, lags_now, lags_beat = [], [], []
    for e in entries:
        a = per_entry.get(e["entry"], empty)
        d = {"entry": short(e["entry"]), "run": short(e["run"]), "segment": e["segment"]}
        d.update(sizes(a))
        lag_now = lag_beat = None
        if e["offset_local"] is not None:
            lag_now = e["offset_local"] - e["upload_offset"]
            lag_beat = e["offset_local"] - e["acked_at_beat"]
            lags_now.append(lag_now)
            lags_beat.append(lag_beat)
        beat = num(e["beat_at"])
        d.update(upload_offset=e["upload_offset"], ack_lag_now=lag_now, ack_lag_at_beat=lag_beat,
                 beat_age_s=round(now - beat) if beat is not None and now is not None else None,
                 recording=e["recording"], sealed=e["sealed"])
        entry_out.append(d)
    worst = sorted((d for d in entry_out if d["ack_lag_at_beat"] is not None),
                   key=lambda d: (d["ack_lag_at_beat"], d["ack_lag_now"]), reverse=True)[:top]

    overall = {"runs": len(runs), "entries": len(entries), "chunks": len(chunks),
               "logical_bytes": sum(logical_all), "chunk_logical": dist(logical_all)}
    if have_r2:
        joined_logical = sum(a["joined_logical"] for a in per_entry.values())
        overall.update(compressed_bytes=sum(compressed_all), ratio=ratio(joined_logical, sum(compressed_all)),
                       chunks_missing_object=missing, chunk_compressed=dist(compressed_all),
                       chunk_ratio=dist([round(x, 3) for x in ratio_all]))

    r2 = None
    if have_r2:
        in_scope = {k: v for k, v in objects.items() if k[len(prefix):].split("/", 1)[0] in run_ids}
        unindexed = {k: v for k, v in in_scope.items() if k not in keys and k.endswith(".zst")}
        docs = {k: v for k, v in in_scope.items() if k not in keys and re.search(r"/(seal|baseline)-[0-9a-f]{64}\.json$", k)}
        other = len(in_scope) - len(in_scope.keys() & keys) - len(unindexed) - len(docs)
        r2 = {"objects_listed": len(objects), "objects_in_scope": len(in_scope),
              "chunk_objects_without_index_row": len(unindexed), "unindexed_bytes": sum(unindexed.values()),
              "documents": len(docs), "document_bytes": sum(docs.values()), "other_objects": other}

    def count(items, key):
        out = {}
        for i in items:
            k = str(i.get(key))
            out[k] = out.get(k, 0) + 1
        return dict(sorted(out.items()))

    gaps = [num(r["max_gap_s"]) for r in runs if r["max_gap_s"] is not None]
    open_live = [d for d, r in zip(run_out, runs) if r["status"] == "open" and now is not None and now < num(r["deadline_at"])]
    heart = {
        "runs_beating_10m": sum(1 for d in run_out if d["silent_s"] is not None and d["silent_s"] <= SILENT_SECONDS),
        "runs_open_silent_10m": sum(1 for d in open_live if d["silent_s"] is None or d["silent_s"] > SILENT_SECONDS),
        "max_gap_s": dist(gaps),
        "runs_with_gap_over_10m": sum(1 for g in gaps if g > SILENT_SECONDS),
        "runs_by_status": count(runs, "status"),
        "runs_by_mode": count(runs, "mode"),
        "runs_flagged": count([r for r in runs if r["flag_reason"]], "flag_reason"),
        "runs_abandoned_with_reason": sum(1 for r in runs if r["abandon_reason_given"]),
        "latest_recording_by_entry": count([e for e in entries if e["beat_at"] is not None], "recording"),
        "entries_final_beat": sum(1 for e in entries if e["beat_final"]),
        "entries_sealed": sum(1 for e in entries if e["sealed"]),
    }
    ack = {"entries_with_heartbeat": len(lags_now), "now": dist(lags_now), "at_heartbeat": dist(lags_beat),
           "negative_now": sum(1 for x in lags_now if x < 0), "worst": worst}
    return {
        "event": {k: event[k] for k in ("id", "mode", "staging", "run_hours")} if event else None,
        "overall": overall, "r2": r2, "ack_lag": ack, "heartbeats": heart,
        "notices": rows.get("notice", []), "not_stored": NOT_STORED,
        "db_summary": (rows.get("summary") or [None])[0],
        "runs": run_out, "entries": entry_out,
    }


# ---- text ----------------------------------------------------------------------------------


def human(n):
    if n is None:
        return "-"
    n = float(n)
    for unit in ("B", "KiB", "MiB", "GiB"):
        if abs(n) < 1024 or unit == "GiB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return str(n)


def fmt_dist(d, f=human):
    return f"p50 {f(d['p50'])}  p90 {f(d['p90'])}  max {f(d['max'])}  (n={d['n']})"


def text_report(rep, scope):
    L = []
    ev = rep["event"]
    L.append(f"event {ev['id']} ({ev['mode']}, {'staging' if ev['staging'] else 'production'})  {scope}")
    o = rep["overall"]
    L.append(f"overall: {o['runs']} runs, {o['entries']} entries, {o['chunks']} chunks, logical {human(o['logical_bytes'])}"
             + (f", compressed {human(o['compressed_bytes'])}, ratio {o['ratio'] or '-'}x" if "ratio" in o else ""))
    L.append(f"  chunk logical:    {fmt_dist(o['chunk_logical'])}")
    if "chunk_compressed" in o:
        L.append(f"  chunk compressed: {fmt_dist(o['chunk_compressed'])}")
        L.append(f"  chunk ratio:      {fmt_dist(o['chunk_ratio'], lambda x: '-' if x is None else f'{x}x')}")
        L.append(f"  chunks with no R2 object: {o['chunks_missing_object']}")
    if rep["r2"]:
        r = rep["r2"]
        L.append(f"r2: {r['objects_listed']} objects listed, {r['objects_in_scope']} in scope; "
                 f"{r['chunk_objects_without_index_row']} chunk objects without an index row ({human(r['unindexed_bytes'])}); "
                 f"{r['documents']} seal/baseline documents; {r['other_objects']} other")
    else:
        L.append("r2: not listed (--no-r2): no compressed sizes")
    L.append("runs:")
    for d in rep["runs"]:
        L.append(f"  {d['run']} {d['mode']:<11} {d['status']:<10} {d['chunks']:>6} chunks  {human(d['logical_bytes']):>10}"
                 + (f"  -> {human(d['compressed_bytes']):>10} ({d['ratio'] or '-'}x)" if "ratio" in d else "")
                 + f"  {human(d['logical_bytes_per_hour'])}/h over {d['active_hours'] or '-'} h"
                 + f"  beats {d['heartbeats']}  max gap {d['max_gap_s'] if d['max_gap_s'] is not None else '-'} s"
                 + f"  silent {d['silent_s'] if d['silent_s'] is not None else '-'} s")
    a = rep["ack_lag"]
    L.append(f"ack lag (latest heartbeat per entry, {a['entries_with_heartbeat']} entries):")
    L.append(f"  vs bytes held now:      {fmt_dist(a['now'])}  ({a['negative_now']} negative: uploaded since)")
    L.append(f"  vs bytes held at beat:  {fmt_dist(a['at_heartbeat'])}")
    for d in a["worst"]:
        L.append(f"  worst {d['entry']} (run {d['run']}): {human(d['ack_lag_at_beat'])} at beat, {human(d['ack_lag_now'])} now, "
                 f"beat {d['beat_age_s']} s ago, recording {d['recording']}")
    h = rep["heartbeats"]
    L.append(f"heartbeats: {h['runs_beating_10m']} runs beating in the last 10 min; {h['runs_open_silent_10m']} open runs silent > 10 min; "
             f"{h['runs_with_gap_over_10m']} runs with a gap > 10 min")
    L.append(f"  max gap per run: {fmt_dist(h['max_gap_s'], lambda x: '-' if x is None else f'{x:.0f} s')}")
    L.append(f"  runs by status {h['runs_by_status']}, by mode {h['runs_by_mode']}, flagged {h['runs_flagged']}, "
             f"abandoned with a reason {h['runs_abandoned_with_reason']}")
    L.append(f"  latest recording by entry {h['latest_recording_by_entry']}; final beats {h['entries_final_beat']}; sealed {h['entries_sealed']}")
    if rep["notices"]:
        L.append("notices: " + ", ".join(f"{n['kind']}/{n['state']} {n['count']}" for n in rep["notices"]))
    else:
        L.append("notices: none")
    L.append("not stored in the database:")
    L.extend(f"  - {s}" for s in rep["not_stored"])
    return "\n".join(L) + "\n"


# ---- main ----------------------------------------------------------------------------------


def parse_since(value, now=None):
    now = now or datetime.now(timezone.utc)
    m = re.fullmatch(r"(\d{1,5})([hd])", value)
    if m:
        n = int(m.group(1))
        t = now - (timedelta(hours=n) if m.group(2) == "h" else timedelta(days=n))
    else:
        try:
            t = datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            raise argparse.ArgumentTypeError("--since takes an RFC 3339 time, a date, or 24h / 7d") from None
        if t.tzinfo is None:
            t = t.replace(tzinfo=timezone.utc)
    return t.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def event_id(value):
    if not EVENT_RE.match(value):
        raise argparse.ArgumentTypeError("an event id is lowercase letters, digits and dashes")
    return value


def parser():
    p = argparse.ArgumentParser(prog="research-staging.py", description=__doc__.split("\n\n")[0])
    p.add_argument("--event", type=event_id, default=DEFAULT_EVENT)
    p.add_argument("--since", type=parse_since, default=None)
    p.add_argument("--json", action="store_true")
    p.add_argument("--top", type=int, default=5)
    p.add_argument("--bucket", default=STAGING_BUCKET)
    p.add_argument("--allow-production", action="store_true")
    p.add_argument("--no-r2", action="store_true")
    p.add_argument("--sql", default=SQL_PATH, help=argparse.SUPPRESS)
    return p


def main(argv=None, env=None, psql=run_psql, fetch=fetch_json, stdout=None, stderr=None):
    env = dict(os.environ if env is None else env)
    stdout, stderr = stdout or sys.stdout, stderr or sys.stderr
    secrets = secrets_of(env)
    try:
        args = parser().parse_args(argv)
        if not BUCKET_RE.match(args.bucket):
            raise Fail("bad bucket name")
        if args.bucket != STAGING_BUCKET and not args.allow_production:
            raise Fail(f"refusing bucket {args.bucket}: only {STAGING_BUCKET} without --allow-production")
        if not args.no_r2:
            for name in ("CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN"):
                if not env.get(name):
                    raise Fail(f"set {name}, or pass --no-r2")
            if not ACCOUNT_RE.match(env["CLOUDFLARE_ACCOUNT_ID"]):
                raise Fail("bad CLOUDFLARE_ACCOUNT_ID")
        with open(args.sql) as f:
            sql = check_read_only(f.read())
        since = args.since or "-infinity"
        rows = query_database(env, args.event, since, sql, runner=psql)
        event = (rows.get("event") or [None])[0]
        if event is None:
            raise Fail(f"no event {args.event} in the database")
        if not event["staging"] and not args.allow_production:
            raise Fail(f"event {args.event} is a production event: pass --allow-production (and --bucket {PRODUCTION_BUCKET})")
        prefix = f"traces/{args.event}/"
        objects = None
        if not args.no_r2:
            if event["staging"] != (args.bucket == STAGING_BUCKET):
                raise Fail(f"event {args.event} is {'staging' if event['staging'] else 'production'}; "
                           f"its objects are not in bucket {args.bucket}")
            objects = list_objects(env["CLOUDFLARE_ACCOUNT_ID"], args.bucket, prefix, env["CLOUDFLARE_API_TOKEN"], fetch)
        rep = analyze(rows, objects, prefix, top=max(0, args.top))
        rep["scope"] = {"since": args.since, "bucket": None if args.no_r2 else args.bucket}
        if args.json:
            text = json.dumps(rep, indent=2, sort_keys=True) + "\n"
        else:
            text = text_report(rep, f"since {args.since or 'the start'}" + ("" if args.no_r2 else f", bucket {args.bucket}"))
        stdout.write(scrub(text, secrets))
        return 0
    except Fail as e:
        stderr.write(scrub(f"research-staging: {e}\n", secrets))
        return 1


if __name__ == "__main__":
    sys.exit(main())
