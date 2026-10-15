#!/bin/sh
# The m2-fake-keyed connector's capture executable (tests/common/m2_cli_support): the keyed
# fixture connector `tests/fixtures/connectors/keyed-per-run` under its own id. It is launched the
# way that connector says: `chat -Q --query=<goal>`, its environment built from nothing plus the
# one provider key the launcher hands the child, stdin /dev/null.
#
# It records what it was started with under $M2_FAKE_OUT (pid, argv one per line, environment,
# the file its CA variable names, `started`), runs $M2_FAKE_OUT/steps.sh when the test wrote one,
# then ends its turn: it writes its own session snapshot under $KEYED_HOME/sessions, prints its
# answer on stdout and `session_id: <id>` on stderr, and exits. The test controls the ending with
# files in $M2_FAKE_OUT: `no-snapshot` (steps.sh wrote its own), `no-marker` (print no identity).
#
# The key it is given is synthetic and test-owned; the environment it records holds it, in a
# directory outside the run.
out="$M2_FAKE_OUT"
echo $$ > "$out/pid.tmp" && mv "$out/pid.tmp" "$out/pid"
printf '%s\n' "$@" > "$out/argv"
env > "$out/env"
if [ -n "${KEYED_CA_BUNDLE:-}" ]; then cp "$KEYED_CA_BUNDLE" "$out/ca.pem"; fi
echo started > "$out/started"
sid="${M2_FAKE_SESSION:-$(printf '20261002_120000_%06x' $$)}"
echo "$sid" > "$out/session-id"
snap="$KEYED_HOME/sessions/session_$sid.json"
mkdir -p "$KEYED_HOME/sessions"
if [ -f "$out/steps.sh" ]; then
  . "$out/steps.sh"
fi
if [ ! -f "$out/no-snapshot" ]; then
  printf '{"session_id":"%s","model":"a-model","messages":[{"role":"user","content":"hi"}]}' "$sid" > "$snap.tmp" && mv "$snap.tmp" "$snap"
fi
echo "the answer"
if [ ! -f "$out/no-marker" ]; then
  echo "session_id: $sid" >&2
fi
exit 0
