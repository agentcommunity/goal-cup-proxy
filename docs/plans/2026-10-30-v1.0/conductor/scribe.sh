#!/bin/bash
# Scribe: merge new lane-log lines (sorted by time) into NIGHT-LOG.md on v1.0/integration. Docs-only commit.
set -u
NIGHT=/root/gc-night; REPO=/home/user/goal-cup-proxy; WT=$NIGHT/scribe-wt
F=docs/plans/2026-10-30-v1.0/NIGHT-LOG.md
exec 9>"$NIGHT/integration.lock"; flock 9
cd "$REPO" && git fetch -q origin v1.0/integration
git worktree remove --force "$WT" 2>/dev/null; rm -rf "$WT"
git worktree add -q --detach "$WT" origin/v1.0/integration && cd "$WT"
NEW=$(mktemp)
for l in "$NIGHT"/lanes/*.log; do
  [ -f "$l" ] || continue
  n=$(wc -l < "$l"); done_n=$(cat "$l.scribed" 2>/dev/null || echo 0)
  [ "$n" -gt "$done_n" ] && tail -n +$((done_n+1)) "$l" | head -n $((n-done_n)) >> "$NEW"
  echo "$n" > "$l.pending"
done
if [ -s "$NEW" ]; then
  sort -s -k1,1 "$NEW" >> "$F"
  git add "$F" && git commit -q -m "docs(plans): night log, scribe merge

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6" && git push -q origin HEAD:v1.0/integration && for l in "$NIGHT"/lanes/*.log; do mv -f "$l.pending" "$l.scribed" 2>/dev/null; done
  echo "scribed $(wc -l < "$NEW") lines"
else rm -f "$NIGHT"/lanes/*.pending; echo "nothing new"; fi
rm -f "$NEW"; cd "$REPO"; git worktree remove --force "$WT"
