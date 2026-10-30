#!/usr/bin/env bash
# Watcher command: four HTTP/2-only requests at once on one connection to a listed host, through the proxy.
# PRD-03 acceptance 3 (PRD-12 section 3.1); curl 8.10.0 and later offers only h2 in ALPN with prior knowledge.
set -eu
u=https://api.openai.com/v1/models
ca=${SSL_CERT_FILE:-${NODE_EXTRA_CA_CERTS:-}}
err=$(mktemp)
curl --version | head -n 1
rc=0
curl -sS -v --http2-prior-knowledge --parallel --parallel-max 4 ${ca:+--cacert "$ca"} -w 'status %{http_code} protocol %{http_version}\n' \
  -o /dev/null -o /dev/null -o /dev/null -o /dev/null "$u" "$u" "$u" "$u" 2>"$err" || rc=$?
grep -i -E 'alpn|^curl:' "$err" || true
rm -f "$err"
echo "curl exit $rc"
exit "$rc"
