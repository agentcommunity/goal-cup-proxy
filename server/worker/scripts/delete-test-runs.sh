#!/bin/sh
# Delete the organizers' test runs (mode "test") in one go: their database rows, then their R2
# objects. A dry run by default: it prints what would go and changes nothing. Only --yes deletes.
#
#   sh scripts/delete-test-runs.sh [--older-than '7 days'] [--yes]
#
# Needs: psql, curl, python3, and in the environment
#   DATABASE_URL           the website's Supabase Postgres URL (it calls goal_cup_tracer.delete_test_runs)
#   CLOUDFLARE_ACCOUNT_ID  and CLOUDFLARE_API_TOKEN (R2 read and write on the buckets below)
# The database holds the test runs of production and staging together, so one call covers both.
# Only these bucket:prefix pairs are touched: goalcup-traces:traces/test/,
# goalcup-traces-staging:traces/test-staging/, goalcup-usage:usage/test/ and
# goalcup-usage-staging:usage/test-staging/ (usage stats). R2 is listed before the database changes,
# so a failed listing changes nothing. A usage bucket that is missing or not open to the token is
# skipped with one line. Without --older-than every test run goes and all prefixes are emptied; with
# it, only what is under <prefix><run>/ for the runs deleted.
set -eu

older='0' yes=no
while [ $# -gt 0 ]; do
  case $1 in
    --older-than) older=${2:?--older-than needs an interval, like '7 days'}; shift ;;
    --yes) yes=yes ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "delete-test-runs: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
: "${DATABASE_URL:?set DATABASE_URL}" "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}" "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN}"
case $older in *\'*) echo "delete-test-runs: bad interval" >&2; exit 2 ;; esac
base="https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets"
tmp=$(mktemp -d) && trap 'rm -rf "$tmp"' EXIT

# list BUCKET PREFIX: add every key under PREFIX to $tmp/keys and set status=ok, or on a failure set
# status to the HTTP status (HTTP 000 when no answer came).
list() {
  status=ok cursor=
  while :; do
    code=$(curl -sS -G -o "$tmp/page.json" -w '%{http_code}' "$base/$1/objects" --data-urlencode "prefix=$2" \
      --data-urlencode per_page=1000 ${cursor:+--data-urlencode "cursor=$cursor"} -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN") || code=000
    cursor=$(python3 - "$tmp/page.json" "$tmp/keys" "$code" <<'PY'
import json, sys
try:
    page = json.load(open(sys.argv[1]))
except Exception:
    page = None
page = page if isinstance(page, dict) else {}
if sys.argv[3] != "200" or not page.get("success"):
    print(f"!HTTP {sys.argv[3]}"); sys.exit()
with open(sys.argv[2], "a") as out:
    for o in page.get("result") or []:
        out.write(o["key"] + "\n")
info = page.get("result_info") or {}
print(info.get("cursor", "") if info.get("is_truncated") else "")
PY
)
    case $cursor in '!'*) status=${cursor#!}; return ;; '') return ;; esac
  done
}

# 1. R2, before anything changes: every object under each prefix.
: >"$tmp/plan"
buckets=
for pair in goalcup-traces:traces/test/ goalcup-traces-staging:traces/test-staging/ \
  goalcup-usage:usage/test/ goalcup-usage-staging:usage/test-staging/; do
  bucket=${pair%%:*} prefix=${pair#*:}
  : >"$tmp/keys"
  list "$bucket" "$prefix"
  case ${prefix%%/*}:$status in
    *:ok) ;;
    usage:'HTTP 403'* | usage:'HTTP 404'*)
      echo "delete-test-runs: R2 $bucket is missing or not open to this token ($status), skipped;" \
        "its test objects stay until a run without --older-than with a token that reaches it" >&2
      continue ;;
    *) echo "delete-test-runs: listing R2 $bucket failed ($status); nothing changed" >&2; exit 1 ;;
  esac
  # Never anything outside the prefix.
  if grep -v "^$prefix" "$tmp/keys" >/dev/null; then
    echo "delete-test-runs: a key outside $bucket/$prefix; nothing changed" >&2
    exit 1
  fi
  sed "s|^|$bucket |" "$tmp/keys" >>"$tmp/plan"
  buckets="$buckets $bucket"
done

# 2. The database. A dry run is the same call in a transaction that is rolled back.
end=ROLLBACK
[ "$yes" = yes ] && end=COMMIT
psql "$DATABASE_URL" -X -q -At -v ON_ERROR_STOP=1 >"$tmp/db.json" <<SQL
BEGIN;
SELECT goal_cup_tracer.delete_test_runs('$older'::interval);
$end;
SQL
python3 - "$tmp/db.json" "$yes" "$tmp/runs" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
open(sys.argv[3], "w").write("".join(r + "\n" for r in d["runs"]))
verb = "Deleted" if sys.argv[2] == "yes" else "Would delete"
print(f"{verb} {len(d['runs'])} test run(s): {', '.join(d['runs']) or '-'}")
print("  rows: " + ", ".join(f"{k} {v}" for k, v in sorted(d["counts"].items())))
PY

# 3. With --older-than, keep only the objects of the runs deleted (<prefix><run>/...).
if [ "$older" != 0 ]; then
  awk 'FILENAME == ARGV[1] { run[$0]; next } { split($2, p, "/") } p[3] in run' "$tmp/runs" "$tmp/plan" >"$tmp/keep"
  mv "$tmp/keep" "$tmp/plan"
fi
for bucket in $buckets; do
  echo "  R2 $bucket: $(grep -c "^$bucket " "$tmp/plan") object(s)"
done

if [ "$yes" = yes ]; then
  failed=0
  while read -r bucket key; do
    enc=$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$key")
    curl -fsS -X DELETE "$base/$bucket/objects/$enc" -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" >/dev/null || failed=$((failed + 1))
  done <"$tmp/plan"
  [ "$failed" -eq 0 ] || { echo "delete-test-runs: $failed object(s) not deleted; run again without --older-than" >&2; exit 1; }
else
  echo "Dry run: nothing changed. Run again with --yes to delete."
fi
