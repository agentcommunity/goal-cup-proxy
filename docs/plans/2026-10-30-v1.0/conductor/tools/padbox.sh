#!/usr/bin/env bash
# Rung 1 recheck (rung2b) driver: one pad box per settings file. Usage: padbox.sh <settings.toml> <keyname> <ENVVAR>
# The key goes in only by sbx.mjs put-env (value read inside node, written 0600); never argv.
set -uo pipefail
settings=$1 keyname=$2 var=$3
wt=/root/gc-wt/rung2b; pad=${PADDIR:-$wt/scripts/pad}; out=/root/gc-night/rung2b
tag=$(basename "$settings" .toml)${TAGSUF:-}; name="pad-r2b-$tag-$(date -u +%H%M%S)"
log=$out/$tag.$name.log
export PAD_NODE_MODULES=/root/gc-tools/node_modules
export NODE_OPTIONS="--use-openssl-ca --no-warnings --require /root/gc-night/rung2b/proxy-preload.cjs"
sbx() { env -u GH_TOKEN -u GOALCUP_TEST_APPROVAL_KEY -u GOALCUP_TEST_MEMBER_ID -u DATABASE_URL -u CLOUDFLARE_API_TOKEN node "$pad/sbx.mjs" "$@"; }
{
echo "== box $name settings $tag $(date -u +%FT%TZ)"
sbx create --name "$name" --minutes 45 --snapshot "$(cat $pad/image.id)" --vcpus 4 || { echo "create failed"; exit 1; }
sbx push-dir "$name" "$pad" "~/repo/scripts/pad" && echo "pushed scripts/pad"
sbx push-dir "$name" "$wt/connectors" "~/repo/connectors" && echo "pushed connectors"
sbx put "$name" "~/repo/settings.toml" --file "$settings" --mode 644 && echo "put settings"
for b in goalcup goalcup-verify; do sbx put "$name" "~/bin/$b" --file "$out/build/$b" --mode 755 && echo "put $b"; done
[ -n "${DIAG:-}" ] && sbx put "$name" "~/repo/diag.py" --file "$out/jobs/diag.py" --mode 644 && echo "put diag"
sbx put-env "$name" "~/.pad-keys/$keyname" --env "$var" && echo "put-env key $keyname (0600)"
cat > "$out/jobs/inbox-$tag.sh" <<'IN'
set -u
cd ~/repo
ls -la ~/.pad-keys | sed 1d | awk '{print $1, $NF}'
~/bin/goalcup --version 2>&1 | head -1
[ -f ~/repo/diag.py ] && export PAD_KEEP_BOXDIR=1
python3.11 scripts/pad/pad settings.toml; echo "PAD_EXIT=$?"
[ -f ~/repo/diag.py ] && python3.11 ~/repo/diag.py
tar -C ~/repo -cf /tmp/pad-out.tar pad-out && echo "tarred pad-out"
IN
sbx exec "$name" --script "$out/jobs/inbox-$tag.sh"; echo "exec exit $?"
mkdir -p "$out/pad-out/$tag"
sbx get "$name" /tmp/pad-out.tar "$out/pad-out/$tag.tar" && tar -C "$out/pad-out/$tag" -xf "$out/pad-out/$tag.tar" && echo "copied out pad-out" && rm -f "$out/pad-out/$tag.tar" && python3 "$out/scrub.py" "$out/pad-out/$tag"
sbx rm "$name"; echo "box removed"
sbx list --prefix "$name"
echo "== end $(date -u +%FT%TZ)"
} > "$log" 2>&1
echo "$log"
