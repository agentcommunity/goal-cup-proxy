#!/bin/sh
# The m2-fake-hermes connector's capture executable (tests/m2_cli_support). It is launched the
# way the Hermes recipe launches the real one: `chat -Q ... --query=<goal>`, its environment built
# from nothing plus the one provider key the launcher hands the child, stdin /dev/null.
#
# It records what it was started with under $M2_FAKE_OUT (pid, argv one per line, environment,
# `started`), runs $M2_FAKE_OUT/steps.sh when the test wrote one, then behaves like a Hermes
# turn: it writes its own session snapshot under $HERMES_HOME/sessions, prints its answer on
# stdout and `session_id: <id>` on stderr, and exits. The test controls the ending with files in
# $M2_FAKE_OUT: `no-snapshot` (steps.sh wrote its own), `no-marker` (print no identity).
#
# The key it is given is synthetic and test-owned; the environment it records holds it, in a
# directory outside the run.
out="$M2_FAKE_OUT"
echo $$ > "$out/pid.tmp" && mv "$out/pid.tmp" "$out/pid"
printf '%s\n' "$@" > "$out/argv"
env > "$out/env"
echo started > "$out/started"
sid="${M2_FAKE_SESSION:-$(printf '20261002_120000_%06x' $$)}"
echo "$sid" > "$out/session-id"
snap="$HERMES_HOME/sessions/session_$sid.json"
mkdir -p "$HERMES_HOME/sessions"
if [ -f "$out/steps.sh" ]; then
  . "$out/steps.sh"
fi
if [ ! -f "$out/no-snapshot" ]; then
  printf '{"session_id":"%s","model":"deepseek-v4-flash","messages":[{"role":"user","content":"hi"}]}' "$sid" > "$snap.tmp" && mv "$snap.tmp" "$snap"
fi
echo "the answer"
if [ ! -f "$out/no-marker" ]; then
  echo "session_id: $sid" >&2
fi
exit 0
