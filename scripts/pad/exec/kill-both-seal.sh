#!/usr/bin/env bash
# Rung 5b (PRD-12 section 1): kill -9 the watchdog and the recorder together while the harness runs, then plain
# goalcup status and goalcup seal --yes in the participant's shell (the dead-entry path: close_dead_entry, then
# the uploader). The harness is never touched. Exits non-zero when the kill did not take or the seal failed.
set -u
eval "set -- $PAD_SHELL_ENV"  # HOME, PATH, TERM and the run's GOALCUP_* values
e=$PAD_ENTRY; st=$e/status.json
read -r wd rec hp < <(python3.11 -I -c 'import json,re,sys
s=json.load(open(sys.argv[1])); w=re.findall("[0-9]+",open(sys.argv[2]).read())
print(w[0] if w else "", s["pid"], s["harness"]["pid"])' "$st" "$e/rec/watchdog.pid" 2>/dev/null)
if [ -z "${wd:-}" ] || [ -z "${rec:-}" ]; then echo "kill did not take: no watchdog or recorder pid"; exit 3; fi
kill -9 "$wd" "$rec" || { echo "kill did not take: kill exit $?"; exit 3; }
echo "killed watchdog $wd recorder $rec at $(date -u +%T.%3N)"
sleep 4
echo "--- goalcup status"; env -i "$@" goalcup status 2>&1
echo "--- goalcup seal --yes"; env -i "$@" goalcup seal --yes 2>&1; c=$?; echo "seal exit $c"
echo "harness alive: $(kill -0 "$hp" 2>/dev/null && echo yes || echo no)"
exit $c
