#!/bin/sh
# The m2-fake-tui connector's capture executable (tests/m2_cli_support, `Rig::chat`). It stands
# in for a harness in chat mode (D57): no goal in its argv, its stdin and stdout are the
# participant's terminal, and the participant types the prompt.
#
# It reads lines from its terminal. The first line it reads creates a new session file under
# its config home (`$CODEX_HOME/sessions/`, the connector's native root), as a measured harness
# does when the first message is submitted, and every line is appended to that file as one JSON
# record. `/exit` ends it with status 0 and writes no session file of its own. `/suspend` stops
# it (`kill -TSTP $$`) as a TUI's own Ctrl-Z handling does, and `/suspend-group` stops its whole
# process group; each prints `tui: resumed` once it is continued.
#
# What it does is reported under $M2_FAKE_OUT: `tui.pid`, `tui.stdin` (`tty` or `not-a-tty`),
# `tui.argv`, `tui.env` (its whole environment), `tui.home` (its HOME, where it also appends a
# line to `tui-runs.txt` at each start), `tui.skills` (what the goal-cup skill folder held at its start), `tui.ready`, and
# `tui.end` (`int`, `hup`, `exit` or `eof`: how it ended). Every byte it prints starts `tui:`, so
# a test can tell its output from anything else on the terminal.
#
# SIGINT, SIGQUIT and SIGHUP are trapped. A non-interactive shell cannot trap a signal that was
# ignored when it started, so a launcher that handed it SIG_IGN would leave it running here, and
# the test that sends the signal would time out. Each ends it, unless the test wrote
# $M2_FAKE_OUT/tui.endure: then it prints `tui: endured <sig>` and reads on.
out="$M2_FAKE_OUT"
# A connector with a shared run home (D59) has no home variable: its home is HOME.
home="${CODEX_HOME:-$HOME}"
echo "$HOME" > "$out/tui.home"
echo run >> "$HOME/tui-runs.txt"
echo $$ > "$out/tui.pid.tmp" && mv "$out/tui.pid.tmp" "$out/tui.pid"
printf '%s\n' "$@" > "$out/tui.argv"
env > "$out/tui.env"
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
endured=""
on_signal() {
  if [ -f "$out/tui.endure" ]; then
    endured=1
    echo "tui: endured $1"
    echo "$1" >> "$out/tui.endured"
    return
  fi
  [ "$1" = int ] && echo "tui: interrupted"
  echo "$1" > "$out/tui.end"
  exit "$2"
}
trap 'on_signal int 130' INT
trap 'on_signal quit 131' QUIT
trap 'on_signal hup 129' HUP
echo "tui: ready"
echo ready > "$out/tui.ready"
session=""
while :; do
  if ! IFS= read -r line; then
    # A trapped signal interrupts the read; only an end of input ends the loop.
    if [ -n "$endured" ]; then
      endured=""
      continue
    fi
    break
  fi
  case "$line" in
    /exit)
      echo "tui: bye"
      echo exit > "$out/tui.end"
      exit 0
      ;;
    /suspend)
      kill -TSTP $$
      echo "tui: resumed"
      continue
      ;;
    /suspend-group)
      kill -TSTP 0
      echo "tui: resumed"
      continue
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
