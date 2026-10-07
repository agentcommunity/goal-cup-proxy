#!/usr/bin/env bash
# Installs every target harness in isolation under this directory.
# Nothing here touches ~/.claude, ~/.codex, ~/.config or global npm.
# Re-runnable. Requires: node >= 22, npm, uv, git.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
NODE_PREFIX="$HERE/node"
HOME_ROOT="$HERE/home"
SRC="$HERE/src"
mkdir -p "$NODE_PREFIX" "$HOME_ROOT" "$SRC" "$HERE/logs"

log() { printf '\n== %s\n' "$*"; }

# ---- npm-distributed harnesses (isolated prefix) ---------------------------
# Package names verified 2026-09-02 in docs/harness-research/*.md
NPM_PKGS=(
  "@openai/codex"          # Codex CLI (a Homebrew copy may also exist globally)
  "opencode-ai"            # OpenCode
  "@deepseek-ai/dsh"       # DeepSeek Harness
  "openclaw"               # OpenClaw
)
for pkg in "${NPM_PKGS[@]}"; do
  log "npm: $pkg -> $NODE_PREFIX"
  npm install --prefix "$NODE_PREFIX" -g "$pkg" >"$HERE/logs/npm-${pkg//[@\/]/_}.log" 2>&1 \
    && echo "ok" || { echo "FAILED (see logs)"; }
done

# ---- Hermes Agent (Python) ---------------------------------------------------
# Try the packaged tool first; fall back to a source checkout run with uv.
log "hermes"
export UV_TOOL_DIR="$HERE/uv-tools" UV_TOOL_BIN_DIR="$HERE/uv-bin"
mkdir -p "$UV_TOOL_DIR" "$UV_TOOL_BIN_DIR"
if uv tool install hermes-agent >"$HERE/logs/hermes-uv.log" 2>&1; then
  echo "ok (uv tool)"
else
  echo "uv tool install failed; cloning source"
  if [ ! -d "$SRC/hermes-agent" ]; then
    git clone --depth 1 https://github.com/NousResearch/hermes-agent "$SRC/hermes-agent" >"$HERE/logs/hermes-git.log" 2>&1
  fi
  (cd "$SRC/hermes-agent" && uv sync >"$HERE/logs/hermes-sync.log" 2>&1) && echo "ok (source, run via: uv run --project $SRC/hermes-agent hermes)" || echo "FAILED (see logs)"
fi

# ---- isolated homes ----------------------------------------------------------
for h in claude codex opencode dsh hermes openclaw cursor; do mkdir -p "$HOME_ROOT/$h"; done

# ---- env file ----------------------------------------------------------------
cat >"$HERE/env.sh" <<EOF
# source this before running any harness for spikes
export GOALCUP_HARNESSES="$HERE"
export PATH="$NODE_PREFIX/bin:$HERE/uv-bin:\$PATH"
# isolated homes (launch pins). Claude Desktop cannot be isolated; see spikes/README.md
export CODEX_HOME="$HOME_ROOT/codex"
export DSH_HOME="$HOME_ROOT/dsh"
export HERMES_HOME="$HOME_ROOT/hermes"
export OPENCLAW_STATE_DIR="$HOME_ROOT/openclaw"      # verify exact var in spike 8
export XDG_DATA_HOME="$HOME_ROOT/opencode/data"      # OpenCode stores under \$XDG_DATA_HOME/opencode
export XDG_CONFIG_HOME="$HOME_ROOT/opencode/config"
# Claude Code CLI: use CLAUDE_CONFIG_DIR only in launch mode, per spike
export GOALCUP_CLAUDE_CONFIG_DIR="$HOME_ROOT/claude"
EOF
log "done. source $HERE/env.sh"
ls "$NODE_PREFIX/bin" 2>/dev/null || true
