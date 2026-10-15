#!/bin/sh
# Delete the organizers' test runs (mode "test") in one go: their database rows, then their R2
# objects. A dry run by default: it prints what would go and changes nothing. Only --yes deletes.
#
#   sh scripts/delete-test-runs.sh [--older-than '7 days'] [--yes]
#
# Needs: psql, curl, python3, and in the environment
#   DATABASE_URL           the website's Supabase Postgres URL (it calls goal_cup_tracer.delete_test_runs)
#   CLOUDFLARE_ACCOUNT_ID  and CLOUDFLARE_API_TOKEN (R2 read and write on the bucket)
# The database holds the test runs of production and staging together, so one call covers both:
# production objects are under goalcup-traces/traces/test/, staging ones under
# goalcup-traces-staging/traces/test-staging/. Nothing outside those two prefixes is ever deleted.
# Without --older-than every test run goes and both prefixes are emptied; with it, only the objects
# of the runs deleted.
set -eu

older='0' yes=no
while [ $# -gt 0 ]; do
  case $1 in
    --older-than) older=${2:?--older-than needs an interval, like '7 days'}; shift ;;
    --yes) yes=yes ;;
    -h | --help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "delete-test-runs: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
: "${DATABASE_URL:?set DATABASE_URL}" "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}" "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN}"
case $older in *\'*) echo "delete-test-runs: bad interval" >&2; exit 2 ;; esac
base="https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets"
tmp=$(mktemp -d) && trap 'rm -rf "$tmp"' EXIT

# 1. The database. A dry run is the same call in a transaction that is rolled back.
end=ROLLBACK
[ "$yes" = yes ] && end=COMMIT
psql "$DATABASE_URL" -X -q -At -v ON_ERROR_STOP=1 >"$tmp/db.json" <<SQL
BEGIN;
SELECT goal_cup_tracer.delete_test_runs('$older'::interval);
$end;
SQL

# 2. R2, per bucket: every object under its prefix (all runs), or only those the database named.
: >"$tmp/plan"
for pair in goalcup-traces:traces/test/ goalcup-traces-staging:traces/test-staging/; do
  bucket=${pair%%:*} prefix=${pair#*:}
  : >"$tmp/keys"
  if [ "$older" = 0 ]; then
    cursor=
    while :; do
      curl -fsS -G "$base/$bucket/objects" --data-urlencode "prefix=$prefix" --data-urlencode per_page=1000 \
        ${cursor:+--data-urlencode "cursor=$cursor"} -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" >"$tmp/page.json"
      cursor=$(python3 - "$tmp/page.json" "$tmp/keys" <<'PY'
import json, sys
page = json.load(open(sys.argv[1]))
assert page.get("success"), page.get("errors")
with open(sys.argv[2], "a") as out:
    for o in page.get("result") or []:
        out.write(o["key"] + "\n")
info = page.get("result_info") or {}
print(info.get("cursor", "") if info.get("is_truncated") else "")
PY
)
      [ -z "$cursor" ] && break
    done
  else
    python3 -c 'import json,sys; [print(k) for k in json.load(open(sys.argv[1]))["r2_keys"] if k.startswith(sys.argv[2])]' "$tmp/db.json" "$prefix" >"$tmp/keys"
  fi
  # Never anything outside the prefix.
  if grep -v "^$prefix" "$tmp/keys" >/dev/null; then
    echo "delete-test-runs: a key outside $bucket/$prefix; nothing deleted from R2" >&2
    exit 1
  fi
  sed "s|^|$bucket |" "$tmp/keys" >>"$tmp/plan"
done

python3 - "$tmp/db.json" "$tmp/plan" "$yes" <<'PY'
import collections, json, sys
d = json.load(open(sys.argv[1]))
plan = [l.split(" ", 1) for l in open(sys.argv[2]).read().split("\n") if l]
verb = "Deleted" if sys.argv[3] == "yes" else "Would delete"
print(f"{verb} {len(d['runs'])} test run(s): {', '.join(d['runs']) or '-'}")
print("  rows: " + ", ".join(f"{k} {v}" for k, v in sorted(d["counts"].items())))
for bucket, n in sorted(collections.Counter(b for b, _ in plan).items()) or [("R2", 0)]:
    print(f"  R2 {bucket}: {n} object(s)")
PY

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
