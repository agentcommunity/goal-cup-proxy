#!/usr/bin/env bash
# Install the tools into an empty home (real npm ci) and check they run with no machine paths.
# Usage: bash test/fresh-clone.sh
set -eu
C=$(cd "$(dirname "$0")/.." && pwd)
H=$(mktemp -d)
trap 'rm -rf "$H"' EXIT
export HOME=$H
unset GC_REPO GC_NIGHT GC_WT GC_TOOLS GC_SESSION GC_COAUTHOR || true
git init -q "$H/w"
bash "$C/install.sh" --dest "$H/gc-tools" --repo "$H/w" --night "$H/gc-night"
n=$(grep -c EnvHttpProxyAgent "$H/gc-tools/node_modules/@vercel/sandbox/dist/api-client/base-client.js" || true)
[ "$n" = 1 ] || { echo "FAIL sandbox patch: $n"; exit 1; }
echo "PASS sandbox patch"
node "$H/gc-tools/box.mjs" --help >/dev/null || { echo "FAIL box --help"; exit 1; }
echo "PASS box --help"
GC_REPO="$H/w" bash "$H/gc-tools/mergeq.sh" 1 || { echo "FAIL mergeq empty queue"; exit 1; }
echo "PASS mergeq empty queue"
hits=$(grep -rnE '/root/|/home/user' "$H/gc-tools" --exclude-dir=node_modules --include='*.sh' --include='*.mjs' --include='*.js' --include='*.cjs' --include='*.py' || true)
[ -z "$hits" ] || { echo "FAIL machine paths:"; echo "$hits"; exit 1; }
echo "PASS no machine paths"
L=$H/gc-night/lanes
[ -f "$L/integrator.log" ] || { echo "FAIL integrator.log not restored"; exit 1; }
cmp -s "$L/integrator.log" "$C/ledger/integrator.log" || { echo "FAIL integrator.log differs"; exit 1; }
if [ -f "$C/ledger/integrator.log.scribed" ]; then
  cmp -s "$L/integrator.log.scribed" "$C/ledger/integrator.log.scribed" || { echo "FAIL .scribed differs"; exit 1; }
fi
echo "PASS ledger restored"
