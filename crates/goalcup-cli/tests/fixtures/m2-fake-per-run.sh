#!/bin/sh
# The m2-fake-per-run connector's capture executable (tests/common/m2_cli_support): the fixture
# connector `tests/fixtures/connectors/chat-per-run` under its own id. It is launched the way
# that connector says: `chat -Q`, its environment built from nothing.
#
# It records what it was started with under $M2_FAKE_OUT (pid, argv one per line, environment,
# the file its CA variable names, `started`), runs $M2_FAKE_OUT/steps.sh when the test wrote one,
# then ends its turn: it writes its own session snapshot under $PERRUN_HOME/sessions, prints its
# answer on stdout, and exits. The test controls the ending with the file `no-snapshot` in
# $M2_FAKE_OUT (steps.sh wrote its own).
out="$M2_FAKE_OUT"
echo $$ > "$out/pid.tmp" && mv "$out/pid.tmp" "$out/pid"
printf '%s\n' "$@" > "$out/argv"
env > "$out/env"
if [ -n "${PERRUN_CA_BUNDLE:-}" ]; then cp "$PERRUN_CA_BUNDLE" "$out/ca.pem"; fi
echo started > "$out/started"
sid="${M2_FAKE_SESSION:-$(printf '20261002_120000_%06x' $$)}"
echo "$sid" > "$out/session-id"
snap="$PERRUN_HOME/sessions/session_$sid.json"
mkdir -p "$PERRUN_HOME/sessions"
if [ -f "$out/steps.sh" ]; then
  . "$out/steps.sh"
fi
if [ ! -f "$out/no-snapshot" ]; then
  printf '{"session_id":"%s","model":"a-model","messages":[{"role":"user","content":"hi"}]}' "$sid" > "$snap.tmp" && mv "$snap.tmp" "$snap"
fi
echo "the answer"
exit 0
