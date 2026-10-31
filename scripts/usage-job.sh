#!/bin/sh
# The organizer usage job (PRD-09 sections 1 and 7): for every sealed entry of a run that has no usage
# object, or one made by an older reader, fetch the server copy, decode it and put the usage objects.
#
#   sh scripts/usage-job.sh [--box NAME] [--entry E] [--dry-run] [--index FILE [--event EV]] [--store DIR] [--usage-store DIR]
#
# Puts usage/{event}/{run}/{entry}.json, {entry}.calls.jsonl and run.json into the bucket named by
# GOALCUP_USAGE_BUCKET (default goalcup-usage; goalcup-usage-staging reads the staging runs only).
# A run is done whole: any stale entry re-reads all of the run's sealed entries. --entry E does only
# E's run and puts E's objects whatever is there (exit 1 if E is not a sealed entry). run.json merges the
# entries: a thread that spans a `continue` has its counts summed over the entries, and takes the
# session's whole native values from its last entry (PRD-09 slice 5). An object made by a newer reader
# (entry objects and run.json alike) is never overwritten, except by --entry. --box NAME decodes in that
# build box ($GOALCUP_BOX_CLI copy in, exec, copy out) and takes the reader version from it;
# no credential enters it. Without it, goalcup-verify (or $GOALCUP_VERIFY) runs here.
# Credentials, from the environment only: DATABASE_URL (read-only) and CLOUDFLARE_ACCOUNT_ID with a read
# token for the traces bucket (fetch-run.sh), and GOALCUP_USAGE_TOKEN (read and write on the usage
# bucket only). None reaches argv or output. --index and --store are fetch-run.sh's index and store; each
# index element also needs "event" (or pass --event EV), and "staging" is honoured when present.
# --usage-store DIR stands in for the usage bucket. --dry-run puts nothing. Exit 1 on a bad fetch or
# decode, 0 otherwise.
set -eu
set +x
umask 077

box= only= dry= index= store= ustore= evarg=
while [ $# -gt 0 ]; do
  case $1 in
    --box) box=${2:?--box needs a name}; shift ;;
    --entry) only=${2:?--entry needs an entry id}; shift ;;
    --index) index=${2:?--index needs a file}; shift ;;
    --event) evarg=${2:?--event needs an event id}; shift ;;
    --store) store=${2:?--store needs a directory}; shift ;;
    --usage-store) ustore=${2:?--usage-store needs a directory}; shift ;;
    --dry-run) dry=1 ;;
    -h | --help) sed -n '2,/^[^#]/{/^#/s/^# \{0,1\}//p;}' "$0"; exit 0 ;;
    *) echo "usage-job: unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
here=$(cd "$(dirname "$0")" && pwd)
gv=${GOALCUP_VERIFY:-goalcup-verify}
bx=${GOALCUP_BOX_CLI:-sandbox}  # the conductor tool with `copy SRC DST` and `exec BOX -- CMD`; box:path names a path in a box
ub=${GOALCUP_USAGE_BUCKET:-goalcup-usage}
case $ub in '' | *[!A-Za-z0-9._-]*) echo "usage-job: bad bucket name" >&2; exit 2 ;; esac
case $ub in *-staging) tb=goalcup-traces-staging staging=true ;; *) tb=goalcup-traces staging=false ;; esac
[ -n "$ustore" ] || : "${GOALCUP_USAGE_TOKEN:?set GOALCUP_USAGE_TOKEN, or pass --usage-store DIR}" "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
case ${CLOUDFLARE_ACCOUNT_ID:-0} in *[!A-Za-z0-9]*) echo "usage-job: bad CLOUDFLARE_ACCOUNT_ID" >&2; exit 2 ;; esac
[ -n "$index" ] || [ -n "${DATABASE_URL:-}" ] || { echo "usage-job: set DATABASE_URL, or pass --index FILE" >&2; exit 2; }
w=$(mktemp -d) || exit 1
trap 'rm -rf "$w"' EXIT
trap 'exit 1' HUP INT TERM

# get KEY DEST: 0 fetched, 1 absent. put KEY FILE. The token reaches curl only in its configuration on stdin.
curl_obj() {
  printf 'url = "https://api.cloudflare.com/client/v4/accounts/%s/r2/buckets/%s/objects/%s"\nheader = "Authorization: Bearer %s"\nsilent\nshow-error\nwrite-out = "%%{http_code}"\n%s\n' \
    "$CLOUDFLARE_ACCOUNT_ID" "$ub" "$(printf %s "$1" | sed 's|/|%2F|g')" "$GOALCUP_USAGE_TOKEN" "$2" | curl -K -
}
get() {
  if [ -n "$ustore" ]; then [ -f "$ustore/$1" ] && cp "$ustore/$1" "$2"; return; fi
  s=$(curl_obj "$1" "output = \"$2\"") || { echo "usage-job: cannot read $1" >&2; exit 1; }
  [ "$s" = 200 ] && return 0
  rm -f "$2"; [ "$s" = 404 ] || { echo "usage-job: reading $1 gave HTTP $s" >&2; exit 1; }
  return 1
}
put() {
  if [ -n "$dry" ]; then echo "would put $1"; return; fi
  if [ -n "$ustore" ]; then mkdir -p "$ustore/$(dirname "$1")"; cp "$2" "$ustore/$1"
  else
    s=$(curl_obj "$1" "request = \"PUT\"
upload-file = \"$2\"") && [ "$s" = 200 ] || { echo "usage-job: putting $1 failed" >&2; exit 1; }
  fi
  echo "put $1"
}
# newer KEY READER: 0 when the object at KEY was made by READER or a newer reader.
newer() {
  get "$1" "$w/old.json" || return 1
  python3 -I - "$w/old.json" "$2" <<'PY'
import json, sys
v = lambda s: tuple(int(x) for x in str(s).split(".") if x.isdigit())
sys.exit(0 if v(json.load(open(sys.argv[1])).get("reader", "0")) >= v(sys.argv[2]) else 1)
PY
}

