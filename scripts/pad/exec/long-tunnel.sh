#!/usr/bin/env bash
# Watcher command: one long-lived stream through the proxy for 90 s to a host that is not a model host.
# PRD-03 acceptance 9 (PRD-12 section 3.1).
set -eu
echo "start $(date -u +%H:%M:%S)Z"
rc=0
out=$(curl -N -sS --max-time 90 -o /dev/null -w 'bytes %{size_download}' https://stream.wikimedia.org/v2/stream/recentchange 2>&1) || rc=$?
echo "end $(date -u +%H:%M:%S)Z, curl exit $rc, $out"
[ "$rc" = 0 ] || [ "$rc" = 28 ]  # 28: stopped at the 90 s limit, as meant
