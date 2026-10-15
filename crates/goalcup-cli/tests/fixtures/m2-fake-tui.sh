#!/bin/sh
# The m2-fake-tui connector's capture executable (tests/m2_cli_support, `Rig::chat`). It stands
# in for a harness in chat mode (D57): no goal in its argv, its stdin and stdout are the
# participant's terminal, and the participant types the prompt.
#
# It reads lines from its terminal. The first line it reads creates a new session file under
# its config home (`$CODEX_HOME/sessions/`, the connector's native root), as a measured harness
# does when the first message is submitted, and every line is appended to that file as one JSON
# record. `/exit` ends it with status 0 and writes no session file of its own.
#
# What it does is reported under $M2_FAKE_OUT: `tui.pid`, `tui.stdin` (`tty` or `not-a-tty`),
# `tui.argv`, `tui.skills` (what the goal-cup skill folder held at its start), `tui.ready`, and
# `tui.end` (`int`, `hup`, `exit` or `eof`: how it ended). Every byte it prints starts `tui:`, so
# a test can tell its output from anything else on the terminal.
#
# SIGINT and SIGHUP are trapped. A non-interactive shell cannot trap a signal that was ignored
# when it started, so a launcher that handed it SIG_IGN would leave it running here, and the test
# that sends the signal would time out.
out="$M2_FAKE_OUT"
home="$CODEX_HOME"
echo $$ > "$out/tui.pid.tmp" && mv "$out/tui.pid.tmp" "$out/tui.pid"
printf '%s\n' "$@" > "$out/tui.argv"
if [ -t 0 ]; then
  echo tty > "$out/tui.stdin"
else
  echo not-a-tty > "$out/tui.stdin"
fi
if [ -d "$home/skills/goal-cup" ]; then
  ls -A "$home/skills/goal-cup" > "$out/tui.skills"
else
  echo absent > "$out/tui.skills"
fi
trap 'echo "tui: interrupted"; echo int > "$out/tui.end"; exit 130' INT
trap 'echo hup > "$out/tui.end"; exit 129' HUP
echo "tui: ready"
echo ready > "$out/tui.ready"
session=""
while IFS= read -r line; do
  case "$line" in
    /exit)
      echo "tui: bye"
      echo exit > "$out/tui.end"
      exit 0
      ;;
  esac
  if [ -z "$session" ]; then
    mkdir -p "$home/sessions"
    session="$home/sessions/session-$(date +%s)-$$.jsonl"
  fi
  printf '{"type":"user","text":"%s"}\n' "$line" >> "$session"
  echo "tui: noted $line"
done
echo eof > "$out/tui.end"