# 1. Sealed entries (event run entry), from the database or the index file.
if [ -n "$index" ]; then
  python3 -I - "$index" "$w" "$staging" "$evarg" <<'PY' >"$w/all" || exit 1
import json, sys
index, w, staging, evarg = sys.argv[1:5]
for e in json.load(open(index)):
    if e.get("sealed_at") and str(e.get("staging", staging == "true")).lower() == staging:
        ev = e.get("event") or evarg
        if not ev:
            sys.exit("usage-job: the index has no event; pass --event EV")
        open(f"{w}/idx-{e['entry_id']}.json", "w").write(json.dumps([e]))
        print(ev, e["run_id"], e["entry_id"])
PY
else
  # The URL goes into a private service file, not onto psql's command line (where `ps` shows it).
  UJ_URL=$DATABASE_URL python3 -I - "$w/pg_service.conf" <<'PY'
import os, shlex, sys, urllib.parse
url = os.environ["UJ_URL"]
kept = ("sslmode", "sslrootcert", "sslcert", "sslkey", "connect_timeout", "application_name")
pairs = {}
if url.startswith(("postgres://", "postgresql://")):
    u = urllib.parse.urlsplit(url)
    for key, value in (("host", u.hostname), ("port", u.port), ("user", u.username),
                       ("password", u.password), ("dbname", u.path.lstrip("/"))):
        if value not in (None, ""):
            pairs[key] = urllib.parse.unquote(str(value))
    pairs.update((k, v) for k, v in urllib.parse.parse_qsl(u.query) if k in kept)
else:
    for word in shlex.split(url):
        key, _, value = word.partition("=")
        pairs[key] = value
if any("\n" in v for v in pairs.values()):
    sys.exit("usage-job: DATABASE_URL has a newline in it")
fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w") as f:
    f.write("[usagejob]\n" + "".join(f"{k}={v}\n" for k, v in pairs.items()))
PY
  PGSERVICEFILE="$w/pg_service.conf" PGSERVICE=usagejob psql -X -q -At -F ' ' -v ON_ERROR_STOP=1 -v staging="$staging" >"$w/all" <<'SQL'
BEGIN READ ONLY;
SELECT ev.id, e.run_id, e.id FROM goal_cup_tracer.entries e
JOIN goal_cup_tracer.runs r ON r.id = e.run_id JOIN goal_cup_tracer.events ev ON ev.id = r.event_id
WHERE e.sealed_at IS NOT NULL AND ev.staging = :'staging' ORDER BY e.run_id, e.segment;
ROLLBACK;
SQL
fi
if [ -n "$box" ]; then v=$($bx exec "$box" -- "$gv" --version </dev/null) || v=
else v=$("$gv" --version </dev/null) || v=; fi
reader=$(printf %s "$v" | awk 'NF{x=$NF} END{print x}')
[ -n "$reader" ] || { echo "usage-job: cannot read the reader version from $gv" >&2; exit 1; }
runs=$(awk '{print $2}' "$w/all" | sort -u)
if [ -n "$only" ]; then
  runs=$(awk -v e="$only" '$3 == e {print $2}' "$w/all")
  [ -n "$runs" ] || { echo "usage-job: $only is not a sealed entry in this listing" >&2; exit 1; }
