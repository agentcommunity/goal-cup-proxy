#!/usr/bin/env bash
# Watcher command: goalcup time-left in the participant's shell, then as sh -lc with the harness's environment.
# PRD-10 acceptance 7 (PRD-12 section 3.1).
set -eu
command -v goalcup >/dev/null || { echo "goalcup is not on PATH"; exit 127; }
echo "GOALCUP_TIME_LEFT_FILE ${GOALCUP_TIME_LEFT_FILE:+set}${GOALCUP_TIME_LEFT_FILE:-not set}"
a=0 b=0
echo "participant shell:"; env -u HTTPS_PROXY -u NO_PROXY -u SSL_CERT_FILE -u NODE_EXTRA_CA_CERTS -u GOALCUP_TIME_LEFT_FILE goalcup time-left 2>&1 || a=$?
echo "exit $a"
echo "harness environment:"; sh -lc 'goalcup time-left' 2>&1 || b=$?
echo "exit $b"
exit $((a ? a : b))
