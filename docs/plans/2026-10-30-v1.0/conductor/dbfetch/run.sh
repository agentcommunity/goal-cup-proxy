#!/usr/bin/env bash
# The conductor's short-lived index box: run the read-only index query of fetch-run.sh per run id.
#   GC_PAD_WT=<worktree holding scripts/fetch-run.sh> dbfetch/run.sh IDS_FILE OUTDIR
# Writes OUTDIR/idx/<id>.json (and .err). DATABASE_URL reaches the box only by sbx put-env (0600);
# every other call drops it, with the other secrets. The box is removed from an EXIT trap.
set -uo pipefail
. "$(dirname "$0")/../lib.sh"
[ $# -ge 2 ] || { echo "usage: dbfetch/run.sh IDS_FILE OUTDIR" >&2; exit 2; }
[ -n "${GC_PAD_WT:-}" ] && [ -f "$GC_PAD_WT/scripts/fetch-run.sh" ] || { echo "GC_PAD_WT unset or has no scripts/fetch-run.sh" >&2; exit 2; }
ids=$1 out=$2
[ -f "$ids" ] || { echo "ids file not found" >&2; exit 2; }
D=$GC_TOOLS/dbfetch
sql=$(mktemp)
name=
cleanup() { [ -n "$name" ] && sbx rm "$name" >/dev/null 2>&1; rm -f "$sql"; }
trap cleanup EXIT
# the SQL block between <<'SQL' and the closing SQL line
awk '/<<.SQL.$/{f=1;next} /^SQL$/{f=0} f' "$GC_PAD_WT/scripts/fetch-run.sh" > "$sql"
if [ "$(head -n1 "$sql")" != "BEGIN READ ONLY;" ] || [ "$(tail -n1 "$sql")" != "ROLLBACK;" ]; then
  echo "fetch-run.sh SQL block not found or not read-only" >&2; exit 4
fi
export PAD_NODE_MODULES=$GC_TOOLS/node_modules
export NODE_OPTIONS="--use-openssl-ca --no-warnings --require $GC_TOOLS/proxy-preload.cjs"
SBX=${GC_SBX:-node $GC_PAD_WT/scripts/pad/sbx.mjs}
NEVER=(GH_TOKEN GOALCUP_TEST_APPROVAL_KEY GOALCUP_TEST_MEMBER_ID DATABASE_URL CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID CLAUDE_API_KEY OPENAI_API_KEY DEEPSEEK_API_KEY GEMINI_API_KEY AI_GATEWAY_API_KEY)
# every call drops all secrets; put-env alone keeps DATABASE_URL
sbx() { local a=() n; for n in "${NEVER[@]}"; do a+=(-u "$n"); done; env "${a[@]}" $SBX "$@"; }
sbx_putenv() { local a=() n; for n in "${NEVER[@]}"; do [ "$n" = DATABASE_URL ] || a+=(-u "$n"); done; env "${a[@]}" $SBX "$@"; }
n=pad-dbf-conductor-$(date -u +%H%M%S)
sbx create --name "$n" --minutes 10 --vcpus 1 >/dev/null || { echo "create failed" >&2; exit 1; }
name=$n
sbx put "$name" "~/ids.txt" --file "$ids" --mode 644 || exit 1
sbx put "$name" "~/query.sql" --file "$sql" --mode 644 || exit 1
sbx put "$name" "~/install.sh" --file "$D/install.sh" --mode 755 || exit 1
sbx put "$name" "~/idx.sh" --file "$D/idx.sh" --mode 755 || exit 1
sbx exec "$name" --script "$D/install.sh" --sudo || exit 1
sbx_putenv put-env "$name" "~/.dburl" --env DATABASE_URL || exit 1
sbx exec "$name" --script "$D/idx.sh"; rc=$?
mkdir -p "$out"
tar=$(mktemp)
if sbx get "$name" /tmp/idx.tar "$tar" && tar -C "$out" -xf "$tar"; then rm -f "$tar"; else rm -f "$tar"; echo "copy out failed" >&2; exit 1; fi
if [ -f "$GC_TOOLS/keyscan.py" ]; then
  python3 "$GC_TOOLS/keyscan.py" "$out/idx"; k=$?
  if [ $k = 1 ]; then rm -rf "$out/idx"; echo "keyscan hit: idx deleted" >&2; exit 1
  elif [ $k != 0 ]; then echo "keyscan exit $k" >&2; exit $k; fi
else echo "SCAN FAILED: $GC_TOOLS/keyscan.py missing; $out/idx is NOT scanned" >&2
fi
exit $rc