fi
fail=0
for run in $runs; do
  stale= ev=; : >"$w/puts"
  while read -r e r en; do
    [ "$r" = "$run" ] || continue
    ev=$e
    if [ -n "$only" ] || ! newer "usage/$e/$r/$en.json" "$reader"; then stale=1; echo "$en" >>"$w/puts"; fi
  done <"$w/all"
  # run.json is put last: a run whose put failed after the entries is stale on its own.
  putrun=; if [ -n "$only" ] || ! newer "usage/$ev/$run/run.json" "$reader"; then stale=1 putrun=1; fi
  [ -n "$stale" ] || continue
  # 2 and 3. Fetch each entry of the run and decode it.
  rd=$w/$run; mkdir -p "$rd/f" "$rd/o"; summaries=; ev=
  while read -r e r en; do
    [ "$r" = "$run" ] || continue
    ev=$e
    set -- --bucket "$tb"
    [ -z "$index" ] || set -- "$@" --index "$w/idx-$en.json"
    [ -z "$store" ] || set -- "$@" --store "$store"
    if ! sh "$here/fetch-run.sh" "$@" "$en" "$rd/f" >/dev/null; then fail=1; continue 2; fi
    if [ -n "$box" ]; then
      $bx copy "$rd/f/$en" "$box:/tmp/uj/$en/in" >/dev/null </dev/null &&
        $bx exec "$box" -- "$gv" usage "/tmp/uj/$en/in" --json --out "/tmp/uj/$en/out" >/dev/null </dev/null &&
        $bx copy "$box:/tmp/uj/$en/out/$en.json" "$rd/o/$en.json" >/dev/null </dev/null &&
        $bx copy "$box:/tmp/uj/$en/out/$en.calls.jsonl" "$rd/o/$en.calls.jsonl" >/dev/null </dev/null || { echo "usage-job: decoding $en in the box failed" >&2; fail=1; continue 2; }
    else
      "$gv" usage "$rd/f/$en" --json --out "$rd/o" >/dev/null </dev/null || { echo "usage-job: decoding $en failed" >&2; fail=1; continue 2; }
    fi
    [ -f "$rd/o/$en.json" ] && [ -f "$rd/o/$en.calls.jsonl" ] || { echo "usage-job: no output for $en" >&2; fail=1; continue 2; }
  done <"$w/all"
  # 4. run.json, in segment order.
  python3 -I - "$run" "$ev" "$rd" <<'PY' || { fail=1; continue; }
import glob, json, os, sys
run, event, rd = sys.argv[1:4]
rows = []
for p in glob.glob(f"{rd}/o/e*.json"):
    entry = os.path.basename(p)[:-5]
    seg = json.load(open(f"{rd}/f/{entry}/fetch.json")).get("segment") or 0
    rows.append((seg, entry, json.load(open(p))))
rows.sort()
KEYS = ("input", "cached", "cache_write", "output", "reasoning")
IDENT = ("parent", "depth", "label")  # from the latest copy that has them
NATIVE = ("native_only", "native_total", "native_until", "native_tokens", "resumed")  # from the latest readable copy
LAST = ("readable", "reason")

def add(a, b):  # a number plus a number; unsaid only when both are
    return a if b is None else b if a is None else a + b

def addall(m, t):
    for k, v in (t or {}).items():
        m[k] = add(m.get(k), v)

threads = {}
for _, _, s in rows:
    for t in s.get("threads") or []:
        m = threads.get(t["id"])
        if m is None:
            m = threads[t["id"]] = {"id": t["id"], "wire_calls": 0, "native_calls": 0, "matched": 0,
                                    "status": {}, "tokens": {}, "models": {}, "effort": [], "first": None, "last": None,
                                    **{k: None for k in IDENT + NATIVE + LAST}}
        for k in ("wire_calls", "native_calls", "matched"):
            m[k] += t.get(k) or 0
        for k in ("status", "tokens", "models"):
            addall(m[k], t.get(k))
        m["effort"] = sorted(set(m["effort"]) | set(t.get("effort") or []))
        for k, f in (("first", min), ("last", max)):
            seen = [x for x in (m[k], t.get(k)) if x]
            m[k] = f(seen) if seen else None
        for k in IDENT:
            if t.get(k) is not None:
                m[k] = t[k]
        if t.get("readable"):  # the last readable entry holds the resumed session's whole native values
            for k in NATIVE:
                m[k] = t.get(k)
        for k in LAST:
            m[k] = t.get(k)
tok = {}
for _, _, s in rows:
    addall(tok, (s.get("tokens") or {}).get("merged"))
tok = {k: tok.get(k) for k in KEYS}
out = {"v": 1, "reader": min((s.get("reader", "0") for _, _, s in rows), key=lambda r: [int(x) for x in r.split(".") if x.isdigit()]),
       "event": event, "run": run,
       "entries": [{"entry": e, "segment": g, "connector": s.get("connector"), "harness_version": s.get("harness_version"),
                    "sealed": s.get("sealed"), "readable": s.get("readable"), "coverage": s.get("coverage"), "read_error": bool(s.get("read_error")),
                    "native_partial": (s.get("tokens") or {}).get("native_partial")} for g, e, s in rows],
       "threads": sorted(threads.values(), key=lambda t: t["id"]),
       "tokens": {"merged": tok, "calls_without_usage": sum((s.get("tokens") or {}).get("calls_without_usage") or 0 for _, _, s in rows)}}
with open(f"{rd}/run.json", "w") as f:
    json.dump(out, f, sort_keys=True, separators=(",", ":"))
    f.write("\n")
PY
  # 5. Put the objects, never over a newer reader's.
  while read -r en; do
    put "usage/$ev/$run/$en.json" "$rd/o/$en.json"; put "usage/$ev/$run/$en.calls.jsonl" "$rd/o/$en.calls.jsonl"
  done <"$w/puts"
  if [ -n "$putrun" ]; then put "usage/$ev/$run/run.json" "$rd/run.json"; fi
  rm -rf "$rd"
done
exit "$fail"
