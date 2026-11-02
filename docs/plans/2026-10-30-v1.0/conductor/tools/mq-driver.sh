#!/bin/bash
# Conductor-run merge queue driver: runs mergeq.sh back to back for up to $1 hours.
end=$(( $(date +%s) + ${1:-6}*3600 ))
while [ "$(date +%s)" -lt "$end" ]; do timeout 600 /root/gc-tools/mergeq.sh 540 >/dev/null 2>&1; done
