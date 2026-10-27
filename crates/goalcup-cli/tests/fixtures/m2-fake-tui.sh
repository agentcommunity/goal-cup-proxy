#!/bin/sh
# The m2-fake-tui connector's capture executable (tests/m2_cli_support, `Rig::chat`). It stands
# in for a harness in chat mode (D57): no goal in its argv, its stdin and stdout are the
# participant's terminal, and the participant types the prompt.
#
# It reads lines from its terminal. The first line it reads creates a new session file under
# its config home (`$CODEX_HOME/sessions/`, the connector's native root), as a measured harness
# does when the first message is submitted, and every line is appended to that file as one JSON
# record. `/call` runs $M2_FAKE_OUT/tui.call.sh (a model request the test wrote) and writes no
# session file, then prints `tui: called`.
# `/exit` ends it with status 0 and writes no session file of its own. `/suspend` stops
# it (`kill -TSTP $$`) as a TUI's own Ctrl-Z handling does, and `/suspend-group` stops its whole
# process group; each prints `tui: resumed` once it is continued.
#
# What it does is reported under $M2_FAKE_OUT: `tui.pid`, `tui.stdin` (`tty` or `not-a-tty`),
# `tui.argv`, `tui.env` (its whole environment), `tui.ca.pem` (the proxy CA it was handed), `tui.home` (its HOME, where it also appends a
# line to `tui-runs.txt` at each start), `tui.skills` (what the goal-cup skill folder held at its start), `tui.ready`, and
# `tui.end` (`int`, `hup`, `exit` or `eof`: how it ended). Every byte it prints starts `tui:`, so
# a test can tell its output from anything else on the terminal.
#
# SIGINT, SIGQUIT and SIGHUP are trapped. A non-interactive shell cannot trap a signal that was
# ignored when it started, so a launcher that handed it SIG_IGN would leave it running here, and
# the test that sends the signal would time out. Each ends it, unless the test wrote
# $M2_FAKE_OUT/tui.endure: then it prints `tui: endured <sig>` and reads on, and prints
# `tui: waiting` once ready and after each.
out="$M2_FAKE_OUT"
# A connector with a shared run home (D59) has no home variable: its home is HOME.
home="${CODEX_HOME:-$HOME}"
# A connector's `capture.configure` command (`configure <key> <value>`, D61): the setting goes
# into `$home/settings.txt`, one line per run of it, and what it was run with goes to
# `tui.configure` (its argv, whether stdin is a terminal) and `tui.configure.env`; it prints one
# `tui: configured` line. With $M2_FAKE_OUT/tui.configure.fail it fails with status 3 instead.
if [ "$1" = configure ]; then
  if [ -t 0 ]; then stdin=tty; else stdin=not-a-tty; fi
  echo "$* $stdin" >> "$out/tui.configure"
  env > "$out/tui.configure.env"
  if [ -f "$out/tui.configure.fail" ]; then
    echo "tui: configure failed"
    exit 3
  fi
  mkdir -p "$home"
  echo "$2=$3" >> "$home/settings.txt"
  echo "tui: configured $2"
  exit 0
fi
# A connector's `[launch_prep] after_exit` command (`gateway stop -y`, D72): it appends one line
# to `tui.after_exit`: its argv, whether the harness it follows still runs (`tui.pid`), whether
# the recording proxy the harness was handed (`tui.env`) still accepts connections, and whether
# a proxy is in its own environment.
if [ "$1" = gateway ]; then
  pid=$(cat "$out/tui.pid" 2>/dev/null)
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then harness=running; else harness=gone; fi
  proxy=$(sed -n 's/^HTTPS_PROXY=//p' "$out/tui.env")
  hostport=${proxy##*@}
  hostport=${hostport%/}
  if [ -n "$proxy" ] && nc -z 127.0.0.1 "${hostport##*:}" 2>/dev/null; then
    listening=proxy-open
  else
    listening=proxy-closed
  fi
  if [ -n "$HTTPS_PROXY" ]; then own=with-proxy; else own=no-proxy; fi
  echo "$* $harness $listening $own" >> "$out/tui.after_exit"
  echo "tui: gateway stopped"
  exit 0
fi
echo "$HOME" > "$out/tui.home"
echo run >> "$HOME/tui-runs.txt"
echo $$ > "$out/tui.pid.tmp" && mv "$out/tui.pid.tmp" "$out/tui.pid"
printf '%s\n' "$@" > "$out/tui.argv"
env > "$out/tui.env"
# The proxy CA the run hands it (its name constraints are the run's approved hosts).
if [ -n "$CODEX_CA_CERTIFICATE" ]; then cp "$CODEX_CA_CERTIFICATE" "$out/tui.ca.pem"; fi
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
    echo "tui: waiting"
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
# In endure mode it says when it is about to wait for the next signal (here, and when a handler
# is done). The line comes just before the read starts, so it does not prove the read has begun:
# a signal sent in that gap is held until the read returns. A test that sends signals therefore
# sends again when no answer comes (`it/start_launch.rs`).
[ -f "$out/tui.endure" ] && echo "tui: waiting"
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
    /call)
      [ -f "$out/tui.call.sh" ] && . "$out/tui.call.sh"
      echo "tui: called"
      continue
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
