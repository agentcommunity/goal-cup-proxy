#!/bin/sh
# The m2-fake connector's capture executable (tests/m2_cli_support). It is launched the way a
# chat-mode harness is (D57), in a test build that runs it with no terminal: no goal in its
# argv, its environment built from nothing but the terminal pass-through, stdin /dev/null, and
# it stays in the launcher's process group.
#
# It records what it was started with under $M2_FAKE_OUT: its pid, its argv (one argument per
# line), its whole environment and what stdin held. It then says whether the recording proxy
# HTTPS_PROXY names was already accepting connections, runs $M2_FAKE_OUT/steps.sh when the
# test wrote one, and ends there when the test-owned file $M2_FAKE_OUT/stop exists. Otherwise
# it waits on one `sleep` (its pid in $M2_FAKE_OUT/sleeper) and expects the tracker to end it;
# forking nothing more while it waits keeps the tree the tracker ends exactly two processes.
out="$M2_FAKE_OUT"
exec 2>>"$out/stderr"
echo $$ > "$out/pid.tmp" && mv "$out/pid.tmp" "$out/pid"
printf '%s\n' "$@" > "$out/argv"
env > "$out/env"
if IFS= read -r line; then
  printf 'read: %s\n' "$line" > "$out/stdin"
else
  echo eof > "$out/stdin"
fi
hostport=${HTTPS_PROXY##*@}
if nc -z 127.0.0.1 "${hostport##*:}" 2>/dev/null; then
  echo listening > "$out/proxy"
else
  echo absent > "$out/proxy"
fi
echo started > "$out/started"
if [ -f "$out/steps.sh" ]; then
  . "$out/steps.sh"
fi
if [ ! -f "$out/stop" ]; then
  sleep 60 &
  echo $! > "$out/sleeper"
  wait $!
fi
[ -f "$out/stop" ] && echo done > "$out/done"
