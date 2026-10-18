#!/bin/sh
# The one stand-in of the connector contract's dynamic half (`tests/full/connector_contract.rs`).
# It stands in for any connector's harness and knows none: the runner installs it under the
# connector's program name (for a chat-mode connector with `#!/usr/bin/env node` in place of
# this first line, beside a stand-in `node`, as the npm harnesses are installed), and writes
# these variables into it, all derived from the connector's `harness.toml`:
#   KIT_OUT           its report directory, outside the run;
#   KIT_MODE          `chat` (interactive) or `headless`;
#   KIT_VERSION_ARGV  the connector's `version_args`, joined by spaces (empty: none);
#   KIT_NATIVE_FILE   the session file its first `[[capture.native]]` source finds: the root,
#                     then the pattern with each `*` as `kit`, each `?` as `k` and `{id}` as
#                     the identity below;
#   KIT_SKILLS_DIR    where the goal-cup skill should be, or empty when the connector has none;
#   KIT_CA_VARS       the connector's `capture.ca.env` names, space-separated;
#   KIT_ID_STREAM     `stdout`, `stderr` or empty: where the identity line goes;
#   KIT_ID_LINE       the identity line in the connector's own grammar, or empty;
#   KIT_SESSION_ARG   preassigned identity's 1-based argument position, or 0;
#   KIT_SESSION_PREFIX text before the id in that argument (for --session-id=<id>, for example);
#   KIT_CONFIGURE     the connector's `capture.configure` commands, one per line, each argument
#                     list joined by spaces (empty: none).
#
# Called with the arguments of one of KIT_CONFIGURE's commands, it appends that line and its
# stdin kind to `configure`, writes its environment to `configure.env`, prints one
# `kit: configured` line and exits 0, writing none of the reports below.
#
# It reports its argv (one per line), environment, stdin kind, what the goal-cup skill folder
# held at its start, and a copy of the file each CA variable names; every byte it prints starts
# `kit:`. In chat mode it reads lines from its terminal and appends each to KIT_NATIVE_FILE as
# one JSON line, creating the file at the first line, as the measured harnesses do; `/exit` ends
# it. Headless, it writes its last argument (the goal's element) as the session file, prints
# its identity line, and exits.
out="$KIT_OUT"
if [ -n "$KIT_VERSION_ARGV" ] && [ "$*" = "$KIT_VERSION_ARGV" ]; then
  echo "kit 1.0.0"
  exit 0
fi
if [ -n "$KIT_CONFIGURE" ] && printf '%s\n' "$KIT_CONFIGURE" | grep -Fxq -- "$*"; then
  if [ -t 0 ]; then stdin=tty; else stdin=not-a-tty; fi
  echo "$* $stdin" >> "$out/configure"
  env > "$out/configure.env"
  echo "kit: configured"
  exit 0
fi
# `goalcup start` mints a 36-byte UUID. Read it from the connector's declared argv slot,
# preserving argv, and use it in the native path rather than inventing a different id.
if [ "$KIT_SESSION_ARG" -gt 0 ]; then
  index=0
  for arg in "$@"; do
    index=$((index + 1))
    if [ "$index" -eq "$KIT_SESSION_ARG" ]; then
      value=${arg#"$KIT_SESSION_PREFIX"}
      session=$(printf '%s' "$value" | cut -c 1-36)
      KIT_NATIVE_FILE="${KIT_NATIVE_FILE%%\{id\}*}$session${KIT_NATIVE_FILE#*\{id\}}"
      break
    fi
  done
fi
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
for var in $KIT_CA_VARS; do
  eval "file=\${$var:-}"
  if [ -n "$file" ] && [ -f "$file" ]; then cp "$file" "$out/ca-$var"; fi
done
identity() {
  case $KIT_ID_STREAM in
    stdout) printf '%s\n' "$KIT_ID_LINE" ;;
    stderr) printf '%s\n' "$KIT_ID_LINE" >&2 ;;
  esac
}
mkdir -p "$(dirname "$KIT_NATIVE_FILE")"
if [ "$KIT_MODE" = headless ]; then
  identity
  for last in "$@"; do :; done
  printf '{"type":"user","text":"%s"}\n' "$last" > "$KIT_NATIVE_FILE.tmp"
  mv "$KIT_NATIVE_FILE.tmp" "$KIT_NATIVE_FILE"
  echo "kit: done"
  exit 0
fi
echo "kit: ready"
while IFS= read -r line; do
  if [ "$line" = /exit ]; then
    echo "kit: bye"
    exit 0
  fi
  printf '{"type":"user","text":"%s"}\n' "$line" >> "$KIT_NATIVE_FILE"
  echo "kit: noted $line"
done
