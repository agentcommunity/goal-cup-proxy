#!/usr/bin/env bash
# Watcher command: a client that trusts only the system CAs (not the run CA) calls a listed host twice, then after 11 minutes.
# PRD-03 acceptance 7 (PRD-12 section 3.1); the record, not this script, decides the pass.
set -eu
ca=/etc/ssl/certs/ca-certificates.crt
[ -f "$ca" ] || ca=/etc/pki/tls/certs/ca-bundle.crt  # Amazon Linux names the system bundle this way
echo "system CA file $ca"
call() {
  rc=0
  out=$(curl -sS --cacert "$ca" -o /dev/null -w 'status %{http_code}' https://api.openai.com/v1/models 2>&1) || rc=$?
  echo "$(date -u +%H:%M:%S)Z call $1: curl exit $rc, $out"
}
call 1
call 2
sleep 660
call 3
