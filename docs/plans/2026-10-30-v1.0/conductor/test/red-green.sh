#!/usr/bin/env bash
# Prove test-first order. For every commit in BASE..HEAD whose subject ends " [red]",
# check it out in a scratch worktree, run the test files it added or changed and require
# a nonzero exit. Then run run-all.sh at HEAD.
# Usage: bash test/red-green.sh BASE      (BASE is a sha or a name)
set -u
BASE=${1:?usage: red-green.sh BASE}
T=$(cd "$(dirname "$0")" && pwd)
R=$(git -C "$T" rev-parse --show-toplevel) || exit 2
cd "$R" || exit 2
W=$(mktemp -d)
trap 'git -C "$R" worktree remove --force "$W/wt" >/dev/null 2>&1; rm -rf "$W"' EXIT
bad=0
for c in $(git rev-list --reverse "$BASE"..HEAD); do
  subj=$(git log -1 --format=%s "$c")
  case $subj in *' [red]') ;; *) continue ;; esac
  git worktree add -q --detach "$W/wt" "$c" || { echo "RED-MISSING $subj (checkout failed)"; bad=1; continue; }
  files=$(git diff-tree --no-commit-id --name-only -r --diff-filter=AM "$c" | grep -E '/test/test_[^/]*\.(sh|mjs|py)$')
  failed=0; ran=0
  for f in $files; do
    ran=1
    case $f in
      *.sh) (cd "$W/wt" && timeout 300 bash "$f") >/dev/null 2>&1 || failed=1 ;;
      *.mjs) (cd "$W/wt" && timeout 300 node "$f") >/dev/null 2>&1 || failed=1 ;;
      *.py) (cd "$W/wt" && timeout 300 python3 -m unittest discover -s "$(dirname "$f")" -p "$(basename "$f")") >/dev/null 2>&1 || failed=1 ;;
    esac
  done
  if [ "$ran" = 1 ] && [ "$failed" = 1 ]; then echo "RED-OK $subj"; else echo "RED-MISSING $subj"; bad=1; fi
  git worktree remove --force "$W/wt"
done
[ "$bad" = 0 ] || exit 1
bash "$T/run-all.sh"
