# Shared paths and the commit trailer for every conductor shell tool.
# Source it:  . "$(dirname "$0")/lib.sh"
# No literal machine paths live in the tools; they all come from here.
GC_TOOLS=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
[ -f "$GC_TOOLS/env.sh" ] && . "$GC_TOOLS/env.sh"
GC_NIGHT=${GC_NIGHT:-$HOME/gc-night}
GC_WT=${GC_WT:-$HOME/gc-wt}
GC_COAUTHOR=${GC_COAUTHOR:-Claude Opus 5.5 <noreply@anthropic.com>}
export GC_TOOLS GC_NIGHT GC_WT GC_COAUTHOR

# Exit 2 unless GC_REPO names a git checkout.
gc_need_repo() {
  if [ -z "${GC_REPO:-}" ] || ! git -C "$GC_REPO" rev-parse --git-dir >/dev/null 2>&1; then
    echo "GC_REPO unset (install.sh --repo)" >&2
    exit 2
  fi
}

# Print the two attribution lines for a commit message, or fail with 2.
gc_trailer() {
  if [[ ! "${GC_SESSION:-}" =~ ^https://claude\.ai/code/session_[A-Za-z0-9]{8,}$ ]]; then
    echo "GC_SESSION unset or not a claude.ai session URL" >&2
    return 2
  fi
  printf '\n\nCo-Authored-By: %s\nClaude-Session: %s' "$GC_COAUTHOR" "$GC_SESSION"
}
