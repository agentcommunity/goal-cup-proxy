#!/usr/bin/env bash
# Rung 5b (PRD-12 section 1): kill -9 the watchdog and the recorder together while the harness runs, then
# plain goalcup status and goalcup seal --yes in the participant's shell (the dead-entry path: close_dead_entry, then
# the uploader). The harness is never touched.
set -u
eval "set -- $PAD_SHELL_ENV"  # HOME, PATH, TERM and the run's GOALCUP_* values
H=$(env -i "$@" sh -c 'printf %s "$GOALCUP_HOME"')
st=$(ls -t "$H"/runs/*/e*/status.json | head -1); e=$(dirname "$st")
wd=$(grep -o '[0-9]*' "$e/rec/watchdog.pid" | head -1)
rec=$(python3.11 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["pid"])' "$st")
hp=$(python3.11 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["harness"]["pid"])' "$st")
echo "entry $(basename "$e") watchdog $wd recorder $rec harness $hp at $(date -u +%T.%3N)"
kill -9 "$wd" "$rec"; echo "kill exit $?"
sleep 4
echo "harness alive: $(kill -0 "$hp" 2>/dev/null && echo yes || echo no)"
echo "--- goalcup status"; env -i "$@" goalcup status 2>&1; echo "status exit $?"
echo "--- goalcup seal --yes at $(date -u +%T.%3N)"; env -i "$@" goalcup seal --yes 2>&1; c=$?; echo "seal exit $c at $(date -u +%T.%3N)"
echo "--- goalcup status"; env -i "$@" goalcup status 2>&1; echo "status exit $?"
echo "harness alive: $(kill -0 "$hp" 2>/dev/null && echo yes || echo no)"
exit $c
