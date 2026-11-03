#!/usr/bin/env bash
# The pad's conductor half for one settings file (where = "sandbox").
#   GC_PAD_WT=<worktree holding scripts/pad> padrun.sh SETTINGS OUTDIR
# GH_TOKEN is dropped (no test repo); DATABASE_URL and CLOUDFLARE_* stay, the pad passes them to fetch-run only.
# Writes OUTDIR/logs/<tag>.log ending in PAD_EXIT=<n>, prints its path. GC_PYTHON overrides python3.11.
set -uo pipefail
. "$(dirname "$0")/lib.sh"
[ $# -ge 2 ] || { echo "usage: padrun.sh SETTINGS OUTDIR" >&2; exit 2; }
[ -n "${GC_PAD_WT:-}" ] && [ -d "$GC_PAD_WT" ] || { echo "GC_PAD_WT unset or not a directory" >&2; exit 2; }
settings=$1; out=$2; tag=$(basename "$settings" .toml)
mkdir -p "$out/logs"
export PAD_NODE_MODULES=$GC_TOOLS/node_modules
export NODE_OPTIONS="--use-openssl-ca --no-warnings --require $GC_TOOLS/proxy-preload.cjs"
cd "$GC_PAD_WT" || exit 2
{ echo "== $tag $(date -u +%FT%TZ)"; env -u GH_TOKEN -u PYTHONPATH "${GC_PYTHON:-python3.11}" scripts/pad/pad "$settings"; echo "PAD_EXIT=$?"; echo "== end $(date -u +%FT%TZ)"; } > "$out/logs/$tag.log" 2>&1
echo "$out/logs/$tag.log"
