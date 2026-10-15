#!/bin/sh
# A chat-mode stand-in for a shipped harness (Codex CLI or Claude Code) in the harness contract
# kit (`tests/full/harness_contract_kit.rs`). The kit installs it under the shipped program's
# name with `#!/usr/bin/env node` in place of this first line, beside a stand-in `node`, as the
# npm harnesses are installed; its report directory is written into it as KIT_OUT.
#
# It reads lines from its terminal and appends each to a session file where the shipped
# connector's native source looks: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`
# or `$CLAUDE_CONFIG_DIR/projects/<project>/<id>.jsonl`, created at the first line, as the
# measured harnesses do. `/exit` ends it. It reports its argv, environment and stdin kind, and
# what the goal-cup skill folder held at its start; every byte it prints starts `kit:`.
out="$KIT_OUT"
printf '%s\n' "$@" > "$out/argv"
env > "$out/env"
if [ -t 0 ]; then echo tty > "$out/stdin"; else echo not-a-tty > "$out/stdin"; fi
id="0199aaaa-bbbb-7ccc-8ddd-$(printf '%012d' $$)"
stamp=$(date -u +%Y-%m-%dT%H-%M-%S)
if [ -n "${CODEX_HOME:-}" ]; then
  home=$CODEX_HOME
  dir="$CODEX_HOME/sessions/$(date +%Y/%m/%d)"
  file="$dir/rollout-$stamp-$id.jsonl"
else
  home=$CLAUDE_CONFIG_DIR
  dir="$CLAUDE_CONFIG_DIR/projects/-kit-workspace"
  file="$dir/$id.jsonl"
fi
if [ -d "$home/skills/goal-cup" ]; then ls -A "$home/skills/goal-cup" > "$out/skills"; else echo absent > "$out/skills"; fi
record() {
  if [ -n "${CODEX_HOME:-}" ]; then
    printf '{"timestamp":"%s","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"%s"}]}}\n' "$stamp" "$1"
  else
    printf '{"type":"user","sessionId":"%s","message":{"role":"user","content":"%s"}}\n' "$id" "$1"
  fi
}
echo "kit: ready"
while IFS= read -r line; do
  if [ "$line" = /exit ]; then
    echo "kit: bye"
    exit 0
  fi
  mkdir -p "$dir"
  record "$line" >> "$file"
  echo "kit: noted $line"
done
