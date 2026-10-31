#!/bin/bash
# Merge one reviewed v1.0/* branch into v1.0/integration (PRD-16 section 4), under one lock.
#   merge.sh <branch> "<merge message first line>" [--refresh-lock] [--dry]
# Steps: fetch, merge --no-ff into a scratch worktree at origin/v1.0/integration, optional Cargo.lock
# refresh in a build box, `gate fast` in a build box as the box's non-root user, push only on PASS.
# Exit: 0 merged, 1 gate FAIL, 3 merge conflict, 4 push refused, 2 usage.
# The caller has already checked ownership and the reviewers; this script does not judge.
set -u
BR=${1:?branch}; MSG=${2:?message}; shift 2
REFRESH=0; DRY=0
for a in "$@"; do case $a in --refresh-lock) REFRESH=1;; --dry) DRY=1;; esac; done
REPO=/home/user/goal-cup-proxy
NIGHT=/root/gc-night
WT=$NIGHT/merge-wt
mkdir -p "$NIGHT/gate-logs"
TRAILER=$'\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_018LU9bbCzN7rHbM4pGrKQgh'
log() { echo "$(date -u +%H:%M)Z | integrator merger $BR | $1 | $2" >> "$NIGHT/lanes/integrator.log"; echo "$1: $2"; }

exec 9>"$NIGHT/integration.lock"
flock 9
cd "$REPO"
for i in 1 2 3 4; do git fetch -q origin v1.0/integration && break; sleep $((2**i)); done
git worktree remove --force "$WT" 2>/dev/null; rm -rf "$WT"
git worktree add -q --detach "$WT" origin/v1.0/integration || exit 2
cd "$WT"
IFS=, read -ra BRS <<< "$BR"
for b in "${BRS[@]}"; do git fetch -q origin "$b"; done
for b in "${BRS[@]}"; do
  git merge --no-ff -q "origin/$b" -m "Merge $b: $MSG$TRAILER" || { git merge --abort; log stop "merge conflict of $b with v1.0/integration; owner of the conflicting file resolves"; exit 3; }
done
git diff --name-only origin/v1.0/integration HEAD > "$NIGHT/gate-logs/$(echo "$BR" | tr / _).files"
TAG=mr/$(echo "$BR" | tr / _)-$$
git branch -f "$TAG" HEAD
JOB=$(mktemp)
if [ $REFRESH = 1 ]; then
  cat > "$JOB" <<'EOF'
set -e
cargo update -w -q || cargo generate-lockfile -q
cargo fetch --locked -q
EOF
  node /root/gc-tools/box.mjs run --repo "$WT" --ref "$TAG" --script "$JOB" --minutes 30 --out "Cargo.lock:$WT/Cargo.lock" > "$NIGHT/gate-logs/lock.log" 2>&1 || { log stop "lock refresh failed, see gate-logs/lock.log"; exit 1; }
  if ! git diff --quiet Cargo.lock; then git add Cargo.lock; git commit -q --amend --no-edit; git branch -f "$TAG" HEAD; fi
fi
cat > "$JOB" <<'EOF'
cargo fetch --locked -q 2>/dev/null || cargo fetch -q
[ -f server/worker/package-lock.json ] && (cd server/worker && npm ci --silent >/dev/null 2>&1)
sh scripts/gate.sh fast
EOF
GL="$NIGHT/gate-logs/$(echo "$BR" | tr / _)-$(date -u +%H%M).log"
node /root/gc-tools/box.mjs run --repo "$WT" --ref "$TAG" --script "$JOB" --minutes 60 > "$GL" 2>&1
LAST=$(grep -E '^gate fast: ' "$GL" | tail -1)
rm -f "$JOB"
if [[ "$LAST" != *PASS* ]]; then
  log gate "$BR on $(git rev-parse --short HEAD): ${LAST:-no gate line} (log $GL)"
  git branch -D -q "$TAG"; exit 1
fi
log gate "$LAST on merge $(git rev-parse --short HEAD) (log $GL)"
if [ $DRY = 1 ]; then git branch -D -q "$TAG"; echo "dry run: not pushed"; exit 0; fi
for i in 1 2 3 4; do git push -q origin HEAD:v1.0/integration && break; sleep $((2**i)); [ $i = 4 ] && { log stop "push refused"; exit 4; }; done
log merge "$BR into v1.0/integration as $(git rev-parse --short HEAD): $MSG"
git branch -D -q "$TAG"
exit 0
