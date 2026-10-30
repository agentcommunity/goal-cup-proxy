#!/usr/bin/env bash
# Watcher command: goalcup time-left in the participant's shell, then as sh -lc with the harness's environment
# (its HOME, PATH and GOALCUP_TIME_LEFT_FILE, which pad-run passes from /proc/<pid>/environ). PRD-10 acceptance 7
# (PRD-12 section 3.1).
set -eu
echo "GOALCUP_TIME_LEFT_FILE ${GOALCUP_TIME_LEFT_FILE:+set}${GOALCUP_TIME_LEFT_FILE:-not set in the harness}"
eval "set -- $PAD_SHELL_ENV"  # the participant's shell: HOME, PATH, TERM and the run's GOALCUP_* values
a=0 b=0
echo "participant shell:"; env -i "$@" goalcup time-left 2>&1 || a=$?
echo "exit $a"
echo "harness environment (HOME $HOME):"; sh -lc 'goalcup time-left' 2>&1 || b=$?
echo "exit $b"
exit $((a ? a : b))
