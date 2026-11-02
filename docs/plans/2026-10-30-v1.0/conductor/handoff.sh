#!/bin/bash
# Regenerate docs/plans/2026-10-30-v1.0/HANDOFF.md and the conductor/ tool copies on v1.0/integration.
set -u
. "$(dirname "$0")/lib.sh"
TRAILER=$(gc_trailer) || exit 2
gc_need_repo
NIGHT=$GC_NIGHT; REPO=$GC_REPO; WT=$NIGHT/handoff-wt
D=docs/plans/2026-10-30-v1.0
exec 9>"$NIGHT/integration.lock"; flock 9
cd "$REPO" && git fetch -q origin
git worktree remove --force "$WT" 2>/dev/null; rm -rf "$WT"
git worktree add -q --detach "$WT" origin/v1.0/integration && cd "$WT"
mkdir -p "$D/conductor"
cp "$GC_TOOLS/box.mjs" "$GC_TOOLS/merge.sh" "$GC_TOOLS/mergeq.sh" "$GC_TOOLS/scribe.sh" "$GC_TOOLS/handoff.sh" "$GC_TOOLS/handoff-head.md" "$D/conductor/"
cp "${GC_LANE:-$GC_TOOLS/gc-lane.js}" "$D/conductor/gc-lane.js"
{
  cat "$GC_TOOLS/handoff-head.md"
  echo
  echo "## Branches ($(date -u '+%d %b %H:%MZ'), integration at $(git rev-parse --short origin/v1.0/integration))"
  echo
  echo "| Branch | Commits not yet on integration | Last merge into integration |"
  echo "|---|---:|---|"
  for b in $(git branch -r | grep -oE 'origin/v1\.0/prd-[0-9]+-[a-z0-9]+$' | sort -u); do
    n=$(git rev-list --count origin/v1.0/integration..$b)
    m=$(git log --first-parent --format='%h %s' origin/v1.0/integration --grep="Merge ${b#origin/}:" -1 | cut -c1-110 | sed 's/|/\//g')
    echo "| \`${b#origin/}\` | $n | ${m:-none} |"
  done
  echo
  echo "Open merge requests in the queue: $(ls $NIGHT/mrq/*.req $NIGHT/mrq/*.claimed 2>/dev/null | wc -l)."
} > "$D/HANDOFF.md"
git add "$D/HANDOFF.md" "$D/conductor"
if git diff --cached --quiet; then echo "handoff unchanged"; else
  git commit -q -m "docs(plans): handoff ledger and conductor tool copies$TRAILER" && git push -q origin HEAD:v1.0/integration && echo "handoff pushed $(git rev-parse --short HEAD)"; fi
cd "$REPO"; git worktree remove --force "$WT"
