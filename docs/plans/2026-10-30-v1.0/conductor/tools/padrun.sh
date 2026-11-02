#!/usr/bin/env bash
# gw3 driver: the pad's conductor half for one settings file (where = "sandbox"), worktree /root/gc-wt/gw2.
# GH_TOKEN dropped (no test repo); DATABASE_URL / CLOUDFLARE_* stay here, pad passes them to fetch-run only.
set -uo pipefail
settings=$1; tag=$(basename "$settings" .toml); out=/root/gc-night/gw3
export PAD_NODE_MODULES=/root/gc-tools/node_modules
export NODE_OPTIONS="--use-openssl-ca --no-warnings --require /root/gc-night/rung1/proxy-preload.cjs"
cd /root/gc-wt/gw2
{ echo "== $tag $(date -u +%FT%TZ)"; env -u GH_TOKEN -u PYTHONPATH python3.11 scripts/pad/pad "$settings"; echo "PAD_EXIT=$?"; echo "== end $(date -u +%FT%TZ)"; } > "$out/logs/$tag.log" 2>&1
echo "$out/logs/$tag.log"
