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
C=$D/conductor
mkdir -p "$C/ledger" "$C/reports" "$C/dbfetch"
for f in box.mjs spend.mjs merge.sh mergeq.sh mq-driver.sh scribe.sh handoff.sh lib.sh install.sh patch-sandbox.sh \
  proxy-preload.cjs padrun.sh padbox.sh keyscan.py rowinfo.py package.json package-lock.json .gitignore handoff-head.md; do
  cp "$GC_TOOLS/$f" "$C/" || { echo "handoff: missing tool file $f" >&2; cd "$REPO"; git worktree remove --force "$WT"; exit 2; }
done
# The snapshot id exists only on a machine that has built one.
[ -f "$GC_TOOLS/build-snapshot.id" ] && cp "$GC_TOOLS/build-snapshot.id" "$C/"
for f in idx.sh install.sh run.sh; do cp "$GC_TOOLS/dbfetch/$f" "$C/dbfetch/"; done
cp "${GC_LANE:-$GC_TOOLS/gc-lane.js}" "$C/gc-lane.js"
# Ledger: every lane log and its scribe mark; never a .pending file.
for f in "$NIGHT"/lanes/*.log "$NIGHT"/lanes/*.log.scribed; do [ -f "$f" ] && cp "$f" "$C/ledger/"; done
# Reports: one text file per run folder, plus the latest review and the pad notes.
[ -f "$C/reports/README.md" ] || printf '# Reports\n\n| Report | Summary |\n|---|---|\n' > "$C/reports/README.md"
for f in "$NIGHT"/*/REPORT.txt; do
  [ -f "$f" ] || continue
  n=$(basename "$(dirname "$f")")
  cp "$f" "$C/reports/$n.txt"
  grep -q "^| $n |" "$C/reports/README.md" || echo "| $n | not summarised yet |" >> "$C/reports/README.md"
done
[ -f "$NIGHT/reviews/LATEST.md" ] && cp "$NIGHT/reviews/LATEST.md" "$C/reports/review-latest.md"
[ -f "$NIGHT/backlog/next-pad-verify.md" ] && cp "$NIGHT/backlog/next-pad-verify.md" "$C/reports/next-pad-verify.md"
[ -f "$NIGHT/finish/FINISH-PLAN.md" ] && cp "$NIGHT/finish/FINISH-PLAN.md" "$C/FINISH-PLAN.md"
# No key value may reach the tree: only exit 0, or 3 with GC_KEYSCAN_ALLOW_EMPTY=1, passes; anything else stops here.
python3 "$GC_TOOLS/keyscan.py" "$C"; ks=$?
if [ "$ks" != 0 ] && { [ "$ks" != 3 ] || [ "${GC_KEYSCAN_ALLOW_EMPTY:-}" != 1 ]; }; then
  echo "handoff: keyscan refused" >&2
  cd "$REPO"; git worktree remove --force "$WT"; exit 6
fi
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
git add "$D/HANDOFF.md" "$C"
if git diff --cached --quiet; then echo "handoff unchanged"; else
  git commit -q -m "docs(plans): handoff ledger and conductor tool copies$TRAILER" && git push -q origin HEAD:v1.0/integration && echo "handoff pushed $(git rev-parse --short HEAD)"; fi
cd "$REPO"; git worktree remove --force "$WT"
