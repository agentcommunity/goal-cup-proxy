#!/bin/sh
# A chat-mode stand-in for any shipped chat-mode connector in the harness contract kit
# (`tests/it/harness_contract_kit.rs`). The kit installs it under the connector's program
# name with `#!/usr/bin/env node` in place of this first line, beside a stand-in `node`, as the
# npm harnesses are installed. The kit writes three variables into it, all derived from the
# connector's `harness.toml`, so this file knows no harness:
#   KIT_OUT          its report directory;
#   KIT_NATIVE_FILE  the session file, where the `[[capture.native]]` source looks: the root,
#                    then the pattern with each `*` (and `{id}`) as `kit`;
#   KIT_SKILLS_DIR   where the goal-cup skill should be, or empty when the connector has none.
#
# It reads lines from its terminal and appends each to KIT_NATIVE_FILE as one JSON line,
# creating the file at the first line, as the measured harnesses do. `/exit` ends it. It
# reports its argv, environment and stdin kind, and what the goal-cup skill folder held at its
# start; every byte it prints starts `kit:`.
out="$KIT_OUT"
# One argument per line; no arguments is an empty file (printf would print one empty line).
: > "$out/argv"
if [ $# -gt 0 ]; then printf '%s\n' "$@" > "$out/argv"; fi
env > "$out/env"
if [ -t 0 ]; then echo tty > "$out/stdin"; else echo not-a-tty > "$out/stdin"; fi
if [ -n "$KIT_SKILLS_DIR" ] && [ -d "$KIT_SKILLS_DIR" ]; then
  ls -A "$KIT_SKILLS_DIR" > "$out/skills"
else
  echo absent > "$out/skills"
fi
echo "kit: ready"
while IFS= read -r line; do
  if [ "$line" = /exit ]; then
    echo "kit: bye"
    exit 0
  fi
  mkdir -p "$(dirname "$KIT_NATIVE_FILE")"
  printf '{"type":"user","text":"%s"}\n' "$line" >> "$KIT_NATIVE_FILE"
  echo "kit: noted $line"
done
