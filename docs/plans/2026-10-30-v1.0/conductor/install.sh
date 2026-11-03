#!/usr/bin/env bash
# Set up a working copy of the conductor tools in any container.
# Usage: install.sh --dest DIR [--repo DIR] [--night DIR] [--wt DIR] [--session URL]
#                  [--coauthor TEXT] [--lane-dir DIR] [--skip-npm]
# Exit 0 ok, 2 usage, 5 sandbox patch refused.
set -u
SRC=$(cd "$(dirname "$0")" && pwd)
dest= repo= night= wt= session= coauthor= lane= skip=0
usage() { echo "usage: install.sh --dest DIR [--repo DIR] [--night DIR] [--wt DIR] [--session URL] [--coauthor TEXT] [--lane-dir DIR] [--skip-npm]" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case $1 in
    --skip-npm) skip=1; shift ;;
    --dest|--repo|--night|--wt|--session|--coauthor|--lane-dir)
      [ $# -ge 2 ] || usage
      case $1 in
        --dest) dest=$2 ;; --repo) repo=$2 ;; --night) night=$2 ;; --wt) wt=$2 ;;
        --session) session=$2 ;; --coauthor) coauthor=$2 ;; --lane-dir) lane=$2 ;;
      esac
      shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$dest" ] || usage
mkdir -p "$dest" || exit 2
dest=$(cd "$dest" && pwd)

# Copy the tools; never the checks, the records or (with npm) node_modules.
excl=(--exclude=./test --exclude=./ledger --exclude=./reports)
if [ "$skip" = 0 ] || [ ! -d "$SRC/node_modules" ]; then excl+=(--exclude=./node_modules); fi
if [ "$SRC" != "$dest" ]; then
  (cd "$SRC" && tar -cf - "${excl[@]}" .) | (cd "$dest" && tar -xf -) || exit 1
fi

if [ "$skip" = 0 ]; then
  (cd "$dest" && npm ci --silent) || { echo "npm ci failed" >&2; exit 1; }
fi

if [ -d "$dest/node_modules/@vercel/sandbox" ]; then
  bash "$dest/patch-sandbox.sh" "$dest/node_modules" || exit $?
else
  echo "patch skipped: no node_modules"
fi

# env.sh holds the given paths and session as defaults only, never a secret value.
# A value the caller already exported wins, so a later session is never stamped
# with this install's session URL.
{
  [ -n "$repo" ] && printf '[ -n "${GC_REPO:-}" ] || export GC_REPO=%q\n' "$repo"
  [ -n "$night" ] && printf '[ -n "${GC_NIGHT:-}" ] || export GC_NIGHT=%q\n' "$night"
  [ -n "$wt" ] && printf '[ -n "${GC_WT:-}" ] || export GC_WT=%q\n' "$wt"
  [ -n "$session" ] && printf '[ -n "${GC_SESSION:-}" ] || export GC_SESSION=%q\n' "$session"
  [ -n "$coauthor" ] && printf '[ -n "${GC_COAUTHOR:-}" ] || export GC_COAUTHOR=%q\n' "$coauthor"
  true
} > "$dest/env.sh"
chmod 0644 "$dest/env.sh"

# --night restores the ledger into a night folder; an existing file is never replaced.
if [ -n "$night" ]; then
  mkdir -p "$night"/{lanes,mrq/done,gate-logs,status,reviews,builds,finish} || exit 1
  : >> "$night/integration.lock"
  for f in "$SRC"/ledger/*.log; do
    [ -f "$f" ] || continue
    # Decide per pair: a kept local log keeps its own counter, so its .scribed is never restored.
    b=$(basename "$f")
    if [ -e "$night/lanes/$b" ]; then echo "kept existing $b"; continue; fi
    cp "$f" "$night/lanes/$b"
    [ -f "$f.scribed" ] && cp "$f.scribed" "$night/lanes/$b.scribed"
  done
fi

if [ -n "$lane" ]; then
  mkdir -p "$lane" && cp "$SRC/gc-lane.js" "$lane/gc-lane.js" || exit 1
fi
exit 0
