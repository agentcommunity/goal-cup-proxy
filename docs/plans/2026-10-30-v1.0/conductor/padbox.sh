#!/usr/bin/env bash
# One pad run inside its own sandbox box (diagnosis driver).
#   GC_PAD_WT=<worktree> GC_BUILD=<dir with goalcup and goalcup-verify> padbox.sh SETTINGS KEYNAME ENVVAR OUTDIR
# The key goes in only by sbx.mjs put-env (value read inside node, written 0600); never argv.
# The box is removed from an EXIT trap whatever fails. Prints the log path.
set -uo pipefail
. "$(dirname "$0")/lib.sh"
[ $# -ge 4 ] || { echo "usage: padbox.sh SETTINGS KEYNAME ENVVAR OUTDIR" >&2; exit 2; }
[ -n "${GC_PAD_WT:-}" ] && [ -d "$GC_PAD_WT" ] || { echo "GC_PAD_WT unset or not a directory" >&2; exit 2; }
[ -n "${GC_BUILD:-}" ] && [ -d "$GC_BUILD" ] || { echo "GC_BUILD unset or not a directory" >&2; exit 2; }
settings=$1 keyname=$2 var=$3 out=$4
pad=$GC_PAD_WT/scripts/pad
tag=$(basename "$settings" .toml)${TAGSUF:-}; name="pad-dx-$tag-$(date -u +%H%M%S)"
mkdir -p "$out/logs" "$out/pad-out"
log=$out/logs/$tag.$name.log
job=$(mktemp)
export PAD_NODE_MODULES=$GC_TOOLS/node_modules
export NODE_OPTIONS="--use-openssl-ca --no-warnings --require $GC_TOOLS/proxy-preload.cjs"
SBX=${GC_SBX:-node $pad/sbx.mjs}
NEVER=(GH_TOKEN GOALCUP_TEST_APPROVAL_KEY GOALCUP_TEST_MEMBER_ID DATABASE_URL CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID)
KEYS=(VERCEL_TOKEN VERCEL_TEAM_ID VERCEL_PROJECT_ID CLAUDE_API_KEY OPENAI_API_KEY DEEPSEEK_API_KEY GEMINI_API_KEY AI_GATEWAY_API_KEY)
# every call drops all secrets; put-env alone keeps the one variable it is asked to read
sbx() { local a=(); local n; for n in "${NEVER[@]}" "${KEYS[@]}"; do a+=(-u "$n"); done; env "${a[@]}" $SBX "$@"; }
sbx_putenv() { local a=(); local n; for n in "${NEVER[@]}" "${KEYS[@]}"; do [ "$n" = "$var" ] || a+=(-u "$n"); done; env "${a[@]}" $SBX "$@"; }
cleanup() { sbx rm "$name" >> "$log" 2>&1; echo "box removed" >> "$log"; rm -f "$job"; }
trap cleanup EXIT
{
echo "== box $name settings $tag $(date -u +%FT%TZ)"
sbx create --name "$name" --minutes 45 --snapshot "$(cat "$pad/image.id")" --vcpus 4 || { echo "create failed"; exit 1; }
sbx push-dir "$name" "$pad" "~/repo/scripts/pad" && echo "pushed scripts/pad"
sbx push-dir "$name" "$GC_PAD_WT/connectors" "~/repo/connectors" && echo "pushed connectors"
sbx put "$name" "~/repo/settings.toml" --file "$settings" --mode 644 && echo "put settings"
for b in goalcup goalcup-verify; do sbx put "$name" "~/bin/$b" --file "$GC_BUILD/$b" --mode 755 && echo "put $b"; done
sbx_putenv put-env "$name" "~/.pad-keys/$keyname" --env "$var" && echo "put-env key $keyname (0600)"
cat > "$job" <<'IN'
set -u
cd ~/repo
ls -la ~/.pad-keys | sed 1d | awk '{print $1, $NF}'
~/bin/goalcup --version 2>&1 | head -1
python3.11 scripts/pad/pad settings.toml; echo "PAD_EXIT=$?"
tar -C ~/repo -cf /tmp/pad-out.tar pad-out && echo "tarred pad-out"
IN
sbx exec "$name" --script "$job"; echo "exec exit $?"
mkdir -p "$out/pad-out/$tag"
if sbx get "$name" /tmp/pad-out.tar "$out/pad-out/$tag.tar" && tar -C "$out/pad-out/$tag" -xf "$out/pad-out/$tag.tar"; then
  echo "copied out pad-out"; rm -f "$out/pad-out/$tag.tar"
  if [ -f "$GC_TOOLS/keyscan.py" ]; then python3 "$GC_TOOLS/keyscan.py" --scrub "$out/pad-out/$tag"
  else echo "SCRUB FAILED: $GC_TOOLS/keyscan.py missing; pad-out/$tag is NOT scrubbed"; fi
else echo "copy out failed"; fi
echo "== end $(date -u +%FT%TZ)"
} > "$log" 2>&1
echo "$log"
