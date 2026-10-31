#!/bin/sh
# The organizer usage job (PRD-09 sections 1 and 7): for every sealed entry of a run that has no usage
# object, or one made by an older reader, fetch the server copy, decode it and put the usage objects.
#
#   sh scripts/usage-job.sh [--box NAME] [--entry E] [--dry-run] [--index FILE] [--store DIR] [--usage-store DIR]
#
# Puts usage/{event}/{run}/{entry}.json, {entry}.calls.jsonl and run.json into the bucket named by
# GOALCUP_USAGE_BUCKET (default goalcup-usage; goalcup-usage-staging reads the staging runs only).
# A run is done whole: any stale entry re-reads all of the run's sealed entries, and --entry E names
# E's run. run.json merges the entries; a thread that spans a `continue` takes the whole values of its
# last entry (the resumed session's own totals, PRD-09 slice 5). An object made by a newer reader is
# never overwritten. --box NAME decodes in that build box (`sandbox copy` in, `sandbox exec`, `sandbox
# copy` out); no credential enters it. Without it, goalcup-verify (or $GOALCUP_VERIFY) runs here.
# Credentials, from the environment only: DATABASE_URL (read-only) and CLOUDFLARE_ACCOUNT_ID with a read
# token for the traces bucket (fetch-run.sh), and GOALCUP_USAGE_TOKEN (read and write on the usage
# bucket only). None reaches argv or output. --index and --store are fetch-run.sh's; --usage-store DIR
# stands in for the usage bucket. --dry-run puts nothing. Exit 1 on a bad fetch or decode, 0 otherwise.
set -eu
set +x
umask 077

box= only= dry= index= store= ustore=
while [ $# -gt 0 ]; do
  case $1 in
    --box) box=${2:?--box needs a name}; shift ;;
    --entry) only=${2:?--entry needs an entry id}; shift ;;
    --index) index=${2:?--index needs a file}; shift ;;
    --store) store=${2:?--store needs a directory}; shift ;;
    --usage-store) ustore=${2:?--usage-store needs a directory}; shift ;;
    --dry-run) dry=1 ;;
    -h | --help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "usage-job: unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
here=$(cd "$(dirname "$0")" && pwd)
gv=${GOALCUP_VERIFY:-goalcup-verify}
ub=${GOALCUP_USAGE_BUCKET:-goalcup-usage}
case $ub in '' | *[!A-Za-z0-9._-]*) echo "usage-job: bad bucket name" >&2; exit 2 ;; esac
case $ub in *-staging) tb=goalcup-traces-staging staging=true ;; *) tb=goalcup-traces staging=false ;; esac
[ -n "$ustore" ] || : "${GOALCUP_USAGE_TOKEN:?set GOALCUP_USAGE_TOKEN, or pass --usage-store DIR}" "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
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
  python3 -I - "$index" "$w" "$staging" <<'PY' >"$w/all"
import json, sys
for e in json.load(open(sys.argv[1])):
    if e.get("sealed_at"):
        open(f"{sys.argv[2]}/idx-{e['entry_id']}.json", "w").write(json.dumps([e]))
        print(e["event"], e["run_id"], e["entry_id"])
PY
else
  # libpq reads a connection URI from PGDATABASE, so the URL stays out of argv.
  PGDATABASE=$DATABASE_URL psql -X -q -At -F ' ' -v ON_ERROR_STOP=1 -v staging="$staging" >"$w/all" <<'SQL'
BEGIN READ ONLY;
SELECT ev.id, e.run_id, e.id FROM goal_cup_tracer.entries e
JOIN goal_cup_tracer.runs r ON r.id = e.run_id JOIN goal_cup_tracer.events ev ON ev.id = r.event_id
WHERE e.sealed_at IS NOT NULL AND ev.staging = :'staging' ORDER BY e.run_id, e.segment;
ROLLBACK;
SQL
fi
reader=$("$gv" --version | awk '{print $NF}')
fail=0
for run in $(awk '{print $2}' "$w/all" | sort -u); do
  stale=
  while read -r ev r en; do
    [ "$r" = "$run" ] || continue
    if [ "$en" = "$only" ] || ! newer "usage/$ev/$r/$en.json" "$reader"; then stale=1; fi
  done <"$w/all"
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
      sandbox copy "$rd/f/$en" "$box:/tmp/uj/$en/in" >/dev/null &&
        sandbox exec "$box" -- "$gv" usage "/tmp/uj/$en/in" --json --out "/tmp/uj/$en/out" >/dev/null &&
        sandbox copy "$box:/tmp/uj/$en/out/$en.json" "$rd/o/$en.json" >/dev/null &&
        sandbox copy "$box:/tmp/uj/$en/out/$en.calls.jsonl" "$rd/o/$en.calls.jsonl" >/dev/null || { echo "usage-job: decoding $en in the box failed" >&2; fail=1; continue 2; }
    else
      "$gv" usage "$rd/f/$en" --json --out "$rd/o" >/dev/null || { echo "usage-job: decoding $en failed" >&2; fail=1; continue 2; }
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
threads = {}
for _, _, s in rows:
    for t in s.get("threads") or []:
        threads[t["id"]] = t  # the last entry's copy holds the resumed session's whole values
KEYS = ("input", "cached", "cache_write", "output", "reasoning")
tok = {k: None for k in KEYS}
for t in threads.values():
    for k in KEYS:
        v = (t.get("tokens") or {}).get(k)
        if v is not None:
            tok[k] = (tok[k] or 0) + v
out = {"v": 1, "reader": min((s.get("reader", "0") for _, _, s in rows), key=lambda r: [int(x) for x in r.split(".") if x.isdigit()]),
       "event": event, "run": run, "sealed": all(s.get("sealed") for _, _, s in rows),
       "entries": [{"entry": e, "segment": g, "connector": s.get("connector"), "harness_version": s.get("harness_version"),
                    "sealed": s.get("sealed"), "readable": s.get("readable"), "coverage": s.get("coverage")} for g, e, s in rows],
       "threads": sorted(threads.values(), key=lambda t: t["id"]),
       "tokens": {"threads": tok, "calls_without_usage": sum((s.get("tokens") or {}).get("calls_without_usage") or 0 for _, _, s in rows)}}
with open(f"{rd}/run.json", "w") as f:
    json.dump(out, f, sort_keys=True, separators=(",", ":"))
    f.write("\n")
PY
  # 5. Put the objects, never over a newer reader's.
  for f in "$rd"/o/e*.json; do
    en=$(basename "$f" .json)
    newer "usage/$ev/$run/$en.json" "$reader" && continue
    put "usage/$ev/$run/$en.json" "$f"; put "usage/$ev/$run/$en.calls.jsonl" "$rd/o/$en.calls.jsonl"
  done
  put "usage/$ev/$run/run.json" "$rd/run.json"
  rm -rf "$rd"
done
exit "$fail"
